//! Sound Panel Audio Playback
//!
//! Воспроизведение аудиофайлов для звуковой панели.
//!
//! Воспроизводит звук на тех же настроенных выходах, что и обычная TTS-озвучка:
//! динамики (когда `audio.speaker_enabled == true`) и виртуальный микрофон
//! (когда выход включён переключателем ROADMAP-142 и задан
//! `audio.virtual_mic_device`). Оба выхода подготавливаются до ожидания
//! завершения любого из них, поэтому sink стартуют одновременно. Ошибка на
//! одном устройстве не отменяет другой выход.
//!
//! Переключатель микрофона (ROADMAP-142) проверяется по живому кэшу настроек
//! на старте клипа и в такте опроса: выключение снимает только локальный
//! mic-sink звучащего клипа, спикер и VBAN не останавливаются. Включение до
//! старта накопленного клипа, наоборот, добавляет микрофонный выход с
//! устройством и громкостью из снимка очереди; уже идущий клип, у которого
//! mic-sink снят выключением, не возобновляется.
//!
//! Дополнительно клип уходит в общий сетевой выход VBAN (когда включён в
//! актуальном runtime): сеть работает и при выключенных, и при отказавших
//! локальных выходах (network-only клип). Локальные выходы, очередь и
//! одновременное звучание с TTS сохраняются.
//!
//! FIFO-очередь ждёт терминального статуса network-only клипа: без рабочего
//! локального выхода сеть — единственный выход, и следующий клип не должен
//! заменять ещё звучащую сетевую дорожку. При рабочем локальном sink очередь
//! ждёт только его, поэтому сетевой хвост не задерживает следующие клипы.

use crate::audio::vban::{VbanOutput, VbanPlayback, VbanPlaybackStatus, VbanSource};
use crate::audio::{
    decode_audio, open_output_sink, resolve_output_device, AudioPcm, OutputConfig,
    OutputDeviceStream,
};
use crate::config::{AppSettings, AudioSettings};
use cpal::traits::DeviceTrait;
use parking_lot::RwLock;
use std::cell::RefCell;
use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Построить включённые конфигурации вывода из настроек аудио.
///
/// Возвращает `(speaker, virtual_mic)`. `None` означает, что соответствующий
/// выход выключен/не настроен. Проценты громкости переводятся в 0.0–1.0.
///
/// Включённость микрофона передаётся отдельно: `mic_enabled` — актуальное
/// (live) состояние переключателя, которое главнее снимка, захваченного при
/// постановке клипа в очередь, в обе стороны. Устройство и громкость при
/// этом всегда берутся из снимка, поэтому накопленный клип уходит в
/// микрофон с сохранённым устройством, если выход включили до его старта.
pub(crate) fn build_output_configs(
    audio_settings: &AudioSettings,
    mic_enabled: bool,
) -> (Option<OutputConfig>, Option<OutputConfig>) {
    let speaker_config = if audio_settings.speaker_enabled {
        Some(OutputConfig {
            device_id: audio_settings.speaker_device.clone(),
            volume: audio_settings.speaker_volume as f32 / 100.0,
        })
    } else {
        None
    };

    // ROADMAP-142: переключатель решает, есть ли у клипа микрофонный выход;
    // сохранённое устройство остаётся нетронутым. No device => no mic.
    let virtual_mic_config = if mic_enabled {
        audio_settings
            .virtual_mic_device
            .as_ref()
            .map(|device_id| OutputConfig {
                device_id: Some(device_id.clone()),
                volume: audio_settings.virtual_mic_volume as f32 / 100.0,
            })
    } else {
        None
    };

    (speaker_config, virtual_mic_config)
}

/// ROADMAP-142: актуальная включённость микрофонного выхода клипа.
///
/// Живой кэш настроек (когда передан воркером) главнее снимка, захваченного
/// при постановке клипа в очередь: накопленный клип не проигрывает устаревшее
/// включённое состояние. Без кэша (тесты) снимок остаётся источником.
fn live_clip_mic_enabled(
    audio_settings: &AudioSettings,
    settings_cache: Option<&RwLock<AppSettings>>,
) -> bool {
    settings_cache
        .map(|cache| cache.read().audio.virtual_mic_enabled)
        .unwrap_or(audio_settings.virtual_mic_enabled)
}

