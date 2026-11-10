use crate::commands::playback::PlaybackState;
use crate::config::SettingsManager;
use crate::ipc::{self, speech as speech_contract, CommandError};
use crate::speech_queue::{
    AcceptedJob, DeliveryPolicy, JobStatus, QueueError, Snapshot, SpeechQueue, SpeechQueueStateDto,
    SubmissionSource,
};
use crate::state::AppState;
use crate::telegram::SileroRuntimeSettings;
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Notify;
use tracing::{info, warn};
use uuid::Uuid;

pub struct SpeechQueueState {
    queue: Arc<Mutex<SpeechQueue>>,
    notify: Arc<Notify>,
}

impl SpeechQueueState {
    pub fn new(queue: SpeechQueue) -> Self {
        Self {
            queue: Arc::new(Mutex::new(queue)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn lock(&self) -> parking_lot::MutexGuard<'_, SpeechQueue> {
        self.queue.lock()
    }

    pub(crate) fn notify_one(&self) {
        self.notify.notify_one();
    }

    pub(crate) fn notifier(&self) -> Arc<Notify> {
        self.notify.clone()
    }

    pub(crate) fn queue_arc(&self) -> Arc<Mutex<SpeechQueue>> {
        self.queue.clone()
    }
}

fn emit_queue_changed(app_handle: &AppHandle, dto: SpeechQueueStateDto) {
    let _ = app_handle.emit(speech_contract::QUEUE_CHANGED_EVENT, dto);
}

fn map_submit_queue_error(error: QueueError) -> CommandError {
    let code = match &error {
        QueueError::EmptyText => speech_contract::error_code::EMPTY_TEXT,
        QueueError::QueueFull(_) => speech_contract::error_code::QUEUE_FULL,
        _ => speech_contract::error_code::QUEUE_REJECTED,
    };
    let retryable = ipc::speech_error_code_to_retryable(code);
    CommandError::new(code, error.to_string(), retryable)
}

fn resolve_silero_speaker(settings_voice: &str, captured: Option<&str>) -> Result<String, String> {
    let trimmed = settings_voice.trim();
    if !trimmed.is_empty() {
        Ok(trimmed.to_string())
    } else if let Some(existing) = captured {
        Ok(existing.to_string())
    } else {
        Err("No active Silero speaker selected".to_string())
    }
}

pub(crate) fn build_snapshot(
    state: &AppState,
    source: SubmissionSource,
    delivery: DeliveryPolicy,
) -> Result<Snapshot, String> {
    let settings = state.settings_cache.read().clone();

    let registry = state.tts_registry.lock();
    let entry = registry
        .active()
        .ok_or_else(|| "No active TTS provider configured".to_string())?;
    let tts_provider = match &entry.provider {
        crate::tts::TtsProvider::Silero(silero) => {
            let speaker = resolve_silero_speaker(
                &settings.tts.telegram.current_voice_id,
                silero.captured_speaker(),
            )?;
            let runtime = SileroRuntimeSettings::new(
                settings.tts.telegram.synthesis_response_timeout_ms,
                settings.tts.telegram.download_retry_delay_ms,
            );
            let captured = silero
                .clone()
                .with_captured_speaker(speaker)
                .with_runtime_settings(runtime);
            crate::tts::TtsProvider::Silero(captured)
        }
        other => other.clone(),
    };
    let provider = tts_provider.provider_kind_str().to_string();
    let voice = tts_provider
        .voice_identity_or_registry(&entry.id)
        .to_string();
    drop(registry);

    let preprocessor = state.editor.get_preprocessor();
    let network_settings = settings.tts.network.clone();
    let accentor_runtime = crate::commands::tts_pipeline::selected_ruaccent_runtime_slot(state);

    Ok(Snapshot {
        provider,
        voice,
        source,
        delivery,
        ai_enabled: settings.editor.ai,
        number_conversion_enabled: settings.editor.number_conversion_enabled,
        audio_effects: settings.audio_effects,
        dsp: settings.dsp,
        audio: settings.audio,
        ai: settings.ai,
        tts_provider,
        preprocessor,
        network_settings,
        accentor_runtime,
    })
}

/// Reusable application-level speech submission seam.
///
/// Builds a provider/settings snapshot, atomically enqueues the job, emits the
/// typed queue event and wakes the speech worker. Shared by the Tauri
/// `submit_speech` command (editor source) and later HTTP handlers
/// (`external` + `audio_only`).
pub fn submit_speech_job(
    app_handle: &AppHandle,
    state: &AppState,
    queue: &SpeechQueueState,
    text: String,
    source: SubmissionSource,
    delivery: DeliveryPolicy,
) -> Result<AcceptedJob, CommandError> {
    let snapshot = build_snapshot(state, source, delivery).map_err(|error| {
        CommandError::new(
            speech_contract::error_code::SNAPSHOT_UNAVAILABLE,
            format!("Snapshot error: {error}"),
            ipc::speech_error_code_to_retryable(speech_contract::error_code::SNAPSHOT_UNAVAILABLE),
        )
    })?;
    let mut q = queue.lock();
    let job_id = q.submit(&text, snapshot).map_err(map_submit_queue_error)?;
    let dto = q.state();
    drop(q);
    emit_queue_changed(app_handle, dto);
    queue.notify_one();
    Ok(AcceptedJob { job_id })
}

#[tauri::command]
pub fn submit_speech(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    queue: State<'_, SpeechQueueState>,
    text: String,
) -> Result<AcceptedJob, CommandError> {
    let prefix = crate::preprocessor::parse_prefix(&text);
    if prefix.twitch_only {
        return Err(CommandError::new(
            speech_contract::error_code::TWITCH_ONLY_ROUTE,
            "Twitch-only route must use deliver_twitch_message".to_string(),
            ipc::speech_error_code_to_retryable(speech_contract::error_code::TWITCH_ONLY_ROUTE),
        ));
    }

    let delivery = DeliveryPolicy::editor(prefix.skip_twitch, prefix.skip_webview);
    submit_speech_job(
        &app_handle,
        &state,
        &queue,
        text,
        SubmissionSource::Editor,
        delivery,
    )
}

#[tauri::command]
pub fn get_speech_queue_state(queue: State<'_, SpeechQueueState>) -> SpeechQueueStateDto {
    queue.lock().state()
}

#[tauri::command]
pub fn retry_speech_job(
    app_handle: AppHandle,
    queue: State<'_, SpeechQueueState>,
    job_id: Uuid,
) -> Result<(), String> {
    let mut q = queue.lock();
    q.retry_job(job_id).map_err(|e| e.to_string())?;
    let dto = q.state();
    drop(q);
    emit_queue_changed(&app_handle, dto);
    queue.notify_one();
    Ok(())
}

#[tauri::command]
pub fn cancel_speech_job(
    app_handle: AppHandle,
    queue: State<'_, SpeechQueueState>,
    playback: State<'_, PlaybackState>,
    job_id: Uuid,
) -> Result<(), String> {
    let handoff_guard = {
        let mut q = queue.lock();
        let status = q
            .get_status(job_id)
            .ok_or_else(|| "Job not found".to_string())?;

        match status {
            JobStatus::Queued | JobStatus::Failed => {
                q.cancel_job(job_id).map_err(|e| e.to_string())?;
                let dto = q.state();
                drop(q);
                info!(job_id = %job_id, observed_status = ?status, "cancel accepted");
                emit_queue_changed(&app_handle, dto);
                queue.notify_one();
                return Ok(());
            }
            JobStatus::Generating => {
                q.cancel_generating_job(job_id).map_err(|e| e.to_string())?;
                let dto = q.state();
                drop(q);
                info!(job_id = %job_id, observed_status = ?status, "cancel accepted");
                emit_queue_changed(&app_handle, dto);
                queue.notify_one();
                return Ok(());
            }
            JobStatus::Ready => {
                let guard = q
                    .get_handoff_guard(job_id)
                    .ok_or_else(|| "Job not found".to_string())?;
                drop(q);
                Some(guard)
            }
            _ => {
                info!(job_id = %job_id, observed_status = ?status, "cancel rejected");
                return Err("Cannot cancel: job is in a non-cancellable state".to_string());
            }
        }
    };

    if let Some(guard) = handoff_guard {
        let _g = guard.lock();

        let pb = &playback.inner().0;
        let id_str = job_id.to_string();

        let is_current = pb.current_id().map(|c| c == id_str).unwrap_or(false);
        if is_current {
            info!(job_id = %job_id, reason = "current_protected", "cancel rejected");
            return Err("Cannot cancel: audio is currently playing".to_string());
        }

        let mut removed_from_tail = false;
        if pb.queued_ids().iter().any(|q| q == &id_str) {
            match pb.remove_queued_item(&id_str) {
                Ok(()) => {
                    removed_from_tail = true;
                }
                Err(ref e) if e.starts_with("NotQueued") => {
                    info!(job_id = %job_id, reason = "became_current_during_removal", "cancel rejected");
                    return Err("Cannot cancel: audio is currently playing".to_string());
                }
                Err(e) => return Err(e),
            }
        }

        let mut q = queue.lock();
        let current_status = q.get_status(job_id);
        match current_status {
            Some(JobStatus::Ready) => {
                q.cancel_ready_job(job_id).map_err(|e| e.to_string())?;
                let dto = q.state();
                drop(q);
                let location = if removed_from_tail { "tail" } else { "absent" };
                info!(job_id = %job_id, location, "cancel accepted");
                emit_queue_changed(&app_handle, dto);
                queue.notify_one();
                Ok(())
            }
            Some(JobStatus::Cancelled) => {
                drop(q);
                info!(job_id = %job_id, "cancel already committed");
                Ok(())
            }
            Some(JobStatus::Playing) => {
                drop(q);
                info!(job_id = %job_id, reason = "became_current", "cancel rejected");
                Err("Cannot cancel: audio is currently playing".to_string())
            }
            other => {
                let msg = format!("Cannot cancel: unexpected status {:?}", other);
                warn!(job_id = %job_id, status = ?other, "unexpected ready-cancel status");
                Err(msg)
            }
        }
    } else {
        unreachable!()
    }
}

#[tauri::command]
pub fn skip_speech_job(
    app_handle: AppHandle,
    queue: State<'_, SpeechQueueState>,
    job_id: Uuid,
) -> Result<(), String> {
    let mut q = queue.lock();
    q.skip_job(job_id).map_err(|e| e.to_string())?;
    let dto = q.state();
    drop(q);
    emit_queue_changed(&app_handle, dto);
    queue.notify_one();
    Ok(())
}

#[tauri::command]
pub fn restore_cancelled_speech_job(
    app_handle: AppHandle,
    queue: State<'_, SpeechQueueState>,
    playback: State<'_, PlaybackState>,
    settings_manager: State<'_, SettingsManager>,
    job_id: Uuid,
) -> Result<(), String> {
    let spoken_text_is_set = {
        let q = queue.lock();
        let status = q
            .get_status(job_id)
            .ok_or_else(|| "Job not found".to_string())?;
        let error = q.get_error(job_id);
        if status != JobStatus::Cancelled || error.is_some() {
            return Err("Cannot restore: job is not user-cancelled".to_string());
        }
        q.get_spoken_text(job_id).is_some()
    };

    if spoken_text_is_set {
        let settings = settings_manager
            .load()
            .map_err(|e| format!("Failed to load settings: {}", e))?;
        let (speaker, mic) = crate::commands::tts_pipeline::compute_output_configs(
            &settings.audio,
            &settings.audio_effects,
        );

        let pb = &playback.inner().0;
        match pb.replay_from_cache(&job_id.to_string(), speaker, mic) {
            Ok(()) => {
                let mut q = queue.lock();
                q.touch_activity(job_id);
                let dto = q.state();
                drop(q);
                emit_queue_changed(&app_handle, dto);
                return Ok(());
            }
            Err(ref e) if is_cache_miss_error(e) => {}
            Err(e) => return Err(e),
        }
    }

    let mut q = queue.lock();
    q.restore_cancelled_job(job_id).map_err(|e| e.to_string())?;
    let dto = q.state();
    drop(q);
    emit_queue_changed(&app_handle, dto);
    queue.notify_one();
    Ok(())
}

pub(crate) fn is_cache_miss_error(err: &str) -> bool {
    err.starts_with("CacheMiss")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_command_name_matches_registered_function() {
        assert_eq!(speech_contract::SUBMIT_COMMAND, stringify!(submit_speech));
    }

    #[test]
    fn submit_queue_full_error_is_structured_and_retryable() {
        let error = map_submit_queue_error(QueueError::QueueFull(10));

        assert_eq!(error.code, speech_contract::error_code::QUEUE_FULL);
        assert!(error.retryable);
        assert!(error.message.contains("queue full"));
    }

    #[test]
    fn submit_empty_text_error_is_structured_and_not_retryable() {
        let error = map_submit_queue_error(QueueError::EmptyText);

        assert_eq!(error.code, speech_contract::error_code::EMPTY_TEXT);
        assert!(!error.retryable);
        assert!(error.message.contains("empty"));
    }

    #[test]
    fn submit_generic_queue_error_is_structured_and_not_retryable() {
        let error = map_submit_queue_error(QueueError::JobNotFound(Uuid::new_v4()));

        assert_eq!(error.code, speech_contract::error_code::QUEUE_REJECTED);
        assert!(!error.retryable);
    }

    #[test]
    fn resolve_silero_speaker_settings_takes_precedence_over_captured() {
        let result = resolve_silero_speaker("baya_16", Some("old_speaker"));
        assert_eq!(result, Ok("baya_16".to_string()));
    }

    #[test]
    fn resolve_silero_speaker_falls_back_to_captured_when_settings_empty() {
        let result = resolve_silero_speaker("", Some("existing_speaker"));
        assert_eq!(result, Ok("existing_speaker".to_string()));
    }

    #[test]
    fn resolve_silero_speaker_trims_settings_whitespace() {
        let result = resolve_silero_speaker("  baya_16  ", Some("old"));
        assert_eq!(result, Ok("baya_16".to_string()));
    }

    #[test]
    fn resolve_silero_speaker_empty_settings_and_no_captured_returns_error() {
        let result = resolve_silero_speaker("", None);
        assert_eq!(result, Err("No active Silero speaker selected".to_string()));
    }

    #[test]
    fn resolve_silero_speaker_whitespace_only_settings_falls_back_to_captured() {
        let result = resolve_silero_speaker("   ", Some("fallback"));
        assert_eq!(result, Ok("fallback".to_string()));
    }

    #[test]
    fn resolve_silero_speaker_whitespace_only_settings_and_no_captured_returns_error() {
        let result = resolve_silero_speaker("   ", None);
        assert_eq!(result, Err("No active Silero speaker selected".to_string()));
    }

    #[test]
    fn is_cache_miss_true_for_cache_miss_prefix() {
        assert!(is_cache_miss_error("CacheMiss: no cached audio for id 'x'"));
        assert!(is_cache_miss_error("CacheMiss"));
    }

    #[test]
    fn is_cache_miss_false_for_other_errors() {
        assert!(!is_cache_miss_error("QueueFull: playback queue is full"));
        assert!(!is_cache_miss_error(
            "AlreadyPending: id 'x' is already queued"
        ));
        assert!(!is_cache_miss_error(""));
        assert!(!is_cache_miss_error("Some other error"));
    }

    #[test]
    fn build_snapshot_captures_timing_values_from_settings() {
        use crate::config::AppSettings;
        use crate::state::AppState;
        use crate::tts::registry::TtsProviderEntry;
        use crate::tts::TtsProvider;

        let state = AppState::new();

        let mut settings = AppSettings::default();
        settings.tts.telegram.synthesis_response_timeout_ms = 7777;
        settings.tts.telegram.download_retry_delay_ms = 888;
        settings.tts.telegram.current_voice_id = "test_vc".to_string();
        *state.settings_cache.write() = settings;

        {
            let mut registry = state.tts_registry.lock();
            let entry = TtsProviderEntry {
                id: "silero".to_string(),
                display_name: "Silero".to_string(),
                provider: TtsProvider::Silero(crate::tts::silero::SileroTts::new()),
            };
            registry.add_or_replace(entry);
            registry.select("silero").unwrap();
        }

        let snapshot = build_snapshot(
            &state,
            SubmissionSource::Editor,
            DeliveryPolicy::editor(false, false),
        )
        .expect("build_snapshot must succeed");

        if let TtsProvider::Silero(silero) = &snapshot.tts_provider {
            let rt = silero.runtime_settings();
            assert_eq!(rt.synthesis_response_timeout.as_millis(), 7777);
            assert_eq!(rt.download_retry_delay.as_millis(), 888);
            assert_eq!(silero.captured_speaker(), Some("test_vc"));
        } else {
            panic!("expected Silero provider");
        }
    }

    #[test]
    fn build_snapshot_captures_number_conversion_and_keeps_it_stable() {
        use crate::config::AppSettings;
        use crate::state::AppState;
        use crate::tts::registry::TtsProviderEntry;
        use crate::tts::TtsProvider;

        let state = AppState::new();

        let mut settings = AppSettings::default();
        settings.editor.number_conversion_enabled = false;
        *state.settings_cache.write() = settings;

        {
            let mut registry = state.tts_registry.lock();
            let entry = TtsProviderEntry {
                id: "local".to_string(),
                display_name: "Local".to_string(),
                provider: TtsProvider::Local(
                    crate::tts::local_http_server::LocalHttpServerTts::new(),
                ),
            };
            registry.add_or_replace(entry);
            registry.select("local").unwrap();
        }

        let snapshot = build_snapshot(
            &state,
            SubmissionSource::Editor,
            DeliveryPolicy::editor(false, false),
        )
        .expect("build_snapshot must succeed");
        assert!(!snapshot.number_conversion_enabled);

        state
            .settings_cache
            .write()
            .editor
            .number_conversion_enabled = true;
        assert!(
            !snapshot.number_conversion_enabled,
            "queued snapshot must retain the value captured at submit time"
        );
    }
}
