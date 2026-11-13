use crate::audio::{
    apply_effects, decode_audio, process_boundaries, AudioEffects, AudioPcm, OutputConfig,
};
use crate::config::{
    AiSettings, AppSettings, AudioEffectsSettings, AudioSettings, DspSettings, NetworkSettings,
};
use crate::speech_queue::{DeliveryPolicy, Snapshot};
use crate::state::AppState;
use crate::stress::adapters::ProviderStressAdapter;
use crate::stress::annotations::StructuredStress;
use crate::stress::runtime::RuAccentRuntimeSlot;
use crate::tts::TtsProvider;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub struct PreparedSpeech {
    pub provider_text: String,
    pub insert_text: String,
    /// ROADMAP-107: request-local external delivery text — the resolved speech
    /// content (editor route prefix stripped / incoming literal) with the
    /// configured replacements and username substitutions applied, but BEFORE
    /// number conversion, AI correction and stress markers. External channels
    /// choose between this and `insert_text` at routing time via their
    /// per-server `send_original_text` setting.
    pub delivery_text: String,
    pub audio: AudioPcm,
    pub provider_name: String,
    pub voice_name: String,
    pub cache_key: String,
    pub cache_hit: bool,
    pub cache_saved: bool,
}

// ── Snapshot-friendly inner helpers ──

/// Apply the configured replacements (including username substitutions)
/// without number conversion.
///
/// This is the replacement half of [`preprocess_text_with_preprocessor`],
/// exposed separately so the external delivery representation can be built
/// after replacements but before number conversion, AI correction and stress
/// markup.
pub(crate) fn apply_replacements_with_preprocessor(
    text: &str,
    preprocessor: Option<&crate::preprocessor::TextPreprocessor>,
) -> String {
    if let Some(p) = preprocessor {
        let processed = p.process(text);
        if processed != text {
            debug!(
                original_len = text.chars().count(),
                processed_len = processed.chars().count(),
                "Replacements applied"
            );
        }
        processed
    } else {
        text.to_string()
    }
}

pub(crate) fn preprocess_text_with_preprocessor(
    text: &str,
    preprocessor: Option<&crate::preprocessor::TextPreprocessor>,
    convert_numbers: bool,
) -> String {
    let text = apply_replacements_with_preprocessor(text, preprocessor);
    if convert_numbers {
        crate::preprocessor::process_numbers(&text)
    } else {
        text
    }
}

pub(crate) async fn ai_correct_text_with_settings(
    text: &str,
    ai_enabled: bool,
    ai_settings: &AiSettings,
    network_settings: &NetworkSettings,
) -> Result<String, String> {
    if !ai_enabled {
        return Ok(text.to_string());
    }
    let client = crate::ai::create_ai_client(ai_settings, network_settings)
        .map_err(|e| format!("Failed to create AI client: {}", e))?;
    match client.correct(text, &ai_settings.prompt).await {
        Ok(corrected) => {
            if corrected != text {
                info!(
                    original = text.len(),
                    corrected = corrected.len(),
                    "AI correction applied"
                );
            }
            Ok(corrected)
        }
        Err(e) => {
            warn!("AI correction failed, using original text: {}", e);
            Ok(text.to_string())
        }
    }
}

pub(crate) async fn synthesize_with_provider(
    provider: &TtsProvider,
    text: &str,
) -> Result<Vec<u8>, String> {
    let audio_data = provider.synthesize(text).await.map_err(|e| {
        error!(error = %e, "synthesize() error");
        format!("Ошибка синтеза: {}", e)
    })?;
    debug!(bytes = audio_data.len(), "Audio synthesized");
    Ok(audio_data)
}

/// Map a TTS provider to its stress-marker adapter.
///
/// Silero consumes `+` markers; Piper/eSpeak consumes U+0301 after the
/// stressed vowel; OpenAI/Fish/Local receive plain text.
fn provider_to_stress_adapter(provider: &TtsProvider) -> ProviderStressAdapter {
    match provider {
        TtsProvider::Silero(_) => ProviderStressAdapter::Silero,
        TtsProvider::Piper(_) => ProviderStressAdapter::Piper,
        TtsProvider::Pocket(tts) if tts.language().eq_ignore_ascii_case("ru") => {
            ProviderStressAdapter::Piper
        }
        TtsProvider::OpenAi(_)
        | TtsProvider::Fish(_)
        | TtsProvider::Local(_)
        | TtsProvider::ElevenLabs(_)
        | TtsProvider::Pocket(_) => ProviderStressAdapter::PlainText,
    }
}

/// Render structured stress for a provider into the text handed to synthesis.
fn adapt_structured_stress(provider: &TtsProvider, stress: &StructuredStress) -> String {
    provider_to_stress_adapter(provider).adapt(stress)
}

/// Split structured stress into the two explicit representations of a phrase.
///
/// `provider_text` is the provider-specific rendering that reaches synthesis and
/// the cache key; `insert_text` is always `StructuredStress.original`, so it is
/// marker-free and never carries `ё` render-base restorations. Only
/// `provider_text` crosses the provider boundary.
fn provider_and_insert_text(provider: &TtsProvider, stress: &StructuredStress) -> (String, String) {
    let provider_text = adapt_structured_stress(provider, stress);
    let insert_text = stress.original.clone();
    (provider_text, insert_text)
}

/// Build structured stress for a text, annotating stress positions via the
/// native `ruaccent_rs` runtime when a slot is present.
///
/// Fail-open: native load/inference errors are logged with `warn!` and the
/// original text is preserved, so synthesis never blocks on the accentor.
fn structured_stress_for_text(
    runtime: Option<&RuAccentRuntimeSlot>,
    text: &str,
) -> Result<StructuredStress, String> {
    let empty = || {
        StructuredStress::new(text.to_string(), vec![])
            .map_err(|e| format!("Structured stress construction failed: {e}"))
    };
    match runtime {
        Some(slot) => match slot.annotate(text) {
            Ok(stress) => Ok(stress),
            Err(e) => {
                warn!("Native stress annotation failed, using original text: {e}");
                empty()
            }
        },
        None => empty(),
    }
}

/// Build structured stress for a text through the single shared native stress
/// pipeline used by both synthesis and preview.
///
/// The pipeline is:
/// 1. parse manual Silero `+` markers into clean text;
/// 2. run the selected native slot if present, otherwise retain the clean text;
/// 3. merge the manual markers over the automatic annotations.
///
/// The result is provider-neutral: `+` is only re-introduced by the Silero /
/// preview adapter at the rendering boundary.
pub(crate) fn native_stress(
    accentor_runtime: Option<&RuAccentRuntimeSlot>,
    text: &str,
) -> Result<StructuredStress, String> {
    let manual = crate::stress::contextual::structured_stress_from_silero_marked(text)?;
    let clean_text = manual.original.clone();
    let automatic = structured_stress_for_text(accentor_runtime, &clean_text)?;
    crate::stress::contextual::merge_manual_stress(manual, automatic)
}

/// Build structured stress through the native accentor, propagating inference
/// and output-validation errors.
///
/// Unlike [`native_stress`], this never fails open: an inference error or an
/// invalid accentor output is returned to the caller. Used by the manual
/// preview path, which must not silently return unchanged text as success.
pub(crate) fn native_stress_strict(
    accentor_runtime: &RuAccentRuntimeSlot,
    text: &str,
) -> Result<StructuredStress, String> {
    let manual = crate::stress::contextual::structured_stress_from_silero_marked(text)?;
    let clean_text = manual.original.clone();
    let automatic = accentor_runtime.annotate(&clean_text)?;
    crate::stress::contextual::merge_manual_stress(manual, automatic)
}

