//! Data Transfer Objects for unified settings loading
//!
//! This module defines DTOs for the `get_all_app_settings` command.
//! These structures serialize all application settings into a single response.

use crate::config::settings::AudioEffectsSettings;
use crate::config::settings::{
    ElevenLabsSettings, FishAudioSettings, LocalTtsSettings, MtProxySettings, NetworkSettings,
    OpenAiSettings, ProxyMode, ProxySettings, ProxyType, Socks5Settings, TelegramTtsSettings,
    TtsSettings, VTubeStudioTypingMode,
};
use crate::config::windows::{
    GlobalSettings, MainWindowSettings, PlaybackWindowSettings, SoundPanelWindowSettings,
    WindowsSettings,
};
use crate::config::{
    AppSettings as ConfigAppSettings, AudioSettings, EditorHotkeySettings, Hotkey, HotkeyModifier,
    HotkeySettings, LoggingSettings, TwitchSettings,
};
use crate::ocr::settings::OcrSettings;
use crate::soundpanel::SoundBinding;
use crate::tts::elevenlabs::{ElevenLabsModel, ElevenLabsVoice, ElevenLabsVoiceClassification};
use crate::tts::TtsProviderType;
use crate::tts::VoiceModel;
use crate::webview::WebViewSettings;
use serde::{Deserialize, Serialize};

// ============================================================================
// Network Settings DTO
// ============================================================================

/// SOCKS5 proxy settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Socks5SettingsDto {
    pub proxy_url: Option<String>,
}

impl From<Socks5Settings> for Socks5SettingsDto {
    fn from(s: Socks5Settings) -> Self {
        Self {
            proxy_url: s.proxy_url,
        }
    }
}

impl From<Socks5SettingsDto> for Socks5Settings {
    fn from(dto: Socks5SettingsDto) -> Self {
        Self {
            proxy_url: dto.proxy_url,
        }
    }
}

/// MTProxy settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtProxySettingsDto {
    pub host: Option<String>,
    pub port: u16,
    pub secret: Option<String>,
    pub dc_id: Option<i32>,
}

impl From<MtProxySettings> for MtProxySettingsDto {
    fn from(s: MtProxySettings) -> Self {
        Self {
            host: s.host,
            port: s.port,
            secret: s.secret,
            dc_id: s.dc_id,
        }
    }
}

impl From<MtProxySettingsDto> for MtProxySettings {
    fn from(dto: MtProxySettingsDto) -> Self {
        Self {
            host: dto.host,
            port: dto.port,
            secret: dto.secret,
            dc_id: dto.dc_id,
        }
    }
}

/// Network settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSettingsDto {
    pub proxy: Socks5SettingsDto,
    pub mtproxy: MtProxySettingsDto,
}

impl From<NetworkSettings> for NetworkSettingsDto {
    fn from(s: NetworkSettings) -> Self {
        Self {
            proxy: s.proxy.into(),
            mtproxy: s.mtproxy.into(),
        }
    }
}

impl From<NetworkSettingsDto> for NetworkSettings {
    fn from(dto: NetworkSettingsDto) -> Self {
        Self {
            proxy: dto.proxy.into(),
            mtproxy: dto.mtproxy.into(),
        }
    }
}

// ============================================================================
// Legacy Proxy Settings DTO
// ============================================================================

/// Proxy type DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyTypeDto {
    Socks5,
    Socks4,
    Http,
}

impl From<ProxyType> for ProxyTypeDto {
    fn from(t: ProxyType) -> Self {
        match t {
            ProxyType::Socks5 => ProxyTypeDto::Socks5,
            ProxyType::Socks4 => ProxyTypeDto::Socks4,
            ProxyType::Http => ProxyTypeDto::Http,
        }
    }
}

impl From<ProxyTypeDto> for ProxyType {
    fn from(dto: ProxyTypeDto) -> Self {
        match dto {
            ProxyTypeDto::Socks5 => ProxyType::Socks5,
            ProxyTypeDto::Socks4 => ProxyType::Socks4,
            ProxyTypeDto::Http => ProxyType::Http,
        }
    }
}

/// Proxy mode DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyModeDto {
    None,
    Socks5,
    MtProxy,
}

impl From<ProxyMode> for ProxyModeDto {
    fn from(m: ProxyMode) -> Self {
        match m {
            ProxyMode::None => ProxyModeDto::None,
            ProxyMode::Socks5 => ProxyModeDto::Socks5,
            ProxyMode::MtProxy => ProxyModeDto::MtProxy,
        }
    }
}

impl From<ProxyModeDto> for ProxyMode {
    fn from(dto: ProxyModeDto) -> Self {
        match dto {
            ProxyModeDto::None => ProxyMode::None,
            ProxyModeDto::Socks5 => ProxyMode::Socks5,
            ProxyModeDto::MtProxy => ProxyMode::MtProxy,
        }
    }
}

/// Proxy settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySettingsDto {
    pub proxy_url: Option<String>,
    pub proxy_type: ProxyTypeDto,
}

impl From<ProxySettings> for ProxySettingsDto {
    fn from(s: ProxySettings) -> Self {
        Self {
            proxy_url: s.proxy_url,
            proxy_type: s.proxy_type.into(),
        }
    }
}

impl From<ProxySettingsDto> for ProxySettings {
    fn from(dto: ProxySettingsDto) -> Self {
        Self {
            proxy_url: dto.proxy_url,
            proxy_type: dto.proxy_type.into(),
        }
    }
}

// ============================================================================
// TTS Settings DTO
// ============================================================================

/// OpenAI TTS settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiSettingsDto {
    pub api_key: Option<String>,
    pub voice: String,
    pub proxy_host: Option<String>,
    pub proxy_port: Option<u16>,
    pub use_proxy: bool,
}

impl From<OpenAiSettings> for OpenAiSettingsDto {
    fn from(s: OpenAiSettings) -> Self {
        Self {
            api_key: s.api_key,
            voice: s.voice,
            proxy_host: s.proxy_host,
            proxy_port: s.proxy_port,
            use_proxy: s.use_proxy,
        }
    }
}

impl From<OpenAiSettingsDto> for OpenAiSettings {
    fn from(dto: OpenAiSettingsDto) -> Self {
        Self {
            api_key: dto.api_key,
            voice: dto.voice,
            proxy_host: dto.proxy_host,
            proxy_port: dto.proxy_port,
            use_proxy: dto.use_proxy,
        }
    }
}

/// Local TTS settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalTtsSettingsDto {
    pub url: String,
}

impl From<LocalTtsSettings> for LocalTtsSettingsDto {
    fn from(s: LocalTtsSettings) -> Self {
        Self { url: s.url }
    }
}

impl From<LocalTtsSettingsDto> for LocalTtsSettings {
    fn from(dto: LocalTtsSettingsDto) -> Self {
        Self { url: dto.url }
    }
}

/// DTO голосовой модели Fish Audio
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceModelDto {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_image: Option<String>,
    pub languages: Vec<String>,
    pub author_nickname: Option<String>,
}

impl From<VoiceModel> for VoiceModelDto {
    fn from(v: VoiceModel) -> Self {
        Self {
            id: v.id,
            title: v.title,
            description: v.description,
            cover_image: v.cover_image,
            languages: v.languages,
            author_nickname: v.author_nickname,
        }
    }
}

impl From<VoiceModelDto> for VoiceModel {
    fn from(dto: VoiceModelDto) -> Self {
        Self {
            id: dto.id,
            title: dto.title,
            description: dto.description,
            cover_image: dto.cover_image,
            languages: dto.languages,
            author_nickname: dto.author_nickname,
        }
    }
}

/// Fish Audio TTS settings DTO
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FishAudioSettingsDto {
    pub api_key: Option<String>,
    #[serde(default)]
    pub voices: Vec<VoiceModelDto>,
    #[serde(default)]
    pub reference_id: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub temperature: f32,
    #[serde(default)]
    pub sample_rate: u32,
    #[serde(default)]
    pub use_proxy: bool,
}

impl From<FishAudioSettings> for FishAudioSettingsDto {
    fn from(s: FishAudioSettings) -> Self {
        Self {
            api_key: s.api_key,
            voices: s.voices.into_iter().map(|v| v.into()).collect(),
            reference_id: s.reference_id,
            format: s.format,
            temperature: s.temperature,
            sample_rate: s.sample_rate,
            use_proxy: s.use_proxy,
        }
    }
}

impl From<FishAudioSettingsDto> for FishAudioSettings {
    fn from(dto: FishAudioSettingsDto) -> Self {
        Self {
            api_key: dto.api_key,
            voices: dto.voices.into_iter().map(|v| v.into()).collect(),
            reference_id: dto.reference_id,
            format: dto.format,
            temperature: dto.temperature,
            sample_rate: dto.sample_rate,
            use_proxy: dto.use_proxy,
        }
    }
}

/// ElevenLabs voice descriptor DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevenLabsVoiceDto {
    pub voice_id: String,
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub preview_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<ElevenLabsVoiceClassification>,
}

impl From<ElevenLabsVoice> for ElevenLabsVoiceDto {
    fn from(v: ElevenLabsVoice) -> Self {
        Self {
            voice_id: v.voice_id,
            name: v.name,
            category: v.category,
            labels: v.labels,
            preview_url: v.preview_url,
            classification: v.classification,
        }
    }
}

impl From<ElevenLabsVoiceDto> for ElevenLabsVoice {
    fn from(dto: ElevenLabsVoiceDto) -> Self {
        Self {
            voice_id: dto.voice_id,
            name: dto.name,
            category: dto.category,
            labels: dto.labels,
            preview_url: dto.preview_url,
            classification: dto.classification,
        }
    }
}

/// ElevenLabs model descriptor DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevenLabsModelDto {
    pub model_id: String,
    pub name: String,
    #[serde(default)]
    pub can_use_style: bool,
    #[serde(default)]
    pub can_use_speaker_boost: bool,
}

impl From<ElevenLabsModel> for ElevenLabsModelDto {
    fn from(m: ElevenLabsModel) -> Self {
        Self {
            model_id: m.model_id,
            name: m.name,
            can_use_style: m.can_use_style,
            can_use_speaker_boost: m.can_use_speaker_boost,
        }
    }
}

