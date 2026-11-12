//! Pocket TTS local provider (ROADMAP-135).
//!
//! The runtime consumes the ONNX export produced for the C++ PocketTTS
//! smoke-cycle and supports two package shapes:
//!
//! * `shtorm-ru-v2-fast` and compatible Russian sets: a SentencePiece `.model`
//!   tokenizer plus a user supplied WAV reference (voice cloning through the
//!   Mimi encoder).
//! * full English Kyutai sets: a HuggingFace `tokenizer.json` plus ready voice
//!   states (`.safetensors`) under `embeddings/`.
//!
//! All model files live inside `models/pocket/<set-id>` and are never copied
//! into the repository. Discovery follows the same root order as Piper
//! (ROADMAP-117): process cwd first, then the managed data root.

pub mod audio;
pub mod descriptor;
pub mod provider;
pub mod runtime;
pub mod safetensors;
pub mod tokenizer;
pub mod voice;

#[cfg(test)]
mod smoke;

pub use descriptor::{
    discover_pocket_models, merge_pocket_roots, scan_pocket_models_root, PocketTokenizerKind,
    PocketVoiceMode,
};
pub use provider::PocketTts;
pub use voice::{
    resolved_selection, validate_selection, with_voice_store, PocketReadiness,
    PocketVoiceSelection, PocketVoiceStore,
};

use std::path::PathBuf;

/// A stable, machine-readable failure code for the UI contract.
///
/// Codes are deliberately independent from user-facing text: the frontend
/// localizes them, while logs retain the internal cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PocketErrorCode {
    PackageInvalid,
    ReferenceRequired,
    ReferenceMissing,
    ReferenceInvalid,
    VoiceMissing,
    RuntimeFailed,
    GenerationFailed,
    Cancelled,
}

impl PocketErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            PocketErrorCode::PackageInvalid => "pocket.package_invalid",
            PocketErrorCode::ReferenceRequired => "pocket.reference_required",
            PocketErrorCode::ReferenceMissing => "pocket.reference_missing",
            PocketErrorCode::ReferenceInvalid => "pocket.reference_invalid",
            PocketErrorCode::VoiceMissing => "pocket.voice_missing",
            PocketErrorCode::RuntimeFailed => "pocket.runtime_failed",
            PocketErrorCode::GenerationFailed => "pocket.generation_failed",
            PocketErrorCode::Cancelled => "pocket.cancelled",
        }
    }
}

/// Runtime failure with a stable code and a safe, short message.
#[derive(Debug, thiserror::Error)]
pub enum PocketError {
    #[error("Pocket model package is incomplete or invalid: {0}")]
    Package(String),
    #[error("Pocket tokenizer is invalid: {0}")]
    Tokenizer(String),
    #[error("Pocket voice reference is required")]
    ReferenceRequired,
    #[error("Pocket voice reference is missing")]
    ReferenceMissing,
    #[error("Pocket voice reference is invalid: {0}")]
    ReferenceInvalid(String),
    #[error("Pocket voice reference exceeds {max_seconds} seconds")]
    ReferenceTooLong { max_seconds: u32 },
    #[error("Pocket voice state is missing: {0}")]
    VoiceMissing(String),
    #[error("Pocket runtime failed: {0}")]
    Runtime(String),
    #[error("Pocket generation failed: {0}")]
    Generation(String),
    #[error("Pocket generation was cancelled")]
    Cancelled,
}

impl PocketError {
    pub fn code(&self) -> PocketErrorCode {
        match self {
            PocketError::Package(_) | PocketError::Tokenizer(_) => PocketErrorCode::PackageInvalid,
            PocketError::ReferenceRequired => PocketErrorCode::ReferenceRequired,
            PocketError::ReferenceMissing => PocketErrorCode::ReferenceMissing,
            PocketError::ReferenceInvalid(_) | PocketError::ReferenceTooLong { .. } => {
                PocketErrorCode::ReferenceInvalid
            }
            PocketError::VoiceMissing(_) => PocketErrorCode::VoiceMissing,
            PocketError::Runtime(_) => PocketErrorCode::RuntimeFailed,
            PocketError::Generation(_) => PocketErrorCode::GenerationFailed,
            PocketError::Cancelled => PocketErrorCode::Cancelled,
        }
    }

