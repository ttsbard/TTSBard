use crate::events::EventSender;
use crate::telegram::{SileroRuntimeSettings, TelegramClient};
use crate::tts::engine::TtsEngine;
use async_trait::async_trait;
use std::io::Cursor;
use std::sync::Arc;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL, CODEC_TYPE_OPUS};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tokio::sync::Mutex;
use tracing::debug;

/// Silero TTS implementation using Telegram bot @silero_voice_bot
#[derive(Clone, Debug)]
pub struct SileroTts {
    // Arc на Option<TelegramClient> - клиент может быть None если не подключен.
    // Контракт разделения этого Arc описан в DECISION-018: движок намеренно
    // держит Arc сам (не просит client у владельца) и обязан соблюдать правило
    // `lock → clone → drop guard → await` (ROADMAP-059 инвариант #3).
    client: Option<Arc<Mutex<Option<TelegramClient>>>>,
    configured: bool,
    event_tx: Option<EventSender>,
    captured_speaker: Option<String>,
    runtime_settings: SileroRuntimeSettings,
}

impl SileroTts {
    pub fn new() -> Self {
        Self {
            client: None,
            configured: false,
            event_tx: None,
            captured_speaker: None,
            runtime_settings: SileroRuntimeSettings::default(),
        }
    }

    pub fn with_event_tx(mut self, event_tx: EventSender) -> Self {
        self.event_tx = Some(event_tx);
        self
    }

    /// Создать SileroTts с Arc на Option<TelegramClient>
    /// Это позволяет получать доступ к клиенту из TelegramState
    pub fn with_telegram_client(client_arc: Arc<Mutex<Option<TelegramClient>>>) -> Self {
        Self {
            client: Some(client_arc),
            configured: true,
            event_tx: None,
            captured_speaker: None,
            runtime_settings: SileroRuntimeSettings::default(),
        }
    }

    pub fn with_captured_speaker(mut self, speaker: String) -> Self {
        let trimmed = speaker.trim().to_string();
        self.captured_speaker = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        };
        self
    }

    pub fn with_runtime_settings(mut self, settings: SileroRuntimeSettings) -> Self {
        self.runtime_settings = settings;
        self
    }

    pub fn captured_speaker(&self) -> Option<&str> {
        self.captured_speaker.as_deref()
    }

    pub fn runtime_settings(&self) -> &SileroRuntimeSettings {
        &self.runtime_settings
    }
}

impl Default for SileroTts {
    fn default() -> Self {
        Self::new()
    }
}

const SILERO_OGG_OPUS_UNSUPPORTED_MSG: &str =
    "Формат аудио Silero не поддерживается. Смените формат ответа бота на MP3 командой /mp3.";

const SILERO_OGG_UNSUPPORTED_MSG: &str =
    "Формат аудио Silero не поддерживается. Смените формат ответа бота на MP3 командой /mp3.";

/// Early guard for known-unsupported Silero audio codecs.
///
/// Silero answers with OGG/Opus voice messages that the local Symphonia build
/// cannot decode. This inspects the actual container bytes before the effects
/// pipeline runs and returns an actionable error only for confirmed OGG files
/// whose codec lacks a decoder. It never decodes full audio; probe failures and
/// truncated files pass through to the regular pipeline diagnostics.
fn unsupported_silero_ogg_codec(audio_data: &[u8]) -> Option<String> {
    // Non-OGG bytes pass through untouched: no allocation, no probing.
    if !audio_data.starts_with(b"OggS") {
        return None;
    }

    let cursor = Cursor::new(audio_data.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let probed = match symphonia::default::get_probe().format(
        &Hint::new(),
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) {
        Ok(probed) => probed,
        Err(_) => return None,
    };

    let format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;

    match symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()) {
        Ok(_) => None,
        Err(SymphoniaError::Unsupported(_)) => {
            if track.codec_params.codec == CODEC_TYPE_OPUS {
                Some(SILERO_OGG_OPUS_UNSUPPORTED_MSG.to_string())
            } else {
                Some(SILERO_OGG_UNSUPPORTED_MSG.to_string())
            }
        }
        Err(_) => None,
    }
}

#[async_trait]
impl TtsEngine for SileroTts {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, String> {
        debug!(text_len = text.len(), "Silero TTS synthesize requested");