impl From<ElevenLabsModelDto> for ElevenLabsModel {
    fn from(dto: ElevenLabsModelDto) -> Self {
        Self {
            model_id: dto.model_id,
            name: dto.name,
            can_use_style: dto.can_use_style,
            can_use_speaker_boost: dto.can_use_speaker_boost,
        }
    }
}

/// ElevenLabs TTS settings DTO
///
/// `Default` mirrors `ElevenLabsSettings::default()` so a `TtsSettingsDto`
/// without an `elevenlabs` section (legacy/imported state) deserializes to
/// valid provider defaults instead of empty/disabled values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevenLabsSettingsDto {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub voice_id: String,
    #[serde(default)]
    pub voices: Vec<ElevenLabsVoiceDto>,
    #[serde(default)]
    pub models: Vec<ElevenLabsModelDto>,
    #[serde(default = "dto_default_elevenlabs_model_id")]
    pub model_id: String,
    #[serde(default = "dto_default_elevenlabs_output_format")]
    pub output_format: String,
    #[serde(default = "dto_default_elevenlabs_stability")]
    pub stability: f32,
    #[serde(default = "dto_default_elevenlabs_similarity_boost")]
    pub similarity_boost: f32,
    #[serde(default = "dto_default_elevenlabs_style")]
    pub style: f32,
    #[serde(default = "dto_default_elevenlabs_use_speaker_boost")]
    pub use_speaker_boost: bool,
    #[serde(default)]
    pub use_proxy: bool,
}

fn dto_default_elevenlabs_model_id() -> String {
    String::new()
}

fn dto_default_elevenlabs_output_format() -> String {
    crate::tts::elevenlabs::DEFAULT_OUTPUT_FORMAT.to_string()
}

fn dto_default_elevenlabs_stability() -> f32 {
    crate::tts::elevenlabs::DEFAULT_STABILITY
}

fn dto_default_elevenlabs_similarity_boost() -> f32 {
    crate::tts::elevenlabs::DEFAULT_SIMILARITY_BOOST
}

fn dto_default_elevenlabs_style() -> f32 {
    crate::tts::elevenlabs::DEFAULT_STYLE
}

fn dto_default_elevenlabs_use_speaker_boost() -> bool {
    crate::tts::elevenlabs::DEFAULT_USE_SPEAKER_BOOST
}

impl Default for ElevenLabsSettingsDto {
    fn default() -> Self {
        ElevenLabsSettings::default().into()
    }
}

impl From<ElevenLabsSettings> for ElevenLabsSettingsDto {
    fn from(s: ElevenLabsSettings) -> Self {
        Self {
            api_key: s.api_key,
            voice_id: s.voice_id,
            // Catalogs live in the separate `elevenlabs-catalog.json` cache and
            // are injected by `SettingsManager::get_elevenlabs_settings_dto`.
            voices: Vec::new(),
            models: Vec::new(),
            model_id: s.model_id,
            output_format: s.output_format,
            stability: s.stability,
            similarity_boost: s.similarity_boost,
            style: s.style,
            use_speaker_boost: s.use_speaker_boost,
            use_proxy: s.use_proxy,
        }
    }
}

impl From<ElevenLabsSettingsDto> for ElevenLabsSettings {
    fn from(dto: ElevenLabsSettingsDto) -> Self {
        Self {
            api_key: dto.api_key,
            voice_id: dto.voice_id,
            model_id: dto.model_id,
            output_format: dto.output_format,
            stability: dto.stability,
            similarity_boost: dto.similarity_boost,
            style: dto.style,
            use_speaker_boost: dto.use_speaker_boost,
            use_proxy: dto.use_proxy,
        }
    }
}

/// Telegram TTS settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramTtsSettingsDto {
    pub api_id: Option<i64>,
    pub proxy_mode: String,
    #[serde(default)]
    pub voices: Vec<crate::telegram::types::VoiceCode>,
    #[serde(default)]
    pub current_voice_id: String,
    #[serde(default = "default_synthesis_response_timeout_ms_dto")]
    pub synthesis_response_timeout_ms: u32,
    #[serde(default = "default_download_retry_delay_ms_dto")]
    pub download_retry_delay_ms: u32,
}

fn default_synthesis_response_timeout_ms_dto() -> u32 {
    60000
}
fn default_download_retry_delay_ms_dto() -> u32 {
    1000
}

impl From<TelegramTtsSettings> for TelegramTtsSettingsDto {
    fn from(s: TelegramTtsSettings) -> Self {
        Self {
            api_id: s.api_id,
            proxy_mode: serde_json::to_value(&s.proxy_mode)
                .map(|v| v.as_str().unwrap_or("none").to_string())
                .unwrap_or_else(|_| "none".to_string()),
            voices: s.voices,
            current_voice_id: s.current_voice_id,
            synthesis_response_timeout_ms: s.synthesis_response_timeout_ms,
            download_retry_delay_ms: s.download_retry_delay_ms,
        }
    }
}

impl From<TelegramTtsSettingsDto> for TelegramTtsSettings {
    fn from(dto: TelegramTtsSettingsDto) -> Self {
        use crate::config::settings::ProxyMode;
        Self {
            api_id: dto.api_id,
            proxy_mode: match dto.proxy_mode.as_str() {
                "socks5" => ProxyMode::Socks5,
                _ => ProxyMode::None,
            },
            voices: dto.voices,
            current_voice_id: dto.current_voice_id,
            synthesis_response_timeout_ms: dto.synthesis_response_timeout_ms,
            download_retry_delay_ms: dto.download_retry_delay_ms,
        }
    }
}

/// TTS settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsSettingsDto {
    pub provider: TtsProviderType,
    pub openai: OpenAiSettingsDto,
    pub local: LocalTtsSettingsDto,
    #[serde(default)]
    pub fish: FishAudioSettingsDto,
    #[serde(default)]
    pub elevenlabs: ElevenLabsSettingsDto,
    pub telegram: TelegramTtsSettingsDto,
    pub network: NetworkSettingsDto,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub providers: Vec<TtsProviderInfoDto>,
    /// Presentation-only list of visible provider IDs (empty is serialized as
    /// omitted). Derived from `TtsSettings::visible_provider_ids` on output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visible_provider_ids: Vec<String>,
}

impl From<TtsSettings> for TtsSettingsDto {
    fn from(s: TtsSettings) -> Self {
        Self {
            provider: s.provider,
            openai: s.openai.into(),
            local: s.local.into(),
            fish: s.fish.into(),
            elevenlabs: s.elevenlabs.into(),
            telegram: s.telegram.into(),
            network: s.network.into(),
            provider_id: s.provider_id,
            providers: Vec::new(),
            visible_provider_ids: s.visible_provider_ids,
        }
    }
}

impl From<TtsSettingsDto> for TtsSettings {
    fn from(dto: TtsSettingsDto) -> Self {
        Self {
            provider: dto.provider,
            openai: dto.openai.into(),
            local: dto.local.into(),
            fish: dto.fish.into(),
            elevenlabs: dto.elevenlabs.into(),
            telegram: dto.telegram.into(),
            network: dto.network.into(),
            provider_id: dto.provider_id,
            visible_provider_ids: dto.visible_provider_ids,
        }
    }
}

/// Runtime TTS provider info DTO (populated from registry, not persisted)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsProviderInfoDto {
    pub id: String,
    pub display_name: String,
    pub kind: String,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_status: Option<String>,
}

// ============================================================================
// WebView Settings DTO
// ============================================================================

/// WebView server settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebViewSettingsDto {
    pub enabled: bool,
    pub start_on_boot: bool,
    pub port: u16,
    pub access_token: Option<String>,
    pub upnp_enabled: bool,
    pub send_original_text: bool,
}

impl From<WebViewSettings> for WebViewSettingsDto {
    fn from(s: WebViewSettings) -> Self {
        Self {
            enabled: s.enabled,
            start_on_boot: s.start_on_boot,
            port: s.port,
            access_token: s.access_token,
            upnp_enabled: s.upnp_enabled,
            send_original_text: s.send_original_text,
        }
    }
}

impl From<WebViewSettingsDto> for WebViewSettings {
    fn from(dto: WebViewSettingsDto) -> Self {
        Self {
            enabled: dto.enabled,
            start_on_boot: dto.start_on_boot,
            port: dto.port,
            access_token: dto.access_token,
            upnp_enabled: dto.upnp_enabled,
            send_original_text: dto.send_original_text,
        }
    }
}

// ============================================================================
// Twitch Settings DTO
// ============================================================================

/// Twitch settings DTO (same as TwitchSettings, already has Serialize/Deserialize)
pub type TwitchSettingsDto = TwitchSettings;

// ============================================================================
// Audio Settings DTO
// ============================================================================

/// Audio settings DTO (same as AudioSettings, already has Serialize/Deserialize)
pub type AudioSettingsDto = AudioSettings;

// ============================================================================
// Audio Effects Settings DTO
// ============================================================================

/// Audio post-processing effects settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioEffectsSettingsDto {
    pub enabled: bool,
    pub pitch: i16,
    pub speed: i16,
    pub volume: i16,
    pub enhance_enabled: bool,
    pub enhance_atten_db: f32,
    pub formant_preserved: bool,
    pub boundary_cleanup_enabled: bool,
}

impl From<AudioEffectsSettings> for AudioEffectsSettingsDto {
    fn from(s: AudioEffectsSettings) -> Self {
        Self {
            enabled: s.enabled,
            pitch: s.pitch,
            speed: s.speed,
            volume: s.volume,
            enhance_enabled: s.enhance_enabled,
            enhance_atten_db: s.enhance_atten_db,
            formant_preserved: s.formant_preserved,
            boundary_cleanup_enabled: s.boundary_cleanup_enabled,
        }
    }
}

