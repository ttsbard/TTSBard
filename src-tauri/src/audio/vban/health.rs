//! VBAN receiver availability monitor (ROADMAP-139).
//!
//! While VBAN output is enabled, a dedicated thread periodically sends a
//! SERVICE_IDENTIFICATION request (PING0, full 704-byte packet) to the saved
//! destination and listens for the reply. The monitor runs independently of
//! any UI screen and of the audio worker: it never blocks the playback path
//! and never stops the sound.
//!
//! Wire facts verified against Receptor 1.0.0.5 (experiment
//! `.work/ai/vban-ping-20261116/report.md`) and the official specification
//! (revision 13, SERVICE Identification, pages 27–30):
//! - The receiver answers only the full 704-byte packet (28-byte header plus
//!   the 676-byte PING0 structure), never a header-only probe.
//! - The reply is sent to the *sender's* UDP [`SERVICE_REPLY_PORT`] no matter
//!   which local port the request used, so the monitor owns that fixed port.
//! - The reply's `nuFrame` is always 0: the documented request-ID echo is not
//!   implemented, so replies are correlated by freshness (a recv window right
//!   after the send, stale datagrams drained before it) plus endpoint, service
//!   type and packet size — never by request ID.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::protocol;

/// The receiver sends its reply to the sender's UDP 6980 regardless of the
/// port the request came from, so the monitor must bind this fixed port.
/// A conflict is an explicit local error, never shared unsafely.
pub(crate) const SERVICE_REPLY_PORT: u16 = 6980;

/// Full PING0 packet: 28-byte VBAN header + 676-byte PING0 structure.
pub(crate) const PING0_PACKET_SIZE: usize = 704;

/// Interval under 10 seconds: an error is established after 3 failures.
const FAST_INTERVAL_SECS: u32 = 10;
const FAST_THRESHOLD: u32 = 3;
/// Interval from 10 seconds inclusive: 2 failures are enough.
const SLOW_THRESHOLD: u32 = 2;

/// How long one probe waits for a reply. Internal value, separate from the
/// user-facing interval; probes never overlap because the loop is sequential.
const PROBE_REPLY_TIMEOUT: Duration = Duration::from_secs(1);

/// Global error toasts live for ~5 s; alerts from all sources (threshold
/// crossing, production send start) are suppressed inside this window so a
/// visible toast is never duplicated.
const ALERT_DEDUP_WINDOW: Duration = Duration::from_secs(5);

/// Label carried in the request's stream-name field (arbitrary per spec).
const REQUEST_LABEL: &str = "TTSBard PING";

/// Failure threshold for a check interval: 3 failures for fast intervals,
/// 2 from 10 seconds inclusive.
pub(crate) fn failure_threshold(interval_secs: u32) -> u32 {
    if interval_secs < FAST_INTERVAL_SECS {
        FAST_THRESHOLD
    } else {
        SLOW_THRESHOLD
    }
}

/// Consecutive-failure counter with the agreed threshold semantics.
struct FailureTracker {
    consecutive: u32,
    threshold: u32,
}

impl FailureTracker {
    fn new(threshold: u32) -> Self {
        Self {
            consecutive: 0,
            threshold: threshold.max(1),
        }
    }

    fn reset(&mut self) {
        self.consecutive = 0;
    }

    /// Re-band the tracker to a new check interval without losing the count
    /// of consecutive misses: the counter tracks the receiver, the threshold
    /// tracks the current interval band.
    fn set_threshold(&mut self, threshold: u32) {
        self.threshold = threshold.max(1);
    }

    /// `true` exactly when this failure reached the threshold (the transition
    /// point into `Failed`); until then the previous status is preserved.
    fn record_failure(&mut self) -> bool {
        self.consecutive = self.consecutive.saturating_add(1);
        self.consecutive >= self.threshold
    }
}

// ---------------------------------------------------------------------------
// PING0 wire format
// ---------------------------------------------------------------------------

/// Payload offsets (relative to the 676-byte PING0 structure, packed
/// little-endian) per the official specification, pages 29–30.
mod ping0_layout {
    pub const BIT_TYPE: usize = 0;
    pub const BIT_FEATURE: usize = 4;
    pub const VERSION: usize = 28;
    pub const APPLICATION_NAME: usize = 292;
    pub const APPLICATION_NAME_LEN: usize = 64;
}

/// Header byte 4: sub-protocol SERVICE (0b011 in the top bits), rate bits zero.
const SERVICE_PROTOCOL: u8 = 0x60;
/// Header byte 5 (spec rev13, pages 27–28, `format_nbs`): the SERVICE
/// function, PING0 = 0; bit 7 marks a reply.
const SERVICE_FUNCTION_PING0: u8 = 0;
const SERVICE_FUNCTION_REPLY: u8 = 0x80;
/// Header byte 6 (spec rev13, pages 27–28, `format_nbc`): the SERVICE type,
/// IDENTIFICATION = 0. Captured replies confirm the layout: `60 80 00 00`.
const SERVICE_TYPE_IDENTIFICATION: u8 = 0;

/// Encode a full SERVICE_IDENTIFICATION request (PING0). The request ID goes
/// to the header `nuFrame` (the documented echo field); the payload stays
/// zero: undefined fields must be ZERO and Receptor answers only the full
/// 704-byte packet.
pub(crate) fn encode_ping0_request(request_id: u32) -> [u8; PING0_PACKET_SIZE] {
    let mut packet = [0u8; PING0_PACKET_SIZE];
    packet[0..4].copy_from_slice(b"VBAN");
    packet[4] = SERVICE_PROTOCOL;
    // Byte 5 carries the SERVICE function (PING0), byte 6 the SERVICE type
    // (IDENTIFICATION). Both are zero; the payload stays zero: undefined
    // fields must be ZERO and Receptor answers only the full 704-byte packet.
    packet[5] = SERVICE_FUNCTION_PING0;
    packet[6] = SERVICE_TYPE_IDENTIFICATION;
    packet[7] = 0;
    let label = REQUEST_LABEL.as_bytes();
    let copied = label.len().min(16);
    packet[8..8 + copied].copy_from_slice(&label[..copied]);
    packet[24..28].copy_from_slice(&request_id.to_le_bytes());
    packet
}

/// Parsed fields of an accepted PING0 reply (diagnostics only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ping0Reply {
    pub request_id_echo: u32,
    pub device_type: u32,
    pub features: u32,
    pub version: [u8; 4],
    pub application_name: String,
}

/// Why a datagram was not accepted as a valid reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ping0Reject {
    /// Not the agreed packet size (Receptor answers only full 704 bytes).
    Size(usize),
    /// Not a VBAN SERVICE_IDENTIFICATION reply header.
    Header,
    /// The datagram came from a different endpoint than the ping target.
    Source,
}

fn ascii_field(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim_end().to_string()
}

/// Validate a datagram as a PING0 reply from `expected` and parse its
/// diagnostic fields. The request-ID echo is deliberately NOT required to
/// match: Receptor 1.0.0.5 always answers with 0.
pub(crate) fn decode_ping0_reply(
    buf: &[u8],
    expected: &SocketAddrV4,
    source: SocketAddr,
) -> Result<Ping0Reply, Ping0Reject> {
    if buf.len() != PING0_PACKET_SIZE {
        return Err(Ping0Reject::Size(buf.len()));
    }
    let header_ok = buf[0..4] == *b"VBAN"
        && buf[4] == SERVICE_PROTOCOL
        && buf[5] & SERVICE_FUNCTION_REPLY == SERVICE_FUNCTION_REPLY
        && buf[5] & !SERVICE_FUNCTION_REPLY == SERVICE_FUNCTION_PING0
        && buf[6] == SERVICE_TYPE_IDENTIFICATION
        && buf[7] == 0;
    if !header_ok {
        return Err(Ping0Reject::Header);
    }
    // Endpoint check: replies are expected from the ping target itself
    // (IP and port), which also drops unrelated VBAN chatter on the port.
    if source.ip() != std::net::IpAddr::V4(*expected.ip()) || source.port() != expected.port() {
        return Err(Ping0Reject::Source);
    }
    let payload = &buf[protocol::HEADER_SIZE..];
    let le32 = |offset: usize| {
        u32::from_le_bytes(
            payload[offset..offset + 4]
                .try_into()
                .expect("PING0 field is 4 bytes"),
        )
    };
    let app = &payload[ping0_layout::APPLICATION_NAME
        ..ping0_layout::APPLICATION_NAME + ping0_layout::APPLICATION_NAME_LEN];
    Ok(Ping0Reply {
        request_id_echo: u32::from_le_bytes(buf[24..28].try_into().expect("nuFrame is 4 bytes")),
        device_type: le32(ping0_layout::BIT_TYPE),
        features: le32(ping0_layout::BIT_FEATURE),
        version: payload[ping0_layout::VERSION..ping0_layout::VERSION + 4]
            .try_into()
            .expect("PING0 version is 4 bytes"),
        application_name: ascii_field(app),
    })
}

/// Build a reply packet in the exact shape captured from Receptor 1.0.0.5
/// (request-ID echo 0, "VBAN Service" label, type/features/version and the
/// application name filled in). Used by the loopback test responder.
#[cfg(test)]
pub(crate) fn build_receptor_style_reply(request_id_echo: u32) -> [u8; PING0_PACKET_SIZE] {
    let mut packet = [0u8; PING0_PACKET_SIZE];
    packet[0..4].copy_from_slice(b"VBAN");
    packet[4] = SERVICE_PROTOCOL;
    packet[5] = SERVICE_FUNCTION_REPLY | SERVICE_FUNCTION_PING0;
    packet[6] = SERVICE_TYPE_IDENTIFICATION;
    packet[7] = 0;
    let label = b"VBAN Service";
    packet[8..8 + label.len()].copy_from_slice(label);
    packet[24..28].copy_from_slice(&request_id_echo.to_le_bytes());
    let payload = &mut packet[protocol::HEADER_SIZE..];
    let mut set32 = |offset: usize, value: u32| {
        payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    };
    set32(ping0_layout::BIT_TYPE, 32);
    set32(ping0_layout::BIT_FEATURE, 66_305);
    payload[ping0_layout::VERSION..ping0_layout::VERSION + 4].copy_from_slice(&[1, 0, 0, 5]);
    let name = b"VBAN-Receptor";
    payload[ping0_layout::APPLICATION_NAME..ping0_layout::APPLICATION_NAME + name.len()]
        .copy_from_slice(name);
    packet
}

// ---------------------------------------------------------------------------
// Monitor
// ---------------------------------------------------------------------------

/// Availability of the configured receiver. `Unknown` until the first verdict,
/// `Alive` on any accepted reply, `Failed` once the error is established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VbanHealthState {
    Unknown,
    Alive,
    Failed(VbanHealthFailure),
}

/// Why the monitor is in the failed state. `LocalSocket` carries a separate
/// explanation (e.g. the reply port is busy) and is never masked as a missing
/// remote reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VbanHealthFailure {
    NoReply,
    LocalSocket(String),
}