        if !self.configured {
            return Err(
                "Silero TTS is not configured. Please connect to Telegram first.".to_string(),
            );
        }

        let client_arc = self
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not set".to_string())?;

        let client = {
            let client_guard = client_arc.lock().await;
            client_guard.as_ref().cloned().ok_or_else(|| {
                "Telegram client not initialized. Please connect to Telegram first.".to_string()
            })?
        };

        if let Some(ref speaker_code) = self.captured_speaker {
            if !speaker_code.is_empty() {
                debug!(speaker = %speaker_code, "Restoring captured Silero speaker before synthesis");
                let set_result = crate::telegram::bot::set_speaker(&client, speaker_code).await?;
                if !set_result {
                    return Err(format!(
                        "Failed to restore captured speaker '{}': invalid voice code",
                        speaker_code
                    ));
                }
            }
        }

        let result =
            crate::telegram::SileroTtsBot::synthesize(&client, text, &self.runtime_settings)
                .await?;

        if !result.success {
            let err = result.error.unwrap_or_else(|| "Unknown error".to_string());
            return Err(err);
        }

        let audio_path = result
            .audio_path
            .as_ref()
            .ok_or_else(|| "No audio path returned".to_string())?;

        // Файл транзиентный: читаем в память и сразу удаляем независимо от
        // исхода чтения (ROADMAP-117). Раньше файлы оставались навсегда.
        let read_result = tokio::fs::read(audio_path).await;
        cleanup_temp_audio(std::path::Path::new(audio_path)).await;

        let audio_data = read_result.map_err(|e| format!("Failed to read audio file: {}", e))?;

        if let Some(message) = unsupported_silero_ogg_codec(&audio_data) {
            return Err(message);
        }

        Ok(audio_data)
    }
}

