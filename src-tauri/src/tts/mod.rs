pub mod elevenlabs;
pub mod engine;
pub mod fish;
pub mod local_http_server;
pub mod openai;
pub mod piper;
pub mod pocket;
pub mod proxy_utils;
pub mod registry;
pub mod silero;

// Реэкспорт VoiceModel для использования в других модулях
pub use fish::VoiceModel;
// Реэкспорт ElevenLabsVoice для использования в других модулях
pub use elevenlabs::ElevenLabsVoice;

use crate::tts::engine::TtsEngine;
use std::sync::Arc;

use crate::tts::piper::runtime::LocalModelTts;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum TtsProviderType {
    #[default]
    OpenAi,
    Silero,
    Local,
    Fish,
    ElevenLabs,
}

#[derive(Clone, Debug)]
pub enum TtsProvider {
    OpenAi(openai::OpenAiTts),
    Silero(silero::SileroTts),
    Local(local_http_server::LocalHttpServerTts),
    Fish(fish::FishTts),
    ElevenLabs(elevenlabs::ElevenLabsTts),
    Piper(Arc<LocalModelTts>),
    Pocket(pocket::PocketTts),
}

impl TtsProvider {
    pub fn prepare(&self) -> Result<(), String> {
        match self {
            TtsProvider::Piper(tts) => tts.prepare(),
            TtsProvider::Pocket(tts) => tts.prepare(),
            _ => Ok(()),
        }
    }

    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>, String> {
        match self {
            TtsProvider::OpenAi(tts) => tts.synthesize(text).await.map_err(|e| e.to_string()),
            TtsProvider::Local(tts) => tts.synthesize(text).await,
            TtsProvider::Silero(tts) => tts.synthesize(text).await,
            TtsProvider::Fish(tts) => tts.synthesize(text).await.map_err(|e| e.to_string()),
            TtsProvider::ElevenLabs(tts) => tts.synthesize(text).await,
            TtsProvider::Piper(tts) => tts.synthesize(text).await,
            TtsProvider::Pocket(tts) => tts.synthesize(text).await,
        }
    }