impl From<AudioEffectsSettingsDto> for AudioEffectsSettings {
    fn from(dto: AudioEffectsSettingsDto) -> Self {
        Self {
            enabled: dto.enabled,
            pitch: dto.pitch,
            speed: dto.speed,
            volume: dto.volume,
            enhance_enabled: dto.enhance_enabled,
            enhance_atten_db: dto.enhance_atten_db,
            formant_preserved: dto.formant_preserved,
            boundary_cleanup_enabled: dto.boundary_cleanup_enabled,
        }
    }
}

// ============================================================================
// DSP Post-Processing Settings DTO
// ============================================================================

/// DSP EQ band DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspEqBandSettingsDto {
    pub enabled: bool,
    pub frequency_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

impl From<crate::config::DspEqBandSettings> for DspEqBandSettingsDto {
    fn from(s: crate::config::DspEqBandSettings) -> Self {
        Self {
            enabled: s.enabled,
            frequency_hz: s.frequency_hz,
            gain_db: s.gain_db,
            q: s.q,
        }
    }
}

impl From<DspEqBandSettingsDto> for crate::config::DspEqBandSettings {
    fn from(dto: DspEqBandSettingsDto) -> Self {
        Self {
            enabled: dto.enabled,
            frequency_hz: dto.frequency_hz,
            gain_db: dto.gain_db,
            q: dto.q,
        }
    }
}

/// DSP EQ settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspEqSettingsDto {
    pub enabled: bool,
    pub low_cut_enabled: bool,
    pub low_cut_hz: f32,
    pub low_cut_slope_db: f32,
    pub bands: Vec<DspEqBandSettingsDto>,
    pub high_shelf_enabled: bool,
    pub high_shelf_hz: f32,
    pub high_shelf_gain_db: f32,
}

impl From<crate::config::DspEqSettings> for DspEqSettingsDto {
    fn from(s: crate::config::DspEqSettings) -> Self {
        Self {
            enabled: s.enabled,
            low_cut_enabled: s.low_cut_enabled,
            low_cut_hz: s.low_cut_hz,
            low_cut_slope_db: s.low_cut_slope_db,
            bands: s.bands.into_iter().map(|b| b.into()).collect(),
            high_shelf_enabled: s.high_shelf_enabled,
            high_shelf_hz: s.high_shelf_hz,
            high_shelf_gain_db: s.high_shelf_gain_db,
        }
    }
}

impl From<DspEqSettingsDto> for crate::config::DspEqSettings {
    fn from(dto: DspEqSettingsDto) -> Self {
        let mut bands = [
            crate::config::DspEqBandSettings::default(),
            crate::config::DspEqBandSettings::default(),
            crate::config::DspEqBandSettings::default(),
        ];
        for (i, b) in dto.bands.into_iter().take(3).enumerate() {
            bands[i] = b.into();
        }
        Self {
            enabled: dto.enabled,
            low_cut_enabled: dto.low_cut_enabled,
            low_cut_hz: dto.low_cut_hz,
            low_cut_slope_db: dto.low_cut_slope_db,
            bands,
            high_shelf_enabled: dto.high_shelf_enabled,
            high_shelf_hz: dto.high_shelf_hz,
            high_shelf_gain_db: dto.high_shelf_gain_db,
        }
    }
}

/// DSP compressor settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspCompressorSettingsDto {
    pub enabled: bool,
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_db: f32,
    pub makeup_db: f32,
}

impl From<crate::config::DspCompressorSettings> for DspCompressorSettingsDto {
    fn from(s: crate::config::DspCompressorSettings) -> Self {
        Self {
            enabled: s.enabled,
            threshold_db: s.threshold_db,
            ratio: s.ratio,
            attack_ms: s.attack_ms,
            release_ms: s.release_ms,
            knee_db: s.knee_db,
            makeup_db: s.makeup_db,
        }
    }
}

impl From<DspCompressorSettingsDto> for crate::config::DspCompressorSettings {
    fn from(dto: DspCompressorSettingsDto) -> Self {
        Self {
            enabled: dto.enabled,
            threshold_db: dto.threshold_db,
            ratio: dto.ratio,
            attack_ms: dto.attack_ms,
            release_ms: dto.release_ms,
            knee_db: dto.knee_db,
            makeup_db: dto.makeup_db,
        }
    }
}

/// DSP limiter settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspLimiterSettingsDto {
    pub enabled: bool,
    pub ceiling_db: f32,
    pub release_ms: f32,
}

impl From<crate::config::DspLimiterSettings> for DspLimiterSettingsDto {
    fn from(s: crate::config::DspLimiterSettings) -> Self {
        Self {
            enabled: s.enabled,
            ceiling_db: s.ceiling_db,
            release_ms: s.release_ms,
        }
    }
}

impl From<DspLimiterSettingsDto> for crate::config::DspLimiterSettings {
    fn from(dto: DspLimiterSettingsDto) -> Self {
        Self {
            enabled: dto.enabled,
            ceiling_db: dto.ceiling_db,
            release_ms: dto.release_ms,
        }
    }
}

/// DSP post-processing settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspSettingsDto {
    pub eq: DspEqSettingsDto,
    pub compressor: DspCompressorSettingsDto,
    pub limiter: DspLimiterSettingsDto,
}

impl From<crate::config::DspSettings> for DspSettingsDto {
    fn from(s: crate::config::DspSettings) -> Self {
        Self {
            eq: s.eq.into(),
            compressor: s.compressor.into(),
            limiter: s.limiter.into(),
        }
    }
}

impl From<DspSettingsDto> for crate::config::DspSettings {
    fn from(dto: DspSettingsDto) -> Self {
        Self {
            eq: dto.eq.into(),
            compressor: dto.compressor.into(),
            limiter: dto.limiter.into(),
        }
    }
}

// ============================================================================
// Logging Settings DTO
// ============================================================================

/// Logging settings DTO (same as LoggingSettings, already has Serialize/Deserialize)
pub type LoggingSettingsDto = LoggingSettings;

// ============================================================================
// Windows Settings DTO
// ============================================================================

/// Main window settings DTO
pub type MainWindowSettingsDto = MainWindowSettings;

/// Sound panel window settings DTO
pub type SoundPanelWindowSettingsDto = SoundPanelWindowSettings;

/// Playback control window settings DTO
pub type PlaybackWindowSettingsDto = PlaybackWindowSettings;

/// Global settings DTO
pub type GlobalSettingsDto = GlobalSettings;

/// Windows settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsSettingsDto {
    pub global: GlobalSettingsDto,
    pub main: MainWindowSettingsDto,
    pub soundpanel: SoundPanelWindowSettingsDto,
    pub playback: PlaybackWindowSettingsDto,
}

impl From<WindowsSettings> for WindowsSettingsDto {
    fn from(s: WindowsSettings) -> Self {
        Self {
            global: s.global,
            main: s.main,
            soundpanel: s.soundpanel,
            playback: s.playback,
        }
    }
}

impl From<WindowsSettingsDto> for WindowsSettings {
    fn from(dto: WindowsSettingsDto) -> Self {
        Self {
            global: dto.global,
            main: dto.main,
            soundpanel: dto.soundpanel,
            playback: dto.playback,
        }
    }
}

// ============================================================================
// General Settings DTO
// ============================================================================

/// General application settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSettingsDto {
    pub hotkey_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    pub ui_language: String,
    /// Interface font family: `"default"` keeps the built-in CSS stack.
    pub ui_font_family: String,
    /// Interface font size: the `html` rem base of the main window in px.
    pub ui_font_size_px: u32,
    pub show_playback_on_start: bool,
    pub start_compact: bool,
    pub hide_on_minimize: bool,
}

impl GeneralSettingsDto {
    pub fn from_config(config: &ConfigAppSettings) -> Self {
        Self {
            hotkey_enabled: config.hotkey_enabled,
            theme: Some(match config.theme {
                crate::config::settings::Theme::Dark => "dark".to_string(),
                crate::config::settings::Theme::Light => "light".to_string(),
            }),
            ui_language: config.ui_language.clone(),
            ui_font_family: config.ui_font_family.clone(),
            ui_font_size_px: config.ui_font_size_px,
            show_playback_on_start: config.show_playback_on_start,
            start_compact: config.start_compact,
            hide_on_minimize: config.hide_on_minimize,
        }
    }
}

// ============================================================================
// Preprocessor Settings DTO
// ============================================================================

/// Preprocessor settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreprocessorSettingsDto {
    pub enabled: bool,
    pub replacements_count: usize,
}

impl PreprocessorSettingsDto {
    pub fn from_preprocessor(preprocessor: Option<&crate::preprocessor::TextPreprocessor>) -> Self {
        if let Some(prep) = preprocessor {
            Self {
                enabled: true,
                replacements_count: prep.replacements_count(),
            }
        } else {
            Self {
                enabled: false,
                replacements_count: 0,
            }
        }
    }
}

// ============================================================================
// SoundPanel Settings DTO
// ============================================================================

/// SoundPanel binding DTO (same as SoundBinding, already has Serialize/Deserialize)
pub type SoundBindingDto = SoundBinding;

// ============================================================================
// Editor Settings DTO
// ============================================================================

/// Homograph/accentor (RUAccent) settings DTO
/// (same as HomographAccentorSettings, already has Serialize/Deserialize)
pub type HomographAccentorSettingsDto = crate::config::HomographAccentorSettings;

/// Editor settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorSettingsDto {
    pub quick: String,
    pub ai: bool,
    pub ai_completion: bool,
    pub autocomplete_enabled: bool,
    pub spellcheck_enabled: bool,
    pub spellcheck_source: SpellSourceDto,
    pub editor_height: u32,
    pub typing_idle_timeout_ms: u32,
    pub typing_enabled: bool,
    pub default_route: String,
    pub keep_text_after_send: bool,
    pub font_family: String,
    pub font_size_px: u32,
    #[serde(default)]
    pub homograph_accentor: HomographAccentorSettingsDto,
}

/// Spell check source DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpellSourceDto {
    Online,
    Offline,
}

// ============================================================================
// AI Settings DTO
// ============================================================================

/// AI provider type DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProviderTypeDto {
    OpenAi,
    ZAi,
    DeepSeek,
    Custom,
}

