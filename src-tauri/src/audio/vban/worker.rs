//! VBAN worker thread: owns PCM conversion, mixing, packet pacing and the UDP
//! socket, so callers never block on network work from the local playback path.
//!
//! Pacing model: each network stream (`SocketAddrV4` + stream name) keeps one
//! monotonic send deadline and one growing `nuFrame` counter. All active tracks
//! on a stream share the stream's packet slots; each track keeps its own sample
//! cursor, so speech and SoundPanel clips mix in real time with independent
//! pause/stop control. Pacing is absolute: a late send still fulfils its
//! scheduled slot, so ordinary jitter neither stretches the stream nor bursts
//! out missed slots. Content overdue by more than [`OVERDUE_SKIP_THRESHOLD`]
//! is skipped instead of accumulating latency, and `Finished` is only reported
//! after the track's last frame has actually been sent at real-time pace.
//!
//! Conversion is gradual: non-48 kHz or mono sources convert in bounded chunks
//! per loop iteration (`pump_conversions`), so a long clip never starves the
//! mixer and control handling (stop/configure/shutdown) stays timely. Stereo
//! 48 kHz sources need no conversion at all and are shared zero-copy.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::protocol::{self, frame_duration, HEADER_SIZE, MAX_PACKET_FRAMES, MAX_PACKET_SIZE};
use super::{VbanPlaybackStatus, VbanSource, VbanTarget};
use crate::audio::AudioPcm;

/// Identity of one network stream: destination and stream name. All tracks on
/// the same key share packet slots and the `nuFrame` counter.
pub(crate) type StreamKey = (SocketAddrV4, String);

/// Capacity of the bounded control queue between API handles and the worker.
pub(crate) const CONTROL_QUEUE_CAPACITY: usize = 1024;

/// One control slot on the shared worker channel.
pub(crate) enum Control {
    Configure(Option<VbanTarget>),
    Start(TrackSpec),
    Test {
        spec: TrackSpec,
        reply: SyncSender<Result<(), String>>,
    },
    Command {
        track: u64,
        cmd: TrackControl,
    },
    Shutdown,
}

pub(crate) enum TrackControl {
    Pause,
    Resume,
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrackKind {
    Production(VbanSource),
    Test,
}

/// Description of one track handed to the worker. Heavy PCM conversion happens
/// on the worker thread, never at `start()` call time.
#[derive(Clone)]
pub(crate) struct TrackSpec {
    pub id: u64,
    pub kind: TrackKind,
    pub stream: StreamKey,
    /// Stream-level gain carried by the caller (test targets keep their own;
    /// production tracks always read the live configured gain instead).
    pub stream_gain: f32,
    pub pcm: Arc<AudioPcm>,
    /// Source gain (speech DSP volume, 1.0 for SoundPanel and tests).
    pub track_gain: f32,
    pub status: Arc<Mutex<VbanPlaybackStatus>>,
}

/// Diagnostics for the production output, read through `VbanOutput`.
#[derive(Default)]
pub(crate) struct WorkerDiagnostics {
    pub last_error: Option<String>,
    /// Production tracks currently pending or sounding (including tails
    /// already sent but not yet reported `Finished`). Test tracks never
    /// count: the diagnostics describe the production sender only.
    pub sending: usize,
}

pub(crate) type WorkerShared = Mutex<WorkerDiagnostics>;

/// Output audio converted per worker iteration for one still-converting track
/// (~100 ms of stream audio): bounded work per loop turn so a long clip never
/// starves the mixer or delays control handling.
const CONVERSION_CHUNK_FRAMES: usize = 4_800;

/// Overdue content is only dropped when the send schedule slipped by more
/// than this. The value sits far above worst-case Windows scheduling jitter
/// (~15 ms without timer cooperation), so ordinary jitter never trims fresh
/// audio; real stalls (>= 10 packet slots) skip ahead instead of accumulating
/// unbounded latency.
const OVERDUE_SKIP_THRESHOLD: Duration = Duration::from_millis(50);

/// Audio buffer of one track at the stream profile (stereo, 48 kHz).
enum TrackAudio {
    /// Owned buffer; grows chunk by chunk while the source is converting.
    Owned(Vec<f32>),
    /// Zero-copy passthrough of the source PCM: it is already stereo at
    /// 48 kHz, so no conversion or copy is needed.
    Shared(Arc<AudioPcm>),
}

struct TrackConversion {
    /// Original source PCM (mono at any rate, or stereo at a non-48 kHz rate).
    source: Arc<AudioPcm>,
    /// Stereo frames in the source.
    source_frames: usize,
}

struct TrackRuntime {
    id: u64,
    kind: TrackKind,
    stream: StreamKey,
    audio: TrackAudio,
    /// Incremental conversion state; `None` once the whole track is available.
    conversion: Option<TrackConversion>,
    /// Next stereo frame to send.
    cursor: usize,
    /// Stereo frames converted (and mixable) so far.
    produced_frames: usize,
    /// Final stereo frame count of the track at the stream profile.
    total_frames: usize,
    state: RunState,
    /// Source gain, applied to the mixed sum of this track during mixing.
    gain: f32,
    /// Gain of the destination stream at request time. Production tracks read
    /// the live configured gain instead; a test keeps the gain of its target.
    stream_gain: f32,
    status: Arc<Mutex<VbanPlaybackStatus>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RunState {
    /// Queued; becomes `Playing` when its first samples are actually sent.
    Pending,
    Playing,
    Paused,
}

struct StreamRuntime {
    /// Monotonic send deadline of the next packet; `None` while the stream has
    /// nothing to send.
    next_send: Option<Instant>,
    /// Last `nuFrame` value sent on this stream; incremented before each send.
    nu_frame: u32,
}

#[derive(Clone)]
struct Production {
    key: StreamKey,
    gain: f32,
}

/// Sounding tail of a track whose last packet was already sent: `Finished`
/// is reported once `deadline` passes. The entry keeps the stream and kind
/// for occupancy checks and the production-only diagnostics count.
struct FinishTail {
    deadline: Instant,
    /// Stream the tail still occupies; a test must not mix into it.
    stream: StreamKey,
    kind: TrackKind,
    status: Arc<Mutex<VbanPlaybackStatus>>,
}

/// One packet built by [`WorkerState::build_due_packets`]; the run loop owns
/// the socket and reports the send result back through
/// [`WorkerState::after_send`].
struct BuiltPacket {
    key: StreamKey,
    data: Vec<u8>,
    /// Track ids that contributed samples to this packet.
    participants: Vec<u64>,
    /// Frames each participant contributed, in participant order.
    takes: Vec<usize>,
    /// Window size of this packet in stereo frames.
    frames: usize,
    /// Wall-clock slot this packet fulfils: the stream deadline, or the
    /// resynchronized time right after overdue content was skipped.
    schedule_base: Instant,
}

struct WorkerState {
    socket: Option<UdpSocket>,
    diagnostics: Arc<WorkerShared>,
    production: Option<Production>,
    tracks: HashMap<u64, TrackRuntime>,
    /// Kept for the lifetime of the process per used stream so `nuFrame`
    /// continues across tracks, pauses, repeats and phrases.
    streams: HashMap<StreamKey, StreamRuntime>,
    /// Tracks whose last packet was sent: sounding until the tail completes.
    finishes: HashMap<u64, FinishTail>,
    mix_scratch: Vec<f32>,
    packet_scratch: Vec<u8>,
    disconnected: bool,
}

impl WorkerState {
    fn new(socket: Option<UdpSocket>, diagnostics: Arc<WorkerShared>) -> Self {
        Self {
            socket,
            diagnostics,
            production: None,
            tracks: HashMap::new(),
            streams: HashMap::new(),
            finishes: HashMap::new(),
            mix_scratch: Vec::new(),
            packet_scratch: vec![0u8; MAX_PACKET_SIZE],
            disconnected: false,
        }
    }