    pub fn provider_kind_str(&self) -> &'static str {
        match self {
            TtsProvider::OpenAi(_) => "openai",
            TtsProvider::Silero(_) => "silero",
            TtsProvider::Local(_) => "local",
            TtsProvider::Fish(_) => "fish",
            TtsProvider::ElevenLabs(_) => "elevenlabs",
            TtsProvider::Piper(_) => "piper",
            TtsProvider::Pocket(_) => "pocket",
        }
    }

    pub fn voice_identity(&self) -> String {
        match self {
            TtsProvider::OpenAi(tts) => tts.voice().to_string(),
            TtsProvider::Fish(tts) => tts.reference_id().to_string(),
            TtsProvider::ElevenLabs(tts) => tts.voice_id().to_string(),
            TtsProvider::Silero(tts) => tts.captured_speaker().unwrap_or("").to_string(),
            TtsProvider::Local(_) => "local".to_string(),
            TtsProvider::Piper(_) => "piper".to_string(),
            TtsProvider::Pocket(tts) => tts.voice_identity(),
        }
    }

    pub fn voice_identity_or_registry(&self, registry_id: &str) -> String {
        match self {
            TtsProvider::OpenAi(tts) => tts.voice().to_string(),
            TtsProvider::Fish(tts) => tts.reference_id().to_string(),
            TtsProvider::ElevenLabs(tts) => tts.voice_id().to_string(),
            TtsProvider::Silero(tts) => tts.captured_speaker().unwrap_or("").to_string(),
            TtsProvider::Local(_) | TtsProvider::Piper(_) => registry_id.to_string(),
            TtsProvider::Pocket(tts) => {
                format!("{registry_id}:{}", tts.voice_identity())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piper_variant_holds_local_model_tts() {
        let tts = LocalModelTts::new("/dummy/model.onnx", "/dummy/model.onnx.json");
        let provider = TtsProvider::Piper(Arc::new(tts));
        assert!(matches!(provider, TtsProvider::Piper(_)));
    }

    #[test]
    fn provider_kind_str_is_type_name() {
        let openai = TtsProvider::OpenAi(openai::OpenAiTts::new("sk-key".into()));
        assert_eq!(openai.provider_kind_str(), "openai");

        let silero = TtsProvider::Silero(silero::SileroTts::new());
        assert_eq!(silero.provider_kind_str(), "silero");

        let local = TtsProvider::Local(local_http_server::LocalHttpServerTts::new());
        assert_eq!(local.provider_kind_str(), "local");

        let fish = TtsProvider::Fish(fish::FishTts::new("fish-key".into()));
        assert_eq!(fish.provider_kind_str(), "fish");

        let piper = TtsProvider::Piper(Arc::new(LocalModelTts::new(
            "/dummy/model.onnx",
            "/dummy/model.onnx.json",
        )));
        assert_eq!(piper.provider_kind_str(), "piper");
    }

    #[test]
    fn openai_voice_identity_changes_with_voice() {
        let mut alloy_tts = openai::OpenAiTts::new("sk-key".into());
        alloy_tts.set_voice("alloy".to_string());
        let alloy = TtsProvider::OpenAi(alloy_tts);

        let mut echo_tts = openai::OpenAiTts::new("sk-key".into());
        echo_tts.set_voice("echo".to_string());
        let echo = TtsProvider::OpenAi(echo_tts);

        assert_eq!(alloy.voice_identity(), "alloy");
        assert_eq!(echo.voice_identity(), "echo");
        assert_ne!(alloy.voice_identity(), echo.voice_identity());
    }

    #[test]
    fn fish_voice_identity_changes_with_reference_id() {
        let mut fish_a = fish::FishTts::new("fish-key".into());
        fish_a.set_reference_id("ref-aaa".to_string());
        let a = TtsProvider::Fish(fish_a);

        let mut fish_b = fish::FishTts::new("fish-key".into());
        fish_b.set_reference_id("ref-bbb".to_string());
        let b = TtsProvider::Fish(fish_b);

        assert_eq!(a.voice_identity(), "ref-aaa");
        assert_eq!(b.voice_identity(), "ref-bbb");
        assert_ne!(a.voice_identity(), b.voice_identity());
    }

    #[test]
    fn elevenlabs_provider_kind_and_voice_identity() {
        let mut tts = elevenlabs::ElevenLabsTts::new("el-key".into());
        tts.set_voice_id("21m00Tcm4TlvDq8ikWAM".to_string());
        let provider = TtsProvider::ElevenLabs(tts);

        assert_eq!(provider.provider_kind_str(), "elevenlabs");
        assert_eq!(provider.voice_identity(), "21m00Tcm4TlvDq8ikWAM");
        assert_eq!(
            provider.voice_identity_or_registry("elevenlabs"),
            "21m00Tcm4TlvDq8ikWAM"
        );
    }

    #[test]
    fn local_and_piper_voice_identity_or_registry_uses_registry_id() {
        let local = TtsProvider::Local(local_http_server::LocalHttpServerTts::new());
        assert_eq!(
            local.voice_identity_or_registry("my-local-id"),
            "my-local-id"
        );

        let piper = TtsProvider::Piper(Arc::new(LocalModelTts::new(
            "/dummy/model.onnx",
            "/dummy/model.onnx.json",
        )));
        assert_eq!(
            piper.voice_identity_or_registry("piper-en_US-amy-low"),
            "piper-en_US-amy-low"
        );
    }

    #[test]
    fn silero_captured_speaker_reflected_in_voice_identity() {
        let default = TtsProvider::Silero(silero::SileroTts::new());
        assert_eq!(default.voice_identity(), "");

        let with_speaker = TtsProvider::Silero(
            silero::SileroTts::new().with_captured_speaker("baya_16".to_string()),
        );
        assert_eq!(with_speaker.voice_identity(), "baya_16");
    }

    #[test]
    fn silero_captured_speaker_preserved_through_clone() {
        let original = silero::SileroTts::new().with_captured_speaker("baya_16".to_string());
        assert_eq!(original.captured_speaker(), Some("baya_16"));

        let cloned = original.clone();
        assert_eq!(cloned.captured_speaker(), Some("baya_16"));
    }

    #[test]
    fn silero_empty_captured_speaker_is_absent_not_silero() {
        let empty_speaker = silero::SileroTts::new().with_captured_speaker(String::new());
        assert_eq!(empty_speaker.captured_speaker(), None);

        let provider = TtsProvider::Silero(empty_speaker);
        assert_eq!(provider.voice_identity(), "");
    }
}