/// Интервал опроса статуса network-only клипа.
const NETWORK_ONLY_STATUS_POLL: Duration = Duration::from_millis(10);

/// Чем уже запущенные выходы клипа удерживают FIFO-очередь.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueWait {
    /// Ни один выход не запустился: ждать нечего (ошибка вызывающего).
    None,
    /// Звучит хотя бы один локальный sink: ждём только его. Сетевой хвост не
    /// должен удлинять задержку очереди при рабочем локальном выходе.
    LocalSinks,
    /// Локальный sink не запустился: VBAN — единственный выход, поэтому ждём
    /// терминального статуса именно этой сетевой дорожки.
    NetworkOnly,
}

/// Выбрать, что удерживает очередь после старта выходов клипа.
fn queue_wait(local_outputs: usize, vban_started: bool) -> QueueWait {
    match (local_outputs, vban_started) {
        (0, false) => QueueWait::None,
        (0, true) => QueueWait::NetworkOnly,
        _ => QueueWait::LocalSinks,
    }
}

/// Терминальный ли статус сетевой дорожки клипа.
fn is_terminal_network_status(status: &VbanPlaybackStatus) -> bool {
    matches!(
        status,
        VbanPlaybackStatus::Finished | VbanPlaybackStatus::Stopped | VbanPlaybackStatus::Failed(_)
    )
}

/// Prepare only PCM off-thread. The decoder never starts playback: dropping
/// its receiver prevents a late result from replacing a newer FIFO clip.
fn prepare_vban_clip(path: &str) -> Result<Receiver<Result<Arc<AudioPcm>, String>>, String> {
    let path = path.to_string();
    let (tx, rx) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("soundpanel-vban-decode".to_string())
        .spawn(move || {
            let _ = tx.send(decode_vban_clip(&path));
        })
        .map_err(|e| format!("Failed to start VBAN decoder: {e}"))?;
    Ok(rx)
}

fn decode_vban_clip(path: &str) -> Result<Arc<AudioPcm>, String> {
    let data = std::fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
    let pcm = decode_audio(&data).map_err(|e| format!("Failed to decode audio: {}", e))?;
    Ok(Arc::new(pcm))
}