/// Resolve the lazy native RUAccent runtime slot for the currently enabled
/// editor homograph/accentor setting.
///
/// Returns `None` when the layer is disabled, the selected pack ID is missing
/// or unknown, no runtime slot was created for the pack, or its runtime is not
/// `Ready`. Never loads models, creates sessions, or alters text — pure slot
/// selection only.
#[allow(dead_code)]
pub(crate) fn selected_ruaccent_runtime_slot(state: &AppState) -> Option<RuAccentRuntimeSlot> {
    let accentor = state
        .settings_cache
        .read()
        .editor
        .homograph_accentor
        .clone();
    if !accentor.enabled {
        return None;
    }
    let pack_id = accentor.accentor_pack_id?;
    state
        .get_ruaccent_runtime_slot(&pack_id)
        .filter(|slot| slot.is_ready())
}

/// Resolve the lazy native RUAccent runtime slot for the editor preview path.
///
/// Unlike [`selected_ruaccent_runtime_slot`], this deliberately ignores the
/// `enabled` flag: the preview is allowed to run whenever a native-ready pack
/// is selected, even if automatic TTS preprocessing is disabled. It still
/// returns `None` when the selected pack ID is missing or unknown, when no
/// runtime slot was created for the pack, or when its runtime is not `Ready`.
/// Never loads models, creates sessions, or alters text — pure slot selection
/// only.
#[allow(dead_code)]
pub(crate) fn preview_ruaccent_runtime_slot(state: &AppState) -> Option<RuAccentRuntimeSlot> {
    let accentor = state
        .settings_cache
        .read()
        .editor
        .homograph_accentor
        .clone();
    let pack_id = accentor.accentor_pack_id?;
    state
        .get_ruaccent_runtime_slot(&pack_id)
        .filter(|slot| slot.is_ready())
}

pub(crate) fn apply_audio_effects_pipeline_with_settings(
    audio_data: Vec<u8>,
    audio_effects: &AudioEffectsSettings,
    dsp: &DspSettings,
) -> Result<AudioPcm, String> {
    let pcm = if audio_effects.enabled {
        let effects = AudioEffects::new(
            audio_effects.pitch,
            audio_effects.speed,
            audio_effects.volume,
        )
        .with_enhance(
            audio_effects.enhance_enabled,
            audio_effects.enhance_atten_db,
        )
        .with_formant_preserved(audio_effects.formant_preserved);

        let original_len = audio_data.len();
        let dsp_config = dsp.to_dsp_config();
        match apply_effects(&audio_data, &effects, Some(&dsp_config)) {
            Ok(pcm) => {
                debug!(
                    original = original_len,
                    frames = pcm.frame_count(),
                    "Audio effects applied"
                );
                pcm
            }
            Err(e) => {
                error!(error = %e, "Failed to apply audio effects");
                return Err(format!("Не удалось применить аудио эффекты: {}", e));
            }
        }
    } else {
        decode_audio(&audio_data).map_err(|e| format!("Audio decode failed: {}", e))?
    };

    if audio_effects.boundary_cleanup_enabled {
        let cleaned = process_boundaries(&pcm);
        if !cleaned.samples.is_empty()
            && cleaned.sample_rate == pcm.sample_rate
            && cleaned.channels == pcm.channels
            && cleaned.frame_count() == pcm.frame_count()
        {
            debug!(frames = cleaned.frame_count(), "Boundary cleanup applied");
            return Ok(cleaned);
        }
        warn!("Boundary cleanup produced invalid result, falling back to original PCM");
    }

    Ok(pcm)
}

// ── Blocking stage isolation ──
//
// Filesystem cache read/decode, effects/DSP/boundary processing, and WAV cache
// encode/write are CPU/IO-bound and must not run on the async worker thread.
// Each stage runs inside `spawn_blocking` with owned inputs so the speech worker
// keeps observing shutdown and other control futures while the stage executes.

/// Map a `spawn_blocking` `JoinError` into the preparation error contract.
fn join_error_to_speech_error(join_err: tokio::task::JoinError) -> String {
    if join_err.is_panic() {
        format!("Speech preparation background task panicked: {join_err}")
    } else {
        format!("Speech preparation background task was cancelled: {join_err}")
    }
}

/// Run a fallible blocking stage off the async worker thread.
///
/// The inner error is already a `String` (the preparation error contract), and
/// a `JoinError` (panic or runtime shutdown) is mapped to the same contract.
async fn spawn_blocking_prepare<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(inner) => inner,
        Err(join_err) => Err(join_error_to_speech_error(join_err)),
    }
}

async fn read_cache_blocking(cache_key: String) -> Result<AudioPcm, String> {
    spawn_blocking_prepare(move || {
        crate::history::read_audio_cache(&cache_key).map_err(|e| e.to_string())
    })
    .await
}

async fn apply_effects_blocking(
    audio_data: Vec<u8>,
    audio_effects: AudioEffectsSettings,
    dsp: DspSettings,
) -> Result<AudioPcm, String> {
    spawn_blocking_prepare(move || {
        apply_audio_effects_pipeline_with_settings(audio_data, &audio_effects, &dsp)
    })
    .await
}

async fn save_cache_blocking(cache_key: String, audio: AudioPcm) -> bool {
    spawn_blocking_prepare(move || {
        crate::history::save_audio_cache(&cache_key, &audio).map_err(|e| e.to_string())
    })
    .await
    .is_ok()
}

/// Run native RUAccent inference off the async worker thread, so the ONNX
/// session never blocks the Tokio executor. The runtime mutex serialization is
/// retained inside the slot, and a panicking blocking task is mapped to the
/// preparation error contract.
async fn native_stress_blocking(
    accentor_runtime: Option<RuAccentRuntimeSlot>,
    text: String,
) -> Result<StructuredStress, String> {
    spawn_blocking_prepare(move || native_stress(accentor_runtime.as_ref(), &text)).await
}

// ── Snapshot-driven preparation (pure, no side effects) ──

/// Resolve the synthesis content text from the submitted string and the
/// delivery policy stored on the snapshot.
///
/// - Editor delivery preserves the current prefix parsing, prefix removal and
///   mismatch protection.
/// - `AudioOnly` and `Incoming` treat the complete submitted string as content;
///   a leading `!` is literal text and is never stripped or interpreted as a
///   route.
fn resolve_speech_content(snapshot: &Snapshot, original_text: &str) -> Result<String, String> {
    match &snapshot.delivery {
        DeliveryPolicy::Editor {
            skip_twitch,
            skip_webview,
        } => {
            let prefix_result = crate::preprocessor::parse_prefix(original_text);
            if prefix_result.skip_twitch != *skip_twitch
                || prefix_result.skip_webview != *skip_webview
            {
                return Err(
                    "Internal error: prefix flags mismatch between snapshot and parsed text"
                        .to_string(),
                );
            }
            Ok(prefix_result.text)
        }
        DeliveryPolicy::AudioOnly | DeliveryPolicy::Incoming { .. } => {
            Ok(original_text.to_string())
        }
    }
}

/// Build the external delivery representation for a submission: resolve the
/// route content, then apply the configured replacements and username
/// substitutions only.
///
/// Number conversion, AI correction and provider stress markup are deliberately
/// excluded — they belong to the TTS synthesis representation and must never be
/// introduced into externally delivered text.
fn resolve_delivery_text(snapshot: &Snapshot, original_text: &str) -> Result<String, String> {
    let content = resolve_speech_content(snapshot, original_text)?;
    Ok(apply_replacements_with_preprocessor(
        &content,
        snapshot.preprocessor.as_ref(),
    ))
}