impl From<crate::config::AiProviderType> for AiProviderTypeDto {
    fn from(t: crate::config::AiProviderType) -> Self {
        match t {
            crate::config::AiProviderType::OpenAi => AiProviderTypeDto::OpenAi,
            crate::config::AiProviderType::ZAi => AiProviderTypeDto::ZAi,
            crate::config::AiProviderType::DeepSeek => AiProviderTypeDto::DeepSeek,
            crate::config::AiProviderType::Custom => AiProviderTypeDto::Custom,
        }
    }
}

impl From<AiProviderTypeDto> for crate::config::AiProviderType {
    fn from(dto: AiProviderTypeDto) -> Self {
        match dto {
            AiProviderTypeDto::OpenAi => crate::config::AiProviderType::OpenAi,
            AiProviderTypeDto::ZAi => crate::config::AiProviderType::ZAi,
            AiProviderTypeDto::DeepSeek => crate::config::AiProviderType::DeepSeek,
            AiProviderTypeDto::Custom => crate::config::AiProviderType::Custom,
        }
    }
}

/// OpenAI AI settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiOpenAiSettingsDto {
    pub api_key: Option<String>,
    #[serde(default)]
    pub use_proxy: bool,
    #[serde(default = "default_openai_model_dto")]
    pub model: String,
}

fn default_openai_model_dto() -> String {
    "gpt-4o-mini".to_string()
}

impl From<crate::config::AiOpenAiSettings> for AiOpenAiSettingsDto {
    fn from(s: crate::config::AiOpenAiSettings) -> Self {
        Self {
            api_key: s.api_key,
            use_proxy: s.use_proxy,
            model: s.model,
        }
    }
}

impl From<AiOpenAiSettingsDto> for crate::config::AiOpenAiSettings {
    fn from(dto: AiOpenAiSettingsDto) -> Self {
        Self {
            api_key: dto.api_key,
            use_proxy: dto.use_proxy,
            model: dto.model,
        }
    }
}

/// Z.ai AI settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiZAiSettingsDto {
    pub url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default = "default_zai_model_dto")]
    pub model: String,
}

fn default_zai_model_dto() -> String {
    "glm-4.5".to_string()
}

impl From<crate::config::AiZAiSettings> for AiZAiSettingsDto {
    fn from(s: crate::config::AiZAiSettings) -> Self {
        Self {
            url: s.url,
            api_key: s.api_key,
            model: s.model,
        }
    }
}

impl From<AiZAiSettingsDto> for crate::config::AiZAiSettings {
    fn from(dto: AiZAiSettingsDto) -> Self {
        Self {
            url: dto.url,
            api_key: dto.api_key,
            model: dto.model,
        }
    }
}

/// DeepSeek AI settings DTO
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiDeepSeekSettingsDto {
    pub api_key: Option<String>,
    #[serde(default)]
    pub use_proxy: bool,
    #[serde(default = "default_deepseek_model_dto")]
    pub model: String,
}

fn default_deepseek_model_dto() -> String {
    "deepseek-chat".to_string()
}

impl From<crate::config::AiDeepSeekSettings> for AiDeepSeekSettingsDto {
    fn from(s: crate::config::AiDeepSeekSettings) -> Self {
        Self {
            api_key: s.api_key,
            use_proxy: s.use_proxy,
            model: s.model,
        }
    }
}

impl From<AiDeepSeekSettingsDto> for crate::config::AiDeepSeekSettings {
    fn from(dto: AiDeepSeekSettingsDto) -> Self {
        Self {
            api_key: dto.api_key,
            use_proxy: dto.use_proxy,
            model: dto.model,
        }
    }
}

/// Custom OpenAI-compatible AI settings DTO
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiCustomSettingsDto {
    pub url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub use_proxy: bool,
    #[serde(default)]
    pub model: String,
}

impl From<crate::config::AiCustomSettings> for AiCustomSettingsDto {
    fn from(s: crate::config::AiCustomSettings) -> Self {
        Self {
            url: s.url,
            api_key: s.api_key,
            use_proxy: s.use_proxy,
            model: s.model,
        }
    }
}

impl From<AiCustomSettingsDto> for crate::config::AiCustomSettings {
    fn from(dto: AiCustomSettingsDto) -> Self {
        Self {
            url: dto.url,
            api_key: dto.api_key,
            use_proxy: dto.use_proxy,
            model: dto.model,
        }
    }
}

/// AI settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettingsDto {
    pub provider: AiProviderTypeDto,
    pub openai: AiOpenAiSettingsDto,
    pub zai: AiZAiSettingsDto,
    #[serde(default)]
    pub deepseek: AiDeepSeekSettingsDto,
    #[serde(default)]
    pub custom: AiCustomSettingsDto,
    pub prompt: String,
    #[serde(default = "default_ai_timeout_dto")]
    pub timeout: u64,
}

fn default_ai_timeout_dto() -> u64 {
    20
}

impl From<crate::config::AiSettings> for AiSettingsDto {
    fn from(s: crate::config::AiSettings) -> Self {
        Self {
            provider: s.provider.into(),
            openai: s.openai.into(),
            zai: s.zai.into(),
            deepseek: s.deepseek.into(),
            custom: s.custom.into(),
            prompt: s.prompt,
            timeout: s.timeout,
        }
    }
}

impl From<AiSettingsDto> for crate::config::AiSettings {
    fn from(dto: AiSettingsDto) -> Self {
        Self {
            provider: dto.provider.into(),
            openai: dto.openai.into(),
            zai: dto.zai.into(),
            deepseek: dto.deepseek.into(),
            custom: dto.custom.into(),
            prompt: dto.prompt,
            timeout: dto.timeout,
        }
    }
}

// ============================================================================
// Hotkey Settings DTO
// ============================================================================

/// Hotkey modifier DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyModifierDto {
    Ctrl,
    Shift,
    Alt,
    Super,
}

impl From<HotkeyModifier> for HotkeyModifierDto {
    fn from(m: HotkeyModifier) -> Self {
        match m {
            HotkeyModifier::Ctrl => HotkeyModifierDto::Ctrl,
            HotkeyModifier::Shift => HotkeyModifierDto::Shift,
            HotkeyModifier::Alt => HotkeyModifierDto::Alt,
            HotkeyModifier::Super => HotkeyModifierDto::Super,
        }
    }
}

impl From<HotkeyModifierDto> for HotkeyModifier {
    fn from(dto: HotkeyModifierDto) -> Self {
        match dto {
            HotkeyModifierDto::Ctrl => HotkeyModifier::Ctrl,
            HotkeyModifierDto::Shift => HotkeyModifier::Shift,
            HotkeyModifierDto::Alt => HotkeyModifier::Alt,
            HotkeyModifierDto::Super => HotkeyModifier::Super,
        }
    }
}

/// Hotkey DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyDto {
    pub modifiers: Vec<HotkeyModifierDto>,
    pub key: String,
}

impl From<Hotkey> for HotkeyDto {
    fn from(h: Hotkey) -> Self {
        Self {
            modifiers: h.modifiers.into_iter().map(|m| m.into()).collect(),
            key: h.key,
        }
    }
}

impl From<HotkeyDto> for Hotkey {
    fn from(dto: HotkeyDto) -> Self {
        Self {
            modifiers: dto.modifiers.into_iter().map(|m| m.into()).collect(),
            key: dto.key,
        }
    }
}

/// Editor hotkey settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorHotkeySettingsDto {
    pub edit_word: HotkeyDto,
    pub submit_continue: HotkeyDto,
    pub submit_keep_text: HotkeyDto,
    pub submit_keep_focus: HotkeyDto,
    pub next_spelling_error: HotkeyDto,
    pub previous_spelling_error: HotkeyDto,
    pub next_tab: HotkeyDto,
    pub previous_tab: HotkeyDto,
    pub cycle_route: HotkeyDto,
    pub toggle_typing: HotkeyDto,
    pub cycle_quick_mode: HotkeyDto,
    pub toggle_history: HotkeyDto,
    pub accent_homographs: HotkeyDto,
    pub approve_next_incoming: HotkeyDto,
    pub edit_next_incoming: HotkeyDto,
}

impl From<EditorHotkeySettings> for EditorHotkeySettingsDto {
    fn from(h: EditorHotkeySettings) -> Self {
        Self {
            edit_word: h.edit_word.into(),
            submit_continue: h.submit_continue.into(),
            submit_keep_text: h.submit_keep_text.into(),
            submit_keep_focus: h.submit_keep_focus.into(),
            next_spelling_error: h.next_spelling_error.into(),
            previous_spelling_error: h.previous_spelling_error.into(),
            next_tab: h.next_tab.into(),
            previous_tab: h.previous_tab.into(),
            cycle_route: h.cycle_route.into(),
            toggle_typing: h.toggle_typing.into(),
            cycle_quick_mode: h.cycle_quick_mode.into(),
            toggle_history: h.toggle_history.into(),
            accent_homographs: h.accent_homographs.into(),
            approve_next_incoming: h.approve_next_incoming.into(),
            edit_next_incoming: h.edit_next_incoming.into(),
        }
    }
}

impl From<EditorHotkeySettingsDto> for EditorHotkeySettings {
    fn from(dto: EditorHotkeySettingsDto) -> Self {
        Self {
            edit_word: dto.edit_word.into(),
            submit_continue: dto.submit_continue.into(),
            submit_keep_text: dto.submit_keep_text.into(),
            submit_keep_focus: dto.submit_keep_focus.into(),
            next_spelling_error: dto.next_spelling_error.into(),
            previous_spelling_error: dto.previous_spelling_error.into(),
            next_tab: dto.next_tab.into(),
            previous_tab: dto.previous_tab.into(),
            cycle_route: dto.cycle_route.into(),
            toggle_typing: dto.toggle_typing.into(),
            cycle_quick_mode: dto.cycle_quick_mode.into(),
            toggle_history: dto.toggle_history.into(),
            accent_homographs: dto.accent_homographs.into(),
            approve_next_incoming: dto.approve_next_incoming.into(),
            edit_next_incoming: dto.edit_next_incoming.into(),
        }
    }
}

