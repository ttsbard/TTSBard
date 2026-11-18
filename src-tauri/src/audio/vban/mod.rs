//! VBAN network audio output (ROADMAP-138): sends speech and SoundPanel clips
//! as one PCM Int16 LE / 48 kHz / stereo stream to a single IPv4 destination.
//!
//! One shared worker thread owns all PCM conversion, mixing and networking;
//! [`VbanOutput::start`] never blocks the local playback path. Speech and
//! SoundPanel each keep one active track and mix into the production stream in
//! real time; tests use a dedicated track on their own target.

pub mod health;
pub(crate) mod protocol;
pub(crate) mod worker;

use std::net::SocketAddrV4;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::audio::AudioPcm;

use health::VbanHealthMonitor;
use worker::{Control, TrackControl, TrackKind, TrackSpec, WorkerShared};

/// The test command waits for the worker's occupancy verdict; the worker
/// answers within one control-queue drain even under full load.
const TEST_REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// VBAN destination for one network stream.
#[derive(Clone, Debug, PartialEq)]
pub struct VbanTarget {
    pub addr: SocketAddrV4,
    pub stream_name: String,
    /// Applied to the summed mix of this stream before rounding and clipping.
    pub gain: f32,
}

/// Which production signal a track carries; one active instance per source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VbanSource {
    Speech,
    SoundPanel,
}

/// Local view of one track's send progress. `Finished` corresponds to the end
/// of the sounding time of the track's frames on the network, not to the
/// moment its samples were queued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VbanPlaybackStatus {
    Pending,
    Playing,
    Paused,
    Finished,
    Stopped,
    Failed(String),
}

struct OutputInner {
    /// `SyncSender` is `!Sync`, so the shared handle guards it behind a mutex;
    /// sends take microseconds and the worker drains its queue every iteration.
    tx: Mutex<SyncSender<Control>>,
    diagnostics: Arc<WorkerShared>,
    join: Mutex<Option<JoinHandle<()>>>,
    /// Caller-side mirror of the configured production target. `configure`
    /// and `start` read and enqueue under this lock, so an accepted `start`
    /// always pairs with the target it observed and is enqueued before any
    /// later `configure` — the worker's FIFO order then matches the mirror.
    production: Mutex<Option<VbanTarget>>,
    next_track: AtomicU64,
}

/// Shared VBAN runtime. Thread-safe, shared through `Arc` by the backend.
///
/// Owns two independent threads: the audio worker (PCM conversion, mixing,
/// networking) and the receiver-health monitor (ROADMAP-139), which probes the
/// saved destination on its own schedule and never blocks the audio path.
pub struct VbanOutput {
    inner: Arc<OutputInner>,
    health: VbanHealthMonitor,
}

/// Handle to one started track. Dropping it does not stop the sound; use
/// [`VbanPlayback::stop`] or output-wide shutdown/configure for that.
pub struct VbanPlayback {
    inner: Arc<PlaybackInner>,
}

struct PlaybackInner {
    track: u64,
    tx: Mutex<SyncSender<Control>>,
    /// Shared with the worker so its updates are visible through `status()`.
    status: Arc<Mutex<VbanPlaybackStatus>>,
}