/// Resolve both representations used by one submission: the external delivery
/// text (replacements only) and the internal TTS text (replacements plus the
/// optional built-in number conversion). Number conversion is captured on the
/// snapshot so queued jobs keep the behavior chosen at submit time.
fn resolve_tts_text(snapshot: &Snapshot, original_text: &str) -> Result<(String, String), String> {
    let delivery_text = resolve_delivery_text(snapshot, original_text)?;
    let tts_text = if snapshot.number_conversion_enabled {
        crate::preprocessor::process_numbers(&delivery_text)
    } else {
        delivery_text.clone()
    };
    Ok((delivery_text, tts_text))
}

/// Error contract for preparation stopped by per-job cancellation.
///
/// The speech worker drops the preparation future on cancellation; this error is
/// the second line of defence for stages that were already dispatched when the
/// token fired (heavy blocking RUAccent / TTS / DSP work).
const SPEECH_PREPARATION_CANCELLED: &str = "Speech preparation cancelled";

/// Bail out between heavy preparation stages once the job was cancelled.
///
/// The blocking stages themselves cannot be interrupted, so every boundary
/// checks the token to avoid starting the next expensive stage for a job whose
/// output will be discarded anyway.
fn ensure_not_cancelled(cancel: &CancellationToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err(SPEECH_PREPARATION_CANCELLED.to_string())
    } else {
        Ok(())
    }
}

pub async fn prepare_speech(
    snapshot: &Snapshot,
    original_text: &str,
    cancel: &CancellationToken,
) -> Result<PreparedSpeech, String> {
    ensure_not_cancelled(cancel)?;

    let (delivery_text, text) = resolve_tts_text(snapshot, original_text)?;

    let text = match ai_correct_text_with_settings(
        &text,
        snapshot.ai_enabled,
        &snapshot.ai,
        &snapshot.network_settings,
    )
    .await
    {
        Ok(corrected) => corrected,
        Err(e) => {
            warn!(
                "AI client construction failed, using uncorrected text: {}",
                e
            );
            text
        }
    };
    ensure_not_cancelled(cancel)?;

    let stress = native_stress_blocking(snapshot.accentor_runtime.clone(), text.clone()).await?;
    ensure_not_cancelled(cancel)?;
    let (provider_text, insert_text) = provider_and_insert_text(&snapshot.tts_provider, &stress);

    let effects_fp =
        crate::history::compute_effects_fingerprint(&snapshot.audio_effects, &snapshot.dsp);
    let cache_key = crate::history::build_cache_key(
        &provider_text,
        &snapshot.provider,
        &snapshot.voice,
        effects_fp,
    );

    match read_cache_blocking(cache_key.clone()).await {
        Ok(pcm) => {
            ensure_not_cancelled(cancel)?;
            return Ok(PreparedSpeech {
                provider_text,
                insert_text,
                delivery_text,
                audio: pcm,
                provider_name: snapshot.provider.clone(),
                voice_name: snapshot.voice.clone(),
                cache_key,
                cache_hit: true,
                cache_saved: false,
            });
        }
        Err(e) => {
            if !e.contains("CacheMiss") {
                return Err(format!("Cache read error (corrupted/unreadable): {}", e));
            }
        }
    }
    ensure_not_cancelled(cancel)?;

    let audio_data = synthesize_with_provider(&snapshot.tts_provider, &provider_text).await?;
    ensure_not_cancelled(cancel)?;
    let audio = apply_effects_blocking(
        audio_data,
        snapshot.audio_effects.clone(),
        snapshot.dsp.clone(),
    )
    .await?;
    ensure_not_cancelled(cancel)?;

    let cache_saved = save_cache_blocking(cache_key.clone(), audio.clone()).await;

    Ok(PreparedSpeech {
        provider_text,
        insert_text,
        delivery_text,
        audio,
        provider_name: snapshot.provider.clone(),
        voice_name: snapshot.voice.clone(),
        cache_key,
        cache_hit: false,
        cache_saved,
    })
}

// ── Public helpers (original API wrappers for backward compatibility) ──

/// 1. Этап предварительной подготовки текста (препроцессор + замена чисел)
pub fn preprocess_text(state: &AppState, text: &str) -> String {
    let convert_numbers = state.settings_cache.read().editor.number_conversion_enabled;
    preprocess_text_with_preprocessor(
        text,
        state.editor.get_preprocessor().as_ref(),
        convert_numbers,
    )
}

/// 2. Этап AI-исправления грамматики (с безопасным fallback)
pub async fn ai_correct_text(state: &AppState, text: &str, settings: &AppSettings) -> String {
    if !settings.editor.ai {
        return text.to_string();
    }

    match state.get_or_create_ai_client(&settings.ai, &settings.tts.network) {
        Ok(client) => match client.correct(text, &settings.ai.prompt).await {
            Ok(corrected) => {
                if corrected != text {
                    info!(
                        original = text.len(),
                        corrected = corrected.len(),
                        "AI correction applied"
                    );
                }
                corrected
            }
            Err(e) => {
                warn!("AI correction failed, using original text: {}", e);
                text.to_string()
            }
        },
        Err(e) => {
            warn!("AI client not available, skipping correction: {}", e);
            text.to_string()
        }
    }
}

/// 3. Этап синтеза аудиоданных через выбранный TTS-провайдер
pub async fn synthesize_audio(state: &AppState, text: &str) -> Result<Vec<u8>, String> {
    let provider = state.get_active_provider().ok_or_else(|| {
        error!("TTS provider not initialized");
        "TTS provider не инициализирован. Выберите провайдера в настройках.".to_string()
    })?;

    let stress =
        native_stress_blocking(selected_ruaccent_runtime_slot(state), text.to_string()).await?;
    let text = adapt_structured_stress(&provider, &stress);

    synthesize_with_provider(&provider, &text).await
}

/// Build the per-phrase output configuration captured by the speech worker.
pub(crate) fn compute_output_configs(
    audio_settings: &AudioSettings,
    effects_settings: &AudioEffectsSettings,
) -> (Option<OutputConfig>, Option<OutputConfig>) {
    let effects_volume = if effects_settings.enabled {
        Some(
            AudioEffects::new(
                effects_settings.pitch,
                effects_settings.speed,
                effects_settings.volume,
            )
            .volume_factor(),
        )
    } else {
        None
    };

    let speaker_config = if audio_settings.speaker_enabled {
        let base_volume = audio_settings.speaker_volume as f32 / 100.0;
        let final_volume = effects_volume
            .map(|ev| base_volume * ev)
            .unwrap_or(base_volume);
        Some(OutputConfig {
            device_id: audio_settings.speaker_device.clone(),
            volume: final_volume,
        })
    } else {
        None
    };

    let virtual_mic_config = audio_settings.virtual_mic_device.as_ref().map(|device_id| {
        let base_volume = audio_settings.virtual_mic_volume as f32 / 100.0;
        let final_volume = effects_volume
            .map(|ev| base_volume * ev)
            .unwrap_or(base_volume);
        OutputConfig {
            device_id: Some(device_id.clone()),
            volume: final_volume,
        }
    });

    (speaker_config, virtual_mic_config)
}