    /// Adjust the production-sender diagnostics count; test tracks never
    /// count toward `is_sending`.
    fn note_sending(&self, kind: TrackKind, delta: i64) {
        if !matches!(kind, TrackKind::Production(_)) {
            return;
        }
        let mut diag = self.diagnostics.lock().expect("VBAN diagnostics poisoned");
        diag.sending = (diag.sending as i64 + delta).max(0) as usize;
    }

    fn set_last_error(&self, error: String) {
        self.diagnostics
            .lock()
            .expect("VBAN diagnostics poisoned")
            .last_error = Some(error);
    }

    fn clear_last_error(&self) {
        self.diagnostics
            .lock()
            .expect("VBAN diagnostics poisoned")
            .last_error = None;
    }

    /// Process one control message. Returns `true` when the worker must stop.
    fn control(&mut self, msg: Control, now: Instant) -> bool {
        match msg {
            Control::Configure(target) => {
                self.configure(target);
                false
            }
            Control::Start(spec) => {
                self.start_production(spec, now);
                false
            }
            Control::Test { spec, reply } => {
                let result = self.start_test(spec, now);
                let _ = reply.send(result);
                false
            }
            Control::Command { track, cmd } => {
                self.track_command(track, cmd, now);
                false
            }
            Control::Shutdown => {
                self.shutdown_all();
                true
            }
        }
    }

    /// Applying a new destination cancels production sound; a gain-only change
    /// takes effect on the next mixed packet without interrupting anything.
    fn configure(&mut self, target: Option<VbanTarget>) {
        match target {
            None => {
                if self.production.take().is_some() {
                    self.cancel_production_tracks();
                    self.clear_last_error();
                }
            }
            Some(target) => {
                let key: StreamKey = (target.addr, target.stream_name);
                let gain = target.gain;
                match &mut self.production {
                    Some(current) if current.key == key => current.gain = gain,
                    _ => {
                        self.production = Some(Production { key, gain });
                        self.cancel_production_tracks();
                        self.clear_last_error();
                    }
                }
            }
        }
    }

    fn cancel_production_tracks(&mut self) {
        let ids: Vec<u64> = self
            .tracks
            .values()
            .filter(|t| matches!(t.kind, TrackKind::Production(_)))
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.remove_track(id, VbanPlaybackStatus::Stopped);
        }
    }

    fn cancel_tests_on(&mut self, key: &StreamKey) {
        let ids: Vec<u64> = self
            .tracks
            .values()
            .filter(|t| t.kind == TrackKind::Test && &t.stream == key)
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.remove_track(id, VbanPlaybackStatus::Stopped);
        }
    }

    /// Starting a production source cancels only that source's previous sound
    /// and preempts a test running on the same network stream.
    fn start_production(&mut self, spec: TrackSpec, now: Instant) {
        let Some(production) = self.production.clone() else {
            // The caller-side mirror prevents this; fail the track explicitly anyway.
            *spec.status.lock().expect("VBAN track status poisoned") =
                VbanPlaybackStatus::Failed("VBAN output is disabled".to_string());
            return;
        };
        self.cancel_tests_on(&production.key);
        self.cancel_source(spec.kind, spec.id);
        self.insert_track(spec, now);
    }

