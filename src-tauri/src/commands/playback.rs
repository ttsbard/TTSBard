use crate::audio::effects;
use crate::audio::{
    decode_audio, AudioEffects, AudioPcm, AudioPlayer, OutputConfig, OutputDeviceInfo,
};
use crate::commands::speech_queue::SpeechQueueState;
use crate::config::SettingsManager;
use crate::playback::{
    project_playback_activity, PlaybackActivityDto, PlaybackManager, PlaybackStateDto,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, OnceLock};
use tauri::{AppHandle, State};
use tracing::{debug, info};

pub struct PlaybackState(pub Arc<PlaybackManager>);

#[tauri::command]
pub fn get_audio_output_format_pending() -> Result<bool, String> {
    crate::audio::output_format_pending()
}

#[tauri::command]
pub async fn set_audio_output_format(
    format: crate::config::AudioOutputFormat,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(&*settings_manager, move |manager| {
        manager.set_audio_output_format(format)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

#[tauri::command]
pub fn playback_pause(playback: State<'_, PlaybackState>) -> Result<(), String> {
    let pb = &playback.inner().0;
    if pb.pause() {
        Ok(())
    } else {
        Err("Нечего приостановить (воспроизведение не активно)".to_string())
    }
}

#[tauri::command]
pub fn playback_resume(playback: State<'_, PlaybackState>) -> Result<(), String> {
    let pb = &playback.inner().0;
    if pb.resume() {
        Ok(())
    } else {
        Err("Нечего возобновить (воспроизведение не приостановлено)".to_string())
    }
}

#[tauri::command]
pub fn playback_stop(playback: State<'_, PlaybackState>) -> Result<(), String> {
    let pb = &playback.inner().0;
    if pb.stop() {
        Ok(())
    } else {
        Err("Нечего остановить (воспроизведение не активно)".to_string())
    }
}

#[tauri::command]
pub fn playback_repeat(playback: State<'_, PlaybackState>) -> Result<(), String> {
    let pb = &playback.inner().0;
    if pb.repeat() {
        Ok(())
    } else {
        Err("Нечего повторить (воспроизведение не активно)".to_string())
    }
}

#[tauri::command]
pub fn replay_phrase(
    id: String,
    playback: State<'_, PlaybackState>,
    queue: State<'_, SpeechQueueState>,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    let settings = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?;
    let (speaker, mic) = crate::commands::tts_pipeline::compute_output_configs(
        &settings.audio,
        &settings.audio_effects,
    );

    let pb = &playback.inner().0;
    pb.replay_from_cache(&id, speaker, mic).map_err(|e| {
        debug!(target: "playback", error = %e, "replay_phrase cache miss");
        e
    })?;
    if let Ok(job_id) = uuid::Uuid::parse_str(&id) {
        let mut q = queue.lock();
        q.touch_activity(job_id);
    }
    Ok(())
}

#[tauri::command]
pub fn cancel_queued_replay(id: String, playback: State<'_, PlaybackState>) -> Result<(), String> {
    let pb = &playback.inner().0;
    pb.cancel_queued_replay(&id)
}

#[tauri::command]
pub fn get_playback_state(playback: State<'_, PlaybackState>) -> PlaybackStateDto {
    let pb = &playback.inner().0;
    let dto = pb.get_state();
    debug!(target: "playback", status=?dto.status, current=dto.current.is_some(), recent=dto.recent.len(), "get_playback_state");
    dto
}

#[tauri::command]
pub fn get_playback_activity(
    playback: State<'_, PlaybackState>,
    queue: State<'_, SpeechQueueState>,
) -> PlaybackActivityDto {
    let pb = &playback.inner().0;
    let q = queue.lock();
    let jobs = q.state().jobs;
    drop(q);

    let snap = pb.snapshot();

    let rows = project_playback_activity(
        &jobs,
        &snap.cache_entries,
        &snap.current_id,
        &snap.queue_ids,
        &snap.pb_status,
    );

    PlaybackActivityDto { rows }
}

/// Get all output devices
#[tauri::command]
pub fn get_output_devices() -> Vec<OutputDeviceInfo> {
    crate::audio::get_output_devices()
}

/// Get virtual mic devices only
#[tauri::command]
pub fn get_virtual_mic_devices() -> Vec<OutputDeviceInfo> {
    crate::audio::get_virtual_mic_devices()
}

/// Get current audio settings
#[tauri::command]
pub fn get_audio_settings(
    settings_manager: State<'_, SettingsManager>,
) -> Result<crate::config::AudioSettings, String> {
    settings_manager
        .load()
        .map(|s| s.audio)
        .map_err(|e| e.to_string())
}

/// Set speaker device
#[tauri::command]
pub async fn set_speaker_device(
    app_handle: AppHandle,
    device_id: Option<String>,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_speaker_device(device_id)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set speaker enabled
#[tauri::command]
pub async fn set_speaker_enabled(
    app_handle: AppHandle,
    enabled: bool,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_speaker_enabled(enabled)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set speaker volume
#[tauri::command]
pub async fn set_speaker_volume(
    app_handle: AppHandle,
    volume: u8,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_speaker_volume(volume)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set virtual mic device
#[tauri::command]
pub async fn set_virtual_mic_device(
    app_handle: AppHandle,
    device_id: Option<String>,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_virtual_mic_device(device_id)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Enable virtual mic
#[tauri::command]
pub async fn enable_virtual_mic(
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    // Enable by setting a device
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_virtual_mic_device(Some("".to_string()))
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Disable virtual mic
#[tauri::command]
pub async fn disable_virtual_mic(
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_virtual_mic_device(None)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set virtual mic volume
#[tauri::command]
pub async fn set_virtual_mic_volume(
    app_handle: AppHandle,
    volume: u8,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_virtual_mic_volume(volume)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Test playback on a specific audio device
/// Plays a short test sound on the specified device with the given volume
#[tauri::command]
pub async fn test_audio_device(device_id: Option<String>, volume: u8) -> Result<(), String> {
    info!(?device_id, volume, "Testing audio device");

    tauri::async_runtime::spawn_blocking(move || {
        let mp3_data = crate::assets::TEST_SOUND_MP3.to_vec();

        let config = crate::audio::OutputConfig {
            device_id,
            volume: volume as f32 / 100.0,
        };

        let mut player = crate::audio::AudioPlayer::new();
        player.play_test_sound_blocking(mp3_data, config)?;

        info!("Test sound playback completed");
        Ok(())
    })
    .await
    .map_err(|e| format!("Ошибка потока воспроизведения: {}", e))?
}

/// Get audio effects settings
#[tauri::command]
pub fn get_audio_effects(
    settings_manager: State<'_, SettingsManager>,
) -> crate::config::AudioEffectsSettings {
    settings_manager.get_audio_effects()
}

/// Set audio effects enabled
#[tauri::command]
pub async fn set_audio_effects_enabled(
    app_handle: AppHandle,
    enabled: bool,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_audio_effects_enabled(enabled)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set audio effects pitch
#[tauri::command]
pub async fn set_audio_effects_pitch(
    app_handle: AppHandle,
    pitch: i16,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_audio_effects_pitch(pitch)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set audio effects speed
#[tauri::command]
pub async fn set_audio_effects_speed(
    app_handle: AppHandle,
    speed: i16,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_audio_effects_speed(speed)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set audio effects volume
#[tauri::command]
pub async fn set_audio_effects_volume(
    app_handle: AppHandle,
    volume: i16,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_audio_effects_volume(volume)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set audio effects enhance (DeepFilterNet noise suppression) enabled
#[tauri::command]
pub async fn set_audio_effects_enhance_enabled(
    app_handle: AppHandle,
    enabled: bool,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_audio_effects_enhance_enabled(enabled)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set audio effects enhance attenuation limit (dB, 5..30)
#[tauri::command]
pub async fn set_audio_effects_enhance_atten_db(
    app_handle: AppHandle,
    atten_db: f32,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_audio_effects_enhance_atten_db(atten_db)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set audio effects formant preservation (Signalsmith formant correction)
#[tauri::command]
pub async fn set_audio_effects_formant_preserved(
    app_handle: AppHandle,
    preserved: bool,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_audio_effects_formant_preserved(preserved)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Get DSP post-processing settings
#[tauri::command]
pub fn get_dsp_settings(
    settings_manager: State<'_, SettingsManager>,
) -> crate::config::DspSettings {
    settings_manager.get_dsp_settings()
}

/// Atomically save all DSP post-processing settings
#[tauri::command]
pub async fn save_dsp_settings(
    dsp: crate::config::DspSettings,
    settings_manager: State<'_, SettingsManager>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_dsp_settings(&dsp)
    })
    .await
    .map_err(|e| format!("Не удалось сохранить DSP настройки: {}", e))?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

// ==================== Preview & Save Commands ====================

pub struct PreviewState {
    stop_flag: Arc<AtomicBool>,
}

impl PreviewState {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn preview_state() -> &'static StdMutex<PreviewState> {
    static STATE: OnceLock<StdMutex<PreviewState>> = OnceLock::new();
    STATE.get_or_init(|| StdMutex::new(PreviewState::new()))
}

/// Preview audio file through speaker with applied effects
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn preview_audio_file(
    file_path: String,
    speaker_device: Option<String>,
    speaker_volume: u8,
    voice_transform_enabled: bool,
    pitch: i16,
    speed: i16,
    volume: i16,
    enhance_enabled: bool,
    enhance_atten_db: f32,
    dsp_settings: Option<crate::config::DspSettings>,
) -> Result<(), String> {
    let speaker_volume = speaker_volume.clamp(0, 100);
    let pitch = pitch.clamp(-100, 100);
    let speed = speed.clamp(-100, 100);
    let volume = volume.clamp(0, 200);
    let enhance_atten_db = enhance_atten_db.clamp(5.0, 30.0);

    let dsp_config = dsp_settings.as_ref().map(|dsp| dsp.to_dsp_config());

    let dsp_has_effect = dsp_config
        .as_ref()
        .map(|d| d.eq.enabled || d.compressor.enabled || d.limiter.enabled)
        .unwrap_or(false);

    let stop_flag = {
        let state = preview_state();
        let mut guard = state
            .lock()
            .map_err(|e| format!("Ошибка блокировки плеера: {}", e))?;

        guard.stop_flag.store(true, Ordering::SeqCst);
        let next_stop_flag = Arc::new(AtomicBool::new(false));
        guard.stop_flag = next_stop_flag.clone();
        next_stop_flag
    };

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    if stop_flag.load(Ordering::SeqCst) {
        return Ok(());
    }

    tauri::async_runtime::spawn_blocking(move || {
        let file_data =
            std::fs::read(&file_path).map_err(|e| format!("Не удалось прочитать файл: {}", e))?;

        let voice_pitch = if voice_transform_enabled { pitch } else { 0 };
        let voice_speed = if voice_transform_enabled { speed } else { 0 };
        let voice_volume = if voice_transform_enabled { volume } else { 100 };

        let effects_config = AudioEffects::new(voice_pitch, voice_speed, voice_volume)
            .with_enhance(enhance_enabled, enhance_atten_db)
            .with_fail_on_enhance_error(true);

        let has_effects = voice_pitch != 0
            || voice_speed != 0
            || voice_volume != 100
            || enhance_enabled
            || dsp_has_effect;

        let pcm: AudioPcm = if has_effects {
            effects::apply_effects(&file_data, &effects_config, dsp_config.as_ref())?
        } else {
            decode_audio(&file_data).map_err(|e| format!("Audio decode failed: {}", e))?
        };

        let output_volume = if voice_transform_enabled {
            (speaker_volume as f32 / 100.0) * (voice_volume as f32 / 100.0)
        } else {
            speaker_volume as f32 / 100.0
        };

        let config = OutputConfig {
            device_id: speaker_device,
            volume: output_volume,
        };

        AudioPlayer::play_preview_pcm_with_stop_flag(stop_flag, &pcm, config)
    })
    .await
    .map_err(|e| format!("Ошибка потока воспроизведения: {}", e))?
}

/// Stop current preview playback
#[tauri::command]
pub fn stop_preview() -> Result<(), String> {
    let state = preview_state();
    let guard = state
        .lock()
        .map_err(|e| format!("Ошибка блокировки плеера: {}", e))?;
    guard.stop_flag.store(true, Ordering::SeqCst);
    Ok(())
}

/// Atomically save all audio effects settings
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn save_audio_effects(
    enabled: bool,
    pitch: i16,
    speed: i16,
    volume: i16,
    enhance_enabled: bool,
    enhance_atten_db: f32,
    formant_preserved: bool,
    boundary_cleanup_enabled: bool,
    settings_manager: State<'_, SettingsManager>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let effects = crate::config::AudioEffectsSettings {
        enabled,
        pitch,
        speed,
        volume,
        enhance_enabled,
        enhance_atten_db,
        formant_preserved,
        boundary_cleanup_enabled,
    };

    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_audio_effects_section(effects)
    })
    .await
    .map_err(|e| format!("Не удалось сохранить настройки: {}", e))?;

    super::emit_settings_changed(&app_handle);
    Ok(())
}