    /// User-facing fallback message. The frontend prefers its own localized
    /// text for [`Self::code`]; this text is used by diagnostics and non-UI
    /// callers, so it never carries a filesystem path.
    pub fn message(&self) -> String {
        match self {
            PocketError::Package(_) => {
                "Набор Pocket TTS неполный или повреждён. Проверьте файлы модели.".to_string()
            }
            PocketError::Tokenizer(_) => {
                "Токенизатор Pocket TTS не загружается. Проверьте комплект модели.".to_string()
            }
            PocketError::ReferenceRequired => {
                "Для этой модели выберите WAV-референс голоса.".to_string()
            }
            PocketError::ReferenceMissing => {
                "Файл референса не найден. Выберите WAV-файл снова.".to_string()
            }
            PocketError::ReferenceTooLong { max_seconds } => {
                format!("Длительность WAV должна быть не больше {max_seconds} секунд.")
            }
            PocketError::ReferenceInvalid(_) => {
                "WAV-референс не читается или не содержит речи. Выберите другой файл.".to_string()
            }
            PocketError::VoiceMissing(_) => {
                "Файл готового голоса не найден. Проверьте комплект модели.".to_string()
            }
            PocketError::Runtime(_) => {
                "Не удалось запустить Pocket TTS. Проверьте файлы модели.".to_string()
            }
            PocketError::Generation(_) => {
                "Синтез Pocket TTS завершился ошибкой. Попробуйте ещё раз.".to_string()
            }
            PocketError::Cancelled => "Синтез отменён.".to_string(),
        }
    }

    /// Internal detail for logs. It may contain paths or ONNX text and must
    /// never be serialized to the UI.
    pub fn detail(&self) -> String {
        self.to_string()
    }
}

/// Input for the UI readiness/selection commands.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PocketVoiceInput {
    ReferenceWav { path: String },
    VoiceState { voice_id: String },
}

impl PocketVoiceInput {
    pub fn to_selection(&self) -> PocketVoiceSelection {
        match self {
            PocketVoiceInput::ReferenceWav { path } => {
                PocketVoiceSelection::ReferenceWav(PathBuf::from(path))
            }
            PocketVoiceInput::VoiceState { voice_id } => {
                PocketVoiceSelection::VoiceState(voice_id.clone())
            }
        }
    }
}

/// One discoverable ready voice of a package.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PocketVoiceDto {
    pub id: String,
    pub display_name: String,
    pub available: bool,
}

/// Selected voice exposed to the UI.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PocketSelectionDto {
    ReferenceWav {
        path: String,
        file_name: String,
    },
    VoiceState {
        voice_id: String,
        display_name: String,
    },
}

/// Full discovery/readiness DTO for the Pocket TTS card.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PocketModelDto {
    /// Registry ID (`local-pocket:<set-id>`), also the provider ID sent back
    /// to `select_tts_provider_by_id`.
    pub id: String,
    pub display_name: String,
    pub language: String,
    pub format: String,
    pub format_version: u32,
    pub package_id: String,
    pub voice_mode: String,
    pub voices: Vec<PocketVoiceDto>,
    pub selection: Option<PocketSelectionDto>,
    pub readiness: PocketReadiness,
    pub reference_duration_seconds: Option<f32>,
    /// Stable failure code when `readiness` is not ready; `None` otherwise.
    pub error_code: Option<String>,
    /// True while the model sessions are loaded. Diagnostic only.
    pub loaded: bool,
}

impl PocketTts {
    pub fn dto(&self) -> PocketModelDto {
        let descriptor = self.descriptor();
        let (readiness, reference_duration_seconds) =
            voice::evaluate_readiness_with_duration(descriptor, self.selection());
        PocketModelDto {
            id: descriptor.id.clone(),
            display_name: descriptor.display_name.clone(),
            language: descriptor.language.clone(),
            format: descriptor.format.clone(),
            format_version: descriptor.format_version,
            package_id: descriptor.package_id.clone(),
            voice_mode: descriptor.primary_voice_mode().as_str().to_string(),
            voices: descriptor
                .voices
                .iter()
                .map(|voice| PocketVoiceDto {
                    id: voice.id.clone(),
                    display_name: voice.display_name.clone(),
                    available: voice.state_path.is_file(),
                })
                .collect(),
            selection: self.selection_dto(),
            readiness,
            reference_duration_seconds,
            error_code: readiness.code().map(str::to_string),
            loaded: self.is_loaded(),
        }
    }
}