impl VbanOutput {
    /// Create the runtime and spawn its worker thread. The instance never
    /// panics on worker failure; operations return errors instead.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(worker::CONTROL_QUEUE_CAPACITY);
        let diagnostics = Arc::new(Mutex::new(worker::WorkerDiagnostics::default()));
        let worker_diagnostics = Arc::clone(&diagnostics);
        let join = thread::Builder::new()
            .name("vban-output".to_string())
            .spawn(move || worker::run(rx, worker_diagnostics))
            .ok();
        Self {
            inner: Arc::new(OutputInner {
                tx: Mutex::new(tx),
                diagnostics,
                join: Mutex::new(join),
                production: Mutex::new(None),
                next_track: AtomicU64::new(1),
            }),
            health: VbanHealthMonitor::new(),
        }
    }

    /// Receiver availability monitor (ROADMAP-139). Independent of the audio
    /// worker; its target follows the saved settings, not the live audio one.
    pub fn health(&self) -> &VbanHealthMonitor {
        &self.health
    }

    /// Set or clear the production destination. Passing a different address,
    /// port or stream name cancels current production sound; a gain-only
    /// change applies from the next packet. Idempotent. The mirror update and
    /// the enqueue are one atomic step, so an already-accepted start can never
    /// be re-targeted by a later configure.
    pub fn configure(&self, target: Option<VbanTarget>) {
        let target = target.map(|mut target| {
            if !target.gain.is_finite() {
                target.gain = 0.0;
            }
            target
        });
        let mut production = self.inner.production.lock().expect("VBAN mirror poisoned");
        *production = target.clone();
        // Blocking send: command state must not be lost, and the worker's
        // bounded per-iteration work keeps the wait short even under load.
        let _ = self
            .inner
            .tx
            .lock()
            .expect("VBAN control channel poisoned")
            .send(Control::Configure(target));
        drop(production);
    }

    /// Whether production output is currently enabled with a saved target.
    /// Read at playback time, independently of a queued clip's settings snapshot.
    pub fn is_configured(&self) -> bool {
        self.inner
            .production
            .lock()
            .expect("VBAN mirror poisoned")
            .is_some()
    }

    /// Start a production track for `source`, replacing only that source's
    /// previous sound. Never blocks on conversion or networking: the worker
    /// converts and resamples off-thread in bounded chunks. Returns `Err` for
    /// unsupported PCM (anything but mono or stereo), a stopped worker, or an
    /// overloaded control queue (fail fast instead of hanging the local
    /// playback thread).
    pub fn start(
        &self,
        source: VbanSource,
        pcm: Arc<AudioPcm>,
        gain: f32,
    ) -> Result<VbanPlayback, String> {
        validate_pcm_profile(&pcm)?;
        if !gain.is_finite() {
            return Err("VBAN source gain must be finite".to_string());
        }
        let (track, status) = {
            // The mirror read and the enqueue are atomic: a start accepted
            // against the observed target is enqueued before any configure
            // that follows, so it can never be redirected to a new target.
            let production = self.inner.production.lock().expect("VBAN mirror poisoned");
            let target = production
                .clone()
                .ok_or_else(|| "VBAN output is not configured".to_string())?;
            let spec = self.make_spec(
                TrackKind::Production(source),
                (target.addr, target.stream_name),
                target.gain,
                pcm,
                gain,
            );
            let accepted = (spec.id, spec.status.clone());
            self.inner
                .tx
                .lock()
                .expect("VBAN control channel poisoned")
                .try_send(Control::Start(spec))
                .map_err(|error| match error {
                    TrySendError::Full(_) => "VBAN output control queue is overloaded".to_string(),
                    TrySendError::Disconnected(_) => "VBAN output is not running".to_string(),
                })?;
            accepted
        };
        Ok(self.playback_handle(track, status))
    }

    /// Start a dedicated test track on `target` with the form's own settings.
    /// Returns `Err` when production occupies the same network stream; if
    /// production starts on that stream mid-test, production wins and the test
    /// is stopped. Repeating a test replaces the previous test on the stream.
    pub fn test(&self, target: VbanTarget, pcm: Arc<AudioPcm>) -> Result<VbanPlayback, String> {
        validate_pcm_profile(&pcm)?;
        if !target.gain.is_finite() {
            return Err("VBAN test gain must be finite".to_string());
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let spec = self.make_spec(
            TrackKind::Test,
            (target.addr, target.stream_name),
            target.gain,
            pcm,
            1.0,
        );
        self.send_spec(Control::Test {
            spec: spec.clone(),
            reply: reply_tx,
        })?;
        match reply_rx.recv_timeout(TEST_REPLY_TIMEOUT) {
            Ok(Ok(())) => Ok(self.playback_handle(spec.id, spec.status)),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("VBAN worker did not respond to the test request".to_string()),
        }
    }

    /// Last production send error, for VBAN diagnostics; cleared when the
    /// destination changes or VBAN is disabled.
    pub fn last_error(&self) -> Option<String> {
        self.inner
            .diagnostics
            .lock()
            .expect("VBAN diagnostics poisoned")
            .last_error
            .clone()
    }

    /// Whether any production track is queued, sounding, or in its sent tail.
    pub fn is_sending(&self) -> bool {
        self.inner
            .diagnostics
            .lock()
            .expect("VBAN diagnostics poisoned")
            .sending
            > 0
    }

    /// Cancel every track and stop the worker and the health monitor.
    /// Subsequent operations fail until a new runtime is created.
    pub fn shutdown(&self) {
        self.health.shutdown();
        let mut production = self.inner.production.lock().expect("VBAN mirror poisoned");
        *production = None;
        let _ = self
            .inner
            .tx
            .lock()
            .expect("VBAN control channel poisoned")
            .send(Control::Shutdown);
        drop(production);
        if let Some(join) = self.inner.join.lock().expect("VBAN join poisoned").take() {
            let _ = join.join();
        }
    }

    fn make_spec(
        &self,
        kind: TrackKind,
        stream: worker::StreamKey,
        stream_gain: f32,
        pcm: Arc<AudioPcm>,
        track_gain: f32,
    ) -> TrackSpec {
        TrackSpec {
            id: self.inner.next_track.fetch_add(1, Ordering::Relaxed),
            kind,
            stream,
            stream_gain,
            pcm,
            track_gain,
            status: Arc::new(Mutex::new(VbanPlaybackStatus::Pending)),
        }
    }

    fn send_spec(&self, control: Control) -> Result<(), String> {
        self.inner
            .tx
            .lock()
            .expect("VBAN control channel poisoned")
            .send(control)
            .map_err(|_| "VBAN output is not running".to_string())
    }

    fn playback_handle(&self, track: u64, status: Arc<Mutex<VbanPlaybackStatus>>) -> VbanPlayback {
        VbanPlayback {
            inner: Arc::new(PlaybackInner {
                track,
                tx: Mutex::new(
                    self.inner
                        .tx
                        .lock()
                        .expect("VBAN control channel poisoned")
                        .clone(),
                ),
                status,
            }),
        }
    }
}