    fn cancel_source(&mut self, kind: TrackKind, except_id: u64) {
        let ids: Vec<u64> = self
            .tracks
            .values()
            .filter(|t| t.kind == kind && t.id != except_id)
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.remove_track(id, VbanPlaybackStatus::Stopped);
        }
    }

    /// A test needs a dedicated track: production occupying the same network
    /// stream wins — including the sounding tail of its last packet — so the
    /// request is rejected instead of mixed into it.
    fn start_test(&mut self, spec: TrackSpec, now: Instant) -> Result<(), String> {
        let production_live = self
            .tracks
            .values()
            .any(|t| matches!(t.kind, TrackKind::Production(_)) && t.stream == spec.stream);
        let production_tail = self.finishes.values().any(|tail| {
            matches!(tail.kind, TrackKind::Production(_)) && tail.stream == spec.stream
        });
        if production_live || production_tail {
            *spec.status.lock().expect("VBAN track status poisoned") = VbanPlaybackStatus::Failed(
                "VBAN production output is active on this stream".to_string(),
            );
            return Err("VBAN production output is active on this stream".to_string());
        }
        self.cancel_tests_on(&spec.stream);
        self.insert_track(spec, now);
        Ok(())
    }

    /// Queue the track without converting: stereo PCM at 48 kHz is shared
    /// zero-copy, everything else converts incrementally on the worker thread
    /// (`pump_conversions`), so `start()` and control handling stay cheap.
    fn insert_track(&mut self, spec: TrackSpec, now: Instant) {
        let pcm = Arc::clone(&spec.pcm);
        let source_frames = pcm.samples.len() / pcm.channels.max(1);
        let total_frames = if pcm.sample_rate == protocol::SAMPLE_RATE {
            source_frames
        } else {
            let out_frames = (source_frames as u64 * u64::from(protocol::SAMPLE_RATE))
                .div_ceil(u64::from(pcm.sample_rate));
            usize::try_from(out_frames).unwrap_or(usize::MAX)
        };
        if total_frames == 0 {
            *spec.status.lock().expect("VBAN track status poisoned") = VbanPlaybackStatus::Finished;
            return;
        }
        let (audio, conversion, produced_frames) =
            if pcm.channels == 2 && pcm.sample_rate == protocol::SAMPLE_RATE {
                (TrackAudio::Shared(pcm), None, total_frames)
            } else {
                (
                    TrackAudio::Owned(Vec::new()),
                    Some(TrackConversion {
                        source: pcm,
                        source_frames,
                    }),
                    0,
                )
            };
        // Fresh content must not be trimmed by schedule debt accumulated
        // before it was queued: drop the overdue blocks of the existing audio
        // and resynchronize, so the new track starts on time while the stream
        // never stretches.
        let overdue_debt = self
            .streams
            .get(&spec.stream)
            .and_then(|stream| stream.next_send)
            .filter(|deadline| now > *deadline + OVERDUE_SKIP_THRESHOLD)
            .map(|deadline| duration_to_frames(now - deadline));
        if let Some(overdue) = overdue_debt {
            let affected: Vec<u64> = self
                .tracks
                .values()
                .filter(|t| {
                    t.stream == spec.stream && t.state != RunState::Paused && t.unsent_total() > 0
                })
                .map(|t| t.id)
                .collect();
            let mut fully_skipped: Vec<u64> = Vec::new();
            for id in affected {
                if let Some(track) = self.tracks.get_mut(&id) {
                    track.cursor = track.cursor.saturating_add(overdue).min(track.total_frames);
                    if track.cursor == track.total_frames {
                        fully_skipped.push(id);
                    }
                }
            }
            for id in fully_skipped {
                if let Some(track) = self.tracks.remove(&id) {
                    self.note_sending(track.kind, -1);
                    *track.status.lock().expect("VBAN track status poisoned") =
                        VbanPlaybackStatus::Finished;
                }
            }
        }
        let stream = self
            .streams
            .entry(spec.stream.clone())
            .or_insert(StreamRuntime {
                next_send: None,
                nu_frame: 0,
            });
        if stream.next_send.is_none() || overdue_debt.is_some() {
            stream.next_send = Some(now);
        }
        self.tracks.insert(
            spec.id,
            TrackRuntime {
                id: spec.id,
                kind: spec.kind,
                stream: spec.stream,
                audio,
                conversion,
                cursor: 0,
                produced_frames,
                total_frames,
                state: RunState::Pending,
                gain: spec.track_gain,
                stream_gain: spec.stream_gain,
                status: spec.status,
            },
        );
        self.note_sending(spec.kind, 1);
    }

    /// Convert one bounded chunk per still-converting track. Called every loop
    /// iteration so a long clip converts gradually (~19x real time with the
    /// 100 ms chunk budget) while mixing and control handling keep running.
    fn pump_conversions(&mut self) {
        for track in self.tracks.values_mut() {
            let Some(conversion) = &track.conversion else {
                continue;
            };
            let pcm = &conversion.source;
            let channels = pcm.channels.max(1);
            let rate = u64::from(pcm.sample_rate);
            let target_end = track
                .produced_frames
                .saturating_add(CONVERSION_CHUNK_FRAMES)
                .min(track.total_frames);
            let TrackAudio::Owned(buffer) = &mut track.audio else {
                track.conversion = None;
                continue;
            };
            buffer.reserve((target_end - track.produced_frames) * protocol::CHANNELS);
            for out_index in track.produced_frames..target_end {
                if rate == u64::from(protocol::SAMPLE_RATE) {
                    // Mono at 48 kHz: duplicate the source frame.
                    let value = sanitize_sample(pcm.samples[out_index * channels]);
                    buffer.push(value);
                    buffer.push(value);
                } else {
                    let position = out_index as u64 * rate;
                    let left = usize::try_from(position / u64::from(protocol::SAMPLE_RATE))
                        .unwrap_or(usize::MAX);
                    let left = left.min(conversion.source_frames - 1);
                    let right = (left + 1).min(conversion.source_frames - 1);
                    let frac = (position % u64::from(protocol::SAMPLE_RATE)) as f32
                        / protocol::SAMPLE_RATE as f32;
                    for channel in 0..protocol::CHANNELS {
                        // Mono sources feed both output channels; stereo keeps
                        // its own channel. Only mono survives the 48 kHz branch
                        // above, so both layouts are possible here.
                        let source_channel = if channels == 1 { 0 } else { channel };
                        let a = sanitize_sample(pcm.samples[left * channels + source_channel]);
                        let b = sanitize_sample(pcm.samples[right * channels + source_channel]);
                        buffer.push(a + (b - a) * frac);
                    }
                }
            }
            track.produced_frames = target_end;
            if track.produced_frames >= track.total_frames {
                track.conversion = None;
            }
        }
    }

    fn remove_track(&mut self, id: u64, status: VbanPlaybackStatus) {
        if let Some(track) = self.tracks.remove(&id) {
            if track.state != RunState::Paused {
                self.note_sending(track.kind, -1);
            }
            *track.status.lock().expect("VBAN track status poisoned") = status;
            return;
        }
        // The track may already be in its sounding tail awaiting `Finished`.
        if let Some(tail) = self.finishes.remove(&id) {
            self.note_sending(tail.kind, -1);
            *tail.status.lock().expect("VBAN track status poisoned") = status;
        }
    }

    fn track_command(&mut self, id: u64, cmd: TrackControl, now: Instant) {
        match cmd {
            TrackControl::Pause => {
                if let Some(track) = self.tracks.get_mut(&id) {
                    if track.state != RunState::Paused {
                        track.state = RunState::Paused;
                        *track.status.lock().expect("VBAN track status poisoned") =
                            VbanPlaybackStatus::Paused;
                        let kind = track.kind;
                        self.note_sending(kind, -1);
                    }
                }
            }
            TrackControl::Resume => {
                let stream_key = self
                    .tracks
                    .get(&id)
                    .filter(|track| track.state == RunState::Paused)
                    .map(|track| track.stream.clone());
                if let Some(key) = stream_key {
                    let track = self.tracks.get_mut(&id).expect("track exists");
                    track.state = RunState::Playing;
                    *track.status.lock().expect("VBAN track status poisoned") =
                        VbanPlaybackStatus::Playing;
                    let kind = track.kind;
                    self.note_sending(kind, 1);
                    let stream = self
                        .streams
                        .get_mut(&key)
                        .expect("stream exists for live track");
                    if stream.next_send.is_none() {
                        stream.next_send = Some(now);
                    }
                }
            }
            TrackControl::Stop => self.remove_track(id, VbanPlaybackStatus::Stopped),
        }
    }

    fn shutdown_all(&mut self) {
        let ids: Vec<u64> = self.tracks.keys().copied().collect();
        for id in ids {
            self.remove_track(id, VbanPlaybackStatus::Stopped);
        }
        let tails: Vec<u64> = self.finishes.keys().copied().collect();
        for id in tails {
            if let Some(tail) = self.finishes.remove(&id) {
                self.note_sending(tail.kind, -1);
                *tail.status.lock().expect("VBAN track status poisoned") =
                    VbanPlaybackStatus::Stopped;
            }
        }
    }

    /// A converting track can temporarily have its cursor beyond the prepared
    /// buffer after an overdue skip. Keep pumping it on bounded timer ticks;
    /// clearing the deadline here would leave the worker blocked on recv().
    fn reschedule_unready_stream(&mut self, key: &StreamKey, now: Instant) {
        let converting = self.tracks.values().any(|track| {
            &track.stream == key
                && track.state != RunState::Paused
                && track.conversion.is_some()
                && track.unsent_total() > 0
        });
        if let Some(stream) = self.streams.get_mut(key) {
            stream.next_send = converting.then(|| now + frame_duration(MAX_PACKET_FRAMES));
        }
    }

    /// Build one packet for every stream whose send deadline has arrived.
    ///
    /// Mixes all unpaused tracks of a stream over the packet window, advances
    /// their cursors, and hands the encoded packet to the caller. Only
    /// [`WorkerState::after_send`] may commit pacing, counters and statuses,
    /// because only it knows the send result.
    fn build_due_packets(&mut self, now: Instant) -> Vec<BuiltPacket> {
        let due: Vec<StreamKey> = self
            .streams
            .iter()
            .filter(|(_, s)| matches!(s.next_send, Some(deadline) if deadline <= now))
            .map(|(k, _)| k.clone())
            .collect();

        let mut packets = Vec::with_capacity(due.len());
        for key in due {
            let mut ids: Vec<u64> = self
                .tracks
                .values()
                .filter(|t| t.stream == key && t.state != RunState::Paused && t.available() > 0)
                .map(|t| t.id)
                .collect();
            ids.sort_unstable();
            if ids.is_empty() {
                self.reschedule_unready_stream(&key, now);
                continue;
            }

            let (schedule_base, overdue_frames) = {
                let stream = self
                    .streams
                    .get(&key)
                    .expect("stream exists for due packet");
                match stream.next_send {
                    Some(deadline) if now > deadline + OVERDUE_SKIP_THRESHOLD => {
                        (now, duration_to_frames(now - deadline))
                    }
                    Some(deadline) => (deadline, 0),
                    None => unreachable!("due stream has a deadline"),
                }
            };
            if overdue_frames > 0 {
                // Substantially overdue blocks are skipped so repeated delays
                // do not stretch the stream or accumulate latency. Paused
                // tracks keep their position; a track whose remaining audio
                // falls entirely inside the skipped window is finished at once.
                let mut fully_skipped: Vec<u64> = Vec::new();
                for id in &ids {
                    if let Some(track) = self.tracks.get_mut(id) {
                        track.cursor = track
                            .cursor
                            .saturating_add(overdue_frames)
                            .min(track.total_frames);
                        if track.cursor == track.total_frames {
                            fully_skipped.push(*id);
                        }
                    }
                }
                for id in fully_skipped {
                    if let Some(track) = self.tracks.remove(&id) {
                        self.note_sending(track.kind, -1);
                        *track.status.lock().expect("VBAN track status poisoned") =
                            VbanPlaybackStatus::Finished;
                    }
                }
                ids.retain(|id| {
                    self.tracks
                        .get(id)
                        .is_some_and(|t| t.available() > 0 && t.unsent_total() > 0)
                });
                if ids.is_empty() {
                    self.reschedule_unready_stream(&key, now);
                    continue;
                }
            }

            let frames = ids
                .iter()
                .filter_map(|id| self.tracks.get(id))
                .map(TrackRuntime::available)
                .max()
                .expect("packet has participants")
                .min(MAX_PACKET_FRAMES);

            // Stream gain: production tracks always follow the live configured
            // gain; a test track keeps the gain of its test target.
            let gain = {
                let first = self.tracks.get(&ids[0]).expect("participants exist");
                match first.kind {
                    TrackKind::Production(_) => self
                        .production
                        .as_ref()
                        .map(|p| p.gain)
                        .unwrap_or(first.stream_gain),
                    TrackKind::Test => first.stream_gain,
                }
            };

            self.mix_scratch.clear();
            self.mix_scratch.resize(frames * protocol::CHANNELS, 0.0);
            let mut takes = Vec::with_capacity(ids.len());
            for id in &ids {
                let track = self.tracks.get_mut(id).expect("participant exists");
                let take = track.available().min(frames);
                let track_gain = track.gain;
                let from = track.cursor * protocol::CHANNELS;
                let to = from + take * protocol::CHANNELS;
                for (mixed, sample) in self.mix_scratch.iter_mut().zip(&track.stereo()[from..to]) {
                    *mixed += sample * track_gain;
                }
                track.cursor += take;
                takes.push(take);
            }

            let next_nu_frame = self
                .streams
                .get(&key)
                .expect("stream exists for due packet")
                .nu_frame
                .wrapping_add(1);
            let stream_name = key.1.clone();
            let packet_scratch = &mut self.packet_scratch;
            protocol::encode_header(packet_scratch, &stream_name, frames, next_nu_frame);
            let payload = protocol::quantize_payload(
                &self.mix_scratch,
                gain,
                &mut packet_scratch[HEADER_SIZE..],
            );
            packets.push(BuiltPacket {
                key,
                data: packet_scratch[..HEADER_SIZE + payload].to_vec(),
                participants: ids,
                takes,
                frames,
                schedule_base,
            });
        }
        packets
    }

    /// Commit the result of a send attempt: pacing and `nuFrame` advance only
    /// on success; a failed send fails the participating tracks and pauses the
    /// stream until the next start/resume.
    fn after_send(&mut self, packet: BuiltPacket, result: Result<(), String>, now: Instant) {
        let BuiltPacket {
            key,
            participants,
            takes,
            frames,
            schedule_base,
            ..
        } = packet;
        let production_packet = participants.iter().any(|id| {
            self.tracks
                .get(id)
                .is_some_and(|track| matches!(track.kind, TrackKind::Production(_)))
        });
        match result {
            Ok(()) => {
                if production_packet {
                    self.clear_last_error();
                }
                let interval = frame_duration(frames);
                if let Some(stream) = self.streams.get_mut(&key) {
                    stream.nu_frame = stream.nu_frame.wrapping_add(1);
                    // Absolute pacing: the next slot follows the schedule the
                    // packet fulfils, not the send moment, so ordinary jitter
                    // never stretches the stream. Skipped windows resynchronize
                    // through `schedule_base`.
                    stream.next_send = Some(schedule_base + interval);
                }
                for (id, take) in participants.iter().zip(&takes) {
                    if let Some(track) = self.tracks.get_mut(id) {
                        if track.state == RunState::Pending {
                            track.state = RunState::Playing;
                            *track.status.lock().expect("VBAN track status poisoned") =
                                VbanPlaybackStatus::Playing;
                        }
                        if track.cursor == track.total_frames {
                            // The tail inside this packet still sounds for
                            // `take` frames; report `Finished` when it ends.
                            let track = self.tracks.remove(id).expect("track exists");
                            self.finishes.insert(
                                track.id,
                                FinishTail {
                                    deadline: now + frame_duration(*take),
                                    stream: track.stream,
                                    kind: track.kind,
                                    status: track.status,
                                },
                            );
                        }
                    }
                }
            }
            Err(error) => {
                for id in participants {
                    self.remove_track(id, VbanPlaybackStatus::Failed(error.clone()));
                }
                if let Some(stream) = self.streams.get_mut(&key) {
                    stream.next_send = None;
                }
                if production_packet {
                    self.set_last_error(error);
                }
            }
        }
    }

    /// Report `Finished` for tracks whose sounding tail has elapsed.
    fn check_finishes(&mut self, now: Instant) {
        let due: Vec<u64> = self
            .finishes
            .iter()
            .filter(|(_, tail)| tail.deadline <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            if let Some(tail) = self.finishes.remove(&id) {
                self.note_sending(tail.kind, -1);
                *tail.status.lock().expect("VBAN track status poisoned") =
                    VbanPlaybackStatus::Finished;
            }
        }
    }

    /// Earliest future deadline across stream sends and track tails; all of
    /// them are at most one packet interval away while any work exists.
    fn next_deadline(&self) -> Option<Instant> {
        self.streams
            .values()
            .filter_map(|s| s.next_send)
            .chain(self.finishes.values().map(|tail| tail.deadline))
            .min()
    }
}