/// Hotkey settings DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeySettingsDto {
    pub main_window: HotkeyDto,
    pub sound_panel: HotkeyDto,
    pub playback_pause: HotkeyDto,
    pub playback_stop: HotkeyDto,
    pub playback_repeat: HotkeyDto,
    pub playback_control_window: HotkeyDto,
    pub return_previous_window: HotkeyDto,
    pub toggle_minimal_mode: HotkeyDto,
    pub ocr_capture: HotkeyDto,
    pub editor: EditorHotkeySettingsDto,
}

impl From<HotkeySettings> for HotkeySettingsDto {
    fn from(h: HotkeySettings) -> Self {
        Self {
            main_window: h.main_window.into(),
            sound_panel: h.sound_panel.into(),
            playback_pause: h.playback_pause.into(),
            playback_stop: h.playback_stop.into(),
            playback_repeat: h.playback_repeat.into(),
            playback_control_window: h.playback_control_window.into(),
            return_previous_window: h.return_previous_window.into(),
            toggle_minimal_mode: h.toggle_minimal_mode.into(),
            ocr_capture: h.ocr_capture.into(),
            editor: h.editor.into(),
        }
    }
}

impl From<HotkeySettingsDto> for HotkeySettings {
    fn from(dto: HotkeySettingsDto) -> Self {
        Self {
            main_window: dto.main_window.into(),
            sound_panel: dto.sound_panel.into(),
            playback_pause: dto.playback_pause.into(),
            playback_stop: dto.playback_stop.into(),
            playback_repeat: dto.playback_repeat.into(),
            playback_control_window: dto.playback_control_window.into(),
            return_previous_window: dto.return_previous_window.into(),
            toggle_minimal_mode: dto.toggle_minimal_mode.into(),
            ocr_capture: dto.ocr_capture.into(),
            editor: dto.editor.into(),
        }
    }
}

// ============================================================================
// VTube Studio Settings DTO
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VTubeStudioTypingActionDto {
    pub output_mode: VTubeStudioTypingMode,
    pub parameter_name: String,
    pub start_hotkey_id: String,
    pub stop_hotkey_id: String,
    pub start_hotkey_name: String,
    pub stop_hotkey_name: String,
    pub item_file_name: String,
    pub item_type: String,
}

impl From<&crate::config::VTubeStudioTypingAction> for VTubeStudioTypingActionDto {
    fn from(a: &crate::config::VTubeStudioTypingAction) -> Self {
        Self {
            output_mode: a.output_mode.clone(),
            parameter_name: a.parameter_name.clone(),
            start_hotkey_id: a.start_hotkey_id.clone(),
            stop_hotkey_id: a.stop_hotkey_id.clone(),
            start_hotkey_name: a.start_hotkey_name.clone(),
            stop_hotkey_name: a.stop_hotkey_name.clone(),
            item_file_name: a.item_file_name.clone(),
            item_type: a.item_type.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VTubeStudioSettingsDto {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub start_on_boot: bool,
    #[serde(rename = "typingAction")]
    pub typing_action: VTubeStudioTypingActionDto,
}

impl From<crate::config::VTubeStudioSettings> for VTubeStudioSettingsDto {
    fn from(s: crate::config::VTubeStudioSettings) -> Self {
        Self {
            enabled: s.enabled,
            host: s.host,
            port: s.port,
            start_on_boot: s.start_on_boot,
            typing_action: (&s.typing_action).into(),
        }
    }
}

/// VRChat settings DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VrchatSettingsDto {
    pub enabled: bool,
    #[serde(default)]
    pub start_on_boot: bool,
    pub host: String,
    pub port: u16,
}

impl Default for VrchatSettingsDto {
    fn default() -> Self {
        Self {
            enabled: false,
            start_on_boot: false,
            host: "127.0.0.1".to_string(),
            port: 9000,
        }
    }
}

impl From<crate::config::VrchatSettings> for VrchatSettingsDto {
    fn from(s: crate::config::VrchatSettings) -> Self {
        Self {
            enabled: s.enabled,
            start_on_boot: s.start_on_boot,
            host: s.host,
            port: s.port,
        }
    }
}

impl From<VrchatSettingsDto> for crate::config::VrchatSettings {
    fn from(dto: VrchatSettingsDto) -> Self {
        Self {
            enabled: dto.enabled,
            start_on_boot: dto.start_on_boot,
            host: dto.host,
            port: dto.port,
        }
    }
}

/// Hotkey info from VTube Studio API (for UI selection)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VtsHotkeyInfoDto {
    #[serde(rename = "hotkeyID")]
    pub hotkey_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub hotkey_type: String,
    pub description: String,
}

// ============================================================================
// OCR Settings DTO
// ============================================================================

/// Persisted one-shot screen OCR settings DTO.
///
/// Reuses the safe `OcrSettings` domain type verbatim: it exposes only the
/// persisted desired state (`enabled`, nullable `model_id`) and no runtime
/// status, capture paths, engine internals or hotkeys.
pub type OcrSettingsDto = OcrSettings;

// ============================================================================
// Main App Settings DTO
// ============================================================================

/// Parameters for creating AppSettingsDto from multiple sources
pub struct AllSourcesParams<'a> {
    pub config: &'a ConfigAppSettings,
    pub webview_settings: &'a WebViewSettings,
    pub twitch_settings: &'a TwitchSettings,
    pub windows_settings: &'a WindowsSettings,
    pub preprocessor: Option<&'a crate::preprocessor::TextPreprocessor>,
    pub soundpanel_bindings: Vec<SoundBinding>,
}

/// Storage settings (user paths, ROADMAP-117)
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct StorageSettingsDto {
    #[serde(default)]
    pub data_dir: Option<String>,
    /// Custom audio cache directory; `None` — default `%LOCALAPPDATA%`.
    #[serde(default)]
    pub audio_cache_dir: Option<String>,
}

/// All application settings in a single DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettingsDto {
    /// One-shot startup notifications; consumed by the first settings request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notifications: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub startup_errors: Vec<String>,
    /// TTS settings
    pub tts: TtsSettingsDto,
    /// WebView settings
    pub webview: WebViewSettingsDto,
    /// Twitch settings
    pub twitch: TwitchSettingsDto,
    /// Windows settings
    pub windows: WindowsSettingsDto,
    /// Audio settings
    pub audio: AudioSettingsDto,
    /// Audio post-processing effects settings
    pub audio_effects: AudioEffectsSettingsDto,
    /// DSP post-processing settings (EQ + compressor + limiter)
    pub dsp: DspSettingsDto,
    /// General settings
    pub general: GeneralSettingsDto,
    /// Editor settings
    pub editor: EditorSettingsDto,
    /// OCR settings (persisted desired state only: `enabled`, nullable `model_id`)
    pub ocr: OcrSettingsDto,
    /// Logging settings
    pub logging: LoggingSettingsDto,
    /// Storage settings (user paths)
    pub storage: StorageSettingsDto,
    /// Preprocessor settings
    pub preprocessor: PreprocessorSettingsDto,
    /// SoundPanel bindings
    pub soundpanel_bindings: Vec<SoundBindingDto>,
    /// AI settings
    pub ai: AiSettingsDto,
    /// Hotkey settings
    pub hotkeys: HotkeySettingsDto,
    /// VTube Studio settings (safe — no token)
    pub vtube_studio: VTubeStudioSettingsDto,
    /// VRChat settings
    pub vrchat: VrchatSettingsDto,
}