/// Best-effort удаление скачанного временного аудиофайла после потребления.
async fn cleanup_temp_audio(path: &std::path::Path) {
    if let Err(e) = tokio::fs::remove_file(path).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                path = %crate::secret_log::safe_path_for_log(path),
                error = %e,
                "Failed to remove temporary TTS audio file"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::effects::decode_audio;

    const SILERO_OPUS_OGG: &[u8] = include_bytes!("testdata/silero-opus.ogg");
    const SILERO_VORBIS_OGG: &[u8] = include_bytes!("testdata/silero-vorbis.ogg");
    const TEST_MP3: &[u8] = include_bytes!("../assets/test_sound.mp3");

    const OPUS_UNSUPPORTED_TEXT: &str =
        "Формат аудио Silero не поддерживается. Смените формат ответа бота на MP3 командой /mp3.";

    #[test]
    fn opus_fixture_yields_exact_actionable_message() {
        let message = unsupported_silero_ogg_codec(SILERO_OPUS_OGG)
            .expect("OGG/Opus fixture must be flagged as an unsupported Silero codec");
        assert_eq!(message, OPUS_UNSUPPORTED_TEXT);
    }

    #[test]
    fn vorbis_fixture_is_not_flagged_as_unsupported() {
        assert_eq!(unsupported_silero_ogg_codec(SILERO_VORBIS_OGG), None);
    }

    #[test]
    fn mp3_fixture_is_not_flagged_as_unsupported() {
        assert_eq!(unsupported_silero_ogg_codec(TEST_MP3), None);
    }

    #[test]
    fn vorbis_fixture_still_decodes_with_existing_pipeline() {
        let pcm = decode_audio(SILERO_VORBIS_OGG).expect("OGG/Vorbis fixture must decode");
        assert!(!pcm.samples.is_empty());
        assert!(pcm.sample_rate > 0);
    }

    #[test]
    fn mp3_fixture_still_decodes_with_existing_pipeline() {
        let pcm = decode_audio(TEST_MP3).expect("MP3 fixture must decode");
        assert!(!pcm.samples.is_empty());
        assert!(pcm.sample_rate > 0);
    }

    #[test]
    fn opus_rejection_is_not_sticky_for_later_mp3_decode() {
        let message = unsupported_silero_ogg_codec(SILERO_OPUS_OGG)
            .expect("OGG/Opus fixture must be rejected first");
        assert!(message.contains("/mp3"));

        assert_eq!(unsupported_silero_ogg_codec(TEST_MP3), None);
        let pcm = decode_audio(TEST_MP3).expect("MP3 must still decode after Opus rejection");
        assert!(!pcm.samples.is_empty());
    }

    #[test]
    fn non_ogg_bytes_pass_through_without_flagging() {
        assert_eq!(unsupported_silero_ogg_codec(&[]), None);
        assert_eq!(unsupported_silero_ogg_codec(b"OggS"), None);

        let random: Vec<u8> = (0u8..=255).cycle().take(128).collect();
        assert_eq!(unsupported_silero_ogg_codec(&random), None);
    }

    #[test]
    fn truncated_ogg_is_not_mislabeled_as_unsupported_codec() {
        let truncated = &SILERO_OPUS_OGG[..16];
        assert_eq!(unsupported_silero_ogg_codec(truncated), None);
    }

    #[test]
    fn captured_speaker_empty_string_treated_as_absent() {
        let tts = SileroTts::new().with_captured_speaker(String::new());
        assert_eq!(tts.captured_speaker(), None);
    }

    #[test]
    fn captured_speaker_whitespace_only_treated_as_absent() {
        let tts = SileroTts::new().with_captured_speaker("   ".to_string());
        assert_eq!(tts.captured_speaker(), None);
    }

    #[test]
    fn captured_speaker_valid_value_is_preserved() {
        let tts = SileroTts::new().with_captured_speaker("baya_16".to_string());
        assert_eq!(tts.captured_speaker(), Some("baya_16"));
    }

    #[test]
    fn captured_speaker_whitespace_is_trimmed() {
        let tts = SileroTts::new().with_captured_speaker("  baya_16  ".to_string());
        assert_eq!(tts.captured_speaker(), Some("baya_16"));
    }

    #[test]
    fn runtime_settings_default_values() {
        let tts = SileroTts::new();
        let rt = tts.runtime_settings();
        assert_eq!(rt.synthesis_response_timeout.as_millis(), 60000);
        assert_eq!(rt.download_retry_delay.as_millis(), 1000);
    }

    #[test]
    fn runtime_settings_non_default_values() {
        let rt = SileroRuntimeSettings::new(5000, 2000);
        assert_eq!(rt.synthesis_response_timeout.as_millis(), 5000);
        assert_eq!(rt.download_retry_delay.as_millis(), 2000);
    }

    #[test]
    fn clone_preserves_runtime_settings() {
        let rt = SileroRuntimeSettings::new(7000, 300);
        let original = SileroTts::new()
            .with_captured_speaker("baya_16".to_string())
            .with_runtime_settings(rt.clone());
        let cloned = original.clone();
        assert_eq!(cloned.captured_speaker(), Some("baya_16"));
        assert_eq!(
            cloned
                .runtime_settings()
                .synthesis_response_timeout
                .as_millis(),
            7000
        );
        assert_eq!(
            cloned.runtime_settings().download_retry_delay.as_millis(),
            300
        );
    }

    #[test]
    fn with_captured_speaker_preserves_runtime_settings() {
        let rt = SileroRuntimeSettings::new(8000, 500);
        let original = SileroTts::new()
            .with_runtime_settings(rt)
            .with_captured_speaker("baya_16".to_string());
        assert_eq!(original.captured_speaker(), Some("baya_16"));
        assert_eq!(
            original
                .runtime_settings()
                .synthesis_response_timeout
                .as_millis(),
            8000
        );
        assert_eq!(
            original.runtime_settings().download_retry_delay.as_millis(),
            500
        );

        let updated = original.with_captured_speaker("new_speaker".to_string());
        assert_eq!(updated.captured_speaker(), Some("new_speaker"));
        assert_eq!(
            updated
                .runtime_settings()
                .synthesis_response_timeout
                .as_millis(),
            8000
        );
        assert_eq!(
            updated.runtime_settings().download_retry_delay.as_millis(),
            500
        );
    }

    #[test]
    fn default_constructor_uses_default_runtime_settings() {
        let tts1 = SileroTts::new();
        let tts2 = SileroTts::with_telegram_client(Arc::new(Mutex::new(None)));
        assert_eq!(
            tts1.runtime_settings()
                .synthesis_response_timeout
                .as_millis(),
            tts2.runtime_settings()
                .synthesis_response_timeout
                .as_millis()
        );
    }
}