impl TrackRuntime {
    /// Stereo samples at the stream profile available to the mixer.
    fn stereo(&self) -> &[f32] {
        match &self.audio {
            TrackAudio::Owned(samples) => samples,
            TrackAudio::Shared(pcm) => &pcm.samples,
        }
    }

    /// Frames mixable right now; a still-converting track may lag behind its
    /// total until [`WorkerState::pump_conversions`] catches up.
    fn available(&self) -> usize {
        self.produced_frames.saturating_sub(self.cursor)
    }

    /// Frames still to send in total; the finish condition.
    fn unsent_total(&self) -> usize {
        self.total_frames.saturating_sub(self.cursor)
    }
}

/// Wall-clock duration expressed in stream frames.
fn duration_to_frames(duration: Duration) -> usize {
    usize::try_from((duration.as_nanos() * u128::from(protocol::SAMPLE_RATE)) / 1_000_000_000)
        .unwrap_or(usize::MAX)
}

/// Convert source PCM to the fixed stream profile: duplicate mono, keep
/// stereo, linear-resample to 48 kHz. Unsupported channel counts are rejected
/// by the API layer before reaching the worker.
fn sanitize_sample(sample: f32) -> f32 {
    if sample.is_finite() {
        sample
    } else {
        0.0
    }
}

