//! Audio Player
//!
//! Общие функции воспроизведения: [`resolve_output_device`] (резолв id→Device с
//! кешем) и [`open_sink_on_device`] (OutputStream+Sink+WAV/MP3-детект+volume).
//! Используются и `PlaybackManager` (основной путь, plan 74), и `AudioPlayer`.
//!
//! `AudioPlayer` — блокирующий плеер для тестового звука (`test_audio_device`):
//! запускает фоновые потоки dual-output со stop_flag и join. Основной путь
//! воспроизведения фраз идёт через `playback::PlaybackManager`, который хранит
//! Sink и поддерживает очередь/pause/resume/seek.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::RwLock;
use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, OutputStream, Sink};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, trace, warn};

/// Keeps the selected output backend alive for the lifetime of its sink.
pub struct OutputDeviceStream {
    _rodio: Option<OutputStream>,
    _cpal: Option<cpal::Stream>,
    mode: OutputFormat,
    broken: Arc<AtomicBool>,
    stream_id: u64,
    created: Instant,
    device_name: String,
}

use crate::config::AudioOutputFormat as OutputFormat;

/// Monotonic id for audio_diag lifecycle logging of output streams.
static STREAM_ID: AtomicU64 = AtomicU64::new(0);

fn next_stream_id() -> u64 {
    STREAM_ID.fetch_add(1, Ordering::SeqCst)
}

impl OutputDeviceStream {
    /// The output format mode this stream was opened in.
    pub(crate) fn output_format(&self) -> OutputFormat {
        self.mode
    }

    /// Lifecycle id of this output stream (for audio_diag logging).
    pub(crate) fn stream_id(&self) -> u64 {
        self.stream_id
    }

    /// Whether the strict-i32 stream has hit its error callback. The default
    /// (rodio) mode cannot detect device loss and always reports `false`.
    pub(crate) fn is_broken(&self) -> bool {
        self.mode == OutputFormat::I32 && self.broken.load(Ordering::SeqCst)
    }

    /// audio_diag binding event: maps an opened stream id to its playback role
    /// and the configured device id (`None` = system default). Emitted by each
    /// caller right after a successful open, without changing open signatures.
    pub(crate) fn log_binding(&self, role: &str, device_id: Option<&str>) {
        debug!(target: "audio_diag", stream_id = self.stream_id(), role,
            device_id = ?device_id, "Output stream bound to role");
    }
}

// Share the SettingsManager cache: no separate mutable format override.
static OUTPUT_SETTINGS: OnceLock<Arc<RwLock<crate::config::AppSettings>>> = OnceLock::new();
static DEFAULT_STREAMS: AtomicUsize = AtomicUsize::new(0);
static I32_STREAMS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn init_output_settings(
    cache: Arc<RwLock<crate::config::AppSettings>>,
) -> Result<(), String> {
    OUTPUT_SETTINGS
        .set(cache)
        .map_err(|_| "Audio output settings already initialized".to_string())
}

pub(crate) fn effective_output_format() -> Result<OutputFormat, String> {
    OUTPUT_SETTINGS
        .get()
        .map(|cache| cache.read().audio.output_format)
        .ok_or_else(|| "Audio output settings not initialized".to_string())
}

fn stream_counter(mode: OutputFormat) -> &'static AtomicUsize {
    match mode {
        OutputFormat::Default => &DEFAULT_STREAMS,
        OutputFormat::I32 => &I32_STREAMS,
    }
}

/// Role tag derived from the legacy dual-player device label.
fn legacy_role(device_label: &str) -> &'static str {
    match device_label {
        "Virtual Mic" => "mic",
        "Test Device" => "test",
        _ => "speaker",
    }
}

impl Drop for OutputDeviceStream {
    fn drop(&mut self) {
        stream_counter(self.mode).fetch_sub(1, Ordering::SeqCst);
        let uptime = self.created.elapsed();
        debug!(target: "audio_diag", stream_id = self.stream_id, device_name = %self.device_name,
            mode = ?self.mode, uptime_ms = uptime.as_millis() as u64, broken = self.is_broken(),
            default_holders = DEFAULT_STREAMS.load(Ordering::SeqCst),
            i32_holders = I32_STREAMS.load(Ordering::SeqCst),
            "Output holder dropped");
    }
}