/// Export raw TTS audio bytes to a file — synthesis only, no effects, no playback.
pub async fn synthesize_and_export(state: &AppState, text: &str, path: &str) -> Result<(), String> {
    let settings = state.settings_cache.read().clone();

    let prefix_result = crate::preprocessor::parse_prefix(text);
    let text = prefix_result.text;

    let text = preprocess_text(state, &text);
    let text = ai_correct_text(state, &text, &settings).await;
    let audio_data = synthesize_audio(state, &text).await?;

    tokio::fs::write(path, &audio_data)
        .await
        .map_err(|e| format!("Failed to write audio file: {}", e))?;

    // Owned handle + blocking pool so the export path never performs disk I/O
    // on an async worker while holding the history slot guard.
    if let Some(history) = state.editor.history_handle() {
        let write = move || history.record_phrase(&text);
        if let Err(error) = crate::history::write_phrase_blocking(write).await {
            tracing::error!(error = %error, "Failed to persist exported phrase history");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech_queue::SubmissionSource;

    fn make_settings(boundary_cleanup: bool) -> AppSettings {
        let mut s = AppSettings::default();
        s.audio_effects.boundary_cleanup_enabled = boundary_cleanup;
        s
    }

    fn apply_pipeline(audio_data: Vec<u8>, settings: &AppSettings) -> Result<AudioPcm, String> {
        apply_audio_effects_pipeline_with_settings(
            audio_data,
            &settings.audio_effects,
            &settings.dsp,
        )
    }

    fn generate_silent_wav() -> Vec<u8> {
        let sample_rate = 48000u32;
        let channels = 1usize;
        let frames = 1000usize;
        let samples: Vec<f32> = (0..frames).map(|_| 0.0f32).collect();
        crate::audio::effects::encode_wav(&samples, sample_rate, channels).expect("encode test WAV")
    }

    /// Boundary cleanup enabled: pipeline must return valid, finite PCM.
    #[test]
    fn pipeline_with_boundary_cleanup_enabled() {
        let wav = generate_silent_wav();
        let settings = make_settings(true);
        let result =
            apply_pipeline(wav, &settings).expect("pipeline with boundary cleanup enabled");
        assert!(result.samples.iter().all(|s| s.is_finite()));
        assert_eq!(result.sample_rate, 48000);
        assert_eq!(result.channels, 1);
    }

    /// Boundary cleanup disabled: pipeline must return valid, finite PCM.
    #[test]
    fn pipeline_with_boundary_cleanup_disabled() {
        let wav = generate_silent_wav();
        let settings = make_settings(false);
        let result =
            apply_pipeline(wav, &settings).expect("pipeline with boundary cleanup disabled");
        assert!(result.samples.iter().all(|s| s.is_finite()));
        assert_eq!(result.sample_rate, 48000);
        assert_eq!(result.channels, 1);
    }

    /// Boundary cleanup disabled must preserve original samples (no fade-in/out).
    #[test]
    fn pipeline_boundary_disabled_preserves_samples() {
        let sample_rate = 48000u32;
        let channels = 1usize;
        let frames = 2000usize;
        let samples: Vec<f32> = (0..frames)
            .map(|i| {
                (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sample_rate as f32).sin() * 0.5
            })
            .collect();
        let wav = crate::audio::effects::encode_wav(&samples, sample_rate, channels)
            .expect("encode test WAV");

        let settings = make_settings(false);
        let result = apply_pipeline(wav, &settings).expect("pipeline with boundary disabled");
        for (a, b) in samples.iter().zip(result.samples.iter()) {
            assert!(
                (a - b).abs() < 0.01,
                "samples changed with boundary disabled"
            );
        }
    }

    /// Boundary cleanup enabled + effects disabled: DeepFilterNet and DSP
    /// must still be inactive (only boundary cleanup runs).
    #[test]
    fn pipeline_boundary_enabled_does_not_enable_enhance_or_dsp() {
        let wav = generate_silent_wav();
        let mut settings = make_settings(true);
        settings.audio_effects.enabled = false;
        settings.audio_effects.enhance_enabled = false;
        settings.dsp.eq.enabled = false;
        settings.dsp.compressor.enabled = false;
        settings.dsp.limiter.enabled = false;

        let result = apply_pipeline(wav, &settings)
            .expect("pipeline with boundary enabled, effects disabled");
        // Sample rate preserved (no DeepFilterNet resampling).
        assert_eq!(result.sample_rate, 48000);
        assert!(result.samples.iter().all(|s| s.is_finite()));
    }

    /// Sample rate, channels, and frame count must be preserved after boundary cleanup.
    #[test]
    fn pipeline_preserves_metadata_with_boundary() {
        let sample_rate = 44100u32;
        let channels = 2usize;
        let frames = 500usize;
        let samples: Vec<f32> = (0..frames * channels)
            .map(|i| (i as f32 * 0.001).sin() * 0.3)
            .collect();
        let wav = crate::audio::effects::encode_wav(&samples, sample_rate, channels)
            .expect("encode stereo WAV");

        let settings = make_settings(true);
        let result = apply_pipeline(wav, &settings).expect("pipeline with boundary");
        assert_eq!(result.sample_rate, sample_rate);
        assert_eq!(result.channels, channels);
        assert_eq!(result.frame_count(), frames);
    }

    // ── Focused snapshot/preparation tests ──

    fn make_snapshot(skip_twitch: bool, skip_webview: bool, ai_enabled: bool) -> Snapshot {
        use crate::config::{
            AiSettings, AudioEffectsSettings, AudioSettings, DspSettings, NetworkSettings,
        };
        Snapshot {
            provider: "test-provider".into(),
            voice: "test-voice".into(),
            source: SubmissionSource::Editor,
            delivery: DeliveryPolicy::editor(skip_twitch, skip_webview),
            ai_enabled,
            number_conversion_enabled: true,
            audio_effects: AudioEffectsSettings::default(),
            dsp: DspSettings::default(),
            audio: AudioSettings::default(),
            ai: AiSettings::default(),
            tts_provider: crate::tts::TtsProvider::Local(
                crate::tts::local_http_server::LocalHttpServerTts::new(),
            ),
            preprocessor: None,
            network_settings: NetworkSettings::default(),
            accentor_runtime: None,
        }
    }

    /// Prefix flag mismatch between snapshot and text fails before synthesis.
    #[tokio::test]
    async fn prefix_flag_mismatch_fails_before_synthesis() {
        let snapshot = make_snapshot(true, false, false);
        let err = prepare_speech(&snapshot, "hello world", &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(
            err.contains("prefix flags mismatch"),
            "expected prefix mismatch error, got: {err}"
        );
    }

    /// An already cancelled job must not start any heavy stage: the token is
    /// checked before preprocessing, AI correction, RUAccent and TTS.
    #[tokio::test]
    async fn cancelled_preparation_bails_out_before_heavy_stages() {
        let snapshot = make_snapshot(false, false, false);
        let cancel = CancellationToken::new();
        cancel.cancel();

        let err = prepare_speech(&snapshot, "hello world", &cancel)
            .await
            .unwrap_err();
        assert!(
            err.contains("cancelled"),
            "expected cancellation error, got: {err}"
        );
    }

    /// Editor delivery preserves prefix parsing and prefix removal.
    #[test]
    fn editor_delivery_parses_and_strips_prefix() {
        let both = make_snapshot(true, true, false);
        assert_eq!(resolve_speech_content(&both, "!!hello").unwrap(), "hello");

        let skip_twitch = make_snapshot(true, false, false);
        assert_eq!(
            resolve_speech_content(&skip_twitch, "!hello").unwrap(),
            "hello"
        );

        let everywhere = make_snapshot(false, false, false);
        assert_eq!(
            resolve_speech_content(&everywhere, "hello").unwrap(),
            "hello"
        );
    }

    /// Editor delivery keeps its prefix mismatch protection intact.
    #[test]
    fn editor_delivery_preserves_prefix_mismatch_protection() {
        let snapshot = make_snapshot(true, false, false);
        let err = resolve_speech_content(&snapshot, "hello").unwrap_err();
        assert!(
            err.contains("prefix flags mismatch"),
            "expected prefix mismatch error, got: {err}"
        );
    }

    /// External audio-only treats the complete string as content; a leading `!`
    /// is literal text and is neither stripped nor interpreted as a route.
    #[test]
    fn audio_only_delivery_treats_full_string_as_content() {
        let mut snapshot = make_snapshot(false, false, false);
        snapshot.source = SubmissionSource::Server;
        snapshot.delivery = DeliveryPolicy::AudioOnly;

        assert_eq!(
            resolve_speech_content(&snapshot, "!!hello").unwrap(),
            "!!hello"
        );
        assert_eq!(
            resolve_speech_content(&snapshot, "!hello").unwrap(),
            "!hello"
        );
        assert_eq!(resolve_speech_content(&snapshot, "hello").unwrap(), "hello");
    }

    // ── External delivery representation tests ──

    /// Build a preprocessor from explicit replacement and username pairs.
    ///
    /// `ReplacementList` is re-exported to the crate only under `#[cfg(test)]`,
    /// so the test names it explicitly and mutates it via its public methods.
    fn make_preprocessor(
        replacements: &[(&str, &str)],
        usernames: &[(&str, &str)],
    ) -> crate::preprocessor::TextPreprocessor {
        let mut list: crate::preprocessor::ReplacementList = Default::default();
        for (key, value) in replacements {
            list.add_replacement(key.to_string(), value.to_string());
        }
        for (key, value) in usernames {
            list.add_username(key.to_string(), value.to_string());
        }
        crate::preprocessor::TextPreprocessor::new(list)
    }

    fn snapshot_with_preprocessor(
        skip_twitch: bool,
        skip_webview: bool,
        ai_enabled: bool,
        preprocessor: crate::preprocessor::TextPreprocessor,
    ) -> Snapshot {
        let mut snapshot = make_snapshot(skip_twitch, skip_webview, ai_enabled);
        snapshot.preprocessor = Some(preprocessor);
        snapshot
    }

    /// Replacement substitutions and username substitutions reach delivery text.
    #[test]
    fn delivery_text_applies_replacements_and_usernames() {
        let snapshot = snapshot_with_preprocessor(
            false,
            false,
            false,
            make_preprocessor(&[("fruit", "яблок")], &[("john", "Джон")]),
        );

        let delivery = resolve_delivery_text(&snapshot, r"У меня \fruit от %john").unwrap();
        assert_eq!(delivery, "У меня яблок от Джон");
    }

    /// Digits remain digits in external delivery while the TTS representation
    /// still converts them.
    #[test]
    fn delivery_text_keeps_digits_while_tts_converts() {
        let preprocessor = make_preprocessor(&[("fruit", "яблок")], &[]);
        let snapshot = snapshot_with_preprocessor(false, false, false, preprocessor.clone());

        let input = r"У меня 5 \fruit";
        let delivery = resolve_delivery_text(&snapshot, input).unwrap();
        assert_eq!(delivery, "У меня 5 яблок");
        assert!(
            delivery.contains('5'),
            "digits must stay in delivery: {delivery}"
        );
        assert!(!delivery.contains("пять"));

        let tts = preprocess_text_with_preprocessor(input, Some(&preprocessor), true);
        assert_eq!(tts, "У меня пять яблок");
        assert!(!tts.contains('5'));
    }

    /// A replacement value containing digits retains those digits in external
    /// delivery.
    #[test]
    fn delivery_text_preserves_digits_inside_replacements() {
        let preprocessor = make_preprocessor(&[("count", "5")], &[]);
        let snapshot = snapshot_with_preprocessor(false, false, false, preprocessor.clone());

        let delivery = resolve_delivery_text(&snapshot, r"\count яблок").unwrap();
        assert_eq!(delivery, "5 яблок");

        let tts = preprocess_text_with_preprocessor(r"\count яблок", Some(&preprocessor), true);
        assert_eq!(tts, "пять яблок");
    }

    /// Snapshot-driven TTS text converts numbers or preserves digit-producing
    /// replacements according to the captured setting.
    #[test]
    fn tts_text_number_conversion_follows_snapshot() {
        let preprocessor = make_preprocessor(&[("count", "5")], &[]);
        let mut snapshot = snapshot_with_preprocessor(false, false, false, preprocessor);

        let (delivery, tts) = resolve_tts_text(&snapshot, r"\count яблок").unwrap();
        assert_eq!(delivery, "5 яблок");
        assert_eq!(tts, "пять яблок");

        snapshot.number_conversion_enabled = false;
        let (delivery, tts) = resolve_tts_text(&snapshot, r"\count яблок").unwrap();
        assert_eq!(delivery, "5 яблок");
        assert_eq!(
            tts, "5 яблок",
            "disabled conversion must keep digits from replacements"
        );
    }

    /// A queued snapshot keeps its captured conversion choice even if the
    /// source snapshot's setting changes afterwards.
    #[test]
    fn tts_text_uses_captured_number_conversion_choice() {
        let preprocessor = make_preprocessor(&[("count", "5")], &[]);
        let mut snapshot = snapshot_with_preprocessor(false, false, false, preprocessor);
        snapshot.number_conversion_enabled = false;
        let queued = snapshot.clone();

        snapshot.number_conversion_enabled = true;

        let (_, tts) = resolve_tts_text(&queued, r"\count яблок").unwrap();
        assert_eq!(tts, "5 яблок");
        let (_, tts) = resolve_tts_text(&snapshot, r"\count яблок").unwrap();
        assert_eq!(tts, "пять яблок");
    }

    /// The legacy preprocessing helper applies replacements in both modes but
    /// converts digits only when the captured setting allows it.
    #[test]
    fn legacy_preprocess_respects_number_conversion_setting() {
        let preprocessor = make_preprocessor(&[("count", "5")], &[]);

        assert_eq!(
            preprocess_text_with_preprocessor(r"\count яблок", Some(&preprocessor), true),
            "пять яблок"
        );
        assert_eq!(
            preprocess_text_with_preprocessor(r"\count яблок", Some(&preprocessor), false),
            "5 яблок"
        );
    }

    /// Delivery text is built before AI correction and stress markup: an
    /// AI-enabled snapshot changes nothing and no provider stress rendering
    /// (combining acute) leaks into the delivered text.
    #[test]
    fn delivery_text_excludes_ai_and_stress_markup() {
        let snapshot = snapshot_with_preprocessor(
            false,
            false,
            true,
            make_preprocessor(&[("fruit", "яблок")], &[]),
        );

        let delivery = resolve_delivery_text(&snapshot, r"\fruit зам+ок").unwrap();
        assert_eq!(delivery, "яблок зам+ок");
        assert!(
            !delivery.contains('\u{0301}'),
            "provider stress markup leaked into delivery: {delivery}"
        );

        let stress = native_stress(None, "зам+ок").expect("stress builds");
        assert_eq!(stress.original, "замок");
    }

    /// Route-prefix stripping remains intact for external delivery, and an
    /// incoming leading `!` stays literal.
    #[test]
    fn delivery_text_preserves_route_prefix_behavior() {
        let preprocessor = make_preprocessor(&[("fruit", "яблок")], &[]);

        let both = snapshot_with_preprocessor(true, true, false, preprocessor.clone());
        assert_eq!(resolve_delivery_text(&both, r"!!\fruit").unwrap(), "яблок");

        let skip_twitch = snapshot_with_preprocessor(true, false, false, preprocessor.clone());
        assert_eq!(
            resolve_delivery_text(&skip_twitch, r"!\fruit").unwrap(),
            "яблок"
        );

        let mut incoming = snapshot_with_preprocessor(false, false, false, preprocessor.clone());
        incoming.source = SubmissionSource::Server;
        incoming.delivery = DeliveryPolicy::AudioOnly;
        assert_eq!(
            resolve_delivery_text(&incoming, r"!\fruit").unwrap(),
            "!яблок"
        );
    }

    /// ai_correct_text_with_settings with ai_enabled=false returns unchanged text.
    #[tokio::test]
    async fn ai_disabled_helper_returns_unchanged_text() {
        let result = ai_correct_text_with_settings(
            "hello world",
            false,
            &crate::config::AiSettings::default(),
            &crate::config::NetworkSettings::default(),
        )
        .await
        .expect("AI-disabled helper should never fail");
        assert_eq!(result, "hello world");
    }

    fn make_audio_settings(speaker_enabled: bool, mic_device: Option<&str>) -> AudioSettings {
        AudioSettings {
            output_format: crate::config::AudioOutputFormat::Default,
            speaker_enabled,
            speaker_device: None,
            speaker_volume: 80,
            virtual_mic_device: mic_device.map(str::to_string),
            virtual_mic_volume: 60,
        }
    }

    #[test]
    fn output_configs_both_enabled() {
        let audio = make_audio_settings(true, Some("mic1"));
        let effects = AudioEffectsSettings::default();
        let (speaker, mic) = compute_output_configs(&audio, &effects);

        assert_eq!(speaker.as_ref().map(|config| config.volume), Some(0.8));
        assert_eq!(
            mic.as_ref().and_then(|config| config.device_id.as_deref()),
            Some("mic1")
        );
        assert_eq!(mic.as_ref().map(|config| config.volume), Some(0.6));
    }

    #[test]
    fn output_configs_speaker_only() {
        let audio = make_audio_settings(true, None);
        let (speaker, mic) = compute_output_configs(&audio, &AudioEffectsSettings::default());

        assert!(speaker.is_some());
        assert!(mic.is_none());
    }

    #[test]
    fn output_configs_mic_only() {
        let audio = make_audio_settings(false, Some("mic1"));
        let (speaker, mic) = compute_output_configs(&audio, &AudioEffectsSettings::default());

        assert!(speaker.is_none());
        assert!(mic.is_some());
    }

    #[test]
    fn output_configs_both_disabled() {
        let audio = make_audio_settings(false, None);
        let (speaker, mic) = compute_output_configs(&audio, &AudioEffectsSettings::default());

        assert!(speaker.is_none());
        assert!(mic.is_none());
    }

    #[test]
    fn output_configs_apply_effects_volume_factor() {
        let audio = make_audio_settings(true, Some("mic1"));
        let effects = AudioEffectsSettings {
            enabled: true,
            volume: 50,
            ..Default::default()
        };
        let (speaker, _) = compute_output_configs(&audio, &effects);
        let expected = 0.8 * (50.0f32 / 100.0);

        assert!((speaker.unwrap().volume - expected).abs() < 0.0001);
    }

    // ── Snapshot identity / cache key separation tests ──

    #[test]
    fn cache_key_differs_for_openai_voices_same_text() {
        let fp = crate::history::compute_effects_fingerprint(
            &crate::config::AudioEffectsSettings::default(),
            &crate::config::DspSettings::default(),
        );
        let key_alloy = crate::history::build_cache_key("hello world", "openai", "alloy", fp);
        let key_echo = crate::history::build_cache_key("hello world", "openai", "echo", fp);
        assert_ne!(key_alloy, key_echo);
    }

    #[test]
    fn cache_key_differs_for_fish_reference_ids_same_text() {
        let fp = crate::history::compute_effects_fingerprint(
            &crate::config::AudioEffectsSettings::default(),
            &crate::config::DspSettings::default(),
        );
        let key_a = crate::history::build_cache_key("hello world", "fish", "ref-aaa", fp);
        let key_b = crate::history::build_cache_key("hello world", "fish", "ref-bbb", fp);
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn snapshot_identity_reflects_provider_voice_not_registry_id() {
        use crate::config::{
            AiSettings, AudioEffectsSettings, AudioSettings, DspSettings, NetworkSettings,
        };

        let mut alloy_tts = crate::tts::openai::OpenAiTts::new("sk-test".into());
        alloy_tts.set_voice("alloy".to_string());
        let snapshot_alloy = Snapshot {
            provider: "openai".into(),
            voice: alloy_tts.voice().to_string(),
            source: SubmissionSource::Editor,
            delivery: DeliveryPolicy::editor(false, false),
            ai_enabled: false,
            number_conversion_enabled: true,
            audio_effects: AudioEffectsSettings::default(),
            dsp: DspSettings::default(),
            audio: AudioSettings::default(),
            ai: AiSettings::default(),
            tts_provider: crate::tts::TtsProvider::OpenAi(alloy_tts),
            preprocessor: None,
            network_settings: NetworkSettings::default(),
            accentor_runtime: None,
        };

        let mut echo_tts = crate::tts::openai::OpenAiTts::new("sk-test".into());
        echo_tts.set_voice("echo".to_string());
        let snapshot_echo = Snapshot {
            provider: "openai".into(),
            voice: echo_tts.voice().to_string(),
            source: SubmissionSource::Editor,
            delivery: DeliveryPolicy::editor(false, false),
            ai_enabled: false,
            number_conversion_enabled: true,
            audio_effects: AudioEffectsSettings::default(),
            dsp: DspSettings::default(),
            audio: AudioSettings::default(),
            ai: AiSettings::default(),
            tts_provider: crate::tts::TtsProvider::OpenAi(echo_tts),
            preprocessor: None,
            network_settings: NetworkSettings::default(),
            accentor_runtime: None,
        };

        assert_ne!(snapshot_alloy.voice, snapshot_echo.voice);
        assert_ne!(snapshot_alloy.provider, "openai-entry-id");
        assert_ne!(snapshot_echo.provider, "openai-entry-id");
        assert_eq!(snapshot_alloy.voice, "alloy");
        assert_eq!(snapshot_echo.voice, "echo");
    }

    #[test]
    fn snapshot_identity_distinct_cache_for_same_text_different_openai() {
        let fp = crate::history::compute_effects_fingerprint(
            &crate::config::AudioEffectsSettings::default(),
            &crate::config::DspSettings::default(),
        );
        let text = "the quick brown fox";
        let key_alloy = crate::history::build_cache_key(text, "openai", "alloy", fp);
        let key_echo = crate::history::build_cache_key(text, "openai", "echo", fp);
        let key_fable = crate::history::build_cache_key(text, "openai", "fable", fp);
        assert_ne!(key_alloy, key_echo);
        assert_ne!(key_alloy, key_fable);
        assert_ne!(key_echo, key_fable);
    }

    // ── Blocking boundary seam tests ──

    /// A control future must keep running while a deliberately delayed blocking
    /// stage is executing (the blocking stage runs off the async worker thread).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn control_future_runs_during_delayed_blocking_stage() {
        use std::time::{Duration, Instant};

        let blocking = spawn_blocking_prepare(move || -> Result<i32, String> {
            std::thread::sleep(Duration::from_millis(200));
            Ok(42)
        });

        // Control future: a short sleep that must complete while the blocking
        // task is still running (200ms). If the blocking stage stalled the async
        // worker, this sleep would take ~200ms, not ~50ms.
        let start = Instant::now();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let control_elapsed = start.elapsed();
        assert!(
            control_elapsed < Duration::from_millis(150),
            "control future was stalled by the blocking stage (took {:?})",
            control_elapsed
        );

        let value = blocking.await.expect("blocking stage result");
        assert_eq!(value, 42);
    }

    /// A panicking blocking stage surfaces as a preparation error (JoinError).
    #[tokio::test]
    async fn blocking_stage_panic_maps_to_speech_error() {
        let result: Result<i32, String> = spawn_blocking_prepare(move || -> Result<i32, String> {
            panic!("intentional panic in blocking stage");
        })
        .await;

        let err = result.expect_err("panicking blocking stage must surface an error");
        assert!(err.contains("panicked"), "unexpected error: {err}");
    }

    /// Cancellation must win over a delayed blocking stage without waiting for it,
    /// mirroring the speech worker's shutdown observation during preparation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellation_wins_over_delayed_blocking_stage() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let cancel = tokio_util::sync::CancellationToken::new();
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let blocking = tokio::spawn(async move {
            spawn_blocking_prepare(move || -> Result<(), String> {
                started_tx.send(()).ok();
                // Simulate a long blocking stage until the test releases it.
                let _ = release_rx.recv();
                Ok(())
            })
            .await
        });

        tokio::task::spawn_blocking(move || started_rx.recv())
            .await
            .expect("start observer must not panic")
            .expect("blocking stage should start");

        let cancel_task = {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                cancel.cancel();
            })
        };

        let start = Instant::now();
        let outcome = tokio::select! {
            result = blocking => Some(result),
            _ = cancel.cancelled() => None,
        };

        assert!(
            outcome.is_none(),
            "cancellation should win over the blocking stage"
        );
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "cancellation should not wait for the blocking stage (took {:?})",
            start.elapsed()
        );

        // Release the detached blocking task so it can finish cleanly.
        release_tx.send(()).ok();
        cancel_task.await.expect("cancel task");
    }

    /// Inner cache-miss errors surface through the blocking seam, not a JoinError.
    #[tokio::test]
    async fn cache_read_blocking_surfaces_cache_miss() {
        let missing = uuid::Uuid::new_v4().to_string();
        let err = read_cache_blocking(missing)
            .await
            .expect_err("missing cache key must error");
        assert!(err.contains("CacheMiss"), "unexpected error: {err}");
    }

    // ── Stress adapter boundary tests ──

    fn openai_provider() -> TtsProvider {
        TtsProvider::OpenAi(crate::tts::openai::OpenAiTts::new("sk-test".into()))
    }

    fn silero_provider() -> TtsProvider {
        TtsProvider::Silero(crate::tts::silero::SileroTts::new())
    }

    fn local_provider() -> TtsProvider {
        TtsProvider::Local(crate::tts::local_http_server::LocalHttpServerTts::new())
    }

    fn fish_provider() -> TtsProvider {
        TtsProvider::Fish(crate::tts::fish::FishTts::new("sk-test".into()))
    }

    fn piper_provider() -> TtsProvider {
        TtsProvider::Piper(std::sync::Arc::new(
            crate::tts::piper::runtime::LocalModelTts::new(
                "/dummy/model.onnx",
                "/dummy/model.onnx.json",
            ),
        ))
    }

    fn all_providers() -> Vec<TtsProvider> {
        vec![
            openai_provider(),
            silero_provider(),
            local_provider(),
            fish_provider(),
            piper_provider(),
        ]
    }

    #[test]
    fn stress_adapter_maps_each_provider_exactly() {
        assert_eq!(
            provider_to_stress_adapter(&openai_provider()),
            ProviderStressAdapter::PlainText
        );
        assert_eq!(
            provider_to_stress_adapter(&silero_provider()),
            ProviderStressAdapter::Silero
        );
        assert_eq!(
            provider_to_stress_adapter(&local_provider()),
            ProviderStressAdapter::PlainText
        );
        assert_eq!(
            provider_to_stress_adapter(&fish_provider()),
            ProviderStressAdapter::PlainText
        );
        assert_eq!(
            provider_to_stress_adapter(&piper_provider()),
            ProviderStressAdapter::Piper
        );
    }

    /// Empty annotations must pass validation and preserve the text for every
    /// provider variant (no accentor runtime attached yet).
    #[test]
    fn empty_annotations_preserve_text_for_all_providers() {
        let text = "привет мир".to_string();
        for provider in all_providers() {
            let stress = StructuredStress::new(text.clone(), vec![])
                .expect("empty annotations must pass validation");
            let adapted = adapt_structured_stress(&provider, &stress);
            assert_eq!(adapted, text);
        }
    }

    /// Silero adaptation inserts `+` before the stressed vowel through the helper.
    #[test]
    fn silero_helper_adapts_annotated_text() {
        use crate::stress::annotations::StressAnnotation;
        let stress = StructuredStress::new(
            "привет".to_string(),
            vec![StressAnnotation {
                word_start: 0,
                word_end: 12,
                stressed_vowel: 8,
            }],
        )
        .expect("valid annotation");
        assert_eq!(
            adapt_structured_stress(&silero_provider(), &stress),
            "прив+ет"
        );
    }

    /// The insert text handed to labels and external routing is the marker-free
    /// `StructuredStress.original`, not the provider-rendered base.
    #[test]
    fn insert_text_is_clean_original_for_silero() {
        let stress = native_stress(None, "зам+ок и м+ука").expect("stress builds");
        let (provider_text, insert_text) = provider_and_insert_text(&silero_provider(), &stress);
        assert_eq!(provider_text, "зам+ок и м+ука");
        assert_eq!(insert_text, "замок и мука");
        assert!(!insert_text.contains('+'));
        assert!(!insert_text.contains('\u{0301}'));
    }

    /// Cache key is derived from `provider_text` (with markers), so it differs
    /// from the clean insert text and re-uses cached audio only on an exact
    /// provider-text match.
    #[test]
    fn cache_key_uses_provider_text_not_insert_text() {
        let stress = native_stress(None, "зам+ок").expect("stress builds");
        let (provider_text, insert_text) = provider_and_insert_text(&silero_provider(), &stress);
        assert_eq!(provider_text, "зам+ок");
        assert_eq!(insert_text, "замок");

        let fp = crate::history::compute_effects_fingerprint(
            &crate::config::AudioEffectsSettings::default(),
            &crate::config::DspSettings::default(),
        );
        let key_provider = crate::history::build_cache_key(&provider_text, "silero", "", fp);
        let key_insert = crate::history::build_cache_key(&insert_text, "silero", "", fp);
        assert_ne!(key_provider, key_insert);
    }

    /// `insert_text` is `StructuredStress.original`, so a `ё` render-base
    /// restoration never leaks into external routing or insertion.
    #[test]
    fn insert_text_uses_original_not_render_base() {
        use crate::stress::annotations::StressAnnotation;
        let stress = StructuredStress::with_render_base(
            "все".to_string(),
            "всё".to_string(),
            vec![StressAnnotation {
                word_start: 0,
                word_end: "все".len(),
                stressed_vowel: "вс".len(),
            }],
        )
        .expect("valid render-base stress");
        let (provider_text, insert_text) = provider_and_insert_text(&silero_provider(), &stress);
        assert_eq!(provider_text, "вс+ё");
        assert_eq!(insert_text, "все");
    }

    // ── Native accentor integration tests ──

    fn native_slot(pack_root: std::path::PathBuf) -> RuAccentRuntimeSlot {
        use crate::stress::packs::{RuAccentPackDescriptor, RuntimeCapability};
        let descriptor = RuAccentPackDescriptor {
            id: "com.example.pipeline".to_string(),
            display_name: "Pipeline".to_string(),
            runtime_version: "upstream".to_string(),
            pack_root,
            runtime_capability: RuntimeCapability::NativeTiny,
            omograph_model_id: "pipeline-model".to_string(),
        };
        RuAccentRuntimeSlot::new(descriptor)
    }

    fn empty_pack_root() -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-tts-pipeline-accentor-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// No accentor slot: the text passes through unchanged for every provider.
    #[test]
    fn accentor_none_preserves_text() {
        let text = "привет мир".to_string();
        let stress = structured_stress_for_text(None, &text).expect("empty stress builds");
        assert!(stress.annotations.is_empty());
        for provider in all_providers() {
            assert_eq!(adapt_structured_stress(&provider, &stress), text);
        }
    }

    /// A non-loadable native pack is fail-open: the error is swallowed and the
    /// original text is synthesized unchanged.
    #[test]
    fn accentor_native_slot_error_is_fail_open() {
        let dir = empty_pack_root();
        let slot = native_slot(dir.clone());
        let text = "привет".to_string();
        let stress = structured_stress_for_text(Some(&slot), &text).expect("fail-open keeps text");
        assert!(stress.annotations.is_empty());
        assert_eq!(adapt_structured_stress(&silero_provider(), &stress), text);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── Shared native_stress pipeline tests ──

    /// With no slot present the pipeline is fail-open: manual markers are
    /// preserved and the rest of the text is retained unchanged.
    #[test]
    fn native_stress_absent_slot_preserves_manual_marks() {
        let stress = native_stress(None, "зам+ок и м+ука").expect("manual-only stress builds");
        assert_eq!(
            adapt_structured_stress(&silero_provider(), &stress),
            "зам+ок и м+ука"
        );
        assert_eq!(stress.original, "замок и мука");
    }

    /// Literal `+` that is not a Silero marker is never treated as stress.
    #[test]
    fn native_stress_keeps_literal_plus_without_slot() {
        let stress = native_stress(None, "C++ и з+амок").expect("literal plus is fine");
        assert_eq!(stress.original, "C++ и замок");
        assert_eq!(
            adapt_structured_stress(&silero_provider(), &stress),
            "C++ и з+амок"
        );
    }

    /// A non-loadable native pack is fail-open and still keeps manual markers
    /// (automatic annotation degrades to empty rather than failing synthesis).
    #[test]
    fn native_stress_manual_marks_win_over_failed_automatic() {
        let dir = empty_pack_root();
        let slot = native_slot(dir.clone());
        let stress = native_stress(Some(&slot), "зам+ок").expect("fail-open keeps manual marks");
        assert_eq!(
            adapt_structured_stress(&silero_provider(), &stress),
            "зам+ок"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Preview and synthesis must render equivalent Silero output through the
    /// same shared helper: both adapt the same `native_stress` result.
    #[test]
    fn native_stress_silero_output_matches_preview_rendering() {
        let stress = native_stress(None, "зам+ок и м+ука").expect("stress builds");
        let via_synthesis = adapt_structured_stress(&silero_provider(), &stress);
        let via_preview = ProviderStressAdapter::Silero.adapt(&stress);
        assert_eq!(via_synthesis, via_preview);
        assert_eq!(via_preview, "зам+ок и м+ука");
    }

    /// The strict preview path propagates an inference error instead of failing
    /// open with unchanged text.
    #[test]
    fn native_stress_strict_propagates_annotation_error() {
        let dir = empty_pack_root();
        let slot = native_slot(dir.clone());

        let err = native_stress_strict(&slot, "замок").unwrap_err();
        assert!(
            !err.contains('/') && !err.contains('\\'),
            "leaked path: {err}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── RUAccent lazy slot selection tests ──

    fn enable_homograph(state: &AppState, pack_id: Option<&str>) {
        use crate::config::HomographAccentorSettings;
        state.settings_cache.write().editor.homograph_accentor = HomographAccentorSettings {
            enabled: true,
            accentor_pack_id: pack_id.map(str::to_string),
            ..Default::default()
        };
    }

    fn select_homograph_disabled(state: &AppState, pack_id: Option<&str>) {
        use crate::config::HomographAccentorSettings;
        state.settings_cache.write().editor.homograph_accentor = HomographAccentorSettings {
            enabled: false,
            accentor_pack_id: pack_id.map(str::to_string),
            ..Default::default()
        };
    }

    #[test]
    fn selected_slot_none_when_layer_disabled() {
        let state = AppState::new();
        assert!(selected_ruaccent_runtime_slot(&state).is_none());
    }

    #[test]
    fn selected_slot_none_when_pack_id_missing() {
        let state = AppState::new();
        enable_homograph(&state, None);
        assert!(selected_ruaccent_runtime_slot(&state).is_none());
    }

    #[test]
    fn selected_slot_none_for_unknown_pack() {
        let state = AppState::new();
        enable_homograph(&state, Some("com.example.missing"));
        assert!(selected_ruaccent_runtime_slot(&state).is_none());
    }

    #[test]
    fn selected_slot_none_for_incomplete_pack() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("incomplete-selected");
        tu::write_incomplete_root(&root, &["legacy"]);
        state.refresh_ruaccent_packs(std::slice::from_ref(&root));

        enable_homograph(&state, Some("ruaccent.upstream.legacy"));
        assert!(selected_ruaccent_runtime_slot(&state).is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn selected_slot_returns_slot_for_complete_pack() {
        use crate::stress::packs::test_util as tu;
        use crate::stress::runtime::RuAccentRuntimeStatus;

        let state = AppState::new();
        let root = tu::unique_test_root("complete-selected");
        tu::write_upstream_pack(&root, &["tiny"]);
        state.refresh_ruaccent_packs(std::slice::from_ref(&root));

        enable_homograph(&state, Some("ruaccent.upstream.tiny"));
        assert!(selected_ruaccent_runtime_slot(&state).is_none());

        state
            .get_ruaccent_runtime_slot("ruaccent.upstream.tiny")
            .expect("slot exists")
            .mark_ready();
        let slot = selected_ruaccent_runtime_slot(&state).expect("complete pack slot selected");
        assert_eq!(slot.descriptor().id, "ruaccent.upstream.tiny");
        assert_eq!(slot.status(), RuAccentRuntimeStatus::Ready);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn selected_slot_none_when_not_ready() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("complete-selected-not-ready");
        tu::write_upstream_pack(&root, &["tiny"]);
        state.refresh_ruaccent_packs(std::slice::from_ref(&root));

        enable_homograph(&state, Some("ruaccent.upstream.tiny"));
        assert!(selected_ruaccent_runtime_slot(&state).is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    // ── Preview resolver (ignores `enabled`) tests ──

    #[test]
    fn preview_slot_returns_slot_when_selected_and_disabled() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("complete-preview-disabled");
        tu::write_upstream_pack(&root, &["tiny"]);
        state.refresh_ruaccent_packs(std::slice::from_ref(&root));

        select_homograph_disabled(&state, Some("ruaccent.upstream.tiny"));
        assert!(preview_ruaccent_runtime_slot(&state).is_none());

        state
            .get_ruaccent_runtime_slot("ruaccent.upstream.tiny")
            .expect("slot exists")
            .mark_ready();
        let slot = preview_ruaccent_runtime_slot(&state)
            .expect("preview resolver must return a slot for a selected+disabled pack");
        assert_eq!(slot.descriptor().id, "ruaccent.upstream.tiny");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn automatic_slot_none_when_selected_and_disabled() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("complete-auto-disabled");
        tu::write_upstream_pack(&root, &["tiny"]);
        state.refresh_ruaccent_packs(std::slice::from_ref(&root));

        select_homograph_disabled(&state, Some("ruaccent.upstream.tiny"));
        assert!(selected_ruaccent_runtime_slot(&state).is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn preview_slot_none_when_pack_id_missing() {
        let state = AppState::new();
        select_homograph_disabled(&state, None);
        assert!(preview_ruaccent_runtime_slot(&state).is_none());
    }

    #[test]
    fn preview_slot_none_for_unknown_pack() {
        let state = AppState::new();
        select_homograph_disabled(&state, Some("com.example.missing"));
        assert!(preview_ruaccent_runtime_slot(&state).is_none());
    }

    #[test]
    fn preview_slot_none_for_incomplete_pack() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("incomplete-preview-disabled");
        tu::write_incomplete_root(&root, &["legacy"]);
        state.refresh_ruaccent_packs(std::slice::from_ref(&root));

        select_homograph_disabled(&state, Some("ruaccent.upstream.legacy"));
        assert!(preview_ruaccent_runtime_slot(&state).is_none());

        std::fs::remove_dir_all(&root).ok();
    }
}