impl Default for VbanOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl VbanPlayback {
    pub fn pause(&self) {
        self.command(TrackControl::Pause);
    }

    pub fn resume(&self) {
        self.command(TrackControl::Resume);
    }

    pub fn stop(&self) {
        self.command(TrackControl::Stop);
    }

    /// Current status as reported by the worker.
    pub fn status(&self) -> VbanPlaybackStatus {
        self.inner
            .status
            .lock()
            .expect("VBAN track status poisoned")
            .clone()
    }

    fn command(&self, cmd: TrackControl) {
        // A dead worker already marked the track terminal; sending is
        // best-effort by design.
        let _ = self
            .inner
            .tx
            .lock()
            .expect("VBAN control channel poisoned")
            .send(Control::Command {
                track: self.inner.track,
                cmd,
            });
    }
}

fn validate_pcm_profile(pcm: &AudioPcm) -> Result<(), String> {
    if pcm.channels != 1 && pcm.channels != 2 {
        return Err(format!(
            "VBAN output supports mono or stereo PCM, got {} channels",
            pcm.channels
        ));
    }
    if pcm.sample_rate == 0 {
        return Err("VBAN output requires a positive sample rate".to_string());
    }
    if !pcm.samples.len().is_multiple_of(pcm.channels) {
        return Err("VBAN PCM samples are not frame-aligned".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, UdpSocket};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::protocol::decode_header;

    fn stereo_pcm(frames: usize, rate: u32, value: f32) -> AudioPcm {
        AudioPcm {
            samples: vec![value; frames * 2],
            sample_rate: rate,
            channels: 2,
        }
    }

    fn loopback_target(listener: &UdpSocket, name: &str, gain: f32) -> VbanTarget {
        VbanTarget {
            addr: SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr().unwrap().port()),
            stream_name: name.to_string(),
            gain,
        }
    }

    /// Full round trip through the real worker: a short phrase crosses UDP
    /// loopback with a sequential `nuFrame`, correct last-packet size and a
    /// `Finished` status that only appears after the tail was sent.
    #[test]
    fn udp_loopback_delivers_sequential_packets_and_finishes() {
        let listener = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback listener");
        listener
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();

        let output = VbanOutput::new();
        let target = loopback_target(&listener, "LOOP", 1.0);
        output.configure(Some(target));

        let pcm = Arc::new(stereo_pcm(300, 48_000, 0.5));
        let handle = output
            .start(VbanSource::Speech, pcm, 1.0)
            .expect("start production");

        // 300 frames: one full packet and a 44-frame tail.
        let mut sizes = Vec::new();
        let mut nu_frames = Vec::new();
        while nu_frames.len() < 2 {
            let mut buf = [0u8; 2048];
            let (len, _) = listener.recv_from(&mut buf).expect("packet arrives");
            let header = decode_header(&buf[..len]).expect("valid VBAN header");
            assert_eq!(header.stream_name, "LOOP");
            assert_eq!(header.channels, 2);
            assert_eq!(header.sample_rate_index, 3);
            assert_eq!(header.bit_format, 1);
            sizes.push(len);
            nu_frames.push(header.nu_frame);
        }
        assert_eq!(sizes, vec![1052, 28 + 44 * 4]);
        assert_eq!(nu_frames, vec![1, 2]);

        let deadline = Instant::now() + Duration::from_secs(2);
        while handle.status() != VbanPlaybackStatus::Finished {
            assert!(Instant::now() < deadline, "track never reached Finished");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!output.is_sending());
        output.shutdown();
    }

    /// A stopped track stops producing packets while the rest of the stream is
    /// unaffected, and statuses come back promptly from the worker.
    #[test]
    fn stop_is_prompt_and_halts_the_stream() {
        let listener = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback listener");
        listener
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();

        let output = VbanOutput::new();
        output.configure(Some(loopback_target(&listener, "STOP", 1.0)));
        let pcm = Arc::new(stereo_pcm(48_000, 48_000, 0.5)); // one full second
        let handle = output.start(VbanSource::Speech, pcm, 1.0).expect("start");

        let mut buf = [0u8; 2048];
        listener.recv_from(&mut buf).expect("first packet");
        handle.stop();

        let deadline = Instant::now() + Duration::from_secs(2);
        while handle.status() != VbanPlaybackStatus::Stopped {
            assert!(Instant::now() < deadline, "stop was not timely");
            thread::sleep(Duration::from_millis(2));
        }
        // Drain the packets already in flight when the stop was processed,
        // then require silence: no catch-up burst follows a stop.
        for _ in 0..64 {
            if listener.recv_from(&mut buf).is_err() {
                break;
            }
        }
        assert!(
            listener.recv_from(&mut buf).is_err(),
            "no packets follow a stop"
        );
        output.shutdown();
    }

    /// The test track reports occupancy conflicts with a live production
    /// stream and never reaches the production destination mixed with it.
    #[test]
    fn test_rejects_production_occupied_stream() {
        let output = VbanOutput::new();
        let listener = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback listener");
        let target = loopback_target(&listener, "CONF", 1.0);
        output.configure(Some(target.clone()));

        let pcm = Arc::new(stereo_pcm(48_000, 48_000, 0.5));
        let production = output
            .start(VbanSource::Speech, pcm.clone(), 1.0)
            .expect("start");
        let deadline = Instant::now() + Duration::from_secs(2);
        while production.status() != VbanPlaybackStatus::Playing {
            assert!(
                Instant::now() < deadline,
                "production never started sounding"
            );
            thread::sleep(Duration::from_millis(2));
        }
        let error = output
            .test(target, pcm)
            .err()
            .expect("test conflicts with live production");
        assert!(error.contains("production"), "unexpected error: {error}");
        output.shutdown();
        let _ = listener;
    }

    #[test]
    fn unsupported_channel_counts_are_rejected_upfront() {
        let output = VbanOutput::new();
        output.configure(Some(VbanTarget {
            addr: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 6980),
            stream_name: "X".to_string(),
            gain: 1.0,
        }));
        let pcm = Arc::new(AudioPcm {
            samples: vec![0.0; 6],
            sample_rate: 48_000,
            channels: 3,
        });
        let error = output
            .start(VbanSource::Speech, pcm, 1.0)
            .err()
            .expect("rejected");
        assert!(error.contains("channels"), "unexpected error: {error}");
        output.shutdown();
    }

    #[test]
    fn start_without_configuration_fails() {
        let output = VbanOutput::new();
        let pcm = Arc::new(stereo_pcm(10, 48_000, 0.0));
        assert!(output.start(VbanSource::Speech, pcm, 1.0).is_err());
        output.shutdown();
    }
}