impl AppSettingsDto {
    /// Create AppSettingsDto from all sources
    pub fn from_all_sources(params: AllSourcesParams<'_>) -> Self {
        Self {
            notifications: Vec::new(),
            startup_errors: Vec::new(),
            tts: params.config.tts.clone().into(),
            webview: params.webview_settings.clone().into(),
            twitch: params.twitch_settings.clone(),
            windows: params.windows_settings.clone().into(),
            audio: params.config.audio.clone(),
            audio_effects: params.config.audio_effects.clone().into(),
            dsp: params.config.dsp.clone().into(),
            general: GeneralSettingsDto::from_config(params.config),
            editor: EditorSettingsDto {
                quick: params.config.editor.quick.as_str().to_string(),
                ai: params.config.editor.ai,
                ai_completion: params.config.editor.ai_completion,
                autocomplete_enabled: params.config.editor.autocomplete_enabled,
                spellcheck_enabled: params.config.editor.spellcheck_enabled,
                spellcheck_source: match params.config.editor.spellcheck_source {
                    crate::config::SpellSource::Online => SpellSourceDto::Online,
                    crate::config::SpellSource::Offline => SpellSourceDto::Offline,
                },
                editor_height: params.config.editor.editor_height,
                typing_idle_timeout_ms: params.config.editor.typing_idle_timeout_ms,
                typing_enabled: params.config.editor.typing_enabled,
                default_route: params.config.editor.default_route.as_str().to_string(),
                keep_text_after_send: params.config.editor.keep_text_after_send,
                font_family: params.config.editor.font_family.clone(),
                font_size_px: params.config.editor.font_size_px,
                homograph_accentor: params.config.editor.homograph_accentor.clone(),
            },
            ocr: params.config.ocr.clone(),
            logging: params.config.logging.clone(),
            storage: StorageSettingsDto {
                data_dir: params.config.storage.data_dir.clone(),
                audio_cache_dir: params.config.storage.audio_cache_dir.clone(),
            },
            preprocessor: PreprocessorSettingsDto::from_preprocessor(params.preprocessor),
            soundpanel_bindings: params.soundpanel_bindings,
            ai: params.config.ai.clone().into(),
            hotkeys: params.config.hotkeys.clone().into(),
            vtube_studio: params.config.vtube_studio.clone().into(),
            vrchat: params.config.vrchat.clone().into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CompactView;

    /// Populated fixture: every reachable array/object shape is present
    fn build_populated() -> AppSettingsDto {
        let tts = TtsSettingsDto {
            provider: TtsProviderType::OpenAi,
            openai: OpenAiSettingsDto {
                api_key: Some("sk-test-key".into()),
                voice: "alloy".into(),
                proxy_host: None,
                proxy_port: None,
                use_proxy: false,
            },
            local: LocalTtsSettingsDto {
                url: "http://localhost:5002".into(),
            },
            fish: FishAudioSettingsDto {
                api_key: Some("fish-test-key".into()),
                voices: vec![VoiceModelDto {
                    id: "voice-1".into(),
                    title: "Test Voice".into(),
                    description: Some("A test voice".into()),
                    cover_image: None,
                    languages: vec!["en".into(), "ru".into()],
                    author_nickname: Some("author1".into()),
                }],
                reference_id: "ref-abc".into(),
                format: "mp3".into(),
                temperature: 0.7,
                sample_rate: 44100,
                use_proxy: true,
            },
            elevenlabs: ElevenLabsSettingsDto {
                api_key: Some("el-test-key".into()),
                voice_id: "el-voice-1".into(),
                voices: vec![ElevenLabsVoiceDto {
                    voice_id: "el-voice-1".into(),
                    name: "Eleven Voice".into(),
                    category: Some("premade".into()),
                    labels: vec!["american".into(), "female".into()],
                    preview_url: Some("https://example.com/preview.mp3".into()),
                    classification: None,
                }],
                models: vec![ElevenLabsModelDto {
                    model_id: "eleven_flash_v2_5".into(),
                    name: "Flash v2.5".into(),
                    can_use_style: true,
                    can_use_speaker_boost: true,
                }],
                model_id: "eleven_flash_v2_5".into(),
                output_format: "mp3_44100_128".into(),
                stability: 0.5,
                similarity_boost: 0.75,
                style: 0.0,
                use_speaker_boost: true,
                use_proxy: false,
            },
            telegram: TelegramTtsSettingsDto {
                api_id: Some(12345),
                proxy_mode: "none".to_string(),
                voices: vec![],
                current_voice_id: String::new(),
                synthesis_response_timeout_ms: 10000,
                download_retry_delay_ms: 1000,
            },
            network: NetworkSettingsDto {
                proxy: Socks5SettingsDto { proxy_url: None },
                mtproxy: MtProxySettingsDto {
                    host: None,
                    port: 443,
                    secret: None,
                    dc_id: None,
                },
            },
            provider_id: Some("openai".into()),
            providers: vec![TtsProviderInfoDto {
                id: "silero-0".into(),
                display_name: "Silero".into(),
                kind: "silero".into(),
                active: false,
                runtime_status: Some("Ready".into()),
            }],
            visible_provider_ids: vec![],
        };

        let webview = WebViewSettingsDto {
            enabled: true,
            start_on_boot: false,
            port: 8080,
            access_token: None,
            upnp_enabled: false,
            send_original_text: true,
        };

        let twitch = TwitchSettingsDto {
            mode: crate::config::TwitchMode::Irc,
            enabled: false,
            username: String::new(),
            token: String::new(),
            channel: String::new(),
            start_on_boot: false,
            send_original_text: true,
        };

        let windows = WindowsSettingsDto {
            global: GlobalSettingsDto {
                exclude_from_capture: false,
            },
            main: MainWindowSettingsDto {
                x: Some(100),
                y: Some(200),
                custom_background: false,
                opacity: 100,
                bg_color: "#1e1e2e".into(),
                custom_opacity: false,
                opacity_compact_only: false,
                compact_width: 400,
                compact_height: 500,
                compact_view: CompactView::Compact,
                hide_extra_window_buttons: true,
            },
            soundpanel: SoundPanelWindowSettingsDto {
                x: None,
                y: None,
                opacity: 90,
                bg_color: "#181825".into(),
                clickthrough: false,
                stay_visible: true,
                hide_on_blur: false,
                appearance_source: "main".into(),
            },
            playback: PlaybackWindowSettingsDto {
                x: None,
                y: None,
                opacity: 100,
                bg_color: "#11111b".into(),
                appearance_source: "main".into(),
            },
        };

        let audio = AudioSettingsDto {
            output_format: crate::config::AudioOutputFormat::Default,
            speaker_device: None,
            speaker_enabled: true,
            speaker_volume: 80,
            virtual_mic_device: Some("CABLE Input".into()),
            virtual_mic_volume: 100,
        };

        let audio_effects = AudioEffectsSettingsDto {
            enabled: true,
            pitch: 0,
            speed: 0,
            volume: 100,
            enhance_enabled: true,
            enhance_atten_db: 12.0,
            formant_preserved: true,
            boundary_cleanup_enabled: true,
        };

        let dsp = DspSettingsDto {
            eq: DspEqSettingsDto {
                enabled: true,
                low_cut_enabled: true,
                low_cut_hz: 80.0,
                low_cut_slope_db: 12.0,
                bands: vec![DspEqBandSettingsDto {
                    enabled: true,
                    frequency_hz: 1000.0,
                    gain_db: 3.0,
                    q: 1.414,
                }],
                high_shelf_enabled: false,
                high_shelf_hz: 8000.0,
                high_shelf_gain_db: 0.0,
            },
            compressor: DspCompressorSettingsDto {
                enabled: false,
                threshold_db: -24.0,
                ratio: 4.0,
                attack_ms: 5.0,
                release_ms: 50.0,
                knee_db: 2.0,
                makeup_db: 0.0,
            },
            limiter: DspLimiterSettingsDto {
                enabled: true,
                ceiling_db: -1.0,
                release_ms: 20.0,
            },
        };

        let general = GeneralSettingsDto {
            hotkey_enabled: true,
            theme: Some("dark".into()),
            ui_language: "ru".into(),
            ui_font_family: "default".into(),
            ui_font_size_px: 16,
            show_playback_on_start: false,
            start_compact: false,
            hide_on_minimize: true,
        };

        let editor = EditorSettingsDto {
            quick: "collapse".into(),
            ai: true,
            ai_completion: false,
            autocomplete_enabled: true,
            spellcheck_enabled: true,
            spellcheck_source: SpellSourceDto::Online,
            editor_height: 300,
            typing_idle_timeout_ms: 800,
            typing_enabled: true,
            default_route: "no_twitch".into(),
            keep_text_after_send: true,
            font_family: "georgia".into(),
            font_size_px: 20,
            homograph_accentor: HomographAccentorSettingsDto {
                enabled: true,
                accentor_pack_id: Some("com.example.ruaccent".into()),
                load_on_start: true,
            },
        };

        let ocr = OcrSettingsDto {
            enabled: true,
            model_id: Some("silero-ocr".to_string()),
            capture_target: crate::ocr::monitors::CaptureTarget::Primary,
        };

        let logging = LoggingSettingsDto {
            enabled: true,
            level: "info".into(),
            module_levels: {
                let mut m = std::collections::HashMap::new();
                m.insert("tts".into(), "debug".into());
                m
            },
        };

        let preprocessor = PreprocessorSettingsDto {
            enabled: true,
            replacements_count: 12,
        };

        let soundpanel_bindings: Vec<SoundBindingDto> = vec![SoundBinding {
            key: 'a',
            description: "Test sound A".into(),
            filename: "test_a.wav".into(),
            original_path: Some("D:\\sounds\\test_a.wav".into()),
        }];

        let ai = AiSettingsDto {
            provider: AiProviderTypeDto::OpenAi,
            openai: AiOpenAiSettingsDto {
                api_key: Some("sk-ai-test".into()),
                use_proxy: false,
                model: "gpt-4o-mini".into(),
            },
            zai: AiZAiSettingsDto {
                url: None,
                api_key: None,
                model: "glm-4.5".into(),
            },
            deepseek: AiDeepSeekSettingsDto {
                api_key: None,
                use_proxy: false,
                model: "deepseek-chat".into(),
            },
            custom: AiCustomSettingsDto {
                url: None,
                api_key: None,
                use_proxy: false,
                model: String::new(),
            },
            prompt: String::new(),
            timeout: 20,
        };

        let hotkeys = HotkeySettingsDto {
            main_window: HotkeyDto {
                modifiers: vec![HotkeyModifierDto::Ctrl],
                key: "F3".into(),
            },
            sound_panel: HotkeyDto {
                modifiers: vec![HotkeyModifierDto::Ctrl],
                key: "F2".into(),
            },
            playback_pause: HotkeyDto {
                modifiers: vec![],
                key: "F5".into(),
            },
            playback_stop: HotkeyDto {
                modifiers: vec![],
                key: "F6".into(),
            },
            playback_repeat: HotkeyDto {
                modifiers: vec![],
                key: "F7".into(),
            },
            playback_control_window: HotkeyDto {
                modifiers: vec![],
                key: "F8".into(),
            },
            return_previous_window: HotkeyDto {
                modifiers: vec![HotkeyModifierDto::Ctrl],
                key: "F".into(),
            },
            toggle_minimal_mode: HotkeyDto {
                modifiers: vec![HotkeyModifierDto::Ctrl],
                key: "F".into(),
            },
            ocr_capture: HotkeyDto {
                modifiers: vec![HotkeyModifierDto::Ctrl, HotkeyModifierDto::Shift],
                key: "P".into(),
            },
            editor: EditorHotkeySettingsDto {
                edit_word: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl],
                    key: "E".into(),
                },
                submit_continue: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl],
                    key: "Enter".into(),
                },
                submit_keep_text: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Alt],
                    key: "Enter".into(),
                },
                submit_keep_focus: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl, HotkeyModifierDto::Alt],
                    key: "Enter".into(),
                },
                next_spelling_error: HotkeyDto {
                    modifiers: vec![],
                    key: "F7".into(),
                },
                previous_spelling_error: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Shift],
                    key: "F7".into(),
                },
                next_tab: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl],
                    key: "Tab".into(),
                },
                previous_tab: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl, HotkeyModifierDto::Shift],
                    key: "Tab".into(),
                },
                cycle_route: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl],
                    key: "R".into(),
                },
                toggle_typing: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl],
                    key: "T".into(),
                },
                cycle_quick_mode: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl],
                    key: "W".into(),
                },
                toggle_history: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl],
                    key: "H".into(),
                },
                accent_homographs: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl],
                    key: "U".into(),
                },
                approve_next_incoming: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl],
                    key: "K".into(),
                },
                edit_next_incoming: HotkeyDto {
                    modifiers: vec![HotkeyModifierDto::Ctrl],
                    key: "I".into(),
                },
            },
        };

        let vtube_studio = VTubeStudioSettingsDto {
            enabled: false,
            host: "127.0.0.1".to_string(),
            port: 8001,
            start_on_boot: false,
            typing_action: VTubeStudioTypingActionDto {
                output_mode: VTubeStudioTypingMode::Event,
                parameter_name: String::new(),
                start_hotkey_id: String::new(),
                stop_hotkey_id: String::new(),
                start_hotkey_name: String::new(),
                stop_hotkey_name: String::new(),
                item_file_name: String::new(),
                item_type: String::new(),
            },
        };

        AppSettingsDto {
            notifications: vec!["startup-message".into()],
            startup_errors: vec!["startup-error".into()],
            tts,
            webview,
            twitch,
            windows,
            audio,
            audio_effects,
            dsp,
            storage: StorageSettingsDto {
                data_dir: None,
                audio_cache_dir: None,
            },
            general,
            editor,
            ocr,
            logging,
            preprocessor,
            soundpanel_bindings,
            ai,
            hotkeys,
            vtube_studio,
            vrchat: VrchatSettingsDto::default(),
        }
    }

    /// Omitted-or-null fixture: skip_serializing_if fields absent, required nullables null, empty collections
    fn build_omitted_or_null() -> AppSettingsDto {
        let tts = TtsSettingsDto {
            provider: TtsProviderType::OpenAi,
            openai: OpenAiSettingsDto {
                api_key: None,
                voice: "alloy".into(),
                proxy_host: None,
                proxy_port: None,
                use_proxy: false,
            },
            local: LocalTtsSettingsDto {
                url: "http://localhost:5002".into(),
            },
            fish: FishAudioSettingsDto {
                api_key: None,
                voices: vec![VoiceModelDto {
                    id: "voice-omit".into(),
                    title: "Omit Voice".into(),
                    description: None,
                    cover_image: None,
                    languages: vec!["en".into()],
                    author_nickname: None,
                }],
                ..Default::default()
            },
            elevenlabs: ElevenLabsSettingsDto {
                api_key: None,
                voice_id: String::new(),
                voices: vec![ElevenLabsVoiceDto {
                    voice_id: "el-voice-omit".into(),
                    name: "Omit Eleven Voice".into(),
                    category: None,
                    labels: vec![],
                    preview_url: None,
                    classification: None,
                }],
                models: vec![],
                model_id: String::new(),
                output_format: String::new(),
                stability: 0.0,
                similarity_boost: 0.0,
                style: 0.0,
                use_speaker_boost: false,
                use_proxy: false,
            },
            telegram: TelegramTtsSettingsDto {
                api_id: None,
                proxy_mode: "none".to_string(),
                voices: vec![],
                current_voice_id: String::new(),
                synthesis_response_timeout_ms: 10000,
                download_retry_delay_ms: 1000,
            },
            network: NetworkSettingsDto {
                proxy: Socks5SettingsDto { proxy_url: None },
                mtproxy: MtProxySettingsDto {
                    host: None,
                    port: 443,
                    secret: None,
                    dc_id: None,
                },
            },
            provider_id: None,
            providers: vec![TtsProviderInfoDto {
                id: "silero-0".into(),
                display_name: "Silero".into(),
                kind: "silero".into(),
                active: false,
                runtime_status: None,
            }],
            visible_provider_ids: vec![],
        };

        let webview = WebViewSettingsDto {
            enabled: false,
            start_on_boot: false,
            port: 8080,
            access_token: None,
            upnp_enabled: false,
            send_original_text: true,
        };

        let twitch = TwitchSettingsDto {
            mode: crate::config::TwitchMode::Irc,
            enabled: false,
            username: String::new(),
            token: String::new(),
            channel: String::new(),
            start_on_boot: false,
            send_original_text: true,
        };

        let windows = WindowsSettingsDto {
            global: GlobalSettingsDto {
                exclude_from_capture: false,
            },
            main: MainWindowSettingsDto {
                x: None,
                y: None,
                custom_background: false,
                opacity: 100,
                bg_color: "#1e1e2e".into(),
                custom_opacity: false,
                opacity_compact_only: false,
                compact_width: 400,
                compact_height: 500,
                compact_view: CompactView::Compact,
                hide_extra_window_buttons: false,
            },
            soundpanel: SoundPanelWindowSettingsDto {
                x: None,
                y: None,
                opacity: 100,
                bg_color: "#181825".into(),
                clickthrough: true,
                stay_visible: false,
                hide_on_blur: true,
                appearance_source: "main".into(),
            },
            playback: PlaybackWindowSettingsDto {
                x: None,
                y: None,
                opacity: 100,
                bg_color: "#11111b".into(),
                appearance_source: "main".into(),
            },
        };

        let audio = AudioSettingsDto {
            output_format: crate::config::AudioOutputFormat::Default,
            speaker_device: None,
            speaker_enabled: false,
            speaker_volume: 0,
            virtual_mic_device: None,
            virtual_mic_volume: 0,
        };

        let audio_effects = AudioEffectsSettingsDto {
            enabled: false,
            pitch: 0,
            speed: 0,
            volume: 0,
            enhance_enabled: false,
            enhance_atten_db: 0.0,
            formant_preserved: false,
            boundary_cleanup_enabled: false,
        };

        let dsp = DspSettingsDto {
            eq: DspEqSettingsDto {
                enabled: false,
                low_cut_enabled: false,
                low_cut_hz: 0.0,
                low_cut_slope_db: 0.0,
                bands: vec![],
                high_shelf_enabled: false,
                high_shelf_hz: 0.0,
                high_shelf_gain_db: 0.0,
            },
            compressor: DspCompressorSettingsDto {
                enabled: false,
                threshold_db: 0.0,
                ratio: 0.0,
                attack_ms: 0.0,
                release_ms: 0.0,
                knee_db: 0.0,
                makeup_db: 0.0,
            },
            limiter: DspLimiterSettingsDto {
                enabled: false,
                ceiling_db: 0.0,
                release_ms: 0.0,
            },
        };

        let general = GeneralSettingsDto {
            hotkey_enabled: false,
            theme: None,
            ui_language: "ru".into(),
            ui_font_family: "default".into(),
            ui_font_size_px: 16,
            show_playback_on_start: false,
            start_compact: false,
            hide_on_minimize: false,
        };

        let editor = EditorSettingsDto {
            quick: "disabled".into(),
            ai: false,
            ai_completion: false,
            autocomplete_enabled: false,
            spellcheck_enabled: false,
            spellcheck_source: SpellSourceDto::Offline,
            editor_height: 200,
            typing_idle_timeout_ms: 500,
            typing_enabled: false,
            default_route: "everywhere".into(),
            keep_text_after_send: false,
            font_family: "system".into(),
            font_size_px: 16,
            homograph_accentor: HomographAccentorSettingsDto {
                enabled: false,
                accentor_pack_id: None,
                load_on_start: false,
            },
        };

        let ocr = OcrSettingsDto {
            enabled: false,
            model_id: None,
            capture_target: crate::ocr::monitors::CaptureTarget::All,
        };

        let logging = LoggingSettingsDto {
            enabled: false,
            level: "warn".into(),
            module_levels: std::collections::HashMap::new(),
        };

        let preprocessor = PreprocessorSettingsDto {
            enabled: false,
            replacements_count: 0,
        };

        let ai = AiSettingsDto {
            provider: AiProviderTypeDto::OpenAi,
            openai: AiOpenAiSettingsDto {
                api_key: None,
                use_proxy: false,
                model: "gpt-4o-mini".into(),
            },
            zai: AiZAiSettingsDto {
                url: None,
                api_key: None,
                model: "glm-4.5".into(),
            },
            deepseek: AiDeepSeekSettingsDto {
                api_key: None,
                use_proxy: false,
                model: "deepseek-chat".into(),
            },
            custom: AiCustomSettingsDto {
                url: None,
                api_key: None,
                use_proxy: false,
                model: String::new(),
            },
            prompt: String::new(),
            timeout: 20,
        };

        let hotkeys = HotkeySettingsDto {
            main_window: HotkeyDto {
                modifiers: vec![],
                key: String::new(),
            },
            sound_panel: HotkeyDto {
                modifiers: vec![],
                key: String::new(),
            },
            playback_pause: HotkeyDto {
                modifiers: vec![],
                key: String::new(),
            },
            playback_stop: HotkeyDto {
                modifiers: vec![],
                key: String::new(),
            },
            playback_repeat: HotkeyDto {
                modifiers: vec![],
                key: String::new(),
            },
            playback_control_window: HotkeyDto {
                modifiers: vec![],
                key: String::new(),
            },
            return_previous_window: HotkeyDto {
                modifiers: vec![],
                key: String::new(),
            },
            toggle_minimal_mode: HotkeyDto {
                modifiers: vec![],
                key: String::new(),
            },
            ocr_capture: HotkeyDto {
                modifiers: vec![],
                key: String::new(),
            },
            editor: EditorHotkeySettingsDto {
                edit_word: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                submit_continue: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                submit_keep_text: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                submit_keep_focus: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                next_spelling_error: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                previous_spelling_error: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                next_tab: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                previous_tab: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                cycle_route: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                toggle_typing: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                cycle_quick_mode: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                toggle_history: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                accent_homographs: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                approve_next_incoming: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
                edit_next_incoming: HotkeyDto {
                    modifiers: vec![],
                    key: String::new(),
                },
            },
        };

        let vtube_studio = VTubeStudioSettingsDto {
            enabled: false,
            host: "127.0.0.1".to_string(),
            port: 8001,
            start_on_boot: false,
            typing_action: VTubeStudioTypingActionDto {
                output_mode: VTubeStudioTypingMode::Event,
                parameter_name: String::new(),
                start_hotkey_id: String::new(),
                stop_hotkey_id: String::new(),
                start_hotkey_name: String::new(),
                stop_hotkey_name: String::new(),
                item_file_name: String::new(),
                item_type: String::new(),
            },
        };

        AppSettingsDto {
            notifications: Vec::new(),
            startup_errors: Vec::new(),
            tts,
            webview,
            twitch,
            windows,
            audio,
            audio_effects,
            dsp,
            storage: StorageSettingsDto {
                data_dir: None,
                audio_cache_dir: None,
            },
            general,
            editor,
            ocr,
            logging,
            preprocessor,
            soundpanel_bindings: vec![SoundBinding {
                key: 'z',
                description: "Null-path sound".into(),
                filename: "z.wav".into(),
                original_path: None,
            }],
            ai,
            hotkeys,
            vtube_studio,
            vrchat: VrchatSettingsDto::default(),
        }
    }

    fn check_fixture_vs_serde(fixture_name: &str, dto: &AppSettingsDto) {
        let current = serde_json::to_string_pretty(dto).expect("serialize");
        let fixture_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../scripts/settings-fixtures");
        let full_path = format!("{}/{}.json", fixture_path, fixture_name);
        let fixture = std::fs::read_to_string(&full_path).unwrap_or_else(|_| {
            panic!(
                "Fixture '{}.json' missing — regenerate with `cargo test app_settings_dto_fixtures_regenerate -- --ignored`",
                fixture_name
            )
        });
        assert_eq!(
            current, fixture,
            "Fixture '{}.json' is stale — regenerate with `cargo test app_settings_dto_fixtures_regenerate -- --ignored`",
            fixture_name
        );
    }

    #[test]
    fn app_settings_dto_fixture_populated_is_current() {
        let dto = build_populated();
        check_fixture_vs_serde("populated", &dto);
    }

    #[test]
    fn app_settings_dto_fixture_omit_null_is_current() {
        let dto = build_omitted_or_null();
        check_fixture_vs_serde("omit-null", &dto);
    }

    #[test]
    fn app_settings_dto_round_trip() {
        let dto = build_populated();
        let json1 = serde_json::to_string_pretty(&dto).expect("first serialize");
        let deserialized: AppSettingsDto = serde_json::from_str(&json1).expect("deserialize");
        let json2 = serde_json::to_string_pretty(&deserialized).expect("second serialize");
        assert_eq!(json1, json2, "Round-trip produced different JSON");
    }

    /// OCR is exposed as exactly `{"enabled":..., "model_id":...,
    /// "capture_target":...}` — the persisted desired state only, with no
    /// runtime status, capture paths, engine internals or hotkeys leaking into
    /// the wire shape.
    #[test]
    fn app_settings_dto_ocr_wire_shape_is_desired_state_only() {
        let dto = build_populated();
        let json = serde_json::to_value(&dto).expect("serialize");
        let ocr = json
            .get("ocr")
            .unwrap_or_else(|| panic!("AppSettingsDto must serialize an `ocr` object"));
        let obj = ocr
            .as_object()
            .unwrap_or_else(|| panic!("`ocr` must serialize as a JSON object"));
        let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["capture_target", "enabled", "model_id"]);
        assert_eq!(obj["enabled"], serde_json::json!(true));
        assert_eq!(obj["model_id"], serde_json::json!("silero-ocr"));
        assert_eq!(
            obj["capture_target"],
            serde_json::json!({ "type": "primary" })
        );

        let compact = serde_json::to_string(&dto).expect("serialize compact");
        assert!(
            compact.contains(
                r#""ocr":{"enabled":true,"model_id":"silero-ocr","capture_target":{"type":"primary"}}"#
            ),
            "unexpected compact OCR wire shape: {compact}"
        );
    }

    /// Backward-compat: a TtsSettingsDto without `visible_provider_ids`
    /// deserializes to an empty list (the field is populated from TtsSettings
    /// on output, not persisted independently).
    #[test]
    fn tts_settings_dto_deserializes_without_visible_provider_ids() {
        let old_json = r#"{
            "provider": "openai",
            "openai": { "api_key": null, "voice": "alloy", "proxy_host": null, "proxy_port": null, "use_proxy": false },
            "local": { "url": "http://127.0.0.1:8124" },
            "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" },
            "network": { "proxy": { "proxy_url": null }, "mtproxy": { "host": null, "port": 8888, "secret": null, "dc_id": null } }
        }"#;
        let dto: TtsSettingsDto = serde_json::from_str(old_json)
            .expect("old TtsSettingsDto without visible_provider_ids must deserialize");
        assert!(dto.visible_provider_ids.is_empty());
    }

    /// Fresh TtsSettings converts to a DTO exposing exactly `["silero"]`.
    #[test]
    fn tts_settings_to_dto_default_visible_provider_ids_is_silero() {
        let dto: TtsSettingsDto = TtsSettings::default().into();
        assert_eq!(dto.visible_provider_ids, vec!["silero".to_string()]);
    }

    /// Round-trip TtsSettings -> DTO -> TtsSettings preserves visible_provider_ids.
    #[test]
    fn tts_settings_dto_visible_provider_ids_round_trip() {
        let settings = TtsSettings {
            visible_provider_ids: vec!["silero".to_string(), "openai".to_string()],
            ..Default::default()
        };
        let dto: TtsSettingsDto = settings.clone().into();
        assert_eq!(
            dto.visible_provider_ids,
            vec!["silero".to_string(), "openai".to_string()]
        );
        let back: TtsSettings = dto.into();
        assert_eq!(back.visible_provider_ids, settings.visible_provider_ids);
    }

    /// `ElevenLabsSettingsDto::default()` mirrors `ElevenLabsSettings::default()`
    /// so code paths that construct a DTO without explicit values stay valid.
    #[test]
    fn elevenlabs_settings_dto_default_matches_domain_defaults() {
        let dto = ElevenLabsSettingsDto::default();
        assert!(dto.model_id.is_empty());
        assert!(dto.models.is_empty());
        assert_eq!(dto.output_format, "mp3_44100_128");
        assert_eq!(dto.stability, 0.5);
        assert_eq!(dto.similarity_boost, 0.75);
        assert_eq!(dto.style, 0.0);
        assert!(dto.use_speaker_boost);
        assert!(!dto.use_proxy);
        assert_eq!(ElevenLabsSettings::from(dto), ElevenLabsSettings::default());
    }

    /// Backward-compat: an `ElevenLabsVoiceDto` JSON without the classification
    /// field still deserializes, yielding an unmarked (None) voice.
    #[test]
    fn elevenlabs_voice_dto_deserializes_without_classification_field() {
        let json =
            r#"{"voice_id":"v1","name":"Voice","category":null,"labels":[],"preview_url":null}"#;
        let dto: ElevenLabsVoiceDto =
            serde_json::from_str(json).expect("voice DTO without classification must deserialize");
        assert!(dto.classification.is_none());

        let voice: ElevenLabsVoice = dto.into();
        assert!(voice.classification.is_none());
    }

    #[test]
    fn elevenlabs_voice_dto_round_trips_each_classification() {
        for (raw, expected) in [
            ("default", Some(ElevenLabsVoiceClassification::Default)),
            ("library", Some(ElevenLabsVoiceClassification::Library)),
        ] {
            let json = format!(
                r#"{{"voice_id":"v1","name":"Voice","category":null,"labels":[],"preview_url":null,"classification":{}}}"#,
                serde_json::to_string(raw).unwrap()
            );
            let dto: ElevenLabsVoiceDto = serde_json::from_str(&json).unwrap();
            assert_eq!(dto.classification, expected);
            let back = serde_json::to_value(&dto).unwrap();
            assert_eq!(back["classification"], serde_json::json!(raw));
        }

        let dto = ElevenLabsVoiceDto {
            voice_id: "v1".into(),
            name: "Voice".into(),
            category: None,
            labels: vec![],
            preview_url: None,
            classification: None,
        };
        let value = serde_json::to_value(&dto).unwrap();
        assert!(value.get("classification").is_none());
        let back: ElevenLabsVoiceDto = serde_json::from_value(value).unwrap();
        assert!(back.classification.is_none());
    }

    /// Legacy/imported state: a TtsSettingsDto JSON without an `elevenlabs`
    /// section deserializes to provider defaults, and converting to the domain
    /// `TtsSettings` keeps them valid.
    #[test]
    fn tts_settings_dto_without_elevenlabs_yields_provider_defaults() {
        let json = r#"{
            "provider": "openai",
            "openai": { "api_key": null, "voice": "alloy", "proxy_host": null, "proxy_port": null, "use_proxy": false },
            "local": { "url": "http://127.0.0.1:8124" },
            "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" },
            "network": { "proxy": { "proxy_url": null }, "mtproxy": { "host": null, "port": 8888, "secret": null, "dc_id": null } }
        }"#;
        let dto: TtsSettingsDto =
            serde_json::from_str(json).expect("TtsSettingsDto without elevenlabs must deserialize");
        assert_eq!(
            ElevenLabsSettings::from(dto.elevenlabs.clone()),
            ElevenLabsSettings::default()
        );

        let settings: TtsSettings = dto.into();
        assert_eq!(settings.elevenlabs, ElevenLabsSettings::default());
    }

    /// A present-but-empty `elevenlabs` object also yields valid provider
    /// defaults through the DTO field-level serde defaults.
    #[test]
    fn elevenlabs_settings_dto_empty_object_yields_provider_defaults() {
        let dto: ElevenLabsSettingsDto =
            serde_json::from_str("{}").expect("empty elevenlabs DTO must deserialize");
        assert_eq!(ElevenLabsSettings::from(dto), ElevenLabsSettings::default());
    }

    /// Regenerates all fixture files. Excluded from standard test runs.
    #[test]
    #[ignore]
    fn app_settings_dto_fixtures_regenerate() {
        let fixture_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../scripts/settings-fixtures");

        let populated = build_populated();
        let populated_json = serde_json::to_string_pretty(&populated).expect("serialize populated");
        std::fs::write(format!("{}/populated.json", fixture_dir), &populated_json)
            .expect("write populated fixture");
        eprintln!("Fixture written: {}/populated.json", fixture_dir);

        let omit_null = build_omitted_or_null();
        let omit_null_json = serde_json::to_string_pretty(&omit_null).expect("serialize omit-null");
        std::fs::write(format!("{}/omit-null.json", fixture_dir), &omit_null_json)
            .expect("write omit-null fixture");
        eprintln!("Fixture written: {}/omit-null.json", fixture_dir);
    }
}