impl VbanHealthState {
    /// Stable state key for the IPC contract (`unknown` / `alive` / `failed`).
    pub fn state_key(&self) -> &'static str {
        match self {
            VbanHealthState::Unknown => "unknown",
            VbanHealthState::Alive => "alive",
            VbanHealthState::Failed(_) => "failed",
        }
    }

    /// Stable reason key for the IPC contract (`no_reply` / `local_socket`).
    pub fn reason_key(&self) -> Option<&'static str> {
        match self {
            VbanHealthState::Failed(VbanHealthFailure::NoReply) => Some("no_reply"),
            VbanHealthState::Failed(VbanHealthFailure::LocalSocket(_)) => Some("local_socket"),
            _ => None,
        }
    }
}

/// Ping destination and cadence. The interval comes from the saved settings
/// (1–300 s); tests use sub-second durations for fast cycles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VbanPingTarget {
    pub addr: SocketAddrV4,
    pub interval: Duration,
}

/// Events the monitor reports to the installed emitter. `Status` fires on
/// every state transition; `Alert` fires when the failed state is established
/// or a production send starts while failed, deduplicated by a 5 s window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VbanHealthEvent {
    Status(VbanHealthState),
    Alert(VbanHealthState),
}

pub type HealthEmitter = Arc<dyn Fn(VbanHealthEvent) + Send + Sync>;

enum HealthControl {
    /// The desired target with the generation installed by `set_target`.
    Target(Option<VbanPingTarget>, u64),
    Shutdown,
}

/// State shared between the monitor thread and readers.
///
/// `generation` is the desired-target counter bumped synchronously by every
/// accepted [`VbanHealthMonitor::set_target`]; `state_generation` tags the
/// verdict with the generation it was established for. A verdict whose tag
/// lags the desired generation is stale: readers must never report or alert
/// on it, and the monitor thread discards probe results for it.
struct HealthShared {
    state: VbanHealthState,
    state_generation: u64,
    generation: u64,
    desired: Option<VbanPingTarget>,
    emitter: Option<HealthEmitter>,
    last_alert: Option<Instant>,
    alert_window: Duration,
}

impl HealthShared {
    /// Clone the emitter out of the lock: events are collected while the
    /// guard is alive and fired after it is dropped (see [`emit_events`]),
    /// so a callback can never run while the shared lock is held.
    fn emitter_clone(&self) -> Option<HealthEmitter> {
        self.emitter.clone()
    }

    /// Build an alert event unless one was already built inside the dedup
    /// window; `now` is injected for tests.
    fn alert_event(&mut self, state: &VbanHealthState, now: Instant) -> Option<VbanHealthEvent> {
        let inside_window = self
            .last_alert
            .is_some_and(|at| now.duration_since(at) < self.alert_window);
        if inside_window {
            return None;
        }
        self.last_alert = Some(now);
        Some(VbanHealthEvent::Alert(state.clone()))
    }

    /// Apply a new verdict and return the events to emit after the lock is
    /// released: every transition reports `Status`; entering the failed
    /// state additionally reports one `Alert`. An unchanged verdict is
    /// silent but re-tagged with the current generation. Verdicts are always
    /// tagged with the current desired generation — the callers have already
    /// proven the verdict describes the desired target.
    fn set_state(&mut self, state: VbanHealthState, now: Instant) -> Vec<VbanHealthEvent> {
        let generation = self.generation;
        if self.state == state {
            self.state_generation = generation;
            return Vec::new();
        }
        let was_failed = matches!(self.state, VbanHealthState::Failed(_));
        self.state = state.clone();
        self.state_generation = generation;
        let mut events = vec![VbanHealthEvent::Status(state.clone())];
        if matches!(state, VbanHealthState::Failed(_)) && !was_failed {
            if let Some(alert) = self.alert_event(&state, now) {
                events.push(alert);
            }
        }
        events
    }
}

/// Fire collected events through the emitter after the shared lock has been
/// released.
fn emit_events(emitter: Option<HealthEmitter>, events: Vec<VbanHealthEvent>) {
    if let Some(emitter) = emitter {
        for event in events {
            emitter(event);
        }
    }
}

/// Shared VBAN receiver health monitor. Thread-safe, owned by
/// [`crate::audio::vban::VbanOutput`]; all UDP work happens on its own thread.
pub struct VbanHealthMonitor {
    shared: Arc<Mutex<HealthShared>>,
    /// Publication ordering lock. Every publisher (`set_target`,
    /// `record_probe`, `record_local_socket_error`, `notify_send_started`)
    /// acquires it *before* the shared state mutex and holds it across the
    /// state commit and the event emission, so commit order equals publish
    /// order: a retarget's reset can never land between a verdict's commit
    /// and its emission, and a verdict committed earlier can never be
    /// published after a reset committed later. The lock is never acquired
    /// while the shared lock is held, so there is no lock-order cycle; an
    /// event callback must never re-enter a publisher method.
    emit_order: Arc<Mutex<()>>,
    tx: Sender<HealthControl>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl VbanHealthMonitor {
    /// Create the monitor and spawn its thread. It stays idle (no socket) until
    /// a target is set; the production reply port is fixed at 6980.
    pub(crate) fn new() -> Self {
        Self::with_timing(SERVICE_REPLY_PORT, PROBE_REPLY_TIMEOUT, ALERT_DEDUP_WINDOW)
    }

    /// Test constructor: an ephemeral reply port and short timings keep the
    /// loopback tests fast and parallel-safe.
    fn with_timing(reply_port: u16, probe_timeout: Duration, alert_window: Duration) -> Self {
        let (tx, rx) = mpsc::channel::<HealthControl>();
        let shared = Arc::new(Mutex::new(HealthShared {
            state: VbanHealthState::Unknown,
            state_generation: 0,
            generation: 0,
            desired: None,
            emitter: None,
            last_alert: None,
            alert_window,
        }));
        let emit_order = Arc::new(Mutex::new(()));
        let thread_shared = Arc::clone(&shared);
        let thread_emit_order = Arc::clone(&emit_order);
        let join = thread::Builder::new()
            .name("vban-health".to_string())
            .spawn(move || {
                run_loop(
                    rx,
                    thread_shared,
                    thread_emit_order,
                    reply_port,
                    probe_timeout,
                )
            })
            .ok();
        Self {
            shared,
            emit_order,
            tx,
            join: Mutex::new(join),
        }
    }

    /// Start checking `target`, stop checking on `None`. A new receiver resets
    /// results and counters and probes immediately; an interval-only change
    /// keeps the current verdict. Idempotent.
    ///
    /// The request is accepted synchronously, before the monitor thread sees
    /// it: the old verdict stops being reported and alerted right away, so a
    /// probe that is still in flight for the previous target can never leak a
    /// stale state or toast. Two halves of that guarantee live on the probe
    /// side: a verdict is dropped when the desired generation has moved past
    /// the probe's own generation, and the commit-plus-emission of every
    /// publisher runs under the publication ordering lock, so the reset this
    /// call publishes can never be overtaken by a stale verdict committed
    /// earlier.
    pub fn set_target(&self, target: Option<VbanPingTarget>) {
        // The publication lock is taken before the shared state and held
        // across the commit *and* the emission below, so another publisher
        // can neither interleave its emission nor commit-and-publish ahead
        // of us after we have committed.
        let _order = self.emit_order.lock().expect("VBAN health emit order");
        let (generation, events, emitter) = {
            let mut shared = self.shared.lock().expect("VBAN health state poisoned");
            if shared.desired == target {
                return;
            }
            let same_receiver = matches!(
                (&target, &shared.desired),
                (Some(new), Some(old)) if new.addr == old.addr
            );
            shared.generation += 1;
            shared.desired = target.clone();
            let generation = shared.generation;
            let events = if same_receiver {
                // Interval-only change: the verdict still describes the same
                // receiver, so it is re-tagged as current instead of reset.
                shared.state_generation = generation;
                Vec::new()
            } else {
                // Disable or receiver change: the stale verdict is dropped at
                // once, not when the thread gets around to it.
                shared.set_state(VbanHealthState::Unknown, Instant::now())
            };
            (generation, events, shared.emitter_clone())
        };
        emit_events(emitter, events);
        let _ = self.tx.send(HealthControl::Target(target, generation));
    }

    /// Current availability verdict.
    pub fn state(&self) -> VbanHealthState {
        self.shared
            .lock()
            .expect("VBAN health state poisoned")
            .state
            .clone()
    }

    /// Install or remove the event emitter (called once from app setup).
    pub fn set_emitter(&self, emitter: Option<HealthEmitter>) {
        self.shared
            .lock()
            .expect("VBAN health state poisoned")
            .emitter = emitter;
    }

    /// Called at the actual start of a production send (speech or SoundPanel
    /// clip). When the receiver is currently failed, emits the same alert as a
    /// threshold crossing, deduplicated so a visible toast is never repeated.
    /// A verdict that belongs to a superseded target is never alerted on.
    /// Returns whether an alert was emitted (for logging). Never blocks on
    /// network or audio work; the publication lock is held only across the
    /// commit and the callback, so the alert keeps its commit order relative
    /// to a concurrent retarget's reset.
    pub fn notify_send_started(&self) -> bool {
        // The publication lock spans the commit and the emission, keeping the
        // alert's publish order consistent with a concurrent retarget's
        // reset.
        let _order = self.emit_order.lock().expect("VBAN health emit order");
        let alerted;
        let mut emit = None;
        {
            let mut shared = self.shared.lock().expect("VBAN health state poisoned");
            if shared.state_generation != shared.generation {
                // The verdict belongs to a superseded target.
                return false;
            }
            if !matches!(shared.state, VbanHealthState::Failed(_)) {
                return false;
            }
            let state = shared.state.clone();
            if let Some(event) = shared.alert_event(&state, Instant::now()) {
                emit = Some((shared.emitter_clone(), event));
                alerted = true;
            } else {
                alerted = false;
            }
        }
        if let Some((Some(emitter), event)) = emit {
            emitter(event);
        }
        alerted
    }

    /// Stop the thread and release the reply port. Subsequent operations are
    /// no-ops until a new monitor is created.
    pub fn shutdown(&self) {
        let _ = self.tx.send(HealthControl::Shutdown);
        if let Some(join) = self.join.lock().expect("VBAN health join poisoned").take() {
            let _ = join.join();
        }
    }
}

impl Drop for VbanHealthMonitor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// One monitoring loop: sequential probes, never overlapping.
struct MonitorLoop {
    rx: Receiver<HealthControl>,
    shared: Arc<Mutex<HealthShared>>,
    /// Publication ordering lock shared with [`VbanHealthMonitor`]; taken
    /// before the shared state in the record paths, see [`Self::record_probe`].
    emit_order: Arc<Mutex<()>>,
    reply_port: u16,
    probe_timeout: Duration,
    target: Option<VbanPingTarget>,
    socket: Option<UdpSocket>,
    tracker: FailureTracker,
    next_probe_at: Instant,
    request_id: u32,
    /// The desired generation this loop has applied; a control whose
    /// generation is older is dropped, because `set_target` sends its control
    /// outside the shared lock and concurrent calls can enqueue them out of
    /// order — the highest generation always wins.
    active_generation: u64,
    /// A shutdown request noticed mid-probe, honoured as soon as it ends.
    shutting_down: bool,
}

fn run_loop(
    rx: Receiver<HealthControl>,
    shared: Arc<Mutex<HealthShared>>,
    emit_order: Arc<Mutex<()>>,
    reply_port: u16,
    probe_timeout: Duration,
) {
    let mut loop_state = MonitorLoop {
        rx,
        shared,
        emit_order,
        reply_port,
        probe_timeout,
        target: None,
        socket: None,
        tracker: FailureTracker::new(SLOW_THRESHOLD),
        next_probe_at: Instant::now(),
        request_id: 0,
        active_generation: 0,
        shutting_down: false,
    };
    loop_state.run();
}

impl MonitorLoop {
    fn run(&mut self) {
        let idle_wait = Duration::from_secs(3600);
        loop {
            let wait = if self.target.is_some() {
                self.next_probe_at.saturating_duration_since(Instant::now())
            } else {
                idle_wait
            };
            match self.rx.recv_timeout(wait) {
                Ok(HealthControl::Target(target, generation)) => {
                    self.handle_target(target, generation)
                }
                Ok(HealthControl::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                    return self.finish()
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.shutting_down {
                return self.finish();
            }
            if self.target.is_some() && Instant::now() >= self.next_probe_at {
                self.probe();
                if self.shutting_down {
                    return self.finish();
                }
                let interval = self
                    .target
                    .as_ref()
                    .map_or(idle_wait, |target| target.interval);
                self.next_probe_at = Instant::now() + interval;
            }
        }
    }

    /// Final cleanup: drop the emitter so late external readers cannot
    /// observe the dead thread.
    fn finish(&mut self) {
        self.shared
            .lock()
            .expect("VBAN health state poisoned")
            .emitter = None;
    }

    /// Apply a `Target` control unless a newer request has already been
    /// applied: `set_target` sends its control after releasing the shared
    /// lock, so two concurrent calls can enqueue their controls in the
    /// opposite order of their generations. The highest generation always
    /// matches the current desired target, so older ones are dropped and the
    /// loop never reverts to a target the user has already superseded.
    fn handle_target(&mut self, target: Option<VbanPingTarget>, generation: u64) {
        if generation < self.active_generation {
            tracing::debug!(
                target: "vban_health",
                "PING0 target control dropped: a newer request was applied already"
            );
            return;
        }
        self.apply_target(target, generation)
    }

    /// Apply a target change: `None` stops checking and releases the port;
    /// a changed receiver resets results and counters and probes immediately;
    /// an interval-only change keeps the verdict and the failure counter but
    /// re-bands the tracker to the new interval. The shared verdict itself is
    /// owned by the synchronous `set_target`, never touched here.
    fn apply_target(&mut self, target: Option<VbanPingTarget>, generation: u64) {
        self.active_generation = generation;
        let new_addr = target.as_ref().map(|target| target.addr);
        let old_addr = self.target.as_ref().map(|target| target.addr);
        let same_receiver = new_addr.is_some() && new_addr == old_addr;
        self.target = target;
        if !same_receiver {
            // Disable or receiver change: reset the counters and release the
            // reply socket; stale in-flight datagrams belong to the old
            // generation and can never affect the new target.
            self.tracker.reset();
            self.socket = None;
        }
        if self.target.is_some() {
            let interval = self
                .target
                .as_ref()
                .map_or(PROBE_REPLY_TIMEOUT, |target| target.interval);
            let threshold =
                failure_threshold(u32::try_from(interval.as_secs()).unwrap_or(u32::MAX));
            if same_receiver {
                // Interval-only change: next probe within the new interval,
                // the consecutive-miss count is preserved, and the threshold
                // follows the new band even across the 10 s boundary.
                self.next_probe_at = self.next_probe_at.min(Instant::now() + interval);
                self.tracker.set_threshold(threshold);
            } else {
                // Enable or receiver change: fresh counters and an immediate
                // probe so the first verdict arrives without delay.
                self.tracker = FailureTracker::new(threshold);
                self.next_probe_at = Instant::now();
            }
        } else {
            self.next_probe_at = Instant::now();
        }
    }

    /// Ensure the reply socket. The receiver replies to the sender's fixed
    /// UDP port, so a bind failure is an explicit local error with its own
    /// explanation — it is retried on the next cycle, and releasing the port
    /// recovers automatically. The error is only reported back: the shared
    /// state is updated after the in-flight staleness check, never here.
    fn ensure_socket(&mut self) -> Result<&UdpSocket, String> {
        if self.socket.is_none() {
            match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, self.reply_port)) {
                Ok(socket) => {
                    self.socket = Some(socket);
                    // A fresh bind also means a fresh verdict window.
                    self.tracker.reset();
                }
                Err(error) => {
                    self.tracker.reset();
                    return Err(format!(
                        "local UDP port {} is unavailable: {error}",
                        self.reply_port
                    ));
                }
            }
        }
        Ok(self.socket.as_ref().expect("socket just ensured"))
    }