/// Local playback alone holds the FIFO when available. Otherwise wait for
/// preparation and the entire network track, polling shutdown in both phases.
/// Only this queue thread can start the prepared track, so abandoned decoder
/// results cannot start after this function returns or preempt the next clip.
///
/// `gate_tick` runs every poll iteration before completion checks: it drops
/// the local mic sink when the output toggle flipped off mid-clip (ROADMAP-142),
/// so «local present/done» reflects the cut within one poll interval.
fn wait_for_clip_outputs(
    mut preparation: Option<Receiver<Result<Arc<AudioPcm>, String>>>,
    vban: Option<&VbanOutput>,
    mut local_present: impl FnMut() -> bool,
    mut local_done: impl FnMut() -> bool,
    mut gate_tick: impl FnMut(),
    shutdown: Option<&CancellationToken>,
) {
    let mut network: Option<VbanPlayback> = None;
    loop {
        if shutdown.is_some_and(|token| token.is_cancelled()) {
            if let Some(handle) = &network {
                handle.stop();
            }
            return;
        }
        gate_tick();
        if let Some(rx) = &preparation {
            match rx.try_recv() {
                Ok(result) => {
                    preparation = None;
                    match result.and_then(|pcm| {
                        let output = vban.ok_or_else(|| "VBAN runtime unavailable".to_string())?;
                        let handle = output.start(VbanSource::SoundPanel, pcm, 1.0)?;
                        // ROADMAP-139: warn about a failed receiver at the
                        // actual clip send start; the monitor dedupes the
                        // alert while the global toast is visible.
                        if output.health().notify_send_started() {
                            warn!(target: "soundpanel", "VBAN clip started while the receiver is failed");
                        }
                        Ok(handle)
                    }) {
                        Ok(handle) => network = Some(handle),
                        Err(e) => error!(error = %e, "VBAN SoundPanel start failed"),
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    preparation = None;
                    error!("VBAN SoundPanel decoder disconnected");
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if local_present() && local_done() {
            // Consume already-ready PCM first: a very short local sound may
            // end during sink setup and must still reach the network. A copy
            // that is still preparing is abandoned without delaying FIFO.
            return;
        }
        if !local_present() && preparation.is_none() {
            match &network {
                Some(handle) if !is_terminal_network_status(&handle.status()) => {}
                Some(handle) => {
                    debug!(status = ?handle.status(), "Network-only clip reached terminal status");
                    return;
                }
                None => {
                    error!("No output could be started (speaker, virtual mic and VBAN all failed)");
                    return;
                }
            }
        }
        thread::sleep(NETWORK_ONLY_STATUS_POLL);
    }
}

/// Воспроизвести аудиофайл
///
/// Поддерживаемые форматы: MP3, WAV, OGG, FLAC (через rodio).
///
/// ROADMAP-142: включённость микрофонного выхода проверяется по живому кэшу
/// `settings_cache` (если он передан воркером) на старте клипа и в каждом
/// такте опроса: выключение снимает только локальный mic-sink звучащего
/// клипа; спикер и VBAN-дорожка не останавливаются. Клипы, накопленные со
/// старым снимком настроек, не проигрывают устаревшее включённое состояние.
///
/// `shutdown` позволяет прервать ожидание network-only клипа при завершении
/// приложения; `None` используется тестами без токена.
pub fn play_audio_file(
    path: &str,
    audio_settings: &AudioSettings,
    vban: Option<&VbanOutput>,
    settings_cache: Option<&RwLock<AppSettings>>,
    shutdown: Option<&CancellationToken>,
) {
    info!(path, "Playing audio");

    // Проверяем существование файла
    if !std::path::Path::new(path).exists() {
        error!(path, "File not found");
        return;
    }

    // Микрофонный выход учитывает живой переключатель в обе стороны: снимок
    // из очереди может быть устаревшим и после включения, и после выключения.
    // Устройство и громкость остаются из снимка; спикер следует своим
    // сохранённым полям снимка.
    let mic_live_enabled = live_clip_mic_enabled(audio_settings, settings_cache);
    let (speaker_config, virtual_mic_config) =
        build_output_configs(audio_settings, mic_live_enabled);

    // Consult live output state, not the settings captured when this clip was
    // queued. Reading/decoding the network copy cannot delay local sink setup.
    let preparation = if vban.is_some_and(VbanOutput::is_configured) {
        match prepare_vban_clip(path) {
            Ok(rx) => Some(rx),
            Err(e) => {
                error!(error = %e, "VBAN SoundPanel preparation failed");
                None
            }
        }
    } else {
        None
    };

    let path = path.to_string();
    // RefCell: sinks live on this queue thread only; the closures below share
    // them for presence/done checks and the mid-clip mic cut.
    let outputs = RefCell::new(Vec::<(OutputDeviceStream, rodio::Sink)>::new());
    let mic_slot = RefCell::new(Option::<usize>::None);

    if let Some(config) = speaker_config {
        match setup_output(&path, &config, "Speaker") {
            Ok(pair) => outputs.borrow_mut().push(pair),
            Err(e) => error!(error = %e, "Speaker playback failed"),
        }
    }

    if let Some(config) = virtual_mic_config {
        match setup_output(&path, &config, "Virtual Mic") {
            Ok(pair) => {
                *mic_slot.borrow_mut() = Some(outputs.borrow().len());
                outputs.borrow_mut().push(pair);
            }
            Err(e) => error!(error = %e, "Virtual mic playback failed"),
        }
    }

    // Bind the decision first: a call expression in a match scrutinee keeps
    // its temporaries (the RefCell borrow) alive for the whole match, while
    // the arms below borrow `outputs` again.
    let wait = queue_wait(outputs.borrow().len(), preparation.is_some());
    match wait {
        QueueWait::None => {
            error!("No output could be started (speaker, virtual mic and VBAN all failed)");
        }
        QueueWait::NetworkOnly | QueueWait::LocalSinks => wait_for_clip_outputs(
            preparation,
            vban,
            || !outputs.borrow().is_empty(),
            || outputs.borrow().iter().all(|(_, sink)| sink.empty()),
            || {
                // Выключенный переключатель снимает только mic-sink; дроп
                // пары stream+sink останавливает подачу в пределах такта.
                let disabled = !live_clip_mic_enabled(audio_settings, settings_cache);
                if disabled {
                    if let Some(slot) = mic_slot.borrow_mut().take() {
                        outputs.borrow_mut().remove(slot);
                        info!(target: "soundpanel", "Virtual mic sink cut by the output toggle");
                    }
                }
            },
            shutdown,
        ),
    }
}

/// Подготовить вывод на одном устройстве.
///
/// Резолвит устройство, открывает файл, декодирует отдельным
/// decoder-экземпляром, создаёт stream+sink, выставляет громкость, добавляет
/// source и возвращает пару stream/sink. Вызывающий должен удерживать stream,
/// пока sink живёт. Любая ошибка возвращается вызывающему потоку и не влияет
/// на другой выход.
fn setup_output(
    path: &str,
    config: &OutputConfig,
    label: &str,
) -> Result<(OutputDeviceStream, rodio::Sink), String> {
    let device = resolve_output_device(&config.device_id, &None)?;

    let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
    info!(label, device_name = %device_name, "Playing on device");

    let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
    let source = rodio::Decoder::new(BufReader::new(file))
        .map_err(|e| format!("Failed to decode audio: {}", e))?;

    let (stream, sink) = open_output_sink(&device)?;
    stream.log_binding(
        if label == "Virtual Mic" {
            "soundpanel_mic"
        } else {
            "soundpanel_speaker"
        },
        config.device_id.as_deref(),
    );

    sink.set_volume(config.volume);
    debug!(label, volume = config.volume, "Volume set");

    sink.append(source);

    Ok((stream, sink))
}

/// Проверить, является ли файл поддерживаемым аудиоформатом
pub fn is_supported_audio_format(filename: &str) -> bool {
    let filename_lower = filename.to_lowercase();

    let supported_extensions = [".mp3", ".wav", ".ogg", ".flac", ".m4a", ".aac", ".wma"];

    supported_extensions
        .iter()
        .any(|ext| filename_lower.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::vban::protocol::{decode_header, HEADER_SIZE};
    use crate::audio::vban::VbanTarget;
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    fn make_audio(speaker_enabled: bool, mic: Option<&str>) -> AudioSettings {
        AudioSettings {
            output_format: crate::config::AudioOutputFormat::Default,
            speaker_enabled,
            speaker_device: None,
            speaker_volume: 80,
            virtual_mic_device: mic.map(str::to_string),
            virtual_mic_enabled: true,
            virtual_mic_volume: 60,
            vban: crate::config::VbanSettings::default(),
        }
    }

    /// Network-only snapshot: VBAN enabled, both local outputs disabled.
    fn network_only_audio() -> AudioSettings {
        let mut audio = make_audio(false, None);
        audio.vban.enabled = true;
        audio
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-sp-audio-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a mono 16-bit PCM WAV at the VBAN stream profile (48 kHz) with a
    /// constant sample value, so received packets can be attributed to a clip.
    fn write_wav(path: &Path, frames: usize, value: f32) {
        const SAMPLE_RATE: u32 = 48_000;
        let sample = (value.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
        let data_len = (frames * 2) as u32;

        let mut bytes = Vec::with_capacity(44 + frames * 2);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
        bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
        bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for _ in 0..frames {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }

    /// Bound loopback listener and matching VBAN target.
    fn loopback_output(stream_name: &str) -> (UdpSocket, VbanOutput) {
        let listener = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
        listener
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let output = VbanOutput::new();
        output.configure(Some(VbanTarget {
            addr: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
            stream_name: stream_name.to_string(),
            gain: 1.0,
        }));
        (listener, output)
    }

    #[test]
    fn test_is_supported_audio_format() {
        assert!(is_supported_audio_format("test.mp3"));
        assert!(is_supported_audio_format("test.wav"));
        assert!(is_supported_audio_format("test.OGG")); // case insensitive
        assert!(!is_supported_audio_format("test.txt"));
        assert!(!is_supported_audio_format("test.doc"));
    }

    #[test]
    fn outputs_both_enabled() {
        let audio = make_audio(true, Some("mic1"));
        let (speaker, mic) = build_output_configs(&audio, audio.virtual_mic_enabled);

        assert!(speaker.is_some());
        assert_eq!(speaker.as_ref().map(|c| c.volume), Some(0.8));
        assert!(mic.is_some());
        assert_eq!(
            mic.as_ref().and_then(|c| c.device_id.as_deref()),
            Some("mic1")
        );
        assert_eq!(mic.as_ref().map(|c| c.volume), Some(0.6));
    }

    #[test]
    fn outputs_speaker_disabled_leaves_mic_only() {
        let audio = make_audio(false, Some("mic1"));
        let (speaker, mic) = build_output_configs(&audio, audio.virtual_mic_enabled);

        assert!(speaker.is_none());
        assert!(mic.is_some());
    }

    #[test]
    fn outputs_mic_missing_leaves_speaker_only() {
        let audio = make_audio(true, None);
        let (speaker, mic) = build_output_configs(&audio, audio.virtual_mic_enabled);

        assert!(speaker.is_some());
        assert!(mic.is_none());
    }

    /// ROADMAP-142: без живого кэша (тесты, независимые запуски) снимок
    /// остаётся источником включённости; сохранённое устройство не трогается.
    #[test]
    fn outputs_mic_toggle_gates_clip_mic_output() {
        let mut audio = make_audio(true, Some("mic1"));
        audio.virtual_mic_enabled = false;
        let (speaker, mic) = build_output_configs(&audio, audio.virtual_mic_enabled);

        assert!(speaker.is_some());
        assert!(mic.is_none());
        assert_eq!(audio.virtual_mic_device.as_deref(), Some("mic1"));
    }

    /// Regression (review 2026-11-19): клип, накопленный при выключенном
    /// переключателе, уходит в микрофон, если до его старта выход включили.
    /// Точная производственная последовательность `play_audio_file`: живой
    /// флаг из кэша, затем построение конфигураций по нему. Устройство и
    /// громкость берутся из снимка очереди, спикер не затронут.
    #[test]
    fn queued_disabled_clip_builds_mic_config_when_live_enabled_before_start() {
        let mut queued = make_audio(true, Some("mic1"));
        queued.virtual_mic_enabled = false; // переключатель был выключен при постановке

        let cache = parking_lot::RwLock::new(crate::config::AppSettings::default());
        cache.write().audio.virtual_mic_enabled = true;
        let mic_live_enabled = live_clip_mic_enabled(&queued, Some(&cache));
        assert!(
            mic_live_enabled,
            "live toggle must win over the queued flag"
        );

        let (speaker, mic) = build_output_configs(&queued, mic_live_enabled);
        assert!(speaker.is_some(), "captured speaker settings are untouched");
        assert_eq!(
            mic.as_ref().and_then(|config| config.device_id.as_deref()),
            Some("mic1"),
            "queued-disabled clip must gain the mic after a live enable"
        );
        assert_eq!(mic.as_ref().map(|config| config.volume), Some(0.6));
    }

    /// Обратное направление: накопленный с включённым микрофоном клип не
    /// получает микрофонный выход, если до старта переключатель выключили.
    #[test]
    fn queued_enabled_clip_loses_mic_config_when_live_disabled_before_start() {
        let queued = make_audio(true, Some("mic1"));

        let cache = parking_lot::RwLock::new(crate::config::AppSettings::default());
        cache.write().audio.virtual_mic_enabled = false;
        let mic_live_enabled = live_clip_mic_enabled(&queued, Some(&cache));
        assert!(!mic_live_enabled);

        let (speaker, mic) = build_output_configs(&queued, mic_live_enabled);
        assert!(speaker.is_some(), "speaker must ignore the mic toggle");
        assert!(
            mic.is_none(),
            "live disable must drop the queued mic output"
        );
    }

    /// No device => no mic: live-включение не создаёт выход без устройства.
    #[test]
    fn live_enabled_without_device_still_has_no_mic_config() {
        let queued = make_audio(true, None);
        let (speaker, mic) = build_output_configs(&queued, true);
        assert!(speaker.is_some());
        assert!(mic.is_none());
    }

    /// ROADMAP-142: живой кэш настроек главнее снимка, накопленного в
    /// очереди, — клип не проигрывает устаревшее включённое состояние
    /// микрофона. Без кэша (тесты, независимые запуски) действует снимок.
    #[test]
    fn live_clip_mic_gate_prefers_the_cache_over_the_queued_snapshot() {
        let enabled_snapshot = make_audio(true, Some("mic1"));
        assert!(enabled_snapshot.virtual_mic_enabled);

        let cache = parking_lot::RwLock::new(crate::config::AppSettings::default());
        cache.write().audio.virtual_mic_enabled = false;
        assert!(
            !live_clip_mic_enabled(&enabled_snapshot, Some(&cache)),
            "queued clip must not feed the mic after the toggle flipped off"
        );

        cache.write().audio.virtual_mic_enabled = true;
        assert!(live_clip_mic_enabled(&enabled_snapshot, Some(&cache)));

        assert!(live_clip_mic_enabled(&enabled_snapshot, None));
        let mut disabled_snapshot = make_audio(true, Some("mic1"));
        disabled_snapshot.virtual_mic_enabled = false;
        assert!(!live_clip_mic_enabled(&disabled_snapshot, None));
    }

    /// ROADMAP-142: такт опроса вызывает gate_tick до проверки завершения —
    /// структурная гарантия того, что выключение видно в каждом такте.
    #[test]
    fn wait_loop_polls_the_mic_gate_tick() {
        let ticks = std::cell::Cell::new(0usize);
        wait_for_clip_outputs(
            None,
            None,
            || false,
            || false,
            || ticks.set(ticks.get() + 1),
            None,
        );
        assert_eq!(
            ticks.get(),
            1,
            "gate tick runs before the completion branch"
        );
    }

    // ── VBAN клипы (ROADMAP-138) ──

    #[test]
    fn play_audio_file_without_any_output_returns_without_panic() {
        // VBAN выключен, локальные выходы выключены: ошибка отсутствия
        // выходов, никакого воспроизведения и чтения файла.
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-sp-audio-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip.mp3");
        std::fs::write(&path, b"fake").unwrap();

        let mut audio = make_audio(false, None);
        audio.vban.enabled = false;
        play_audio_file(path.to_str().unwrap(), &audio, None, None, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn play_audio_file_missing_vban_runtime_with_vban_enabled_is_network_failure() {
        // VBAN включён в снимке, но runtime недоступен и локальные выходы
        // выключены: ни один выход не смог запуститься — выход без паники.
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-sp-audio-novban-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip.mp3");
        std::fs::write(&path, b"fake").unwrap();

        let mut audio = make_audio(false, None);
        audio.vban.enabled = true;
        play_audio_file(path.to_str().unwrap(), &audio, None, None, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn play_audio_file_missing_file_returns_before_outputs() {
        let mut audio = make_audio(true, None);
        audio.vban.enabled = true;
        // Несуществующий файл: ранний выход до построения выходов.
        play_audio_file("Z:/definitely/missing/clip.mp3", &audio, None, None, None);
    }

    #[test]
    fn vban_enabled_snapshot_keeps_local_outputs() {
        // Включённый VBAN не отключает локальные выходы: конфигурации строятся
        // как раньше.
        let mut audio = make_audio(true, Some("mic1"));
        audio.vban.enabled = true;
        let (speaker, mic) = build_output_configs(&audio, audio.virtual_mic_enabled);
        assert!(speaker.is_some());
        assert!(mic.is_some());
    }

    // ── FIFO при выводе только по VBAN ──

    #[test]
    fn queued_clip_follows_live_vban_enablement_instead_of_its_snapshot() {
        let dir = temp_dir("live-vban");
        let clip = dir.join("clip.wav");
        write_wav(&clip, 960, 0.5);

        // Capture the queued clip before the user enables the output.
        let mut queued_audio = make_audio(false, None);
        assert!(!queued_audio.vban.enabled);
        let (listener, output) = loopback_output("SPLIVE");
        play_audio_file(
            clip.to_str().unwrap(),
            &queued_audio,
            Some(&output),
            None,
            None,
        );
        let mut buf = [0u8; 2048];
        let (len, _) = listener
            .recv_from(&mut buf)
            .expect("live enabled output sends queued clip");
        assert_eq!(decode_header(&buf[..len]).unwrap().stream_name, "SPLIVE");
        for _ in 0..3 {
            listener
                .recv_from(&mut buf)
                .expect("all four packets arrive");
        }

        // Disabling also wins over an older enabled snapshot.
        queued_audio.vban.enabled = true;
        output.configure(None);
        play_audio_file(
            clip.to_str().unwrap(),
            &queued_audio,
            Some(&output),
            None,
            None,
        );
        listener
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        assert!(listener.recv_from(&mut buf).is_err());
        output.shutdown();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn local_completion_abandons_pending_network_decode_without_waiting() {
        let (tx, rx) = mpsc::sync_channel(1);
        // A decoder has not produced anything. The local sink is already done:
        // completion must return without receiving or waiting for that decoder.
        wait_for_clip_outputs(Some(rx), None, || true, || true, || {}, None);
        let pcm = Arc::new(AudioPcm {
            samples: vec![0.0; 2],
            sample_rate: 48_000,
            channels: 2,
        });
        assert!(
            tx.send(Ok(pcm)).is_err(),
            "late PCM must have no playback consumer"
        );
    }

    #[test]
    fn ready_network_copy_still_plays_if_short_local_sound_has_ended() {
        let (listener, output) = loopback_output("SPSHORT");
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(Ok(Arc::new(AudioPcm {
            samples: vec![0.5; 960],
            sample_rate: 48_000,
            channels: 1,
        })))
        .unwrap();
        wait_for_clip_outputs(Some(rx), Some(&output), || true, || true, || {}, None);
        let mut buf = [0u8; 2048];
        listener
            .recv_from(&mut buf)
            .expect("ready short sound must reach VBAN");
        output.shutdown();
    }

    #[test]
    fn shutdown_abandons_pending_network_decode() {
        let (tx, rx) = mpsc::sync_channel(1);
        let shutdown = CancellationToken::new();
        shutdown.cancel();
        wait_for_clip_outputs(Some(rx), None, || false, || false, || {}, Some(&shutdown));
        assert!(tx.send(Err("late decode failure".to_string())).is_err());
    }

    #[test]
    fn queue_wait_is_network_only_only_without_a_local_sink() {
        // Сеть не должна удерживать очередь, когда звучит локальный sink.
        assert_eq!(queue_wait(0, false), QueueWait::None);
        assert_eq!(queue_wait(0, true), QueueWait::NetworkOnly);
        assert_eq!(queue_wait(1, false), QueueWait::LocalSinks);
        assert_eq!(queue_wait(2, true), QueueWait::LocalSinks);
    }

    /// Regression: два network-only клипа подряд. Первый `play_audio_file`
    /// обязан звучать полностью и только затем вернуться, иначе второй старт
    /// отменит его дорожку. UDP loopback проверяет реальные пакеты обоих
    /// клипов и сквозную нумерацию `nuFrame`.
    #[test]
    fn network_only_clips_wait_for_each_clip_and_do_not_truncate_the_first() {
        const FRAMES: usize = 12_000; // 250 ms at 48 kHz
        const EXPECTED_PACKETS: usize = FRAMES.div_ceil(256);

        let dir = temp_dir("fifo");
        let first = dir.join("first.wav");
        let second = dir.join("second.wav");
        // Разные уровни позволяют отнести каждый пакет к своему клипу.
        write_wav(&first, FRAMES, 0.5);
        write_wav(&second, FRAMES, -0.5);

        let (listener, output) = loopback_output("SPFIFO");
        let reader = thread::spawn(move || {
            let mut packets: Vec<(u32, usize, i16)> = Vec::new();
            let mut buf = [0u8; 2048];
            while let Ok((len, _)) = listener.recv_from(&mut buf) {
                let header = decode_header(&buf[..len]).expect("valid VBAN header");
                let sample = i16::from_le_bytes([buf[HEADER_SIZE], buf[HEADER_SIZE + 1]]);
                packets.push((header.nu_frame, header.samples_per_packet, sample));
            }
            packets
        });

        let audio = network_only_audio();
        let shutdown = CancellationToken::new();

        let started = Instant::now();
        play_audio_file(
            first.to_str().unwrap(),
            &audio,
            Some(&output),
            None,
            Some(&shutdown),
        );
        let first_elapsed = started.elapsed();

        let started = Instant::now();
        play_audio_file(
            second.to_str().unwrap(),
            &audio,
            Some(&output),
            None,
            Some(&shutdown),
        );
        let second_elapsed = started.elapsed();

        let packets = reader.join().expect("packet reader joins");
        output.shutdown();
        std::fs::remove_dir_all(&dir).ok();

        assert!(
            first_elapsed >= Duration::from_millis(200),
            "first network-only clip must hold the queue until it sounds, took {first_elapsed:?}"
        );
        assert!(
            second_elapsed >= Duration::from_millis(200),
            "second network-only clip must hold the queue until it sounds, took {second_elapsed:?}"
        );
        assert_eq!(
            packets.len(),
            EXPECTED_PACKETS * 2,
            "every packet of both clips must arrive"
        );
        assert_eq!(
            packets.iter().map(|packet| packet.0).collect::<Vec<_>>(),
            (1..=(EXPECTED_PACKETS * 2) as u32).collect::<Vec<_>>(),
            "both clips share one stream counter"
        );

        let (first_packets, second_packets) = packets.split_at(EXPECTED_PACKETS);
        assert!(
            first_packets.iter().all(|packet| packet.2 == 16_384),
            "the first clip sounds in full before the second starts"
        );
        assert!(
            second_packets.iter().all(|packet| packet.2 == -16_384),
            "the second clip is not mixed with a still-playing first clip"
        );
        let tail = FRAMES - (EXPECTED_PACKETS - 1) * 256;
        assert_eq!(first_packets.last().map(|packet| packet.1), Some(tail));
        assert_eq!(second_packets.last().map(|packet| packet.1), Some(tail));
    }

    /// Shutdown во время network-only клипа обязан прервать ожидание и
    /// остановить дорожку, а не дожидаться конца звучания.
    #[test]
    fn shutdown_aborts_network_only_wait_and_stops_the_clip() {
        const FRAMES: usize = 48_000; // one second: far longer than the join bound

        let dir = temp_dir("shutdown");
        let clip = dir.join("long.wav");
        write_wav(&clip, FRAMES, 0.25);

        let (listener, output) = loopback_output("SPSHUT");
        let output = Arc::new(output);
        let audio = network_only_audio();
        let shutdown = CancellationToken::new();

        let worker_output = Arc::clone(&output);
        let worker_shutdown = shutdown.clone();
        let clip_path = clip.to_str().unwrap().to_string();
        let worker_audio = audio.clone();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            play_audio_file(
                &clip_path,
                &worker_audio,
                Some(worker_output.as_ref()),
                None,
                Some(&worker_shutdown),
            );
            let _ = done_tx.send(());
        });

        // Клип уже звучит: queue-поток ждёт его терминального статуса.
        let mut buf = [0u8; 2048];
        listener
            .recv_from(&mut buf)
            .expect("network-only clip starts sending");

        shutdown.cancel();
        done_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("shutdown must abort the wait long before the one-second clip ends");
        worker.join().expect("clip thread joins");

        let deadline = Instant::now() + Duration::from_millis(500);
        while output.is_sending() {
            assert!(
                Instant::now() < deadline,
                "network-only clip must stop after shutdown"
            );
            thread::sleep(Duration::from_millis(5));
        }

        output.shutdown();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Отказ сети (VBAN runtime есть, но цель не сконфигурирована) не должен
    /// подвешивать очередь на длительность файла.
    #[test]
    fn network_only_clip_without_configured_target_returns_instead_of_waiting() {
        const FRAMES: usize = 48_000;

        let dir = temp_dir("no-target");
        let clip = dir.join("clip.wav");
        write_wav(&clip, FRAMES, 0.5);

        let output = VbanOutput::new(); // no configure: start fails
        let audio = network_only_audio();
        let started = Instant::now();
        play_audio_file(clip.to_str().unwrap(), &audio, Some(&output), None, None);
        let elapsed = started.elapsed();

        output.shutdown();
        std::fs::remove_dir_all(&dir).ok();

        assert!(
            elapsed < Duration::from_millis(500),
            "failed network start must not wait out the clip, took {elapsed:?}"
        );
    }
}
