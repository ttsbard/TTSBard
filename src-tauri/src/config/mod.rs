//! Configuration module
//!
//! Manages all application configuration stored in %APPDATA%\ttsbard\

mod constants;
pub mod dto;
mod hotkeys;
mod persistence;
mod recovery;
mod settings;
mod validation;
mod windows;

pub use constants::*;
pub use dto::VbanSettingsDto;
pub use dto::{
    AllSourcesParams, AppSettingsDto, TtsProviderInfoDto, VTubeStudioSettingsDto,
    VrchatSettingsDto, VtsHotkeyInfoDto,
};
pub use hotkeys::{EditorHotkeySettings, Hotkey, HotkeyModifier, HotkeySettings};
pub use persistence::{backup_json_copy, config_write_lock, replace_file_atomically};
pub use recovery::{
    normalize_detected_locale, normalize_detected_theme, BackupOutcome, SettingsFailureStage,
    SettingsLoadFailure,
};
pub(crate) use settings::write_default_settings;
pub use settings::{
    normalize_typing_idle_timeout_ms, validate_vrchat_host, validate_vtube_host, AiCustomSettings,
    AiDeepSeekSettings, AiOpenAiSettings, AiProviderType, AiSettings, AiZAiSettings, AppSettings,
    AudioEffectsSettings, AudioOutputFormat, AudioSettings, DspCompressorSettings,
    DspEqBandSettings, DspEqSettings, DspLimiterSettings, DspSettings, EditorRoute,
    HomographAccentorSettings, LoggingSettings, MtProxySettings, NetworkSettings, ProxyMode,
    ProxyType, QuickEditorMode, SettingsManager, SpellSource, Theme, TwitchMode, TwitchSettings,
    VTubeStudioSettings, VTubeStudioTypingAction, VTubeStudioTypingMode, VbanSettings,
    VrchatSettings,
};
pub use validation::{is_valid_hex_color, validate_port};
pub use windows::{
    clamp_compact_size, compact_physical_bounds, CompactView, WindowsManager, WindowsSettings,
};