pub(crate) fn output_format_pending() -> Result<bool, String> {
    let opposite = match effective_output_format()? {
        OutputFormat::Default => OutputFormat::I32,
        OutputFormat::I32 => OutputFormat::Default,
    };
    Ok(stream_counter(opposite).load(Ordering::SeqCst) != 0)
}

fn select_i32_config(
    default: &cpal::SupportedStreamConfig,
    ranges: impl IntoIterator<Item = cpal::SupportedStreamConfigRange>,
) -> Result<cpal::SupportedStreamConfig, String> {
    ranges
        .into_iter()
        .find(|range| {
            range.sample_format() == cpal::SampleFormat::I32
                && range.channels() == default.channels()
                && range.min_sample_rate() <= default.sample_rate()
                && range.max_sample_rate() >= default.sample_rate()
        })
        .map(|range| range.with_sample_rate(default.sample_rate()))
        .ok_or_else(|| {
            format!(
                "No supported i32 output at {} Hz / {} channels; i32 mode does not fall back",
                default.sample_rate().0,
                default.channels()
            )
        })
}

/// Rodio changes an idle queue's source only in next(). Prefetch one sample
/// so the mixer sees its actual metadata, and end the converter's input frame
/// whenever channels/rate change. This keeps resampling continuous within a
/// source without reading or buffering a whole sound ahead of playback.
struct PrimedSinkQueue {
    queue: rodio::queue::SourcesQueueOutput<f32>,
    pending: Option<f32>,
    channels: u16,
    sample_rate: u32,
    frame_ended: std::cell::Cell<bool>,
}

impl PrimedSinkQueue {
    fn new(mut queue: rodio::queue::SourcesQueueOutput<f32>) -> Self {
        let pending = queue.next();
        Self {
            channels: rodio::Source::channels(&queue),
            sample_rate: rodio::Source::sample_rate(&queue),
            queue,
            pending,
            frame_ended: std::cell::Cell::new(false),
        }
    }
}

impl Iterator for PrimedSinkQueue {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.frame_ended.get() {
            return None;
        }
        let sample = self.pending.take()?;
        self.pending = self.queue.next();
        let channels = rodio::Source::channels(&self.queue);
        let sample_rate = rodio::Source::sample_rate(&self.queue);
        self.frame_ended
            .set(channels != self.channels || sample_rate != self.sample_rate);
        self.channels = channels;
        self.sample_rate = sample_rate;
        Some(sample)
    }
}