/// Wait for the next deadline or the next control message. Control messages
/// always win over long waits; the last two milliseconds before a deadline use
/// high-resolution sleeps so packet pacing stays tight.
enum Wait {
    Deadline,
    Message(Control),
    Disconnected,
}

const FINE_WAIT: Duration = Duration::from_millis(2);

fn wait_until(rx: &Receiver<Control>, deadline: Option<Instant>) -> Wait {
    let Some(deadline) = deadline else {
        return match rx.recv() {
            Ok(msg) => Wait::Message(msg),
            Err(_) => Wait::Disconnected,
        };
    };
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Wait::Deadline;
        }
        let remaining = deadline - now;
        if remaining > FINE_WAIT {
            match rx.recv_timeout(remaining - FINE_WAIT) {
                Ok(msg) => return Wait::Message(msg),
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Wait::Disconnected,
            }
        }
        thread::sleep(remaining);
    }
}

/// Worker entry point. Binds one outgoing-only UDP socket and loops: drain
/// control messages, send due packets, then wait for the next deadline or
/// message. Every deadline is at most one packet interval ahead, so control
/// commands stay timely even while streams are saturating the schedule.
pub(crate) fn run(rx: Receiver<Control>, diagnostics: Arc<WorkerShared>) {
    let socket = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(socket) => {
            // Outgoing traffic only; a full send buffer skips packets instead
            // of accumulating a backlog of old blocks.
            let _ = socket.set_nonblocking(true);
            Some(socket)
        }
        Err(error) => {
            diagnostics
                .lock()
                .expect("VBAN diagnostics poisoned")
                .last_error = Some(format!("VBAN socket bind failed: {error}"));
            None
        }
    };

    let mut state = WorkerState::new(socket, diagnostics);
    loop {
        let mut halt = false;
        loop {
            match rx.try_recv() {
                Ok(msg) => {
                    if state.control(msg, Instant::now()) {
                        halt = true;
                        break;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    state.disconnected = true;
                    break;
                }
            }
        }
        if halt {
            break;
        }

        let now = Instant::now();
        state.pump_conversions();
        state.check_finishes(now);
        for packet in state.build_due_packets(now) {
            let result = match &state.socket {
                Some(socket) => socket
                    .send_to(&packet.data, packet.key.0)
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                None => Err("VBAN UDP socket is unavailable".to_string()),
            };
            state.after_send(packet, result, Instant::now());
        }

        let deadline = state.next_deadline();
        if state.disconnected {
            if deadline.is_none() {
                break;
            }
            if let Some(deadline) = deadline {
                let now = Instant::now();
                if deadline > now {
                    thread::sleep((deadline - now).min(FINE_WAIT));
                }
            }
            continue;
        }
        match wait_until(&rx, deadline) {
            Wait::Message(msg) => {
                if state.control(msg, Instant::now()) {
                    break;
                }
            }
            Wait::Deadline | Wait::Disconnected => {}
        }
    }
    // Reap anything still running so statuses never stay `Playing` after the
    // worker is gone.
    state.shutdown_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_arc() -> Arc<Mutex<VbanPlaybackStatus>> {
        Arc::new(Mutex::new(VbanPlaybackStatus::Pending))
    }

    fn status_of(status: &Arc<Mutex<VbanPlaybackStatus>>) -> VbanPlaybackStatus {
        status.lock().expect("status poisoned").clone()
    }

    fn spec(
        id: u64,
        kind: TrackKind,
        key: &StreamKey,
        pcm: AudioPcm,
        track_gain: f32,
    ) -> TrackSpec {
        TrackSpec {
            id,
            kind,
            stream: key.clone(),
            stream_gain: 1.0,
            pcm: Arc::new(pcm),
            track_gain,
            status: status_arc(),
        }
    }

    fn stereo(frames: usize, value: f32) -> AudioPcm {
        AudioPcm {
            samples: vec![value; frames * 2],
            sample_rate: 48_000,
            channels: 2,
        }
    }

    /// Send every due packet (with a fake socket that always succeeds) and
    /// return the last sent packet's wire bytes.
    fn step(state: &mut WorkerState, now: Instant) -> Option<Vec<u8>> {
        let packets = state.build_due_packets(now);
        let mut sent = None;
        for packet in packets {
            sent = Some(packet.data.clone());
            state.after_send(packet, Ok(()), now);
        }
        sent
    }

    fn decode(data: &[u8]) -> protocol::DecodedHeader {
        protocol::decode_header(data).expect("valid packet")
    }

    fn payload_i16(data: &[u8]) -> Vec<i16> {
        data[HEADER_SIZE..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect()
    }

    fn loopback_key(name: &str) -> StreamKey {
        (
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 6980),
            name.to_string(),
        )
    }

    fn configured_state(key: &StreamKey, gain: f32) -> WorkerState {
        let mut state = WorkerState::new(None, Arc::new(Mutex::new(WorkerDiagnostics::default())));
        state.configure(Some(VbanTarget {
            addr: key.0,
            stream_name: key.1.clone(),
            gain,
        }));
        state
    }

    #[test]
    fn converts_mono_to_stereo_and_resamples_exactly() {
        let key = loopback_key("CONV");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        let mono = AudioPcm {
            samples: vec![0.25, 0.5],
            sample_rate: 24_000,
            channels: 1,
        };
        state.control(
            Control::Start(spec(
                1,
                TrackKind::Production(VbanSource::Speech),
                &key,
                mono,
                1.0,
            )),
            now,
        );
        state.pump_conversions();
        let track = state.tracks.get(&1).expect("track queued");
        // 2 mono frames at 24 kHz -> 4 stereo frames at 48 kHz: the resampler
        // interpolates the ramp, then mono is duplicated to stereo. A source
        // this short fits into a single conversion chunk.
        assert_eq!(
            track.stereo().to_vec(),
            vec![0.25, 0.25, 0.375, 0.375, 0.5, 0.5, 0.5, 0.5]
        );
        assert!(
            track.conversion.is_none(),
            "tiny source converts in one chunk"
        );

        let stereo_44k1 = AudioPcm {
            samples: vec![0.0, 0.0, 1.0, 1.0],
            sample_rate: 44_100,
            channels: 2,
        };
        state.control(
            Control::Start(spec(
                2,
                TrackKind::Production(VbanSource::Speech),
                &key,
                stereo_44k1,
                1.0,
            )),
            now,
        );
        state.pump_conversions();
        let track = state.tracks.get(&2).expect("second track queued");
        // 2 frames * 48000 / 44100 rounds up to 3 stereo frames.
        assert_eq!(track.stereo().len(), 3 * protocol::CHANNELS);
        assert!(track.stereo().iter().all(|s| s.is_finite()));
    }

    #[test]
    fn incremental_conversion_streams_and_finishes() {
        let key = loopback_key("CONV2");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        // 12 000 mono frames at 24 kHz -> 24 000 stereo frames at 48 kHz:
        // conversion cannot finish in one loop iteration, so packets must
        // start before the whole source is converted.
        let mono = AudioPcm {
            samples: vec![0.5; 12_000],
            sample_rate: 24_000,
            channels: 1,
        };
        let track_spec = spec(
            1,
            TrackKind::Production(VbanSource::Speech),
            &key,
            mono,
            1.0,
        );
        let status = track_spec.status.clone();
        state.control(Control::Start(track_spec), now);

        let mut slot = now;
        let mut packets = 0usize;
        let mut last_frames = 0usize;
        loop {
            state.pump_conversions();
            state.check_finishes(slot);
            if status_of(&status) == VbanPlaybackStatus::Finished {
                break;
            }
            let data = step(&mut state, slot).expect("conversion keeps the stream fed");
            let header = decode(&data);
            assert!(header.samples_per_packet <= MAX_PACKET_FRAMES);
            last_frames = header.samples_per_packet;
            if packets == 0 {
                // Mono DC 0.5 is duplicated to stereo, then quantized.
                assert_eq!(payload_i16(&data)[0], 16_384);
            }
            packets += 1;
            assert!(packets < 200, "stream never finishes");
            slot += frame_duration(MAX_PACKET_FRAMES);
        }

        // 93 full packets plus a 192-frame tail.
        assert_eq!(packets, 94);
        assert_eq!(last_frames, 24_000 - 93 * MAX_PACKET_FRAMES);
        assert_eq!(
            state.streams.get(&key).expect("stream kept").nu_frame as usize,
            packets
        );
        assert_eq!(state.diagnostics.lock().unwrap().sending, 0);
    }

    #[test]
    fn mixes_two_sources_with_levels_and_clipping() {
        let key = loopback_key("MIX");
        let mut state = configured_state(&key, 0.5);
        let now = Instant::now();

        // Speech DC 1.0 with source gain 0.8; clip DC -0.5 with gain 1.0.
        state.control(
            Control::Start(spec(
                1,
                TrackKind::Production(VbanSource::Speech),
                &key,
                stereo(256, 1.0),
                0.8,
            )),
            now,
        );
        state.control(
            Control::Start(spec(
                2,
                TrackKind::Production(VbanSource::SoundPanel),
                &key,
                stereo(256, -0.5),
                1.0,
            )),
            now,
        );

        let data = step(&mut state, now).expect("packet due");
        assert_eq!(data.len(), MAX_PACKET_SIZE);
        // (1.0 * 0.8 + -0.5) * 0.5 = 0.15 -> 0.15 * 32767 = 4915.05 -> 4915
        assert_eq!(payload_i16(&data)[0], 4_915);

        // A clipped mix saturates instead of wrapping; speech finished with the
        // first packet, so the new phrase is alone.
        let slot = now + frame_duration(256);
        state.control(
            Control::Start(spec(
                3,
                TrackKind::Production(VbanSource::Speech),
                &key,
                stereo(256, 2.0),
                1.0,
            )),
            slot,
        );
        let data = step(&mut state, slot).expect("packet due");
        assert_eq!(
            payload_i16(&data)[0],
            32_767,
            "2.0 * 0.5 clips to full scale"
        );
    }

    #[test]
    fn pause_and_stop_of_one_source_leave_the_other_untouched() {
        let key = loopback_key("IND");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        // Speech is a ramp (frame i sounds (i as f32) / 1000), clip is DC 0.5.
        let mut ramp = Vec::with_capacity(1024 * 2);
        for frame in 0..1024 {
            let value = frame as f32 / 1000.0;
            ramp.push(value);
            ramp.push(value);
        }
        let speech_spec = spec(
            1,
            TrackKind::Production(VbanSource::Speech),
            &key,
            AudioPcm {
                samples: ramp,
                sample_rate: 48_000,
                channels: 2,
            },
            1.0,
        );
        let speech_status = speech_spec.status.clone();
        state.control(Control::Start(speech_spec), now);
        let clip_spec = spec(
            2,
            TrackKind::Production(VbanSource::SoundPanel),
            &key,
            stereo(1024, 0.5),
            1.0,
        );
        let clip_status = clip_spec.status.clone();
        state.control(Control::Start(clip_spec), now);

        let slot1 = now + frame_duration(256);
        let slot2 = slot1 + frame_duration(256);
        let slot3 = slot2 + frame_duration(256);

        let data = step(&mut state, now).expect("packet 1");
        let first = payload_i16(&data)[0] as f32 / 32_767.0;
        // Both sources at full level: ramp frame 0 (0.0) + clip (0.5).
        assert!((first - 0.5).abs() < 1.0 / 32_767.0);

        // Pause speech only: the clip keeps sounding at full level.
        state.control(
            Control::Command {
                track: 1,
                cmd: TrackControl::Pause,
            },
            slot1,
        );
        assert_eq!(status_of(&speech_status), VbanPlaybackStatus::Paused);
        let data = step(&mut state, slot1).expect("packet 2");
        let clip_only = payload_i16(&data)[0] as f32 / 32_767.0;
        assert!(
            (clip_only - 0.5).abs() < 1.0 / 32_767.0,
            "clip keeps its level while speech is paused"
        );
        assert_eq!(status_of(&clip_status), VbanPlaybackStatus::Playing);

        // Resume speech: it continues from its cursor (frame 256), not from zero.
        state.control(
            Control::Command {
                track: 1,
                cmd: TrackControl::Resume,
            },
            slot2,
        );
        let data = step(&mut state, slot2).expect("packet 3");
        let resumed = payload_i16(&data)[0] as f32 / 32_767.0;
        let expected = 256.0 / 1000.0 + 0.5;
        assert!(
            (resumed - expected).abs() < 1.0 / 32_767.0,
            "speech resumed at its cursor, got {resumed}"
        );

        // Stop the clip: speech continues alone.
        state.control(
            Control::Command {
                track: 2,
                cmd: TrackControl::Stop,
            },
            slot2,
        );
        assert_eq!(status_of(&clip_status), VbanPlaybackStatus::Stopped);
        let data = step(&mut state, slot3).expect("packet 4");
        let speech_only = payload_i16(&data)[0] as f32 / 32_767.0;
        let expected = 512.0 / 1000.0;
        assert!(
            (speech_only - expected).abs() < 1.0 / 32_767.0,
            "clip is gone after stop, got {speech_only}"
        );
    }

    #[test]
    fn nu_frame_is_sequential_across_tracks_pauses_and_phrases() {
        let key = loopback_key("SEQ");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        // Phrase 1: one full packet, then it ends on its own.
        state.control(
            Control::Start(spec(
                1,
                TrackKind::Production(VbanSource::Speech),
                &key,
                stereo(256, 0.1),
                1.0,
            )),
            now,
        );
        let data = step(&mut state, now).expect("phrase 1 packet");
        assert_eq!(decode(&data).nu_frame, 1);

        // Phrase 2 on the same stream: the counter keeps growing.
        let slot = now + frame_duration(256);
        state.control(
            Control::Start(spec(
                2,
                TrackKind::Production(VbanSource::Speech),
                &key,
                stereo(512, 0.1),
                1.0,
            )),
            slot,
        );
        let data = step(&mut state, slot).expect("phrase 2 packet");
        assert_eq!(decode(&data).nu_frame, 2);
        assert_eq!(state.streams.get(&key).unwrap().nu_frame, 2);

        // Pause the track: nothing is sent, the counter holds.
        state.control(
            Control::Command {
                track: 2,
                cmd: TrackControl::Pause,
            },
            slot,
        );
        assert!(step(&mut state, slot + frame_duration(256)).is_none());
        assert_eq!(state.streams.get(&key).unwrap().nu_frame, 2);

        // Resume: numbering continues where it stopped.
        let resume_at = slot + frame_duration(512);
        state.control(
            Control::Command {
                track: 2,
                cmd: TrackControl::Resume,
            },
            resume_at,
        );
        let data = step(&mut state, resume_at).expect("resumed packet");
        assert_eq!(decode(&data).nu_frame, 3);
    }

    #[test]
    fn last_packet_carries_the_exact_tail_size() {
        let key = loopback_key("TAIL");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        // 300 frames: one full packet plus a 44-frame tail.
        state.control(
            Control::Start(spec(
                1,
                TrackKind::Production(VbanSource::Speech),
                &key,
                stereo(300, 0.5),
                1.0,
            )),
            now,
        );
        let data = step(&mut state, now).expect("full packet");
        assert_eq!(data.len(), MAX_PACKET_SIZE);
        assert_eq!(decode(&data).samples_per_packet, 256);

        let slot = now + frame_duration(256);
        let data = step(&mut state, slot).expect("tail packet");
        assert_eq!(data.len(), protocol::packet_size(44));
        assert_eq!(decode(&data).samples_per_packet, 44);
        // The stream is idle now; no further packets are scheduled.
        assert!(step(&mut state, slot + frame_duration(256)).is_none());
    }

    #[test]
    fn finished_matches_the_sounding_time_of_the_tail() {
        let key = loopback_key("FIN");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        let track_spec = spec(
            1,
            TrackKind::Production(VbanSource::Speech),
            &key,
            stereo(300, 0.5),
            1.0,
        );
        let status = track_spec.status.clone();
        state.control(Control::Start(track_spec), now);
        step(&mut state, now).expect("full packet");
        assert_eq!(status_of(&status), VbanPlaybackStatus::Playing);

        // The tail packet (44 frames) is sent one slot later; Finished must
        // appear only after those 44 frames have finished sounding.
        let slot = now + frame_duration(256);
        step(&mut state, slot).expect("tail packet");
        assert_eq!(
            status_of(&status),
            VbanPlaybackStatus::Playing,
            "tail still sounds"
        );
        assert_eq!(
            state.diagnostics.lock().unwrap().sending,
            1,
            "a sounding production tail keeps the sender busy"
        );
        state.check_finishes(slot + frame_duration(44) - Duration::from_nanos(1));
        assert_eq!(status_of(&status), VbanPlaybackStatus::Playing);
        state.check_finishes(slot + frame_duration(44));
        assert_eq!(status_of(&status), VbanPlaybackStatus::Finished);
        assert_eq!(state.diagnostics.lock().unwrap().sending, 0);
    }

    #[test]
    fn replacing_a_source_cancels_only_that_source() {
        let key = loopback_key("REPL");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        let speech1 = spec(
            1,
            TrackKind::Production(VbanSource::Speech),
            &key,
            stereo(24_000, 0.5),
            1.0,
        );
        let speech1_status = speech1.status.clone();
        state.control(Control::Start(speech1), now);
        let clip = spec(
            2,
            TrackKind::Production(VbanSource::SoundPanel),
            &key,
            stereo(24_000, 0.25),
            1.0,
        );
        let clip_status = clip.status.clone();
        state.control(Control::Start(clip), now);

        // A new speech phrase stops speech 1 only; nothing was sent yet.
        let speech2 = spec(
            3,
            TrackKind::Production(VbanSource::Speech),
            &key,
            stereo(128, 0.9),
            1.0,
        );
        let speech2_status = speech2.status.clone();
        state.control(Control::Start(speech2), now);
        assert_eq!(status_of(&speech1_status), VbanPlaybackStatus::Stopped);
        assert_eq!(status_of(&clip_status), VbanPlaybackStatus::Pending);
        assert_eq!(status_of(&speech2_status), VbanPlaybackStatus::Pending);
        assert_eq!(state.tracks.len(), 2, "only speech 2 and the clip remain");
        assert_eq!(state.diagnostics.lock().unwrap().sending, 2);
    }

    #[test]
    fn production_start_preempts_a_test_on_the_same_stream() {
        let key = loopback_key("PRE");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        let test_spec = spec(1, TrackKind::Test, &key, stereo(24_000, 0.5), 1.0);
        let test_status = test_spec.status.clone();
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        state.control(
            Control::Test {
                spec: test_spec,
                reply: reply_tx,
            },
            now,
        );
        reply_rx
            .recv()
            .expect("test accepted")
            .expect("no conflict with idle production");
        let data = step(&mut state, now).expect("test packet");
        assert_eq!(decode(&data).nu_frame, 1);

        // Production starting on the same stream stops the test track.
        let slot = now + frame_duration(256);
        state.control(
            Control::Start(spec(
                2,
                TrackKind::Production(VbanSource::Speech),
                &key,
                stereo(256, 0.5),
                1.0,
            )),
            slot,
        );
        assert_eq!(status_of(&test_status), VbanPlaybackStatus::Stopped);
        assert_eq!(state.tracks.len(), 1, "only production remains");
        // The packet counter survived the preemption and keeps growing.
        let data = step(&mut state, slot).expect("packet");
        assert_eq!(decode(&data).nu_frame, 2);
    }

    #[test]
    fn test_rejects_a_production_occupied_stream_in_the_worker() {
        let key = loopback_key("OCC");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        state.control(
            Control::Start(spec(
                1,
                TrackKind::Production(VbanSource::Speech),
                &key,
                stereo(24_000, 0.5),
                1.0,
            )),
            now,
        );
        let test_spec = spec(2, TrackKind::Test, &key, stereo(128, 0.5), 1.0);
        let test_status = test_spec.status.clone();
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        state.control(
            Control::Test {
                spec: test_spec,
                reply: reply_tx,
            },
            now,
        );
        let error = reply_rx
            .recv()
            .expect("test answered")
            .expect_err("production occupies the stream");
        assert!(error.contains("production"), "unexpected error: {error}");
        assert_eq!(status_of(&test_status), VbanPlaybackStatus::Failed(error));
    }

    #[test]
    fn test_rejects_a_stream_with_a_sounding_production_tail() {
        let key = loopback_key("TAILOCC");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        // 300 frames: the second packet carries the 44-frame tail, and the
        // track moves to its sounding tail instead of reporting Finished.
        state.control(
            Control::Start(spec(
                1,
                TrackKind::Production(VbanSource::Speech),
                &key,
                stereo(300, 0.5),
                1.0,
            )),
            now,
        );
        step(&mut state, now).expect("full packet");
        let slot = now + frame_duration(256);
        step(&mut state, slot).expect("tail packet");

        // The track is gone but its tail still sounds: the stream stays
        // occupied, so the test must not mix into it.
        let test_spec = spec(2, TrackKind::Test, &key, stereo(128, 0.5), 1.0);
        let test_status = test_spec.status.clone();
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        state.control(
            Control::Test {
                spec: test_spec,
                reply: reply_tx,
            },
            slot,
        );
        let error = reply_rx
            .recv()
            .expect("test answered")
            .expect_err("sounding production tail occupies the stream");
        assert!(error.contains("production"), "unexpected error: {error}");
        assert_eq!(status_of(&test_status), VbanPlaybackStatus::Failed(error));

        // Once the tail has sounded, the same stream accepts the test again.
        let after_tail = slot + frame_duration(44);
        state.check_finishes(after_tail);
        assert_eq!(state.diagnostics.lock().unwrap().sending, 0);
        let test_spec = spec(3, TrackKind::Test, &key, stereo(128, 0.5), 1.0);
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        state.control(
            Control::Test {
                spec: test_spec,
                reply: reply_tx,
            },
            after_tail,
        );
        reply_rx
            .recv()
            .expect("test answered")
            .expect("test accepted after the tail sounded");
    }

    #[test]
    fn test_tracks_stay_out_of_production_sending_diagnostics() {
        let key = loopback_key("DIAG");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        let test_spec = spec(1, TrackKind::Test, &key, stereo(300, 0.5), 1.0);
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        state.control(
            Control::Test {
                spec: test_spec,
                reply: reply_tx,
            },
            now,
        );
        reply_rx
            .recv()
            .expect("test answered")
            .expect("test accepted");
        assert_eq!(
            state.diagnostics.lock().unwrap().sending,
            0,
            "a queued test track is not production output"
        );

        step(&mut state, now).expect("full packet");
        let slot = now + frame_duration(256);
        step(&mut state, slot).expect("tail packet");
        assert_eq!(
            state.diagnostics.lock().unwrap().sending,
            0,
            "a sounding test tail is not production output"
        );
    }

    #[test]
    fn send_failure_fails_participants_and_reports_production_error() {
        let key = loopback_key("FAIL");
        let mut state = configured_state(&key, 1.0);
        let now = Instant::now();

        let track_spec = spec(
            1,
            TrackKind::Production(VbanSource::Speech),
            &key,
            stereo(512, 0.5),
            1.0,
        );
        let status = track_spec.status.clone();
        state.control(Control::Start(track_spec), now);

        let packets = state.build_due_packets(now);
        assert_eq!(packets.len(), 1);
        let packet = packets.into_iter().next().unwrap();
        state.after_send(packet, Err("sendto failed".to_string()), now);
        assert_eq!(
            status_of(&status),
            VbanPlaybackStatus::Failed("sendto failed".to_string())
        );
        assert_eq!(
            state.diagnostics.lock().unwrap().last_error.as_deref(),
            Some("sendto failed"),
            "production send errors surface in diagnostics"
        );
        assert_eq!(state.diagnostics.lock().unwrap().sending, 0);
        // The stream pauses until the next start instead of retrying the old block.
        assert!(state
            .build_due_packets(now + frame_duration(256))
            .is_empty());
    }

    #[test]
    fn stalled_conversion_keeps_waking_until_track_finishes() {
        for source in [VbanSource::Speech, VbanSource::SoundPanel] {
            let key = loopback_key("STALL");
            let mut state = configured_state(&key, 1.0);
            let now = Instant::now();
            let track_spec = spec(
                1,
                TrackKind::Production(source),
                &key,
                AudioPcm {
                    samples: vec![0.5; 48_000],
                    sample_rate: 24_000,
                    channels: 1,
                },
                1.0,
            );
            let status = track_spec.status.clone();
            state.control(Control::Start(track_spec), now);
            state.pump_conversions();
            step(&mut state, now).expect("first packet");

            // The skipped cursor outruns the two conversion chunks prepared so
            // far. There is no other stream or tail to wake the worker for us.
            let mut slot = now + Duration::from_millis(500);
            state.pump_conversions();
            assert!(step(&mut state, slot).is_none());
            let track = state.tracks.get(&1).expect("unfinished track");
            assert!(track.cursor > track.produced_frames);
            assert_eq!(status_of(&status), VbanPlaybackStatus::Playing);

            let mut resumed_packets = 0;
            for _ in 0..1000 {
                slot = state.next_deadline().expect("conversion needs a wakeup");
                state.pump_conversions();
                state.check_finishes(slot);
                if step(&mut state, slot).is_some() {
                    resumed_packets += 1;
                }
                if status_of(&status) == VbanPlaybackStatus::Finished {
                    break;
                }
            }
            assert!(resumed_packets > 0, "sending resumes without a command");
            assert_eq!(status_of(&status), VbanPlaybackStatus::Finished);
            assert_eq!(state.diagnostics.lock().unwrap().sending, 0);
        }
    }

    #[test]
    fn production_retry_clears_error_but_test_sends_do_not() {
        let key = loopback_key("RECOVER");
        let mut state = configured_state(&key, 1.0);
        let mut now = Instant::now();
        state.control(
            Control::Start(spec(
                1,
                TrackKind::Production(VbanSource::Speech),
                &key,
                stereo(256, 0.5),
                1.0,
            )),
            now,
        );
        let packet = state.build_due_packets(now).pop().expect("first packet");
        state.after_send(packet, Err("temporary failure".to_string()), now);

        // Testing the saved stream while production is idle must neither
        // overwrite the production error on failure nor clear it on success.
        for (id, result) in [(2, Err("test failure".to_string())), (3, Ok(()))] {
            let test_spec = spec(id, TrackKind::Test, &key, stereo(256, 0.5), 1.0);
            state.start_test(test_spec, now).unwrap();
            let packet = state.build_due_packets(now).pop().expect("test packet");
            state.after_send(packet, result, now);
            now += frame_duration(256);
            state.check_finishes(now);
            assert_eq!(
                state.diagnostics.lock().unwrap().last_error.as_deref(),
                Some("temporary failure")
            );
        }

        let retry = spec(
            4,
            TrackKind::Production(VbanSource::Speech),
            &key,
            stereo(256, 0.5),
            1.0,
        );
        let retry_status = retry.status.clone();
        state.control(Control::Start(retry), now);
        step(&mut state, now).expect("successful production retry");
        state.check_finishes(now + frame_duration(256));
        assert_eq!(status_of(&retry_status), VbanPlaybackStatus::Finished);
        let diagnostics = state.diagnostics.lock().unwrap();
        assert_eq!(diagnostics.sending, 0);
        assert!(diagnostics.last_error.is_none());
    }
}