    /// One non-overlapping probe: drain stale datagrams, send the full PING0
    /// request, wait for a validated reply within the probe timeout. The
    /// verdict is tagged with the generation the probe was sent under and
    /// applied only while the desired generation is still that one: any
    /// accepted `set_target` — disable, receiver change or interval-only —
    /// invalidates the in-flight verdict, so a result can never outlive the
    /// desire it was asked under, including the A → B → A detour where the
    /// address alone would look current again.
    fn probe(&mut self) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let probe_generation = self.active_generation;
        self.request_id = self.request_id.wrapping_add(1);
        let request = encode_ping0_request(self.request_id);
        let probe_timeout = self.probe_timeout;
        // The socket borrow is scoped so the verdict below can update the
        // shared state freely.
        enum Outcome {
            Replied(bool),
            LocalSocket(String),
        }
        let outcome = match self.ensure_socket() {
            Err(detail) => Outcome::LocalSocket(detail),
            Ok(socket) => {
                // Drop replies that arrived after the previous window closed:
                // they belong to an older request and must never count here.
                if socket.set_nonblocking(true).is_ok() {
                    let mut drain_buf = [0u8; 2048];
                    while socket.recv_from(&mut drain_buf).is_ok() {}
                    let _ = socket.set_nonblocking(false);
                }

                if let Err(error) = socket.send_to(&request, target.addr) {
                    // A local send error means the receiver is unreachable: it
                    // counts as a failed probe, with the OS detail logged.
                    tracing::debug!(target: "vban_health", error = %error, "PING0 send failed");
                    Outcome::Replied(false)
                } else {
                    let deadline = Instant::now() + probe_timeout;
                    let mut buf = [0u8; 2048];
                    loop {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            break Outcome::Replied(false);
                        }
                        if let Err(error) = socket.set_read_timeout(Some(remaining)) {
                            tracing::debug!(
                                target: "vban_health",
                                error = %error,
                                "PING0 recv timeout setup failed"
                            );
                            break Outcome::Replied(false);
                        }
                        match socket.recv_from(&mut buf) {
                            Ok((len, source)) => {
                                if decode_ping0_reply(&buf[..len], &target.addr, source).is_ok() {
                                    break Outcome::Replied(true);
                                }
                                // Unrelated datagram: keep waiting for a valid reply.
                            }
                            Err(error) => match error.kind() {
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                                    break Outcome::Replied(false)
                                }
                                std::io::ErrorKind::Interrupted => continue,
                                _ => {
                                    tracing::debug!(
                                        target: "vban_health",
                                        error = %error,
                                        "PING0 recv failed"
                                    );
                                    break Outcome::Replied(false);
                                }
                            },
                        }
                    }
                }
            }
        };
        // Let the user's disable or retarget win over this probe's verdict.
        // This drain-and-check is a fast path: the authoritative staleness
        // decision happens under the shared lock inside the record methods,
        // because a set_target that arrives after this check has already
        // bumped the desired generation — the record must not resurrect the
        // probe's verdict.
        self.drain_pending_controls();
        if self.shutting_down || self.target.as_ref().map(|t| t.addr) != Some(target.addr) {
            tracing::debug!(
                target: "vban_health",
                "PING0 verdict discarded: the target changed while the probe was in flight"
            );
            return;
        }
        match outcome {
            Outcome::Replied(replied) => self.record_probe(
                replied,
                VbanHealthFailure::NoReply,
                target.addr,
                probe_generation,
            ),
            Outcome::LocalSocket(detail) => {
                self.record_local_socket_error(detail, probe_generation)
            }
        }
    }

    /// Process the control messages that arrived while a probe was in flight
    /// so the verdict is judged against the current desired target. A
    /// shutdown noticed here is honoured as soon as the probe ends.
    fn drain_pending_controls(&mut self) {
        while let Ok(control) = self.rx.try_recv() {
            match control {
                HealthControl::Target(target, generation) => self.handle_target(target, generation),
                HealthControl::Shutdown => self.shutting_down = true,
            }
        }
    }

    /// Apply a probe verdict: a reply resets the counter and turns the status
    /// green (also recovering from a failed state); a miss keeps the previous
    /// status until the threshold is reached, then establishes the error.
    ///
    /// `probe_generation` is the desired generation the probe was sent under.
    /// The verdict is applied only while the desired generation is still that
    /// one, judged under the shared lock: this closes both the drain →
    /// retarget → record window (a `set_target` that slips in between the
    /// post-probe control drain and the verdict application has already
    /// bumped the generation and published the reset) and the A → B → A
    /// detour, where a bare address comparison would accept a verdict that
    /// was asked under an older desire. A stale verdict is dropped whole —
    /// no state change, no events, no tracker movement.
    ///
    /// The publication ordering lock is taken *before* the shared lock and
    /// held across the commit and the emission: commit order equals publish
    /// order, so a retarget that commits after this verdict cannot have its
    /// reset published first — the reviewer's commit → retarget → emit
    /// permutation is impossible by construction.
    fn record_probe(
        &mut self,
        replied: bool,
        failure: VbanHealthFailure,
        probe_addr: SocketAddrV4,
        probe_generation: u64,
    ) {
        let now = Instant::now();
        // The publication lock is taken before the shared state and held
        // across the commit *and* the emission below — commit order equals
        // publish order.
        let _order = self.emit_order.lock().expect("VBAN health emit order");
        let (emitter, events) = {
            let mut shared = self.shared.lock().expect("VBAN health state poisoned");
            if shared.generation != probe_generation {
                tracing::debug!(
                    target: "vban_health",
                    probe_addr = %probe_addr,
                    probe_generation,
                    desired_generation = shared.generation,
                    "PING0 verdict discarded under the lock: the desired generation moved on"
                );
                return;
            }
            let events = if replied {
                self.tracker.reset();
                shared.set_state(VbanHealthState::Alive, now)
            } else if self.tracker.record_failure() {
                shared.set_state(VbanHealthState::Failed(failure), now)
            } else {
                Vec::new()
            };
            (shared.emitter_clone(), events)
        };
        emit_events(emitter, events);
    }

    /// Establish the local-socket error with its own reason. The same
    /// generation check and publication ordering as
    /// [`MonitorLoop::record_probe`] apply: a bind failure observed by a
    /// superseded probe is dropped, and the next probe of the actual target
    /// re-reports it if the port is still busy.
    fn record_local_socket_error(&mut self, detail: String, probe_generation: u64) {
        let now = Instant::now();
        let _order = self.emit_order.lock().expect("VBAN health emit order");
        let (emitter, events) = {
            let mut shared = self.shared.lock().expect("VBAN health state poisoned");
            if shared.generation != probe_generation {
                return;
            }
            let events = shared.set_state(
                VbanHealthState::Failed(VbanHealthFailure::LocalSocket(detail)),
                now,
            );
            (shared.emitter_clone(), events)
        };
        emit_events(emitter, events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket as TestSocket;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const PROBE: Duration = Duration::from_millis(120);
    const INTERVAL: Duration = Duration::from_millis(220);
    const ALERT_WINDOW: Duration = Duration::from_millis(400);

    type EventLog = Arc<Mutex<Vec<VbanHealthEvent>>>;

    fn ping_target(port: u16, interval: Duration) -> VbanPingTarget {
        VbanPingTarget {
            addr: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
            interval,
        }
    }

    /// Install an event recorder.
    fn install_recorder(monitor: &VbanHealthMonitor) -> EventLog {
        let log: EventLog = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&log);
        monitor.set_emitter(Some(Arc::new(move |event| {
            recorder.lock().unwrap().push(event);
        })));
        log
    }

    /// Poll until `predicate` holds, with a deadline; panics with the last
    /// state otherwise.
    fn wait_for_state(
        monitor: &VbanHealthMonitor,
        predicate: impl Fn(&VbanHealthState) -> bool,
        timeout: Duration,
    ) -> VbanHealthState {
        let deadline = Instant::now() + timeout;
        loop {
            let state = monitor.state();
            if predicate(&state) {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "state never reached the expected value, last: {state:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Poll until the atomic counter reaches `minimum`, with a deadline.
    fn wait_for_count(counter: &AtomicUsize, minimum: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while counter.load(Ordering::SeqCst) < minimum {
            assert!(
                Instant::now() < deadline,
                "counter never reached {minimum}, last: {}",
                counter.load(Ordering::SeqCst)
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Ports outside the OS ephemeral range (Windows hands out 49152+),
    /// unique within the test process. The monitor's reply port and dead ping
    /// destinations use them: a port that is only bound and released (an
    /// ephemeral `bind(0)` probe) can be handed right back to another
    /// parallel test, which makes the fixed-port binds race.
    fn next_test_port() -> u16 {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let seq = NEXT.fetch_add(1, Ordering::SeqCst);
        21_000 + (seq % 8_000) as u16
    }

    /// Responder that answers every PING0 request with a Receptor-style reply
    /// from its own bound socket, until dropped. It binds an ephemeral port
    /// and keeps it for its whole lifetime, so no other test can take it.
    struct Responder {
        port: u16,
        shutdown: Option<Box<dyn FnOnce()>>,
    }

    impl Responder {
        fn start() -> Self {
            let socket = TestSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("responder bind");
            let port = socket.local_addr().expect("responder addr").port();
            socket
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            let stop = Arc::new(AtomicUsize::new(0));
            let stop_flag = Arc::clone(&stop);
            let handle = thread::Builder::new()
                .name("ping-responder".to_string())
                .spawn(move || {
                    let mut buf = [0u8; 2048];
                    while stop_flag.load(Ordering::SeqCst) == 0 {
                        if let Ok((len, source)) = socket.recv_from(&mut buf) {
                            if len == PING0_PACKET_SIZE {
                                let reply = build_receptor_style_reply(0);
                                let _ = socket.send_to(&reply, source);
                            }
                        }
                    }
                })
                .expect("responder thread");
            Self {
                port,
                shutdown: Some(Box::new(move || {
                    stop.store(1, Ordering::SeqCst);
                    handle.join().ok();
                })),
            }
        }
    }

    impl Drop for Responder {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                shutdown();
            }
        }
    }

    /// A receiver that answers PING0 only after an explicit release, letting
    /// a test hold a monitor probe in flight deterministically.
    struct LateResponder {
        port: u16,
        requests: Arc<AtomicUsize>,
        replies: Arc<AtomicUsize>,
        release: Arc<AtomicUsize>,
        shutdown: Option<Box<dyn FnOnce()>>,
    }

    impl LateResponder {
        fn start() -> Self {
            let socket = TestSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("late responder bind");
            let port = socket.local_addr().expect("late responder addr").port();
            socket
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            let requests = Arc::new(AtomicUsize::new(0));
            let replies = Arc::new(AtomicUsize::new(0));
            let release = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicUsize::new(0));
            let handle = thread::Builder::new()
                .name("late-responder".to_string())
                .spawn({
                    let requests = Arc::clone(&requests);
                    let replies = Arc::clone(&replies);
                    let release = Arc::clone(&release);
                    let stop = Arc::clone(&stop);
                    move || {
                        let mut buf = [0u8; 2048];
                        while stop.load(Ordering::SeqCst) == 0 {
                            let Ok((len, source)) = socket.recv_from(&mut buf) else {
                                continue;
                            };
                            if len != PING0_PACKET_SIZE {
                                continue;
                            }
                            requests.fetch_add(1, Ordering::SeqCst);
                            // Hold the probe in flight until the test releases.
                            let deadline = Instant::now() + Duration::from_secs(10);
                            while release.load(Ordering::SeqCst) == 0 {
                                if Instant::now() >= deadline {
                                    break;
                                }
                                thread::sleep(Duration::from_millis(10));
                            }
                            let reply = build_receptor_style_reply(0);
                            let _ = socket.send_to(&reply, source);
                            replies.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                })
                .expect("late responder thread");
            Self {
                port,
                requests,
                replies,
                release,
                shutdown: Some(Box::new(move || {
                    stop.store(1, Ordering::SeqCst);
                    handle.join().ok();
                })),
            }
        }

        fn release(&self) {
            self.release.store(1, Ordering::SeqCst);
        }
    }

    impl Drop for LateResponder {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                shutdown();
            }
        }
    }

    // --------------------------- wire format ---------------------------

    #[test]
    fn ping0_request_is_a_full_service_packet() {
        let packet = encode_ping0_request(7);
        assert_eq!(packet.len(), 704);
        assert_eq!(&packet[0..4], b"VBAN");
        assert_eq!(packet[4], 0x60, "sub-protocol SERVICE");
        assert_eq!(packet[5], 0, "PING0 request function");
        assert_eq!(packet[6], 0, "IDENTIFICATION service type");
        assert_eq!(packet[7], 0, "format bit must be zero");
        let label_end = packet[8..24]
            .iter()
            .position(|&b| b == 0)
            .expect("label is zero padded");
        assert_eq!(&packet[8..8 + label_end], REQUEST_LABEL.as_bytes());
        assert_eq!(
            u32::from_le_bytes(packet[24..28].try_into().unwrap()),
            7,
            "request id lives in the header nuFrame"
        );
        assert!(
            packet[protocol::HEADER_SIZE..].iter().all(|&b| b == 0),
            "payload stays zero"
        );
    }

    #[test]
    fn decode_accepts_receptor_style_reply_with_zero_request_id() {
        let packet = build_receptor_style_reply(0);
        let target = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 6980);
        let reply = decode_ping0_reply(&packet, &target, "127.0.0.1:6980".parse().unwrap())
            .expect("real Receptor reply must be accepted");
        assert_eq!(reply.request_id_echo, 0, "Receptor always echoes 0");
        assert_eq!(reply.device_type, 32);
        assert_eq!(reply.features, 66_305);
        assert_eq!(reply.version, [1, 0, 0, 5]);
        assert_eq!(reply.application_name, "VBAN-Receptor");
    }

    #[test]
    fn decode_rejects_short_header_only_packets() {
        let target = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 6980);
        let source: SocketAddr = "127.0.0.1:6980".parse().unwrap();
        let full = build_receptor_style_reply(0);
        assert_eq!(
            decode_ping0_reply(&full[..28], &target, source),
            Err(Ping0Reject::Size(28)),
            "header-only probe answers were never observed"
        );
        let mut short = full.to_vec();
        short.truncate(703);
        assert_eq!(
            decode_ping0_reply(&short, &target, source),
            Err(Ping0Reject::Size(703))
        );
    }

    /// Byte-for-byte PING0 reply header captured from the real receiver at
    /// 192.168.8.148:6980 (experiment `.work/ai/vban-ping-20261116/`,
    /// `ping-port6980-results.json`, seq 102/104/105/106 all identical):
    /// `"VBAN"`, 0x60, function 0x80 (reply + PING0) in byte 5, service 0x00
    /// in byte 6, reserved 0x00, then the 16-byte "VBAN Service" label. The
    /// fixture is independent of [`build_receptor_style_reply`] on purpose:
    /// the synthetic responder must never define the wire format.
    fn captured_real_reply() -> [u8; PING0_PACKET_SIZE] {
        const HEADER_HEX: &str = "5642414e608000005642414e20536572766963650000000000000000";
        fn hex_byte(pair: &[u8]) -> u8 {
            let hi = (pair[0] as char).to_digit(16).expect("hex digit") as u8;
            let lo = (pair[1] as char).to_digit(16).expect("hex digit") as u8;
            (hi << 4) | lo
        }
        let header: [u8; 28] = {
            let bytes = HEADER_HEX.as_bytes();
            assert_eq!(bytes.len(), 56, "captured header is 28 bytes");
            let mut header = [0u8; 28];
            for (i, field) in header.iter_mut().enumerate() {
                *field = hex_byte(&bytes[i * 2..i * 2 + 2]);
            }
            header
        };
        let mut packet = [0u8; PING0_PACKET_SIZE];
        packet[..header.len()].copy_from_slice(&header);
        // Payload fields from the same capture: version 1.0.0.5 at offset 28
        // and the application name at offset 292 — both relative to the
        // payload, like the decoder's `ping0_layout` offsets. The type/
        // feature words are not recorded in the capture and are not
        // validated by the decoder.
        let payload = &mut packet[protocol::HEADER_SIZE..];
        payload[ping0_layout::VERSION..ping0_layout::VERSION + 4].copy_from_slice(&[1, 0, 0, 5]);
        let name = b"VBAN-Receptor";
        payload[ping0_layout::APPLICATION_NAME..ping0_layout::APPLICATION_NAME + name.len()]
            .copy_from_slice(name);
        packet
    }

    #[test]
    fn real_wire_reply_with_captured_header_is_accepted() {
        let packet = captured_real_reply();
        // The capture itself: SERVICE protocol, reply+PING0 function in byte
        // 5, IDENTIFICATION service in byte 6.
        assert_eq!(&packet[0..4], b"VBAN");
        assert_eq!(packet[4], 0x60);
        assert_eq!(packet[5], 0x80);
        assert_eq!(packet[6], 0x00);
        // The ping destination is the capture's responder, so the endpoint
        // check accepts the packet.
        let target = SocketAddrV4::new(Ipv4Addr::new(192, 168, 8, 148), 6980);
        let source: SocketAddr = "192.168.8.148:6980".parse().unwrap();
        let reply = decode_ping0_reply(&packet, &target, source)
            .expect("the captured real-wire reply must be accepted");
        assert_eq!(reply.request_id_echo, 0);
        assert_eq!(reply.application_name, "VBAN-Receptor");
        assert_eq!(reply.version, [1, 0, 0, 5]);
    }

    #[test]
    fn swapped_function_and_service_bytes_are_rejected() {
        // The old decoder read the reply flag from byte 6 and the service
        // from byte 5 — exactly the swap of the real wire. That layout must
        // stay rejected.
        let mut packet = captured_real_reply();
        packet.swap(5, 6);
        assert_eq!(&packet[4..8], &[0x60, 0x00, 0x80, 0x00]);
        let target = SocketAddrV4::new(Ipv4Addr::new(192, 168, 8, 148), 6980);
        let source: SocketAddr = "192.168.8.148:6980".parse().unwrap();
        assert_eq!(
            decode_ping0_reply(&packet, &target, source),
            Err(Ping0Reject::Header),
            "function and service bytes must not be swapped"
        );
    }

    #[test]
    fn decode_rejects_wrong_source_endpoint() {
        let packet = build_receptor_style_reply(0);
        let target = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 6980);
        assert_eq!(
            decode_ping0_reply(&packet, &target, "127.0.0.1:50000".parse().unwrap()),
            Err(Ping0Reject::Source),
            "a different source port is not the ping target"
        );
        assert_eq!(
            decode_ping0_reply(&packet, &target, "127.0.0.2:6980".parse().unwrap()),
            Err(Ping0Reject::Source),
            "a different source IP is not the ping target"
        );
    }

    #[test]
    fn decode_rejects_non_service_or_request_headers() {
        let target = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 6980);
        let source: SocketAddr = "127.0.0.1:6980".parse().unwrap();
        let mut packet = build_receptor_style_reply(0);

        packet[4] = 0x03; // audio sub-protocol, not SERVICE
        assert_eq!(
            decode_ping0_reply(&packet, &target, source),
            Err(Ping0Reject::Header)
        );

        packet[4] = 0x60;
        packet[5] = SERVICE_FUNCTION_PING0; // request, not a reply
        assert_eq!(
            decode_ping0_reply(&packet, &target, source),
            Err(Ping0Reject::Header)
        );

        packet[5] = SERVICE_FUNCTION_REPLY | 1; // reply of another function
        assert_eq!(
            decode_ping0_reply(&packet, &target, source),
            Err(Ping0Reject::Header)
        );

        packet[5] = SERVICE_FUNCTION_REPLY;
        packet[6] = 1; // another service type
        assert_eq!(
            decode_ping0_reply(&packet, &target, source),
            Err(Ping0Reject::Header)
        );

        packet[6] = SERVICE_TYPE_IDENTIFICATION;
        packet[0] = b'X'; // broken magic
        assert_eq!(
            decode_ping0_reply(&packet, &target, source),
            Err(Ping0Reject::Header)
        );
    }

    #[test]
    fn threshold_matches_interval_bands() {
        for secs in [1, 2, 9] {
            assert_eq!(failure_threshold(secs), 3, "{secs}s is a fast interval");
        }
        for secs in [10, 11, 30, 300] {
            assert_eq!(failure_threshold(secs), 2, "{secs}s is a slow interval");
        }
    }

    #[test]
    fn tracker_trips_only_at_the_threshold_and_resets_on_success() {
        let mut tracker = FailureTracker::new(2);
        assert!(!tracker.record_failure(), "first failure keeps the status");
        assert!(tracker.record_failure(), "second failure trips threshold 2");
        tracker.reset();
        assert!(
            !tracker.record_failure(),
            "success reset clears the counter"
        );

        let mut fast = FailureTracker::new(3);
        assert!(!fast.record_failure());
        assert!(!fast.record_failure());
        assert!(fast.record_failure(), "third failure trips threshold 3");
    }

    // --------------------------- monitor loop ---------------------------

    #[test]
    fn no_reply_reaches_failed_after_threshold_then_recovers() {
        let reply_port = next_test_port();
        let dead_dest = next_test_port();
        let monitor = VbanHealthMonitor::with_timing(reply_port, PROBE, ALERT_WINDOW);
        let log = install_recorder(&monitor);
        monitor.set_target(Some(ping_target(dead_dest, INTERVAL)));

        // Sub-second interval is a fast interval: 3 consecutive misses.
        let failed = wait_for_state(
            &monitor,
            |state| matches!(state, VbanHealthState::Failed(VbanHealthFailure::NoReply)),
            Duration::from_secs(5),
        );
        assert_eq!(failed.reason_key(), Some("no_reply"));
        let alert = log.lock().unwrap().iter().any(|e| {
            matches!(
                e,
                VbanHealthEvent::Alert(state)
                    if matches!(state, VbanHealthState::Failed(VbanHealthFailure::NoReply))
            )
        });
        assert!(alert, "crossing the threshold emits one alert");

        // The first reply restores green immediately. The responder owns its
        // own port, so the monitor is retargeted to it (a receiver change
        // probes at once).
        let responder = Responder::start();
        monitor.set_target(Some(ping_target(responder.port, INTERVAL)));
        wait_for_state(
            &monitor,
            |state| *state == VbanHealthState::Alive,
            Duration::from_secs(5),
        );
        drop(responder);
        monitor.shutdown();
    }

    #[test]
    fn stale_reply_from_a_previous_request_is_discarded() {
        let reply_port = next_test_port();
        let monitor = VbanHealthMonitor::with_timing(reply_port, PROBE, ALERT_WINDOW);
        install_recorder(&monitor);

        // The responder answers only the first request and late — after the
        // first probe window closed. The stale datagram must be drained before
        // the next probe and never counted as that probe's reply. Its socket
        // stays bound for the whole test, so the port stays reserved.
        let socket = TestSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("responder bind");
        let dest_port = socket.local_addr().expect("responder addr").port();
        thread::spawn(move || {
            let mut buf = [0u8; 2048];
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            if socket.recv_from(&mut buf).is_ok() {
                thread::sleep(PROBE + Duration::from_millis(40));
                let reply_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, reply_port));
                let _ = socket.send_to(&build_receptor_style_reply(0), reply_addr);
            }
        });

        monitor.set_target(Some(ping_target(dest_port, INTERVAL)));
        // Threshold 3 for the fast interval: if the stale reply were accepted,
        // the second probe would report success and the state would never fail.
        wait_for_state(
            &monitor,
            |state| matches!(state, VbanHealthState::Failed(VbanHealthFailure::NoReply)),
            Duration::from_secs(5),
        );
        monitor.shutdown();
    }

    #[test]
    fn receiver_change_resets_counters_and_probes_immediately() {
        let reply_port = next_test_port();
        let dest_a = next_test_port();
        let dest_b = next_test_port();
        let monitor = VbanHealthMonitor::with_timing(reply_port, PROBE, ALERT_WINDOW);
        let log = install_recorder(&monitor);

        // Failed probes on A below the fast threshold of 3: the grey verdict
        // is preserved. Windows reports an unreachable localhost destination
        // immediately (ICMP-derived connection reset), so a probe can finish
        // in no time; 300 ms covers at most two probes at the 220 ms interval
        // under any cadence — always below the threshold of 3.
        monitor.set_target(Some(ping_target(dest_a, INTERVAL)));
        thread::sleep(Duration::from_millis(300));
        assert_eq!(
            monitor.state(),
            VbanHealthState::Unknown,
            "below the threshold the previous (grey) status is kept"
        );

        // Receiver change resets the counters: B needs its own 3 misses, so
        // the first failed probe on B must not trip the error.
        monitor.set_target(Some(ping_target(dest_b, INTERVAL)));
        thread::sleep(PROBE + Duration::from_millis(60));
        assert_eq!(
            monitor.state(),
            VbanHealthState::Unknown,
            "counters were reset by the receiver change"
        );
        // ... and probing continues on B until its own threshold.
        wait_for_state(
            &monitor,
            |state| matches!(state, VbanHealthState::Failed(VbanHealthFailure::NoReply)),
            Duration::from_secs(5),
        );
        let events = log.lock().unwrap().clone();
        assert!(
            events.iter().all(|e| matches!(
                e,
                VbanHealthEvent::Status(_) | VbanHealthEvent::Alert(_)
            ) && !matches!(
                e,
                VbanHealthEvent::Status(VbanHealthState::Alive)
            )),
            "no spurious verdicts during the A/B phases: {events:?}"
        );
        assert!(
            events.contains(&VbanHealthEvent::Status(VbanHealthState::Failed(
                VbanHealthFailure::NoReply
            ))),
            "the B phase reaches its own threshold: {events:?}"
        );
        monitor.shutdown();
    }

    #[test]
    fn disable_stops_probing_releases_the_port_and_reenable_probes_at_once() {
        let reply_port = next_test_port();
        let monitor = VbanHealthMonitor::with_timing(reply_port, PROBE, ALERT_WINDOW);
        let log = install_recorder(&monitor);

        let responder = Responder::start();
        monitor.set_target(Some(ping_target(responder.port, INTERVAL)));
        wait_for_state(
            &monitor,
            |state| *state == VbanHealthState::Alive,
            Duration::from_secs(5),
        );

        monitor.set_target(None);
        // The shared verdict resets synchronously, but the reply port is
        // released only when the monitor thread processes the disable, so
        // poll for the release itself.
        let mut reclaimed = None;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(socket) = TestSocket::bind((Ipv4Addr::LOCALHOST, reply_port)) {
                reclaimed = Some(socket);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            reclaimed.is_some(),
            "disabling must release the fixed reply port"
        );

        // No probes run while disabled: the status stays grey.
        thread::sleep(INTERVAL * 2 + PROBE);
        assert_eq!(monitor.state(), VbanHealthState::Unknown);

        // Re-enabling probes immediately: green again without waiting an
        // interval. The reclaimed test socket must first release the port,
        // and the new responder owns its own freshly bound port.
        drop(reclaimed);
        let responder2 = Responder::start();
        monitor.set_target(Some(ping_target(responder2.port, INTERVAL)));
        wait_for_state(
            &monitor,
            |state| *state == VbanHealthState::Alive,
            Duration::from_secs(5),
        );
        assert!(matches!(
            log.lock().unwrap().last(),
            Some(VbanHealthEvent::Status(VbanHealthState::Alive))
        ));
        drop(responder);
        drop(responder2);
        monitor.shutdown();
    }

    #[test]
    fn local_port_conflict_has_its_own_reason_and_recovers() {
        // Another application holds the reply port. The blocker mirrors the
        // monitor's own wildcard bind exactly, so the conflict is a duplicate
        // bind, deterministic by construction; its socket stays bound for the
        // whole episode, so no other test can free or take the port.
        let blocker = TestSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).expect("blocker bind");
        let reply_port = blocker.local_addr().expect("blocker addr").port();
        let dead_dest = next_test_port();
        let monitor = VbanHealthMonitor::with_timing(reply_port, PROBE, ALERT_WINDOW);
        let log = install_recorder(&monitor);

        monitor.set_target(Some(ping_target(dead_dest, INTERVAL)));
        let failed = wait_for_state(
            &monitor,
            |state| {
                matches!(
                    state,
                    VbanHealthState::Failed(VbanHealthFailure::LocalSocket(_))
                )
            },
            Duration::from_secs(3),
        );
        assert_eq!(failed.reason_key(), Some("local_socket"));
        assert!(matches!(
            &failed,
            VbanHealthState::Failed(VbanHealthFailure::LocalSocket(detail))
                if detail.contains(&reply_port.to_string())
        ));

        // The local episode alerts exactly once even while the bind keeps failing.
        thread::sleep(INTERVAL * 3);
        let alerts = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| {
                matches!(e, VbanHealthEvent::Alert(state) if matches!(state,
                    VbanHealthState::Failed(VbanHealthFailure::LocalSocket(_))))
            })
            .count();
        assert_eq!(alerts, 1, "a stable local error must not re-alert");

        // Releasing the port recovers automatically: rebind, then a reply.
        drop(blocker);
        let responder = Responder::start();
        monitor.set_target(Some(ping_target(responder.port, INTERVAL)));
        wait_for_state(
            &monitor,
            |state| *state == VbanHealthState::Alive,
            Duration::from_secs(5),
        );
        drop(responder);
        monitor.shutdown();
    }

    #[test]
    fn send_start_while_failed_alerts_once_inside_the_window() {
        let reply_port = next_test_port();
        let dead_dest = next_test_port();
        let monitor = VbanHealthMonitor::with_timing(reply_port, PROBE, ALERT_WINDOW);
        let log = install_recorder(&monitor);
        monitor.set_target(Some(ping_target(dead_dest, INTERVAL)));
        wait_for_state(
            &monitor,
            |state| matches!(state, VbanHealthState::Failed(VbanHealthFailure::NoReply)),
            Duration::from_secs(5),
        );

        // The threshold crossing already alerted; an immediate send start is
        // suppressed so the visible toast is not duplicated.
        assert!(!monitor.notify_send_started(), "inside the toast window");
        let alerts_now = alert_count(&log);
        assert_eq!(alerts_now, 1);

        // After the window a later send start warns again (same episode).
        thread::sleep(ALERT_WINDOW + Duration::from_millis(80));
        assert!(monitor.notify_send_started(), "after the toast window");
        assert_eq!(alert_count(&log), 2);
        monitor.shutdown();
    }

    #[test]
    fn send_start_while_healthy_or_unknown_never_alerts() {
        let reply_port = next_test_port();
        let monitor = VbanHealthMonitor::with_timing(reply_port, PROBE, ALERT_WINDOW);
        install_recorder(&monitor);
        assert!(!monitor.notify_send_started(), "unknown state is silent");

        let responder = Responder::start();
        monitor.set_target(Some(ping_target(responder.port, INTERVAL)));
        wait_for_state(
            &monitor,
            |state| *state == VbanHealthState::Alive,
            Duration::from_secs(5),
        );
        assert!(!monitor.notify_send_started(), "alive state is silent");
        drop(responder);
        monitor.shutdown();
    }

    #[test]
    fn disabled_monitor_stays_idle_and_shutdown_is_idempotent() {
        let reply_port = next_test_port();
        let monitor = VbanHealthMonitor::with_timing(reply_port, PROBE, ALERT_WINDOW);
        monitor.set_target(None);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(monitor.state(), VbanHealthState::Unknown);
        monitor.shutdown();
        monitor.shutdown(); // second shutdown is a no-op
    }

    /// A monitor loop without a running thread, for deterministic unit tests
    /// of the scheduling and threshold decisions. The channel's sender is
    /// returned as well, so a test can queue controls exactly the way racing
    /// `set_target` calls would.
    fn bare_loop() -> (Sender<HealthControl>, MonitorLoop) {
        let (tx, rx) = mpsc::channel();
        (
            tx,
            MonitorLoop {
                rx,
                shared: Arc::new(Mutex::new(HealthShared {
                    state: VbanHealthState::Unknown,
                    state_generation: 0,
                    generation: 0,
                    desired: None,
                    emitter: None,
                    last_alert: None,
                    alert_window: ALERT_WINDOW,
                })),
                emit_order: Arc::new(Mutex::new(())),
                reply_port: 0,
                probe_timeout: PROBE,
                target: None,
                socket: None,
                tracker: FailureTracker::new(FAST_THRESHOLD),
                next_probe_at: Instant::now(),
                request_id: 0,
                active_generation: 0,
                shutting_down: false,
            },
        )
    }

    /// Mirror the synchronous section of `set_target` for a receiver change
    /// or a disable: bump the generation, replace the desired target, reset
    /// the shared verdict to `Unknown` and publish the reset event. Unit
    /// tests use it to stage the exact state a racing `set_target` leaves
    /// behind before the monitor thread notices the control message.
    fn sync_retarget(shared: &mut HealthShared, target: Option<VbanPingTarget>, log: &EventLog) {
        shared.generation += 1;
        shared.desired = target;
        let events = shared.set_state(VbanHealthState::Unknown, Instant::now());
        log.lock().unwrap().extend(events);
    }

    #[test]
    fn interval_only_change_rebands_the_threshold_across_ten_seconds() {
        let (_tx, mut l) = bare_loop();
        // Fast band: a 2 s interval needs 3 consecutive misses.
        l.apply_target(Some(ping_target(1, Duration::from_secs(2))), 1);
        assert_eq!(l.tracker.threshold, FAST_THRESHOLD);
        l.tracker.record_failure();
        l.tracker.record_failure();
        // Slowed to 10 s: the threshold follows the new band (2) while the
        // count of consecutive misses is preserved — the next miss trips.
        l.apply_target(Some(ping_target(1, Duration::from_secs(10))), 2);
        assert_eq!(l.tracker.threshold, SLOW_THRESHOLD);
        assert_eq!(l.tracker.consecutive, 2);
        assert!(
            l.tracker.record_failure(),
            "the next miss trips the re-banded threshold"
        );
        // Sped back up below 10 s: 3 misses are required again.
        l.tracker.reset();
        l.apply_target(Some(ping_target(1, Duration::from_secs(9))), 3);
        assert_eq!(l.tracker.threshold, FAST_THRESHOLD);
        assert!(!l.tracker.record_failure());
        assert!(!l.tracker.record_failure());
        assert!(
            l.tracker.record_failure(),
            "the fast band still needs 3 misses"
        );
    }

    /// The drain → retarget → record race, deterministic: the probe on A has
    /// already passed its post-probe control drain (nothing was queued then)
    /// when a `set_target(B)` sneaks in — it resets the shared verdict and
    /// queues the control. The verdict for A must be dropped under the lock:
    /// no state change, no events, no tracker movement, and the queued
    /// control applies normally afterwards.
    #[test]
    fn stale_verdict_is_dropped_when_a_retarget_slips_past_the_drain() {
        let (tx, mut l) = bare_loop();
        let log: EventLog = Arc::new(Mutex::new(Vec::new()));

        // The loop is probing receiver A (generation 1); the receiver is
        // currently green.
        let target_a = ping_target(31_001, INTERVAL);
        l.apply_target(Some(target_a.clone()), 1);
        {
            let mut shared = l.shared.lock().unwrap();
            shared.generation = 1;
            shared.desired = Some(target_a.clone());
            shared.state = VbanHealthState::Alive;
            shared.state_generation = 1;
        }
        // One more miss trips the threshold: the next failed probe would
        // publish Failed + Alert if the race were unguarded.
        l.tracker.record_failure();
        l.tracker.record_failure();

        // The probe ended, drained nothing — and then the user retargeted to
        // B: the synchronous reset has already been published.
        let target_b = ping_target(31_002, INTERVAL);
        {
            let mut shared = l.shared.lock().unwrap();
            sync_retarget(&mut shared, Some(target_b.clone()), &log);
        }
        tx.send(HealthControl::Target(Some(target_b.clone()), 2))
            .unwrap();

        // The stale verdict for A must not resurrect the reset.
        l.record_probe(false, VbanHealthFailure::NoReply, target_a.addr, 1);
        {
            let shared = l.shared.lock().unwrap();
            assert_eq!(shared.state, VbanHealthState::Unknown, "reset survives");
            assert_eq!(shared.state_generation, shared.generation);
        }
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, VbanHealthEvent::Status(VbanHealthState::Unknown))),
            "the reset itself was published: {:?}",
            log.lock().unwrap()
        );
        assert!(
            !log.lock().unwrap().iter().any(|e| matches!(
                e,
                VbanHealthEvent::Status(VbanHealthState::Failed(_)) | VbanHealthEvent::Alert(_)
            )),
            "a stale Failed/Alert must never be published after the reset"
        );

        // The queued control still applies, and the loop continues on B.
        l.drain_pending_controls();
        assert_eq!(l.target.as_ref().map(|t| t.addr), Some(target_b.addr));
        assert_eq!(l.active_generation, 2);
        assert_eq!(
            l.tracker.consecutive, 0,
            "the receiver change reset counters"
        );
    }

    /// Same race against a disable: a reply that lands after `set_target(None)`
    /// must stay dropped — a disabled monitor is never turned green by a past
    /// target's reply, synchronously or from the loop.
    #[test]
    fn stale_reply_is_dropped_after_a_disable_slips_past_the_drain() {
        let (_tx, mut l) = bare_loop();
        let log: EventLog = Arc::new(Mutex::new(Vec::new()));

        let target_a = ping_target(31_003, INTERVAL);
        l.apply_target(Some(target_a.clone()), 1);
        {
            let mut shared = l.shared.lock().unwrap();
            shared.generation = 1;
            shared.desired = Some(target_a.clone());
        }

        // Disabled while the probe was finishing; the reset is published.
        {
            let mut shared = l.shared.lock().unwrap();
            sync_retarget(&mut shared, None, &log);
        }

        l.record_probe(true, VbanHealthFailure::NoReply, target_a.addr, 1);
        let shared = l.shared.lock().unwrap();
        assert_eq!(
            shared.state,
            VbanHealthState::Unknown,
            "a past target's reply must not turn a disabled monitor green"
        );
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .all(|e| matches!(e, VbanHealthEvent::Status(VbanHealthState::Unknown))),
            "no Alive status may follow the disable: {:?}",
            log.lock().unwrap()
        );
    }

    /// A local bind failure observed by a probe whose target was already
    /// superseded is dropped with the same rule; the next probe of the actual
    /// target re-reports a still-busy port on its own.
    #[test]
    fn stale_local_socket_error_is_dropped_after_a_retarget() {
        let (tx, mut l) = bare_loop();
        let log: EventLog = Arc::new(Mutex::new(Vec::new()));

        let target_a = ping_target(31_004, INTERVAL);
        let target_b = ping_target(31_005, INTERVAL);
        l.apply_target(Some(target_a.clone()), 1);
        {
            let mut shared = l.shared.lock().unwrap();
            shared.generation = 1;
            shared.desired = Some(target_a.clone());
        }
        {
            let mut shared = l.shared.lock().unwrap();
            sync_retarget(&mut shared, Some(target_b.clone()), &log);
        }
        tx.send(HealthControl::Target(Some(target_b.clone()), 2))
            .unwrap();

        l.record_local_socket_error("local UDP port 6980 is unavailable".into(), 1);
        let shared = l.shared.lock().unwrap();
        assert_eq!(
            shared.state,
            VbanHealthState::Unknown,
            "the superseded probe's local error must not be applied"
        );
        assert!(
            !log.lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, VbanHealthEvent::Status(VbanHealthState::Failed(_)))),
            "no stale local-socket failure after the reset: {:?}",
            log.lock().unwrap()
        );
    }

    /// Two `set_target` calls run concurrently; each sends its control after
    /// releasing the lock, so the loop can receive B (generation 3) before
    /// C (generation 2). The older control must be dropped: the loop never
    /// reverts to a target the user has already superseded.
    #[test]
    fn reordered_target_controls_are_applied_last_wins() {
        let (tx, mut l) = bare_loop();

        // Generation 1: the loop is on A. Then set_target(B) wins generation 2
        // and set_target(C) generation 3, but C's control is enqueued first.
        let target_a = ping_target(31_006, INTERVAL);
        l.apply_target(Some(target_a), 1);
        let target_b = ping_target(31_007, INTERVAL);
        let target_c = ping_target(31_008, INTERVAL);
        tx.send(HealthControl::Target(Some(target_c.clone()), 3))
            .unwrap();
        tx.send(HealthControl::Target(Some(target_b.clone()), 2))
            .unwrap();

        l.drain_pending_controls();
        assert_eq!(
            l.target.as_ref().map(|t| t.addr),
            Some(target_c.addr),
            "the highest generation wins regardless of arrival order"
        );
        assert_eq!(l.active_generation, 3);

        // A straggler from an even older request must not revert the loop.
        let target_old = ping_target(31_009, INTERVAL);
        tx.send(HealthControl::Target(Some(target_old), 1)).unwrap();
        l.drain_pending_controls();
        assert_eq!(l.target.as_ref().map(|t| t.addr), Some(target_c.addr));
        assert_eq!(l.active_generation, 3, "the stale control was skipped");
    }

    /// An interval-only change still preserves the verdict: the stored
    /// `Failed` verdict stays in place, re-tagged as current by the
    /// synchronous section, and no reset is published. A probe that was in
    /// flight under the older generation is discarded — the next probe (the
    /// scheduler pulls it to within the new interval) re-establishes
    /// freshness — so a result asked under an older desire never outlives
    /// that desire even when the receiver address is unchanged.
    #[test]
    fn interval_change_preserves_the_verdict_and_discards_the_in_flight_probe() {
        let (_tx, mut l) = bare_loop();
        let log: EventLog = Arc::new(Mutex::new(Vec::new()));

        let target_a = ping_target(31_010, INTERVAL);
        l.apply_target(Some(target_a.clone()), 1);
        {
            let mut shared = l.shared.lock().unwrap();
            shared.generation = 1;
            shared.desired = Some(target_a.clone());
            shared.state = VbanHealthState::Failed(VbanHealthFailure::NoReply);
            shared.state_generation = 1;
        }

        // The user slows the interval (same address): generation 2, the
        // verdict is re-tagged, no reset. The in-flight probe then replies.
        {
            let mut shared = l.shared.lock().unwrap();
            shared.generation = 2;
            shared.desired = Some(ping_target(31_010, INTERVAL * 4));
            shared.state_generation = 2;
        }

        l.record_probe(true, VbanHealthFailure::NoReply, target_a.addr, 1);
        let shared = l.shared.lock().unwrap();
        assert_eq!(
            shared.state,
            VbanHealthState::Failed(VbanHealthFailure::NoReply),
            "an interval-only change must keep the stored verdict"
        );
        assert_eq!(
            shared.state_generation, shared.generation,
            "the kept verdict carries the current generation"
        );
        assert!(
            log.lock().unwrap().is_empty(),
            "the discarded probe publishes nothing: {:?}",
            log.lock().unwrap()
        );
    }

    /// The reviewer's commit → retarget → emit permutation, made impossible:
    /// the verdict's commit and emission share the publication lock, so a
    /// retarget cannot commit — let alone publish its reset — while a
    /// committed verdict is being emitted. The retarget commits only after
    /// the verdict's publication finished, so the emitted order is exactly
    /// [Failed, Alert, Unknown]: the old verdict is never published after
    /// the reset. Signalled, not timed, for the invariant.
    #[test]
    fn verdict_committed_under_the_publication_lock_publishes_before_a_later_reset() {
        let (_tx, mut l) = bare_loop();
        let log: EventLog = Arc::new(Mutex::new(Vec::new()));

        // The loop is probing receiver A (generation 1); one miss below the
        // threshold, so this verdict would cross it. The emitter blocks
        // mid-publication until the test releases it.
        let target_a = ping_target(31_011, INTERVAL);
        l.apply_target(Some(target_a.clone()), 1);
        {
            let mut shared = l.shared.lock().unwrap();
            shared.generation = 1;
            shared.desired = Some(target_a.clone());
            shared.state = VbanHealthState::Alive;
            shared.state_generation = 1;
        }
        l.tracker.record_failure();
        l.tracker.record_failure();
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        {
            let emitter_log = Arc::clone(&log);
            let release_rx = Mutex::new(release_rx);
            let first_publication = Arc::new(AtomicBool::new(true));
            let first = Arc::clone(&first_publication);
            l.shared.lock().unwrap().emitter = Some(Arc::new(move |event| {
                // Only the first event parks mid-publication; the rest flow
                // once the release arrives.
                if first.swap(false, Ordering::SeqCst) {
                    let _ = entered_tx.send(());
                    release_rx
                        .lock()
                        .unwrap()
                        .recv()
                        .expect("the release signal arrives");
                }
                emitter_log.lock().unwrap().push(event);
            }));
        }

        // Clone the Arc fields while the loop mutex is still free; the record
        // thread holds it for the whole blocking emission below.
        let (emit_order, shared_arc) = {
            let loop_guard = &l;
            (
                Arc::clone(&loop_guard.emit_order),
                Arc::clone(&loop_guard.shared),
            )
        };
        let l = Arc::new(Mutex::new(l));
        let record_loop = Arc::clone(&l);
        let record_addr = target_a.clone();
        let record = std::thread::spawn(move || {
            record_loop.lock().unwrap().record_probe(
                false,
                VbanHealthFailure::NoReply,
                record_addr.addr,
                1,
            )
        });
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the record enters its emission");

        // A retarget shaped exactly like the real `set_target` commit
        // section (publication lock first) must wait for the emission. It
        // uses the Arcs cloned before the record started and never touches
        // the loop mutex the record holds.
        let target_b = ping_target(31_012, INTERVAL);
        let (committed_tx, committed_rx) = mpsc::channel::<()>();
        let retarget_log = Arc::clone(&log);
        let retarget = std::thread::spawn(move || {
            let _order = emit_order.lock().unwrap();
            let mut shared = shared_arc.lock().unwrap();
            sync_retarget(&mut shared, Some(target_b.clone()), &retarget_log);
            drop(shared);
            drop(_order);
            let _ = committed_tx.send(());
        });
        assert!(
            committed_rx
                .recv_timeout(Duration::from_millis(150))
                .is_err(),
            "a retarget must not commit while a committed verdict is being published"
        );

        release_tx.send(()).expect("the release is sent");
        record.join().expect("the record finishes");
        committed_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the retarget commits after the emission");
        retarget.join().expect("the retarget thread finishes");

        let logged = log.lock().unwrap().clone();
        assert_eq!(
            logged,
            vec![
                VbanHealthEvent::Status(VbanHealthState::Failed(VbanHealthFailure::NoReply)),
                VbanHealthEvent::Alert(VbanHealthState::Failed(VbanHealthFailure::NoReply)),
                VbanHealthEvent::Status(VbanHealthState::Unknown),
            ],
            "the committed verdict publishes before the later reset: {logged:?}"
        );
        assert_eq!(
            l.lock().unwrap().shared.lock().unwrap().state,
            VbanHealthState::Unknown,
            "the retarget's reset lands last"
        );
    }

    /// The opposite winner: a reset that took the publication slot first
    /// commits and publishes before the pending verdict can even acquire the
    /// lock. The verdict then sees the moved-on generation under the shared
    /// lock and is dropped whole — the stale Failed/Alert is never published
    /// after the reset.
    #[test]
    fn reset_winning_the_publication_slot_discards_the_pending_verdict() {
        let (tx, mut l) = bare_loop();
        let log: EventLog = Arc::new(Mutex::new(Vec::new()));

        let target_a = ping_target(31_013, INTERVAL);
        l.apply_target(Some(target_a.clone()), 1);
        {
            let mut shared = l.shared.lock().unwrap();
            shared.generation = 1;
            shared.desired = Some(target_a.clone());
            shared.state = VbanHealthState::Alive;
            shared.state_generation = 1;
        }
        l.tracker.record_failure();
        l.tracker.record_failure();

        // The publication slot belongs to the reset: hold the lock exactly
        // like the real `set_target` section does, commit the reset inside
        // it, and only then let the record take its turn. The loop mutex is
        // free while the slot is held — the record thread needs it.
        let l = Arc::new(Mutex::new(l));
        let (emit_order, shared_arc) = {
            let loop_guard = l.lock().unwrap();
            (
                Arc::clone(&loop_guard.emit_order),
                Arc::clone(&loop_guard.shared),
            )
        };
        let order_guard = emit_order.lock().unwrap();
        let target_b = ping_target(31_014, INTERVAL);
        {
            let mut shared = shared_arc.lock().unwrap();
            sync_retarget(&mut shared, Some(target_b.clone()), &log);
        }
        tx.send(HealthControl::Target(Some(target_b.clone()), 2))
            .unwrap();

        let (record_done, record_done_rx) = mpsc::channel::<()>();
        let record_loop = Arc::clone(&l);
        let record = std::thread::spawn(move || {
            record_loop.lock().unwrap().record_probe(
                false,
                VbanHealthFailure::NoReply,
                target_a.addr,
                1,
            );
            let _ = record_done.send(());
        });
        assert!(
            record_done_rx
                .recv_timeout(Duration::from_millis(150))
                .is_err(),
            "the record must wait for the publication slot before it can commit"
        );
        assert!(
            !log.lock().unwrap().iter().any(|e| matches!(
                e,
                VbanHealthEvent::Status(VbanHealthState::Failed(_)) | VbanHealthEvent::Alert(_)
            )),
            "the pending verdict publishes nothing while the slot is held"
        );

        drop(order_guard);
        record_done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the record completes once the slot frees");
        record.join().expect("the record thread finishes");

        let logged = log.lock().unwrap().clone();
        assert_eq!(
            logged,
            vec![VbanHealthEvent::Status(VbanHealthState::Unknown)],
            "only the reset was published: {logged:?}"
        );
        assert_eq!(
            shared_arc.lock().unwrap().state,
            VbanHealthState::Unknown,
            "the reset survives the discarded verdict"
        );
        assert_eq!(
            l.lock().unwrap().tracker.consecutive,
            2,
            "the discarded verdict must not move the tracker"
        );
    }

    /// A → B → A detour: a verdict asked under generation 1 must stay
    /// discarded even though the desired address is A again — the address
    /// alone cannot tell a detour from a live desire. The next probe of A
    /// (the current generation) applies normally.
    #[test]
    fn abba_verdict_from_a_detour_is_discarded_but_a_fresh_probe_applies() {
        let (tx, mut l) = bare_loop();
        let log: EventLog = Arc::new(Mutex::new(Vec::new()));

        let target_a = ping_target(31_015, INTERVAL);
        let target_b = ping_target(31_016, INTERVAL);
        l.apply_target(Some(target_a.clone()), 1);
        {
            let mut shared = l.shared.lock().unwrap();
            shared.generation = 1;
            shared.desired = Some(target_a.clone());
            // The receiver is green when the user starts retargeting.
            shared.state = VbanHealthState::Alive;
            shared.state_generation = 1;
            let recorder = Arc::clone(&log);
            shared.emitter = Some(Arc::new(move |event| {
                recorder.lock().unwrap().push(event);
            }));
        }

        // While the probe is in flight: retarget to B, then back to A. Both
        // accepted synchronously; only the first reset publishes (the second
        // finds the verdict already Unknown).
        {
            let mut shared = l.shared.lock().unwrap();
            sync_retarget(&mut shared, Some(target_b.clone()), &log);
            sync_retarget(&mut shared, Some(target_a.clone()), &log);
        }
        tx.send(HealthControl::Target(Some(target_b.clone()), 2))
            .unwrap();
        tx.send(HealthControl::Target(Some(target_a.clone()), 3))
            .unwrap();

        // The loop drains both controls: the highest generation wins and the
        // receiver is A again.
        l.drain_pending_controls();
        assert_eq!(l.active_generation, 3);
        assert_eq!(l.target.as_ref().map(|t| t.addr), Some(target_a.addr));

        // The generation-1 verdict for A is stale despite the address match.
        l.record_probe(false, VbanHealthFailure::NoReply, target_a.addr, 1);
        {
            let shared = l.shared.lock().unwrap();
            assert_eq!(
                shared.state,
                VbanHealthState::Unknown,
                "a detour verdict must not overwrite the resets"
            );
        }
        let logged = log.lock().unwrap().clone();
        assert_eq!(
            logged,
            vec![VbanHealthEvent::Status(VbanHealthState::Unknown)],
            "only the first retarget's reset was published: {logged:?}"
        );
        assert_eq!(
            l.tracker.consecutive, 0,
            "the stale verdict must not move the tracker"
        );

        // A fresh probe of A under the current generation applies normally.
        l.record_probe(true, VbanHealthFailure::NoReply, target_a.addr, 3);
        let shared = l.shared.lock().unwrap();
        assert_eq!(shared.state, VbanHealthState::Alive);
        assert_eq!(shared.state_generation, 3);
        let logged = log.lock().unwrap().clone();
        assert!(
            logged.ends_with(&[VbanHealthEvent::Status(VbanHealthState::Alive)]),
            "the fresh verdict is published: {logged:?}"
        );
        assert!(
            !logged.iter().any(|e| matches!(
                e,
                VbanHealthEvent::Status(VbanHealthState::Failed(_)) | VbanHealthEvent::Alert(_)
            )),
            "no stale verdict is published across the detour: {logged:?}"
        );
    }

    /// `set_target`'s reset commits atomically with its publication: while
    /// another publisher holds the publication lock, a retarget can neither
    /// commit (the verdict stays green) nor emit; once the slot frees, the
    /// reset lands in one piece.
    #[test]
    fn set_target_publishes_its_reset_atomically_with_the_commit() {
        let monitor = Arc::new(VbanHealthMonitor::with_timing(
            next_test_port(),
            PROBE,
            ALERT_WINDOW,
        ));
        let log = install_recorder(&monitor);
        let target_a = ping_target(31_017, INTERVAL);
        let target_b = ping_target(31_018, INTERVAL);
        monitor.set_target(Some(target_a));
        // Simulate a current green verdict for A.
        {
            let mut shared = monitor.shared.lock().unwrap();
            shared.state = VbanHealthState::Alive;
            shared.state_generation = shared.generation;
        }

        let (done, done_rx) = mpsc::channel::<()>();
        let order_guard = monitor.emit_order.lock().unwrap();
        let retarget_monitor = Arc::clone(&monitor);
        let retarget = std::thread::spawn(move || {
            retarget_monitor.set_target(Some(target_b));
            let _ = done.send(());
        });
        assert!(
            done_rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "the retarget must wait for the publication slot"
        );
        assert_eq!(
            monitor.state(),
            VbanHealthState::Alive,
            "the retarget cannot commit while the slot is held"
        );
        assert!(
            log.lock().unwrap().is_empty(),
            "no reset is published before the commit: {:?}",
            log.lock().unwrap()
        );

        drop(order_guard);
        retarget.join().expect("the retarget thread finishes");
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the retarget completes after the slot frees");
        assert_eq!(monitor.state(), VbanHealthState::Unknown);
        let logged = log.lock().unwrap().clone();
        assert_eq!(
            logged,
            vec![VbanHealthEvent::Status(VbanHealthState::Unknown)],
            "the reset publishes exactly once, with its commit: {logged:?}"
        );
        monitor.shutdown();
    }

    /// `notify_send_started`'s alert is also atomic with its commit: while
    /// the publication slot is held elsewhere, no alert is built or emitted;
    /// after the slot frees, exactly one alert publishes.
    #[test]
    fn notify_send_started_publishes_its_alert_atomically_with_the_commit() {
        let monitor = Arc::new(VbanHealthMonitor::with_timing(
            next_test_port(),
            PROBE,
            ALERT_WINDOW,
        ));
        let log = install_recorder(&monitor);
        {
            let mut shared = monitor.shared.lock().unwrap();
            shared.state = VbanHealthState::Failed(VbanHealthFailure::NoReply);
            shared.state_generation = shared.generation;
        }

        let (done, done_rx) = mpsc::channel::<bool>();
        let order_guard = monitor.emit_order.lock().unwrap();
        let notify_monitor = Arc::clone(&monitor);
        let notify = std::thread::spawn(move || {
            let alerted = notify_monitor.notify_send_started();
            let _ = done.send(alerted);
        });
        assert!(
            done_rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "the alert must wait for the publication slot"
        );
        assert!(
            !log.lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, VbanHealthEvent::Alert(_))),
            "no alert is emitted before the commit"
        );

        drop(order_guard);
        let alerted = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the alert completes after the slot frees");
        notify.join().expect("the notify thread finishes");
        assert!(alerted, "the alert publishes once the slot frees");
        assert_eq!(alert_count(&log), 1);
        monitor.shutdown();
    }

    #[test]
    fn send_start_alert_ignores_a_verdict_from_a_superseded_target() {
        let monitor = VbanHealthMonitor::with_timing(next_test_port(), PROBE, ALERT_WINDOW);
        {
            let mut shared = monitor.shared.lock().unwrap();
            shared.generation = 7;
            shared.state_generation = 5;
            shared.state = VbanHealthState::Failed(VbanHealthFailure::NoReply);
        }
        assert!(
            !monitor.notify_send_started(),
            "a verdict from a superseded target must stay silent"
        );
        // An interval-only change re-tags the verdict as current: alerts flow.
        monitor.shared.lock().unwrap().state_generation = 7;
        assert!(monitor.notify_send_started(), "the current verdict alerts");
        monitor.shutdown();
    }

    #[test]
    fn duplicate_set_target_is_ignored_synchronously() {
        let monitor = VbanHealthMonitor::with_timing(next_test_port(), PROBE, ALERT_WINDOW);
        let target = ping_target(65_000, INTERVAL);
        monitor.set_target(Some(target.clone()));
        let generation = monitor.shared.lock().unwrap().generation;
        assert!(generation >= 1, "the first request arms the monitor");
        monitor.set_target(Some(target));
        assert_eq!(
            monitor.shared.lock().unwrap().generation,
            generation,
            "an idempotent request must not re-arm the monitor"
        );
        monitor.shutdown();
    }

    #[test]
    fn disable_discards_the_in_flight_probe_verdict() {
        let monitor =
            VbanHealthMonitor::with_timing(next_test_port(), Duration::from_secs(5), ALERT_WINDOW);
        let log = install_recorder(&monitor);
        let late = LateResponder::start();
        monitor.set_target(Some(ping_target(late.port, INTERVAL)));
        wait_for_count(&late.requests, 1, Duration::from_secs(5));

        // Disabled while the probe is still waiting for its reply.
        monitor.set_target(None);
        assert_eq!(
            monitor.state(),
            VbanHealthState::Unknown,
            "disable resets the verdict synchronously"
        );
        assert!(
            !monitor.notify_send_started(),
            "a disabled target never alerts"
        );

        // The late reply lands inside the probe window, but it belongs to the
        // disabled target: the verdict is discarded, never emitted.
        late.release();
        wait_for_count(&late.replies, 1, Duration::from_secs(5));
        thread::sleep(Duration::from_millis(200));
        assert_eq!(monitor.state(), VbanHealthState::Unknown);
        assert!(
            !log.lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, VbanHealthEvent::Status(VbanHealthState::Alive))),
            "a stale reply must not turn the disabled monitor green"
        );
        drop(late);
        monitor.shutdown();
    }

    #[test]
    fn retarget_discards_the_in_flight_probe_verdict_and_probes_the_new_receiver() {
        let monitor =
            VbanHealthMonitor::with_timing(next_test_port(), Duration::from_secs(5), ALERT_WINDOW);
        let log = install_recorder(&monitor);
        let late = LateResponder::start();
        let dead = next_test_port();
        monitor.set_target(Some(ping_target(late.port, INTERVAL)));
        wait_for_count(&late.requests, 1, Duration::from_secs(5));

        // Retargeted to a dead receiver while the probe waits.
        monitor.set_target(Some(ping_target(dead, INTERVAL)));
        assert_eq!(monitor.state(), VbanHealthState::Unknown);

        // The stale reply must not turn the monitor green for the new receiver.
        late.release();
        wait_for_count(&late.replies, 1, Duration::from_secs(5));
        thread::sleep(Duration::from_millis(200));
        assert!(
            !log.lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, VbanHealthEvent::Status(VbanHealthState::Alive))),
            "a stale reply must never be applied to the new receiver"
        );

        // And the monitor continues with the new receiver to its own verdict.
        wait_for_state(
            &monitor,
            |state| matches!(state, VbanHealthState::Failed(VbanHealthFailure::NoReply)),
            Duration::from_secs(5),
        );
        drop(late);
        monitor.shutdown();
    }

    fn alert_count(log: &EventLog) -> usize {
        log.lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, VbanHealthEvent::Alert(_)))
            .count()
    }

    // --------------------------- shared-state unit ---------------------------

    #[test]
    fn alert_dedup_respects_the_window() {
        let mut shared = HealthShared {
            state: VbanHealthState::Unknown,
            state_generation: 0,
            generation: 0,
            desired: None,
            emitter: None,
            last_alert: None,
            alert_window: Duration::from_secs(5),
        };
        let failed = VbanHealthState::Failed(VbanHealthFailure::NoReply);
        let now = Instant::now();
        assert!(
            shared.alert_event(&failed, now).is_some(),
            "first alert passes"
        );
        assert!(
            shared
                .alert_event(&failed, now + Duration::from_secs(2))
                .is_none(),
            "inside the window the alert is suppressed"
        );
        assert!(
            shared
                .alert_event(&failed, now + Duration::from_secs(6))
                .is_some(),
            "after the window the alert passes again"
        );
    }

    #[test]
    fn state_transition_into_failed_emits_status_then_alert() {
        let mut shared = HealthShared {
            state: VbanHealthState::Alive,
            state_generation: 0,
            generation: 3,
            desired: None,
            emitter: Some(Arc::new(|_event| {
                panic!("events must be returned, never emitted under the lock");
            })),
            last_alert: None,
            alert_window: ALERT_WINDOW,
        };
        let failed = VbanHealthState::Failed(VbanHealthFailure::NoReply);
        let events = shared.set_state(failed.clone(), Instant::now());
        assert_eq!(
            events,
            vec![
                VbanHealthEvent::Status(failed.clone()),
                VbanHealthEvent::Alert(failed.clone()),
            ],
            "the transition reports status and one alert"
        );
        assert_eq!(
            shared.state_generation, 3,
            "tagged with the current generation"
        );
        // A repeated identical set_state is silent (the verdict is re-tagged).
        let events = shared.set_state(failed.clone(), Instant::now());
        assert!(events.is_empty(), "no duplicate events: {events:?}");
        assert_eq!(shared.state_generation, 3, "the verdict is re-tagged");
    }
}