impl rodio::Source for PrimedSinkQueue {
    fn current_frame_len(&self) -> Option<usize> {
        // UniformSourceIterator calls this before rebuilding its converters.
        // Keep returning None from next() at a boundary until that happens;
        // a resampler can request several input samples while draining a frame.
        self.frame_ended.set(false);
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Reads persisted settings independently for each newly opened output.
/// Rodio 0.19 hides format fallback; the i32 branch opens CPAL directly so that
/// successful playback really uses the requested format. PCM and mixing stay f32.
pub(crate) fn open_output_sink(
    device: &cpal::Device,
) -> Result<(OutputDeviceStream, Sink), String> {
    let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
    let stream_id = next_stream_id();
    let created = Instant::now();
    let result = (|| {
        // Local copy: branches move the name into struct fields / callbacks
        // while the outer error log still needs the original.
        let device_name = device_name.clone();
        let mode = effective_output_format()?;
        let default = device
            .default_output_config()
            .map_err(|e| format!("Failed to get default output configuration: {e}"))?;
        info!(target: "audio_diag", stream_id, device_name = %device_name, mode = ?mode,
            sample_format = ?default.sample_format(), sample_rate = default.sample_rate().0,
            channels = default.channels(), buffer_size = ?default.buffer_size(),
            "Audio output default configuration (requested)");

        let ranges = match device.supported_output_configs() {
            Ok(ranges) => ranges.collect::<Vec<_>>(),
            Err(e) => {
                warn!(target: "audio_diag", stream_id, device_name = %device_name, error = %e,
                    "Cannot enumerate supported output formats");
                if mode == OutputFormat::I32 {
                    return Err(format!(
                        "Cannot enumerate formats for strict i32 output: {e}"
                    ));
                }
                Vec::new()
            }
        };
        for range in &ranges {
            trace!(target: "audio_diag", device_name = %device_name, sample_format = ?range.sample_format(),
                min_sample_rate = range.min_sample_rate().0, max_sample_rate = range.max_sample_rate().0,
                channels = range.channels(), buffer_size = ?range.buffer_size(),
                "Audio output supported configuration");
        }

        if mode == OutputFormat::Default {
            let (stream, handle) = OutputStream::try_from_device_config(device, default.clone())
                .map_err(|e| format!("Failed to create output stream: {e}"))?;
            let sink = Sink::try_new(&handle).map_err(|e| format!("Failed to create sink: {e}"))?;
            stream_counter(mode).fetch_add(1, Ordering::SeqCst);
            info!(target: "audio_diag", stream_id, device_name = %device_name,
                mode = ?mode,
                requested_sample_format = ?default.sample_format(),
                requested_sample_rate = default.sample_rate().0,
                requested_channels = default.channels(),
                requested_buffer_size = ?default.buffer_size(),
                actual_format_available = false,
                default_holders = DEFAULT_STREAMS.load(Ordering::SeqCst),
                i32_holders = I32_STREAMS.load(Ordering::SeqCst),
                "Output stream open: default mode (rodio 0.19 may silently fall back, actual format is unavailable; fields are requested)");
            return Ok((
                OutputDeviceStream {
                    _rodio: Some(stream),
                    _cpal: None,
                    mode,
                    broken: Arc::new(AtomicBool::new(false)),
                    stream_id,
                    created,
                    device_name: device_name.clone(),
                },
                sink,
            ));
        }

        let selected = select_i32_config(&default, ranges)?;
        let (controller, mut mixer) =
            rodio::dynamic_mixer::mixer::<f32>(selected.channels(), selected.sample_rate().0);
        let (sink, queue) = Sink::new_idle();
        controller.add(PrimedSinkQueue::new(queue));
        let broken = Arc::new(AtomicBool::new(false));
        let broken_cb = Arc::clone(&broken);
        let cb_stream_id = stream_id;
        let cb_device_name = device_name.clone();
        let stream = device
            .build_output_stream::<i32, _, _>(
                &selected.config(),
                move |data, _| {
                    for sample in data {
                        *sample = <i32 as cpal::Sample>::from_sample(mixer.next().unwrap_or(0.0));
                    }
                },
                // The error callback may run on the audio stream thread (WASAPI),
                // so keep it minimal.
                move |err| {
                    broken_cb.store(true, Ordering::SeqCst);
                    warn!(target: "audio_diag", stream_id = cb_stream_id,
                        device_name = %cb_device_name, error = %err,
                        "Strict i32 output stream error callback");
                },
                None,
            )
            .map_err(|e| format!("Failed to create strict i32 output stream (no fallback): {e}"))?;
        stream
            .play()
            .map_err(|e| format!("Failed to start strict i32 output stream: {e}"))?;
        stream_counter(mode).fetch_add(1, Ordering::SeqCst);
        info!(target: "audio_diag", stream_id, device_name = %device_name,
            mode = ?mode,
            requested_default_sample_format = ?default.sample_format(),
            requested_default_sample_rate = default.sample_rate().0,
            requested_default_channels = default.channels(),
            requested_default_buffer_size = ?default.buffer_size(),
            selected_sample_format = ?selected.sample_format(),
            selected_sample_rate = selected.sample_rate().0,
            selected_channels = selected.channels(),
            selected_buffer_size = ?selected.config().buffer_size,
            actual_format_available = true,
            default_holders = DEFAULT_STREAMS.load(Ordering::SeqCst),
            i32_holders = I32_STREAMS.load(Ordering::SeqCst),
            "Output stream open: verified i32 explicit configuration (no fallback)");
        Ok((
            OutputDeviceStream {
                _rodio: None,
                _cpal: Some(stream),
                mode,
                broken,
                stream_id,
                created,
                device_name,
            },
            sink,
        ))
    })();
    if let Err(ref err) = result {
        error!(target: "audio_diag", stream_id, device_name = %device_name, error = %err,
            "Audio output initialization failed");
    }
    result.map_err(|err| format!("Output device '{device_name}': {err}"))
}

/// Конфигурация вывода звука
#[derive(Clone, Debug)]
pub struct OutputConfig {
    pub device_id: Option<String>, // None = устройство по умолчанию
    pub volume: f32,               // 0.0 - 1.0
}

/// Резолв device_id (строка-индекс или None=default) в cpal::Device с кешем.
/// Возвращает Result с человекочитаемой ошибкой.
pub fn resolve_output_device(
    device_id: &Option<String>,
    cached: &Option<Arc<RwLock<HashMap<String, cpal::Device>>>>,
) -> Result<cpal::Device, String> {
    trace!(target: "audio_diag", requested_device_id = ?device_id, "Resolving output device");
    match device_id {
        Some(dev_id) => {
            if let Some(cache) = cached {
                let c = cache.read();
                if let Some(device) = c.get(dev_id) {
                    let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
                    trace!(target: "audio_diag", requested_device_id = %dev_id, device_name = %device_name,
                        "Output device cache hit");
                    debug!(device_name = %device_name, "Using cached device");
                    let device_clone = device.clone();
                    drop(c);
                    return Ok(device_clone);
                }
                drop(c);
            }
            trace!(target: "audio_diag", requested_device_id = %dev_id, "Output device cache miss");
            let index: usize = dev_id
                .parse()
                .map_err(|_| {
                    warn!(target: "audio_diag", requested_device_id = %dev_id, "Invalid output device ID");
                    format!("Invalid device ID: {}", dev_id)
                })?;
            let host = cpal::default_host();
            let mut output_devices = host.output_devices().map_err(|e| {
                warn!(target: "audio_diag", requested_device_id = %dev_id, error = %e,
                    "Failed to enumerate output devices");
                format!("Failed to get output devices: {}", e)
            })?;
            let device = if tracing::enabled!(target: "audio_diag", tracing::Level::DEBUG) {
                // Same enumeration order as the plain path; collected only for debug logging.
                let devices: Vec<_> = output_devices.collect();
                for (i, d) in devices.iter().enumerate() {
                    let name = d.name().unwrap_or_else(|_| "Unknown".to_string());
                    debug!(target: "audio_diag", index = i, device_name = %name,
                        "Enumerated CPAL output device");
                }
                devices.into_iter().nth(index)
            } else {
                output_devices.nth(index)
            };
            let device = device.ok_or_else(|| {
                warn!(target: "audio_diag", requested_device_id = %dev_id, index, "Output device not found");
                format!("Device not found: {}", dev_id)
            })?;
            let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
            debug!(target: "audio_diag", requested_device_id = %dev_id,
                resolved_device_name = %device_name, "Output device resolved by index");
            Ok(device)
        }
        None => {
            let host = cpal::default_host();
            let device = host
                .default_output_device()
                .ok_or_else(|| {
                    warn!(target: "audio_diag", requested_device_id = "default", "No default output device");
                    "No default output device".to_string()
                })?;
            let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
            debug!(target: "audio_diag", default_device_name = %device_name,
                "Using default output device");
            Ok(device)
        }
    }
}

/// Создаёт OutputStream + Sink на устройстве, декодирует data (MP3/WAV auto),
/// выставляет volume, append source. Возвращает (OutputStream, Sink).
/// НЕ ждёт завершения, НЕ трогает потоки — чистая синхронная операция.
pub fn open_sink_on_device(
    device: &cpal::Device,
    data: &[u8],
    volume: f32,
) -> Result<(OutputDeviceStream, Sink), String> {
    let (_stream, sink) = open_output_sink(device)?;

    let is_wav = data.len() > 4 && &data[0..4] == b"RIFF";
    let format_name = if is_wav { "WAV" } else { "MP3" };
    debug!(
        "Detected {} format, decoding with rodio::Decoder",
        format_name
    );

    let cursor = Cursor::new(data.to_vec());
    let source = Decoder::new(cursor).map_err(|e| format!("Failed to decode audio: {}", e))?;

    sink.set_volume(volume);
    debug!(volume = volume, "Volume set");

    sink.append(source);
    Ok((_stream, sink))
}

/// Play interleaved PCM into an existing `Sink` without opening a stream.
/// Builds a `SamplesBuffer` from `AudioPcm` (channels/rate taken from the PCM),
/// sets the sink volume, and appends. Never opens a stream or touches the mode
/// counters — used to reuse a persistent sink across phrases.
pub(crate) fn play_pcm_in_sink(sink: &Sink, pcm: &crate::audio::AudioPcm, volume: f32) {
    debug!(
        channels = pcm.channels,
        sample_rate = pcm.sample_rate,
        frames = pcm.frame_count(),
        "Creating PCM SamplesBuffer source"
    );

    let source = SamplesBuffer::new(pcm.channels as u16, pcm.sample_rate, pcm.samples.clone());

    sink.set_volume(volume);
    debug!(volume, "Volume set");

    sink.append(source);
}

/// Create OutputStream + Sink on device from interleaved PCM f32 samples.
/// Uses `rodio::buffer::SamplesBuffer` instead of WAV/MP3 decoder.
pub fn open_sink_on_device_pcm(
    device: &cpal::Device,
    pcm: &crate::audio::AudioPcm,
    volume: f32,
) -> Result<(OutputDeviceStream, Sink), String> {
    let (_stream, sink) = open_output_sink(device)?;
    play_pcm_in_sink(&sink, pcm, volume);
    Ok((_stream, sink))
}

/// Аудио плеер с поддержкой dual output
pub struct AudioPlayer {
    stop_flag: Arc<AtomicBool>,
    /// Хранит handle предыдущих потоков для корректного завершения
    active_threads: Vec<JoinHandle<()>>,
}

impl AudioPlayer {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            active_threads: Vec::new(),
        }
    }

    /// Остановить предыдущую сессию и открыть новую.
    ///
    /// Каждая сессия получает собственный `Arc<AtomicBool>`: старые потоки,
    /// не завершившиеся за `JOIN_TIMEOUT`, остаются со своим установленным
    /// флагом и не могут сбросить в `false` флаг новой сессии.
    fn begin_playback_session(&mut self) -> Arc<AtomicBool> {
        // Останавливаем предыдущее воспроизведение
        self.stop_flag.store(true, Ordering::SeqCst);

        // Ждём завершения старых потоков с реальным таймаутом
        const JOIN_TIMEOUT: Duration = Duration::from_secs(1);
        let deadline = Instant::now() + JOIN_TIMEOUT;
        let mut still_active = Vec::new();

        for handle in self.active_threads.drain(..) {
            if !handle.is_finished() {
                still_active.push(handle);
            }
        }

        // Присоединяем потоки с таймаутом
        for handle in still_active {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                error!("Join timeout reached, some threads may still be running");
                break;
            }
            // Примечание: join() блокирующий, но это допустимо для десктопного приложения
            // Потоки должны быстро завершаться после установки своего stop_flag
            if let Err(e) = handle.join() {
                error!(error = ?e, "Thread join error");
            }
        }

        // Новая сессия не разделяет флаг с незавершёнными старыми потоками
        let session_stop_flag = Arc::new(AtomicBool::new(false));
        self.stop_flag = session_stop_flag.clone();
        session_stop_flag
    }

    /// Воспроизвести MP3 данные асинхронно на одно или два устройства
    /// Uses Arc to share audio data efficiently between multiple outputs
    /// cached_devices: Optional cache of audio devices to avoid enumeration
    pub fn play_mp3_async_dual(
        &mut self,
        mp3_data: Vec<u8>,
        speaker_config: Option<OutputConfig>,
        virtual_mic_config: Option<OutputConfig>,
        cached_devices: Option<Arc<RwLock<HashMap<String, cpal::Device>>>>,
    ) -> Result<(), String> {
        // Останавливаем предыдущую сессию и получаем собственный флаг новой.
        let stop_flag = self.begin_playback_session();

        info!(data_size = mp3_data.len(), "Starting dual output playback");

        // Проверяем, что хотя бы один вывод включен
        if speaker_config.is_none() && virtual_mic_config.is_none() {
            return Err("No output devices configured".to_string());
        }

        // Получаем конфигурацию громкости для логирования
        let speaker_vol = speaker_config.as_ref().map(|c| c.volume).unwrap_or(0.0);
        let mic_vol = virtual_mic_config.as_ref().map(|c| c.volume).unwrap_or(0.0);
        debug!(
            speaker_volume = speaker_vol * 100.0,
            virtual_mic_volume = mic_vol * 100.0,
            "Volume configuration"
        );

        // Use Arc to share audio data efficiently (Arc<[u8]> instead of Arc<Vec<u8>> for better cache efficiency)
        let shared_data: Arc<[u8]> = mp3_data.into();

        // Собираем handles новых потоков
        let mut new_handles = Vec::new();

        // Запускаем воспроизведение на динамике
        if let Some(config) = speaker_config {
            let session_stop_flag = stop_flag.clone();
            let data = Arc::clone(&shared_data); // Cheap Arc clone, not Vec clone
            let devices_cache = cached_devices.clone();

            let handle = thread::spawn(move || {
                debug!("Speaker thread started");
                if let Err(e) =
                    Self::play_to_device(session_stop_flag, data, config, "Speaker", devices_cache)
                {
                    error!(error = %e, "Speaker playback error");
                }
                debug!("Speaker thread finished");
            });
            new_handles.push(handle);
        }

        // Запускаем воспроизведение на виртуальном микрофоне
        if let Some(config) = virtual_mic_config {
            let session_stop_flag = stop_flag.clone();
            let data = Arc::clone(&shared_data); // Cheap Arc clone, not Vec clone
            let devices_cache = cached_devices.clone();

            let handle = thread::spawn(move || {
                debug!("Virtual mic thread started");
                if let Err(e) = Self::play_to_device(
                    session_stop_flag,
                    data,
                    config,
                    "Virtual Mic",
                    devices_cache,
                ) {
                    error!(error = %e, "Virtual mic playback error");
                }
                debug!("Virtual mic thread finished");
            });
            new_handles.push(handle);
        }

        // Сохраняем handles для последующего управления
        self.active_threads = new_handles;

        Ok(())
    }

    /// Воспроизвести на конкретном устройстве (в отдельном потоке)
    fn play_to_device(
        stop_flag: Arc<AtomicBool>,
        mp3_data: Arc<[u8]>,
        config: OutputConfig,
        device_label: &str,
        cached_devices: Option<Arc<RwLock<HashMap<String, cpal::Device>>>>,
    ) -> Result<(), String> {
        let device = resolve_output_device(&config.device_id, &cached_devices)?;

        let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        info!(device_label = %device_label, device_name = %device_name,
            "Playing on device");

        let (stream, sink) = open_sink_on_device(&device, &mp3_data, config.volume)?;
        stream.log_binding(legacy_role(device_label), config.device_id.as_deref());

        while !sink.empty() {
            if stop_flag.load(Ordering::SeqCst) {
                debug!(device_label = %device_label,
                    "Playback stopped by flag");
                sink.stop();
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        if !stop_flag.load(Ordering::SeqCst) {
            debug!(device_label = %device_label, "Playback completed");
        }

        Ok(())
    }

    /// Воспроизвести тестовый звук на одном устройстве (блокирующе)
    /// Используется для тестирования аудиоустройств
    pub fn play_test_sound_blocking(
        &mut self,
        mp3_data: Vec<u8>,
        config: OutputConfig,
    ) -> Result<(), String> {
        info!("Playing test sound (blocking)");

        let stop_flag = Arc::new(AtomicBool::new(false));
        let data: Arc<[u8]> = mp3_data.into();

        // Use existing play_to_device logic but block until completion
        let result = Self::play_to_device(stop_flag.clone(), data, config, "Test Device", None);

        // Clean up the stop flag
        drop(stop_flag);

        result
    }
    /// Play preview audio with external stop flag (for preview commands).
    /// This is a STATIC method that uses a shared AtomicBool for cancellation.
    pub fn play_preview_with_stop_flag(
        stop_flag: Arc<AtomicBool>,
        audio_data: Vec<u8>,
        config: OutputConfig,
    ) -> Result<(), String> {
        let device = resolve_output_device(&config.device_id, &None)?;
        let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        debug!(device_name = %device_name, "Playing preview audio");

        let (stream, sink) = open_sink_on_device(&device, &audio_data, config.volume)?;
        stream.log_binding("preview", config.device_id.as_deref());

        while !sink.empty() {
            if stop_flag.load(Ordering::SeqCst) {
                debug!("Preview playback stopped by flag");
                sink.stop();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        debug!("Preview playback finished");
        Ok(())
    }

    /// Play preview PCM audio with external stop flag.
    /// Uses `SamplesBuffer` — no WAV encoder/decoder involved.
    pub fn play_preview_pcm_with_stop_flag(
        stop_flag: Arc<AtomicBool>,
        pcm: &crate::audio::AudioPcm,
        config: OutputConfig,
    ) -> Result<(), String> {
        let device = resolve_output_device(&config.device_id, &None)?;
        let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        debug!(device_name = %device_name, channels=pcm.channels, sr=pcm.sample_rate, "Playing PCM preview");

        let (stream, sink) = open_sink_on_device_pcm(&device, pcm, config.volume)?;
        stream.log_binding("preview", config.device_id.as_deref());

        while !sink.empty() {
            if stop_flag.load(Ordering::SeqCst) {
                debug!("Preview PCM playback stopped by flag");
                sink.stop();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        debug!("Preview PCM playback finished");
        Ok(())
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn new_session_gets_its_own_stop_flag() {
        let mut player = AudioPlayer::new();

        let first = player.begin_playback_session();
        let second = player.begin_playback_session();

        assert!(!Arc::ptr_eq(&first, &second));
        assert!(first.load(Ordering::SeqCst));
        assert!(!second.load(Ordering::SeqCst));
        assert!(Arc::ptr_eq(&player.stop_flag, &second));
    }

    #[test]
    fn timed_out_session_flag_is_not_reset_by_new_session() {
        // A thread from a previous session that outlived JOIN_TIMEOUT keeps a
        // clone of the old flag. Starting a new session must leave that flag
        // set and keep the new session's flag cleared.
        let mut player = AudioPlayer::new();
        let old_session = player.begin_playback_session();
        let still_running_thread_flag = old_session.clone();

        let new_session = player.begin_playback_session();

        assert!(still_running_thread_flag.load(Ordering::SeqCst));
        assert!(!new_session.load(Ordering::SeqCst));
    }
}

#[cfg(test)]
mod output_format_tests {
    use super::*;

    fn default_config() -> cpal::SupportedStreamConfig {
        cpal::SupportedStreamConfig::new(
            8,
            cpal::SampleRate(48000),
            cpal::SupportedBufferSize::Unknown,
            cpal::SampleFormat::F32,
        )
    }

    fn range(
        channels: u16,
        min: u32,
        max: u32,
        format: cpal::SampleFormat,
    ) -> cpal::SupportedStreamConfigRange {
        cpal::SupportedStreamConfigRange::new(
            channels,
            cpal::SampleRate(min),
            cpal::SampleRate(max),
            cpal::SupportedBufferSize::Unknown,
            format,
        )
    }

    #[test]
    fn i32_selection_preserves_rate_and_channels() {
        let selected = select_i32_config(
            &default_config(),
            [
                range(8, 48000, 48000, cpal::SampleFormat::F32),
                range(2, 48000, 48000, cpal::SampleFormat::I32),
                range(8, 44100, 44100, cpal::SampleFormat::I32),
                range(8, 44100, 96000, cpal::SampleFormat::I32),
            ],
        )
        .unwrap();
        assert_eq!(selected.sample_format(), cpal::SampleFormat::I32);
        assert_eq!(selected.sample_rate().0, 48000);
        assert_eq!(selected.channels(), 8);
    }

    #[test]
    fn i32_selection_does_not_fall_back_or_change_rate() {
        for ranges in [
            vec![],
            vec![range(8, 48000, 48000, cpal::SampleFormat::F32)],
            vec![range(2, 48000, 48000, cpal::SampleFormat::I32)],
            vec![range(8, 48001, 96000, cpal::SampleFormat::I32)],
            vec![range(8, 44100, 47999, cpal::SampleFormat::I32)],
        ] {
            assert!(select_i32_config(&default_config(), ranges).is_err());
        }
    }

    #[test]
    fn i32_selection_accepts_rate_boundaries() {
        for (min, max) in [(48000, 48000), (44100, 48000), (48000, 96000)] {
            let selected = select_i32_config(
                &default_config(),
                [range(8, min, max, cpal::SampleFormat::I32)],
            )
            .unwrap();
            assert_eq!(selected.sample_rate().0, 48000);
        }
    }

    #[test]
    fn strict_i32_mixer_keeps_stereo_frames_and_silence() {
        let (controller, mut mixer) = rodio::dynamic_mixer::mixer::<f32>(2, 48000);
        let (sink, queue) = Sink::new_idle();
        sink.append(SamplesBuffer::new(
            2,
            48000,
            vec![0.25f32, -0.5, 0.5, -0.25],
        ));
        controller.add(PrimedSinkQueue::new(queue));
        let samples: Vec<i32> = mixer
            .by_ref()
            .take(4)
            .map(<i32 as cpal::Sample>::from_sample)
            .collect();
        assert_eq!(
            samples,
            vec![536870912, -1073741824, 1073741824, -536870912]
        );
        assert_eq!(
            <i32 as cpal::Sample>::from_sample(mixer.next().unwrap_or(0.0)),
            0
        );
    }

    #[test]
    fn strict_i32_mixer_keeps_stereo_after_idle_silence() {
        let (controller, mut mixer) = rodio::dynamic_mixer::mixer::<f32>(2, 48000);
        let (sink, queue) = Sink::new_idle();
        controller.add(PrimedSinkQueue::new(queue));
        assert!(mixer.by_ref().take(64).all(|sample| sample == 0.0));
        for _ in 0..2 {
            sink.append(SamplesBuffer::new(
                2,
                48000,
                vec![0.25f32, -0.5, 0.5, -0.25],
            ));
            let first = mixer
                .by_ref()
                .take(8192)
                .find(|sample| *sample != 0.0)
                .expect("queued stereo must follow idle silence");
            let samples: Vec<i32> = std::iter::once(first)
                .chain(mixer.by_ref().take(3))
                .map(<i32 as cpal::Sample>::from_sample)
                .collect();
            assert_eq!(
                samples,
                vec![536870912, -1073741824, 1073741824, -536870912]
            );
            assert!(mixer.by_ref().take(64).all(|sample| sample == 0.0));
        }
    }

    #[test]
    fn strict_i32_mixer_handles_unknown_length_and_channel_changes() {
        struct UnknownLength(SamplesBuffer<f32>);
        impl Iterator for UnknownLength {
            type Item = f32;
            fn next(&mut self) -> Option<f32> {
                self.0.next()
            }
        }
        impl rodio::Source for UnknownLength {
            fn current_frame_len(&self) -> Option<usize> {
                rodio::Source::current_frame_len(&self.0)
            }
            fn channels(&self) -> u16 {
                rodio::Source::channels(&self.0)
            }
            fn sample_rate(&self) -> u32 {
                rodio::Source::sample_rate(&self.0)
            }
            fn total_duration(&self) -> Option<Duration> {
                None
            }
        }
        let (controller, mut mixer) = rodio::dynamic_mixer::mixer::<f32>(2, 48000);
        let (sink, queue) = Sink::new_idle();
        // Preserve stereo -> mono -> stereo without an iterator length hint.
        let stereo: Vec<f32> = [0.25, -0.5].into_iter().cycle().take(2048).collect();
        sink.append(UnknownLength(SamplesBuffer::new(2, 48000, stereo.clone())));
        sink.append(UnknownLength(SamplesBuffer::new(1, 48000, vec![0.5; 8])));
        sink.append(UnknownLength(SamplesBuffer::new(2, 48000, stereo.clone())));
        controller.add(PrimedSinkQueue::new(queue));
        let expected: Vec<f32> = stereo
            .iter()
            .copied()
            .chain([0.5; 16])
            .chain(stereo.iter().copied())
            .collect();
        let actual: Vec<i32> = mixer
            .by_ref()
            .take(expected.len())
            .map(<i32 as cpal::Sample>::from_sample)
            .collect();
        assert_eq!(
            actual,
            expected
                .into_iter()
                .map(<i32 as cpal::Sample>::from_sample)
                .collect::<Vec<_>>()
        );
        assert!(mixer.take(64).all(|sample| sample == 0.0));
    }

    #[test]
    fn strict_i32_mixer_preserves_continuous_resampling() {
        let pcm: Vec<f32> = (0..1024).map(|i| ((i % 31) as f32 - 15.0) / 32.0).collect();
        let expected: Vec<i32> = rodio::source::UniformSourceIterator::<_, f32>::new(
            SamplesBuffer::new(1, 24000, pcm.clone()),
            2,
            48000,
        )
        .map(<i32 as cpal::Sample>::from_sample)
        .collect();
        let (controller, mut mixer) = rodio::dynamic_mixer::mixer::<f32>(2, 48000);
        let (sink, queue) = Sink::new_idle();
        sink.append(SamplesBuffer::new(1, 24000, pcm));
        controller.add(PrimedSinkQueue::new(queue));
        let actual: Vec<i32> = mixer
            .by_ref()
            .take(expected.len())
            .map(<i32 as cpal::Sample>::from_sample)
            .collect();
        assert_eq!(actual, expected);
        assert!(mixer.take(64).all(|sample| sample == 0.0));
    }
}
