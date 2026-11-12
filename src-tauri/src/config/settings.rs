//! Application settings configuration
//!
//! Manages all application settings stored in settings.json

use anyhow::{Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::persistence;
use super::recovery::{self, SettingsFailureStage, SettingsLoadFailure};

use super::hotkeys::HotkeySettings;
use super::validation::{validate_port, validate_volume};
use crate::input_server::{IncomingRoute, IncomingSettings, InputServerSettings};
use crate::ocr::settings::OcrSettings;
use crate::tts::elevenlabs::{ElevenLabsModel, ElevenLabsVoice};
use crate::tts::TtsProviderType;
use tracing::{info, warn};

// ==================== Audio Settings ====================

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AudioOutputFormat {
    #[default]
    Default,
    I32,
}

/// Audio output settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioSettings {
    #[serde(default)]
    pub output_format: AudioOutputFormat,
    pub speaker_device: Option<String>,
    #[serde(default = "default_speaker_enabled")]
    pub speaker_enabled: bool,
    #[serde(default = "default_speaker_volume")]
    pub speaker_volume: u8,
    pub virtual_mic_device: Option<String>,
    #[serde(default = "default_virtual_mic_volume")]
    pub virtual_mic_volume: u8,
}

fn default_speaker_enabled() -> bool {
    true
}
fn default_speaker_volume() -> u8 {
    80
}
fn default_virtual_mic_volume() -> u8 {
    100
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            output_format: AudioOutputFormat::Default,
            speaker_device: None,
            speaker_enabled: true,
            speaker_volume: 80,
            virtual_mic_device: None,
            virtual_mic_volume: 100,
        }
    }
}

// ==================== Audio Effects Settings ====================

/// Audio post-processing effects settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioEffectsSettings {
    #[serde(default = "default_effects_enabled")]
    pub enabled: bool,
    #[serde(default = "default_pitch")]
    pub pitch: i16, // -100 to +100 (percent → -12..+12 semitones)
    #[serde(default = "default_speed")]
    pub speed: i16, // -100 to +100 (percent → 0.75..1.50 tempo factor)
    #[serde(default = "default_volume")]
    pub volume: i16, // 0 to 200 (percent, 100 = normal)
    /// Включить очистку речи от шума (DeepFilterNet)
    #[serde(default = "default_enhance_enabled")]
    pub enhance_enabled: bool,
    /// Глубина очистки (attenuation limit) в dB: 5..30.
    /// Меньше — мягче, больше — сильнее подавление шума.
    #[serde(default = "default_enhance_atten_db")]
    pub enhance_atten_db: f32,
    /// Сохранять тембр голоса при изменении высоты (Signalsmith formant correction).
    /// По умолчанию включено. Не зависит от DeepFilterNet.
    #[serde(default = "default_formant_preserved")]
    pub formant_preserved: bool,
    /// Включить per-phrase boundary cleanup (DC offset removal + fade-in/out).
    /// По умолчанию включено. Не зависит от DeepFilterNet и DSP.
    #[serde(default = "default_boundary_cleanup_enabled")]
    pub boundary_cleanup_enabled: bool,
}

fn default_effects_enabled() -> bool {
    false
}
fn default_pitch() -> i16 {
    0
}
fn default_speed() -> i16 {
    0
}
fn default_volume() -> i16 {
    100
}
fn default_enhance_enabled() -> bool {
    false
}
fn default_enhance_atten_db() -> f32 {
    12.0
}
fn default_formant_preserved() -> bool {
    true
}
fn default_boundary_cleanup_enabled() -> bool {
    true
}

impl Default for AudioEffectsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            pitch: 0,
            speed: 0,
            volume: 100,
            enhance_enabled: false,
            enhance_atten_db: 12.0,
            formant_preserved: true,
            boundary_cleanup_enabled: true,
        }
    }
}

// ==================== DSP Post-Processing Settings ====================

/// DSP post-processing EQ band
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DspEqBandSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_band_freq")]
    pub frequency_hz: f32,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default = "default_band_q")]
    pub q: f32,
}

fn default_band_freq() -> f32 {
    2500.0
}
fn default_band_q() -> f32 {
    0.7
}

impl Default for DspEqBandSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            frequency_hz: 2500.0,
            gain_db: 0.0,
            q: 0.7,
        }
    }
}

/// DSP post-processing EQ settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DspEqSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub low_cut_enabled: bool,
    #[serde(default = "default_low_cut_hz")]
    pub low_cut_hz: f32,
    #[serde(default = "default_low_cut_slope")]
    pub low_cut_slope_db: f32,
    #[serde(default = "default_dsp_bands")]
    pub bands: [DspEqBandSettings; 3],
    #[serde(default)]
    pub high_shelf_enabled: bool,
    #[serde(default = "default_high_shelf_hz")]
    pub high_shelf_hz: f32,
    #[serde(default)]
    pub high_shelf_gain_db: f32,
}

fn default_low_cut_hz() -> f32 {
    80.0
}
fn default_low_cut_slope() -> f32 {
    12.0
}
fn default_dsp_bands() -> [DspEqBandSettings; 3] {
    [
        DspEqBandSettings::default(),
        DspEqBandSettings::default(),
        DspEqBandSettings::default(),
    ]
}
fn default_high_shelf_hz() -> f32 {
    8000.0
}

impl Default for DspEqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            low_cut_enabled: false,
            low_cut_hz: 80.0,
            low_cut_slope_db: 12.0,
            bands: default_dsp_bands(),
            high_shelf_enabled: false,
            high_shelf_hz: 8000.0,
            high_shelf_gain_db: 0.0,
        }
    }
}

/// DSP post-processing compressor settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DspCompressorSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_comp_threshold")]
    pub threshold_db: f32,
    #[serde(default = "default_comp_ratio")]
    pub ratio: f32,
    #[serde(default = "default_comp_attack")]
    pub attack_ms: f32,
    #[serde(default = "default_comp_release")]
    pub release_ms: f32,
    #[serde(default = "default_comp_knee")]
    pub knee_db: f32,
    #[serde(default)]
    pub makeup_db: f32,
}

fn default_comp_threshold() -> f32 {
    -18.0
}
fn default_comp_ratio() -> f32 {
    2.0
}
fn default_comp_attack() -> f32 {
    8.0
}
fn default_comp_release() -> f32 {
    120.0
}
fn default_comp_knee() -> f32 {
    6.0
}

impl Default for DspCompressorSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_db: -18.0,
            ratio: 2.0,
            attack_ms: 8.0,
            release_ms: 120.0,
            knee_db: 6.0,
            makeup_db: 0.0,
        }
    }
}

/// DSP post-processing limiter settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DspLimiterSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_lim_ceiling")]
    pub ceiling_db: f32,
    #[serde(default = "default_lim_release")]
    pub release_ms: f32,
}

fn default_lim_ceiling() -> f32 {
    -1.0
}
fn default_lim_release() -> f32 {
    50.0
}

impl Default for DspLimiterSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            ceiling_db: -1.0,
            release_ms: 50.0,
        }
    }
}

/// DSP post-processing settings
///
/// Contains independent `enabled` flags for EQ, compressor, and limiter.
/// Each block can be bypassed individually at runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DspSettings {
    #[serde(default)]
    pub eq: DspEqSettings,
    #[serde(default)]
    pub compressor: DspCompressorSettings,
    #[serde(default)]
    pub limiter: DspLimiterSettings,
}

impl DspSettings {
    /// Convert to runtime DSP configuration with validation/clamping.
    pub fn to_dsp_config(&self) -> crate::audio::DspConfig {
        use crate::audio::{CompressorConfig, DspConfig, EqBand, EqConfig, LimiterConfig};

        let bands: [EqBand; 3] = std::array::from_fn(|i| {
            let b = &self.eq.bands[i];
            EqBand {
                enabled: b.enabled,
                frequency_hz: b.frequency_hz.clamp(20.0, 20000.0),
                gain_db: b.gain_db.clamp(-24.0, 24.0),
                q: b.q.clamp(0.1, 10.0),
            }
        });

        DspConfig {
            eq: EqConfig {
                enabled: self.eq.enabled,
                low_cut_enabled: self.eq.low_cut_enabled,
                low_cut_hz: self.eq.low_cut_hz.clamp(10.0, 500.0),
                low_cut_slope_db: self.eq.low_cut_slope_db.clamp(6.0, 48.0),
                bands,
                high_shelf_enabled: self.eq.high_shelf_enabled,
                high_shelf_hz: self.eq.high_shelf_hz.clamp(1000.0, 20000.0),
                high_shelf_gain_db: self.eq.high_shelf_gain_db.clamp(-24.0, 24.0),
            },
            compressor: CompressorConfig {
                enabled: self.compressor.enabled,
                threshold_db: self.compressor.threshold_db.clamp(-60.0, 0.0),
                ratio: self.compressor.ratio.clamp(1.0, 20.0),
                attack_ms: self.compressor.attack_ms.clamp(0.1, 500.0),
                release_ms: self.compressor.release_ms.clamp(1.0, 2000.0),
                knee_db: self.compressor.knee_db.clamp(0.0, 20.0),
                makeup_db: self.compressor.makeup_db.clamp(-12.0, 24.0),
            },
            limiter: LimiterConfig {
                enabled: self.limiter.enabled,
                ceiling_db: self.limiter.ceiling_db.clamp(-12.0, 0.0),
                release_ms: self.limiter.release_ms.clamp(1.0, 500.0),
            },
        }
    }
}

// ==================== TTS Settings ====================

/// TTS provider settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TtsSettings {
    #[serde(default)]
    pub provider: TtsProviderType,
    pub openai: OpenAiSettings,
    pub local: LocalTtsSettings,
    #[serde(default)]
    pub fish: FishAudioSettings,
    #[serde(default)]
    pub elevenlabs: ElevenLabsSettings,
    pub telegram: TelegramTtsSettings,
    #[serde(default)]
    pub network: NetworkSettings,
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Presentation-only list of provider IDs the frontend shows. Owned by the
    /// frontend (runtime-derived); the backend never validates or mutates it.
    #[serde(default = "default_visible_provider_ids")]
    pub visible_provider_ids: Vec<String>,
}

fn default_visible_provider_ids() -> Vec<String> {
    vec!["silero".to_string()]
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            provider: TtsProviderType::OpenAi,
            openai: OpenAiSettings::default(),
            local: LocalTtsSettings::default(),
            fish: FishAudioSettings::default(),
            elevenlabs: ElevenLabsSettings::default(),
            telegram: TelegramTtsSettings::default(),
            network: NetworkSettings::default(),
            provider_id: None,
            visible_provider_ids: default_visible_provider_ids(),
        }
    }
}

/// Local TTS server settings
///
/// Compatible with TTSVoiceWizard "Locally Hosted" mode.
/// Default URL matches TITTS.py server endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalTtsSettings {
    #[serde(default = "default_local_tts_url")]
    pub url: String,
}

fn default_local_tts_url() -> String {
    "http://127.0.0.1:8124".to_string()
}

impl Default for LocalTtsSettings {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:8124".to_string(),
        }
    }
}

/// Telegram TTS settings (for Silero)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TelegramTtsSettings {
    pub api_id: Option<i64>,
    #[serde(default)]
    pub proxy_mode: ProxyMode,
    /// Список сохраненных кодов голосов
    #[serde(default)]
    pub voices: Vec<crate::telegram::types::VoiceCode>,
    /// Текущий выбранный ID голоса
    #[serde(default)]
    pub current_voice_id: String,
    /// Ожидание связанного audio/text reply после отправки текста (ms)
    #[serde(default = "default_synthesis_response_timeout_ms")]
    pub synthesis_response_timeout_ms: u32,
    /// Пауза после пустого скачанного файла перед следующей попыткой (ms)
    #[serde(default = "default_download_retry_delay_ms")]
    pub download_retry_delay_ms: u32,
}

fn default_synthesis_response_timeout_ms() -> u32 {
    60000
}
fn default_download_retry_delay_ms() -> u32 {
    1000
}

pub const SYNTHESIS_RESPONSE_TIMEOUT_MIN_MS: u32 = 1000;
// SYNTHESIS_RESPONSE_TIMEOUT is unbounded above per ROADMAP-133
pub const DOWNLOAD_RETRY_DELAY_MIN_MS: u32 = 100;
pub const DOWNLOAD_RETRY_DELAY_MAX_MS: u32 = 10_000;

impl Default for TelegramTtsSettings {
    fn default() -> Self {
        Self {
            api_id: None,
            proxy_mode: ProxyMode::None,
            voices: Vec::new(),
            current_voice_id: String::new(),
            synthesis_response_timeout_ms: default_synthesis_response_timeout_ms(),
            download_retry_delay_ms: default_download_retry_delay_ms(),
        }
    }
}

/// Proxy mode for Telegram connection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    #[default]
    None,
    Socks5,
    MtProxy,
}

impl ProxyMode {
    /// Detect proxy mode from URL scheme
    ///
    /// Returns the appropriate ProxyMode based on the URL prefix.
    /// Unknown URLs return None.
    pub fn from_url(url: &str) -> Self {
        let url_lower = url.to_lowercase();
        if url_lower.starts_with("socks5://") || url_lower.starts_with("socks5h://") {
            ProxyMode::Socks5
        } else if url_lower.starts_with("mtproxy://") || url_lower.starts_with("mtproto://") {
            ProxyMode::MtProxy
        } else {
            ProxyMode::None
        }
    }
}

// ==================== Network Settings ====================

/// SOCKS5 proxy settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Socks5Settings {
    /// SOCKS5 proxy URL (socks5://user:pass@host:port)
    pub proxy_url: Option<String>,
}

/// MTProxy settings for Telegram
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MtProxySettings {
    /// MTProxy server host (IP or domain)
    pub host: Option<String>,
    /// MTProxy server port
    #[serde(default = "default_mtproxy_port")]
    pub port: u16,
    /// MTProxy secret key (hex or base64 encoded)
    pub secret: Option<String>,
    /// Optional DC ID (data center ID)
    pub dc_id: Option<i32>,
}

fn default_mtproxy_port() -> u16 {
    8888
}

impl Default for MtProxySettings {
    fn default() -> Self {
        Self {
            host: None,
            port: 8888,
            secret: None,
            dc_id: None,
        }
    }
}

/// Unified network settings containing all proxy configurations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NetworkSettings {
    /// SOCKS5 proxy settings
    #[serde(default)]
    pub proxy: Socks5Settings,
    /// MTProxy settings
    #[serde(default)]
    pub mtproxy: MtProxySettings,
}

// ==================== Legacy Proxy Settings (for migration) ====================

/// Legacy proxy settings (deprecated, use NetworkSettings instead)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxySettings {
    /// Unified proxy URL (socks5://, socks4://, http://user:pass@host:port)
    pub proxy_url: Option<String>,
    /// Proxy type for UI selection
    #[serde(default)]
    pub proxy_type: ProxyType,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            proxy_url: None,
            proxy_type: ProxyType::Socks5,
        }
    }
}

/// Proxy type enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProxyType {
    #[default]
    Socks5,
    Socks4,
    Http,
}

/// OpenAI TTS settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAiSettings {
    pub api_key: Option<String>,
    #[serde(default = "default_openai_voice")]
    pub voice: String,
    /// Legacy proxy host (for backward compatibility)
    #[serde(default)]
    pub proxy_host: Option<String>,
    /// Legacy proxy port (for backward compatibility)
    #[serde(default)]
    pub proxy_port: Option<u16>,
    /// Use unified proxy from global proxy settings
    #[serde(default)]
    pub use_proxy: bool,
}

fn default_openai_voice() -> String {
    "alloy".to_string()
}

impl Default for OpenAiSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            voice: "alloy".to_string(),
            proxy_host: None,
            proxy_port: None,
            use_proxy: false,
        }
    }
}

/// Настройки Fish Audio TTS
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FishAudioSettings {
    pub api_key: Option<String>,
    /// Список сохранённых голосовых моделей с метаданными
    #[serde(default)]
    pub voices: Vec<crate::tts::VoiceModel>,
    /// Текущий выбранный ID голосовой модели
    #[serde(default)]
    pub reference_id: String,
    /// Формат аудио (mp3, wav, pcm, opus)
    #[serde(default = "default_fish_format")]
    pub format: String,
    /// Температура (0.0-1.0)
    #[serde(default = "default_fish_temperature")]
    pub temperature: f32,
    /// Частота дискретизации (Гц)
    #[serde(default = "default_fish_sample_rate")]
    pub sample_rate: u32,
    /// Использовать унифицированный прокси из глобальных настроек
    #[serde(default)]
    pub use_proxy: bool,
}

fn default_fish_format() -> String {
    "mp3".to_string()
}
fn default_fish_temperature() -> f32 {
    0.7
}
fn default_fish_sample_rate() -> u32 {
    44100
}

impl Default for FishAudioSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            voices: Vec::new(),
            reference_id: String::new(),
            format: "mp3".to_string(),
            temperature: 0.7,
            sample_rate: 44100,
            use_proxy: false,
        }
    }
}

// ==================== ElevenLabs Settings ====================

/// Настройки ElevenLabs TTS
///
/// `#[serde(default)]` на структуре гарантирует безопасную миграцию старого
/// `settings.json` без секции ElevenLabs. Каталоги голосов и моделей хранятся
/// отдельно в `elevenlabs-catalog.json` и здесь не присутствуют.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElevenLabsSettings {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub voice_id: String,
    #[serde(default = "default_elevenlabs_model_id")]
    pub model_id: String,
    #[serde(default = "default_elevenlabs_output_format")]
    pub output_format: String,
    /// Параметры генерации в диапазоне 0.0..=1.0.
    #[serde(default = "default_elevenlabs_stability")]
    pub stability: f32,
    #[serde(default = "default_elevenlabs_similarity_boost")]
    pub similarity_boost: f32,
    #[serde(default = "default_elevenlabs_style")]
    pub style: f32,
    #[serde(default = "default_elevenlabs_use_speaker_boost")]
    pub use_speaker_boost: bool,
    #[serde(default)]
    pub use_proxy: bool,
}

fn default_elevenlabs_model_id() -> String {
    String::new()
}

fn default_elevenlabs_output_format() -> String {
    crate::tts::elevenlabs::DEFAULT_OUTPUT_FORMAT.to_string()
}

fn default_elevenlabs_stability() -> f32 {
    crate::tts::elevenlabs::DEFAULT_STABILITY
}

fn default_elevenlabs_similarity_boost() -> f32 {
    crate::tts::elevenlabs::DEFAULT_SIMILARITY_BOOST
}

fn default_elevenlabs_style() -> f32 {
    crate::tts::elevenlabs::DEFAULT_STYLE
}

fn default_elevenlabs_use_speaker_boost() -> bool {
    crate::tts::elevenlabs::DEFAULT_USE_SPEAKER_BOOST
}

impl Default for ElevenLabsSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            voice_id: String::new(),
            model_id: default_elevenlabs_model_id(),
            output_format: default_elevenlabs_output_format(),
            stability: default_elevenlabs_stability(),
            similarity_boost: default_elevenlabs_similarity_boost(),
            style: default_elevenlabs_style(),
            use_speaker_boost: default_elevenlabs_use_speaker_boost(),
            use_proxy: false,
        }
    }
}

/// Current on-disk format version of the ElevenLabs catalog cache.
///
/// Bump only when the serialized shape of [`ElevenLabsCatalogCache`] changes in
/// a way old data cannot be migrated in place. Unsupported versions on disk are
/// ignored and replaced with an empty current-version cache.
const ELEVENLABS_CATALOG_CURRENT_VERSION: u32 = 1;

/// Versioned on-disk cache for the ElevenLabs voice and model catalogs.
///
/// Stored at `<config_dir>/elevenlabs-catalog.json` and kept separate from
/// `settings.json`: the catalogs are account data that is refreshed independently
/// and must never be persisted into the main settings file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ElevenLabsCatalogCache {
    #[serde(default = "default_elevenlabs_catalog_version")]
    pub version: u32,
    #[serde(default)]
    pub voices: Vec<ElevenLabsVoice>,
    #[serde(default)]
    pub models: Vec<ElevenLabsModel>,
}

/// Missing `version` in an old cache file means it predates versioning, which
/// started at the current version.
fn default_elevenlabs_catalog_version() -> u32 {
    ELEVENLABS_CATALOG_CURRENT_VERSION
}

impl Default for ElevenLabsCatalogCache {
    fn default() -> Self {
        Self {
            version: ELEVENLABS_CATALOG_CURRENT_VERSION,
            voices: Vec::new(),
            models: Vec::new(),
        }
    }
}

impl ElevenLabsCatalogCache {
    /// Parse the catalog file, safely returning an empty current-version cache
    /// when the file is missing, corrupt, or carries an unsupported format
    /// version so application startup is never blocked.
    fn load_from_disk(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(e) => {
                warn!(error = %e, "elevenlabs-catalog.json unreadable, using empty catalog");
                return Self::default();
            }
        };
        match serde_json::from_str::<Self>(&content) {
            Ok(cache) if cache.version == ELEVENLABS_CATALOG_CURRENT_VERSION => cache,
            Ok(cache) => {
                warn!(
                    version = cache.version,
                    current = ELEVENLABS_CATALOG_CURRENT_VERSION,
                    "elevenlabs-catalog.json has unsupported version, using empty catalog"
                );
                Self::default()
            }
            Err(e) => {
                warn!(error = %e, "elevenlabs-catalog.json corrupt, using empty catalog");
                Self::default()
            }
        }
    }
}

// ==================== Twitch Settings ====================

/// Twitch connection transport mode (ROADMAP-132)
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TwitchMode {
    #[default]
    Irc,
    Api,
}

/// Twitch chat integration settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TwitchSettings {
    #[serde(default)]
    pub mode: TwitchMode,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub start_on_boot: bool,
    /// ROADMAP-107: send the user's original text to Twitch instead of the
    /// TTS-processed representation. Missing field in legacy settings
    /// deserializes to `true`.
    #[serde(default = "default_send_original_text")]
    pub send_original_text: bool,
}

fn default_send_original_text() -> bool {
    true
}

impl Default for TwitchSettings {
    fn default() -> Self {
        Self {
            mode: TwitchMode::default(),
            enabled: false,
            username: String::new(),
            token: String::new(),
            channel: String::new(),
            start_on_boot: false,
            send_original_text: true,
        }
    }
}

impl TwitchSettings {
    /// Check if settings are valid
    pub fn is_valid(&self) -> Result<(), String> {
        match self.mode {
            TwitchMode::Irc => {
                if twitch_irc::validate::validate_login(&self.username.to_ascii_lowercase())
                    .is_err()
                {
                    return Err("twitch.invalid_username".to_string());
                }
                if self.token.trim().is_empty() {
                    return Err("twitch.missing_token".to_string());
                }
                if twitch_irc::validate::validate_login(&self.channel.to_ascii_lowercase()).is_err()
                {
                    return Err("twitch.invalid_channel".to_string());
                }
                Ok(())
            }
            TwitchMode::Api => Ok(()),
        }
    }
}

// ==================== VTube Studio Settings ====================

fn default_vtube_port() -> u16 {
    8001
}

fn default_vtube_host() -> String {
    "127.0.0.1".to_string()
}

/// Проверяет адрес сервера VTube Studio Plugin API: IPv4 или имя компьютера
/// (включая mDNS-суффикс `.local`) без схемы, пути и порта. Порт задаётся
/// отдельным полем. IPv6 не поддерживается: `:` зарезервирован разделителем
/// порта в `ws://host:port`.
pub fn validate_vtube_host(host: &str) -> Result<(), String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("Host must not be empty".to_string());
    }
    if host.len() > 253 {
        return Err("Host is too long (max 253 characters)".to_string());
    }
    if host.contains("://")
        || host.contains('/')
        || host.contains('\\')
        || host.contains(':')
        || host.contains(char::is_whitespace)
    {
        return Err(format!(
            "Invalid host '{}': use an IP address or computer name without scheme, path, or port",
            host
        ));
    }
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return Ok(());
    }
    let labels: Vec<_> = host.split('.').collect();
    if labels.len() == 4
        && labels.iter().all(|label| {
            !label.is_empty() && label.len() <= 3 && label.bytes().all(|c| c.is_ascii_digit())
        })
    {
        return Err("Invalid IPv4 address".to_string());
    }
    let invalid_label = |label: &str| {
        label.is_empty()
            || label.len() > 63
            || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            || label.starts_with('-')
            || label.ends_with('-')
    };
    if host.split('.').any(invalid_label) {
        return Err(format!(
            "Invalid host '{}': use an IPv4 address or a computer name (letters, digits, hyphens, dots)",
            host
        ));
    }
    Ok(())
}

/// VTube Studio typing output mode
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VTubeStudioTypingMode {
    Event,
    Hotkeys,
    Item,
}

/// VTube Studio typing action configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VTubeStudioTypingAction {
    pub output_mode: VTubeStudioTypingMode,
    pub parameter_name: String,
    pub start_hotkey_id: String,
    pub stop_hotkey_id: String,
    #[serde(default)]
    pub start_hotkey_name: String,
    #[serde(default)]
    pub stop_hotkey_name: String,
    #[serde(default)]
    pub item_file_name: String,
    #[serde(default)]
    pub item_type: String,
}

impl Default for VTubeStudioTypingAction {
    fn default() -> Self {
        Self {
            output_mode: VTubeStudioTypingMode::Event,
            parameter_name: "TTSBardTyping".to_string(),
            start_hotkey_id: String::new(),
            stop_hotkey_id: String::new(),
            start_hotkey_name: String::new(),
            stop_hotkey_name: String::new(),
            item_file_name: String::new(),
            item_type: String::new(),
        }
    }
}

/// VTube Studio integration settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VTubeStudioSettings {
    #[serde(default)]
    pub enabled: bool,
    /// Адрес сервера Plugin API: IP или имя компьютера. Старые настройки без
    /// этого поля загружаются как `127.0.0.1` (ROADMAP-120).
    #[serde(default = "default_vtube_host")]
    pub host: String,
    #[serde(default = "default_vtube_port")]
    pub port: u16,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub start_on_boot: bool,
    #[serde(default)]
    pub typing_action: VTubeStudioTypingAction,
}

impl Default for VTubeStudioSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_vtube_host(),
            port: default_vtube_port(),
            token: None,
            start_on_boot: false,
            typing_action: VTubeStudioTypingAction::default(),
        }
    }
}

// ==================== VRChat Settings ====================

pub fn default_vrchat_host() -> String {
    "127.0.0.1".to_string()
}

pub fn default_vrchat_port() -> u16 {
    9000
}

/// VRChat OSC chatbox output settings (ROADMAP-130)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VrchatSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub start_on_boot: bool,
    #[serde(default = "default_vrchat_host")]
    pub host: String,
    #[serde(default = "default_vrchat_port")]
    pub port: u16,
}

impl Default for VrchatSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            start_on_boot: false,
            host: default_vrchat_host(),
            port: default_vrchat_port(),
        }
    }
}

impl VrchatSettings {
    /// At launch, the explicit autostart preference controls runtime output.
    pub fn runtime_at_startup(&self) -> Self {
        let mut runtime = self.clone();
        runtime.enabled = runtime.start_on_boot;
        runtime
    }
}

/// Validates destination host for VRChat OSC.
pub fn validate_vrchat_host(host: &str) -> Result<(), String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("Host must not be empty".to_string());
    }
    if host.len() > 253 {
        return Err("Host is too long (max 253 characters)".to_string());
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    if host.contains("://")
        || host.contains('/')
        || host.contains('\\')
        || host.contains(':')
        || host.contains(char::is_whitespace)
    {
        return Err(format!(
            "Invalid host '{}': use an IP address or computer name without scheme, path, or port",
            host
        ));
    }
    let labels: Vec<_> = host.split('.').collect();
    if labels.len() == 4
        && labels.iter().all(|label| {
            !label.is_empty() && label.len() <= 3 && label.bytes().all(|c| c.is_ascii_digit())
        })
    {
        return Err("Invalid IPv4 address".to_string());
    }
    let invalid_label = |label: &str| {
        label.is_empty()
            || label.len() > 63
            || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            || label.starts_with('-')
            || label.ends_with('-')
    };
    if host.split('.').any(invalid_label) {
        return Err(format!(
            "Invalid host '{}': use an IP address or computer name (letters, digits, hyphens, dots)",
            host
        ));
    }
    Ok(())
}

// ==================== WebView Settings ====================
// WebView server settings are defined in webview module and re-exported here
use crate::webview::WebViewSettings;

// ==================== Logging Settings ====================

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoggingSettings {
    #[serde(default = "default_logging_enabled")]
    pub enabled: bool,
    #[serde(default = "default_logging_level")]
    pub level: String,
    /// Per-module log levels (только для редактирования в settings.json вручную)
    /// Пример: { "ttsbard::telegram": "debug", "ttsbard::webview": "trace" }
    #[serde(default)]
    pub module_levels: HashMap<String, String>,
}

fn default_logging_enabled() -> bool {
    false
}
fn default_logging_level() -> String {
    "info".to_string()
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            level: "info".to_string(),
            module_levels: HashMap::new(),
        }
    }
}

// ==================== Theme ====================

/// Application theme
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

fn default_theme() -> Theme {
    Theme::Dark
}

// ==================== UI Language ====================

/// Legacy default for a persisted settings file that predates `ui_language`.
/// Distinct from `AppSettings::default()` ("en"): an old file without the field
/// keeps the previous behavior (Russian UI), while a fresh install starts in
/// English.
fn default_ui_language() -> String {
    "ru".to_string()
}

/// True if `tag` is a locale tag the catalog can load: the exact rule shared
/// with `crate::localization` so a persisted tag always names a catalog file.
/// Rejects empty segments (`en--BR`, `-en`), single-letter or numeric primaries
/// that cannot name a catalog file, and any path/separator or non-ASCII input.
fn is_valid_ui_language(tag: &str) -> bool {
    crate::localization::is_valid_locale_code(tag)
}

/// Forgiving deserializer for `ui_language`: a present-but-invalid value (non-
/// string, empty, path characters, non-ASCII) normalizes to `"en"` so a bad
/// value never discards the rest of the settings. A missing field is handled by
/// `#[serde(default = "default_ui_language")]` and yields `"ru"`.
fn deserialize_ui_language<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(value) if is_valid_ui_language(&value) => value,
        _ => "en".to_string(),
    })
}

// ==================== Editor Settings ====================

/// Spell check source
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SpellSource {
    Online,
    #[default]
    Offline,
}

/// Default editor font selection. Other values are installed Windows font
/// family names collected from DirectWrite at application startup.
fn default_editor_font_family() -> String {
    "default".to_string()
}

/// Keep a valid persisted family name, while allowing new fonts installed by
/// the user without a release. Non-string and blank values recover safely.
fn deserialize_font_family<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(value) if !value.trim().is_empty() => value,
        _ => default_editor_font_family(),
    })
}

/// Minimum/maximum/default editor font size in pixels.
pub const EDITOR_FONT_SIZE_MIN_PX: u32 = 12;
pub const EDITOR_FONT_SIZE_MAX_PX: u32 = 96;
const EDITOR_FONT_SIZE_DEFAULT_PX: u32 = 16;

fn default_editor_font_size_px() -> u32 {
    EDITOR_FONT_SIZE_DEFAULT_PX
}

/// Forgiving deserializer for `font_size_px`: any value that is not an integer
/// within `12..=96` falls back to the default `16`.
fn deserialize_font_size_px<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Number(n) => n
            .as_u64()
            .filter(|v| {
                *v >= EDITOR_FONT_SIZE_MIN_PX as u64 && *v <= EDITOR_FONT_SIZE_MAX_PX as u64
            })
            .map(|v| v as u32)
            .unwrap_or(EDITOR_FONT_SIZE_DEFAULT_PX),
        _ => EDITOR_FONT_SIZE_DEFAULT_PX,
    })
}

/// Default UI font selection. Other values are installed Windows font family
/// names; `"default"` keeps the built-in CSS stack (`var(--font-sans)`).
fn default_ui_font_family() -> String {
    "default".to_string()
}

/// Keep a valid persisted UI family name; blank and non-string values recover
/// safely like the editor family.
fn deserialize_ui_font_family<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(value) if !value.trim().is_empty() => value,
        _ => default_ui_font_family(),
    })
}

/// Minimum/maximum/default interface font size in pixels. The size sets the
/// `html` rem base of the main window, so the range stays narrow (14..=20).
pub const UI_FONT_SIZE_MIN_PX: u32 = 14;
pub const UI_FONT_SIZE_MAX_PX: u32 = 20;
const UI_FONT_SIZE_DEFAULT_PX: u32 = 16;

fn default_ui_font_size_px() -> u32 {
    UI_FONT_SIZE_DEFAULT_PX
}

/// Forgiving deserializer for `ui_font_size_px`: any value that is not an
/// integer within `14..=20` falls back to the default `16`.
fn deserialize_ui_font_size_px<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Number(n) => n
            .as_u64()
            .filter(|v| *v >= UI_FONT_SIZE_MIN_PX as u64 && *v <= UI_FONT_SIZE_MAX_PX as u64)
            .map(|v| v as u32)
            .unwrap_or(UI_FONT_SIZE_DEFAULT_PX),
        _ => UI_FONT_SIZE_DEFAULT_PX,
    })
}

/// Quick editor behavior mode
///
/// Controls how the main window reacts after Enter/Esc in the quick editor.
/// Serialized as lowercase strings; deserialization also accepts legacy bool values
/// (`false` → Disabled, `true` → Collapse) for backward compatibility.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuickEditorMode {
    #[default]
    Disabled,
    Collapse,
    ReturnFocus,
}

impl QuickEditorMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            QuickEditorMode::Disabled => "disabled",
            QuickEditorMode::Collapse => "collapse",
            QuickEditorMode::ReturnFocus => "return_focus",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "disabled" => Some(QuickEditorMode::Disabled),
            "collapse" => Some(QuickEditorMode::Collapse),
            "return_focus" => Some(QuickEditorMode::ReturnFocus),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for QuickEditorMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct QuickEditorModeVisitor;

        impl<'de> serde::de::Visitor<'de> for QuickEditorModeVisitor {
            type Value = QuickEditorMode;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a boolean or string ('disabled', 'collapse', 'return_focus')")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v {
                    Ok(QuickEditorMode::Collapse)
                } else {
                    Ok(QuickEditorMode::Disabled)
                }
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                QuickEditorMode::from_str(v)
                    .ok_or_else(|| E::unknown_variant(v, &["disabled", "collapse", "return_focus"]))
            }
        }

        deserializer.deserialize_any(QuickEditorModeVisitor)
    }
}

/// Route of the current phrase: where a submitted phrase is delivered.
///
/// Serialized as snake_case strings. Deserialization maps any unknown value to
/// `Everywhere` (the default), so old or corrupted settings load gracefully.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EditorRoute {
    #[default]
    Everywhere,
    NoTwitch,
    VoiceOnly,
    TwitchOnly,
}

impl EditorRoute {
    pub fn as_str(&self) -> &'static str {
        match self {
            EditorRoute::Everywhere => "everywhere",
            EditorRoute::NoTwitch => "no_twitch",
            EditorRoute::VoiceOnly => "voice_only",
            EditorRoute::TwitchOnly => "twitch_only",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "everywhere" => Some(EditorRoute::Everywhere),
            "no_twitch" => Some(EditorRoute::NoTwitch),
            "voice_only" => Some(EditorRoute::VoiceOnly),
            "twitch_only" => Some(EditorRoute::TwitchOnly),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for EditorRoute {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EditorRouteVisitor;

        impl<'de> serde::de::Visitor<'de> for EditorRouteVisitor {
            type Value = EditorRoute;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str(
                    "a route string ('everywhere', 'no_twitch', 'voice_only', 'twitch_only')",
                )
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(EditorRoute::from_str(v).unwrap_or(EditorRoute::Everywhere))
            }
        }

        deserializer.deserialize_str(EditorRouteVisitor)
    }
}

/// Настройки слоя омографов и ударений (RUAccent) перед синтезом.
///
/// Включение только сохраняет настройку; модель не загружается до явной команды
/// загрузки либо опции загрузки при старте. `accentor_pack_id` — stable id
/// найденного native pack.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct HomographAccentorSettings {
    pub enabled: bool,
    pub accentor_pack_id: Option<String>,
    /// Загружать выбранную модель RUAccent при старте приложения.
    #[serde(default)]
    pub load_on_start: bool,
}

/// Editor settings for quick and AI editor modes
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct EditorSettings {
    #[serde(default)]
    pub quick: QuickEditorMode,
    #[serde(default)]
    pub ai: bool,
    #[serde(default)]
    pub ai_completion: bool,
    #[serde(default = "default_true")]
    pub autocomplete_enabled: bool,
    /// Convert built-in digits to Russian words before synthesis.
    #[serde(default = "default_true")]
    pub number_conversion_enabled: bool,
    #[serde(default)]
    pub spellcheck_enabled: bool,
    #[serde(default)]
    pub spellcheck_source: SpellSource,
    #[serde(default = "default_editor_height")]
    pub editor_height: u32,
    #[serde(default = "default_typing_idle_timeout_ms")]
    pub typing_idle_timeout_ms: u32,
    #[serde(default = "default_true")]
    pub typing_enabled: bool,
    #[serde(default)]
    pub default_route: EditorRoute,
    #[serde(default)]
    pub keep_text_after_send: bool,
    #[serde(
        default = "default_editor_font_family",
        deserialize_with = "deserialize_font_family"
    )]
    pub font_family: String,
    #[serde(
        default = "default_editor_font_size_px",
        deserialize_with = "deserialize_font_size_px"
    )]
    pub font_size_px: u32,
    #[serde(default)]
    pub homograph_accentor: HomographAccentorSettings,
}

fn default_editor_height() -> u32 {
    340
}

fn default_true() -> bool {
    true
}

fn default_typing_idle_timeout_ms() -> u32 {
    800
}

const TYPING_IDLE_TIMEOUT_MIN_MS: u32 = 200;
const TYPING_IDLE_TIMEOUT_MAX_MS: u32 = 5000;

pub fn normalize_typing_idle_timeout_ms(ms: u32) -> u32 {
    ms.clamp(TYPING_IDLE_TIMEOUT_MIN_MS, TYPING_IDLE_TIMEOUT_MAX_MS)
}

fn editor_action_label(action_id: &str) -> &str {
    super::hotkeys::EditorHotkeySettings::action_label(action_id)
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            quick: QuickEditorMode::Disabled,
            ai: false,
            ai_completion: false,
            autocomplete_enabled: true,
            number_conversion_enabled: true,
            spellcheck_enabled: true,
            spellcheck_source: SpellSource::Offline,
            editor_height: 340,
            typing_idle_timeout_ms: 800,
            typing_enabled: true,
            default_route: EditorRoute::Everywhere,
            keep_text_after_send: false,
            font_family: default_editor_font_family(),
            font_size_px: EDITOR_FONT_SIZE_DEFAULT_PX,
            homograph_accentor: HomographAccentorSettings::default(),
        }
    }
}

// ==================== AI Settings ====================

/// AI provider type for text correction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AiProviderType {
    #[default]
    OpenAi,
    ZAi,
    DeepSeek,
    Custom,
}

/// OpenAI settings for AI text correction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AiOpenAiSettings {
    pub api_key: Option<String>,
    #[serde(default)]
    pub use_proxy: bool,
    #[serde(default = "default_openai_model")]
    pub model: String,
}

fn default_openai_model() -> String {
    "gpt-4o-mini".to_string()
}

/// Z.ai settings (Anthropic-compatible) for AI text correction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AiZAiSettings {
    pub url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default = "default_zai_model")]
    pub model: String,
}

fn default_zai_model() -> String {
    "glm-4.5".to_string()
}

/// DeepSeek settings (OpenAI-compatible) for AI text correction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiDeepSeekSettings {
    pub api_key: Option<String>,
    #[serde(default)]
    pub use_proxy: bool,
    #[serde(default = "default_deepseek_model")]
    pub model: String,
}

fn default_deepseek_model() -> String {
    "deepseek-chat".to_string()
}

/// Explicit Default: `#[derive(Default)]` would give `model = ""` (the `serde(default = ...)`
/// attribute only applies during deserialization, not to `Default::default()`). Since `deepseek`
/// is `#[serde(default)]` inside `AiSettings`, an old config without it goes through
/// `AiDeepSeekSettings::default()` — which must yield the real default model, not empty.
impl Default for AiDeepSeekSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            use_proxy: false,
            model: default_deepseek_model(),
        }
    }
}

/// Custom OpenAI-compatible settings for AI text correction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AiCustomSettings {
    pub url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub use_proxy: bool,
    #[serde(default)]
    pub model: String,
}

/// AI settings for text correction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiSettings {
    #[serde(default)]
    pub provider: AiProviderType,
    #[serde(default)]
    pub openai: AiOpenAiSettings,
    #[serde(default)]
    pub zai: AiZAiSettings,
    #[serde(default)]
    pub deepseek: AiDeepSeekSettings,
    #[serde(default)]
    pub custom: AiCustomSettings,
    #[serde(default = "default_ai_prompt")]
    pub prompt: String,
    #[serde(default = "default_ai_timeout")]
    pub timeout: u64,
}

fn default_ai_prompt() -> String {
    "Ты - корректор русского текста для TTS (текст-в-речь). Исправь текст следуя правилам:

1. Исправь орфографические и пунктуационные ошибки
2. Замени все числа на их словесную форму (123 -> \"сто двадцать три\", 15.5 -> \"пятнадцать целых пять десятых\")
3. Исправь ошибки раскладки (ghbdtn -> привет, rfr ltkf -> как дела)
4. Удали лишние пробелы и символы

ВАЖНО: Выведи ТОЛЬКО исправленный текст, без объяснений и комментариев.".to_string()
}

fn default_ai_timeout() -> u64 {
    20
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: AiProviderType::OpenAi,
            openai: AiOpenAiSettings::default(),
            zai: AiZAiSettings::default(),
            deepseek: AiDeepSeekSettings::default(),
            custom: AiCustomSettings::default(),
            prompt: default_ai_prompt(),
            timeout: default_ai_timeout(),
        }
    }
}

// ==================== Storage ====================

/// Пользовательские пути хранения (ROADMAP-117).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct StorageSettings {
    /// Корень данных программы (`%LOCALAPPDATA%\ttsbard` по умолчанию),
    /// содержащий `audio_cache` и `models`. `None` — дефолт.
    #[serde(default)]
    pub data_dir: Option<String>,
    /// Каталог аудио-кеша истории фраз; `None` — дефолт
    /// (`%LOCALAPPDATA%\ttsbard\audio_cache`).
    #[serde(default)]
    pub audio_cache_dir: Option<String>,
    /// Одноразовый перенос legacy-кеша из Roaming уже выполнен (ROADMAP-117).
    /// Отсутствующее поле трактуется как `false`. Backend-only маркер: в IPC
    /// DTO не выводится.
    #[serde(default)]
    pub legacy_audio_cache_migrated: bool,
    /// Одноразовый перенос legacy-моделей из Roaming уже выполнен. Не зависит
    /// от маркера audio-кеша. Отсутствующее поле трактуется как `false`.
    /// Backend-only маркер: в IPC DTO не выводится.
    #[serde(default)]
    pub legacy_models_migrated: bool,
}

// ==================== Main App Settings ====================

/// All application settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub audio: AudioSettings,
    pub tts: TtsSettings,
    #[serde(default)]
    pub audio_effects: AudioEffectsSettings,
    #[serde(default)]
    pub dsp: DspSettings,
    #[serde(default)]
    pub hotkey_enabled: bool,
    #[serde(default)]
    pub editor: EditorSettings,
    #[serde(default = "default_theme")]
    pub theme: Theme,
    /// Interface font family: `"default"` keeps the built-in CSS stack, other
    /// values are installed Windows family names.
    #[serde(
        default = "default_ui_font_family",
        deserialize_with = "deserialize_ui_font_family"
    )]
    pub ui_font_family: String,
    /// Interface font size: the `html` rem base of the main window in px.
    #[serde(
        default = "default_ui_font_size_px",
        deserialize_with = "deserialize_ui_font_size_px"
    )]
    pub ui_font_size_px: u32,
    /// Язык интерфейса: простой ASCII-тег (`en`, `ru`, `pt-BR`, `zh-Hant`).
    /// Отсутствующее в старом файле поле даёт `ru`; некорректное значение
    /// нормализуется к `en`.
    #[serde(
        default = "default_ui_language",
        deserialize_with = "deserialize_ui_language"
    )]
    pub ui_language: String,
    pub twitch: TwitchSettings,
    #[serde(default)]
    pub webview: WebViewSettings,
    #[serde(default)]
    pub input_server: InputServerSettings,
    /// Source-neutral Incoming policy (auto-play for server/OCR intake).
    #[serde(default)]
    pub incoming: IncomingSettings,
    #[serde(default)]
    pub ocr: OcrSettings,
    #[serde(default)]
    pub logging: LoggingSettings,
    #[serde(default)]
    pub storage: StorageSettings,
    #[serde(default)]
    pub ai: AiSettings,
    #[serde(default)]
    pub hotkeys: HotkeySettings,
    #[serde(default)]
    pub vtube_studio: VTubeStudioSettings,
    #[serde(default)]
    pub vrchat: VrchatSettings,
    /// Показывать окно управления воспроизведением при запуске
    #[serde(default)]
    pub show_playback_on_start: bool,
    /// Запускать главное окно сразу в компактном режиме
    #[serde(default)]
    pub start_compact: bool,
    /// Скрывать свёрнутое главное окно с панели задач (в трей)
    #[serde(default)]
    pub hide_on_minimize: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            audio: AudioSettings::default(),
            tts: TtsSettings::default(),
            audio_effects: AudioEffectsSettings::default(),
            dsp: DspSettings::default(),
            hotkey_enabled: true,
            editor: EditorSettings::default(),
            theme: Theme::Dark,
            ui_font_family: default_ui_font_family(),
            ui_font_size_px: UI_FONT_SIZE_DEFAULT_PX,
            ui_language: "en".to_string(),
            twitch: TwitchSettings::default(),
            webview: WebViewSettings::default(),
            input_server: InputServerSettings::default(),
            incoming: IncomingSettings::default(),
            ocr: OcrSettings::default(),
            logging: LoggingSettings::default(),
            storage: StorageSettings::default(),
            ai: AiSettings::default(),
            hotkeys: HotkeySettings::default(),
            vtube_studio: VTubeStudioSettings::default(),
            vrchat: VrchatSettings::default(),
            show_playback_on_start: false,
            start_compact: false,
            hide_on_minimize: false,
        }
    }
}

impl AppSettings {
    /// Validate all settings and fix invalid values
    pub fn validate(&mut self) {
        // Validate audio volumes
        self.audio.speaker_volume = validate_volume(self.audio.speaker_volume);
        self.audio.virtual_mic_volume = validate_volume(self.audio.virtual_mic_volume);

        // Validate webview port
        if let Err(e) = validate_port(self.webview.port) {
            warn!(error = %e, "Invalid webview port, using default");
            self.webview.port = 10100;
        }

        // Validate input server port
        if let Err(e) = validate_port(self.input_server.port) {
            warn!(error = %e, "Invalid input server port, using default");
            self.input_server.port = 10101;
        }

        // Validate VRChat settings
        if let Err(e) = validate_port(self.vrchat.port) {
            warn!(error = %e, "Invalid VRChat port, using default");
            self.vrchat.port = default_vrchat_port();
        }
        if let Err(e) = validate_vrchat_host(&self.vrchat.host) {
            warn!(error = %e, "Invalid VRChat host, using default");
            self.vrchat.host = default_vrchat_host();
        }

        // Validate and clamp Silero timing settings (enforce min, unbounded above per ROADMAP-133)
        {
            let old = self.tts.telegram.synthesis_response_timeout_ms;
            self.tts.telegram.synthesis_response_timeout_ms =
                old.max(SYNTHESIS_RESPONSE_TIMEOUT_MIN_MS);
            if self.tts.telegram.synthesis_response_timeout_ms != old {
                warn!(
                    synthesis_response_timeout_ms = old,
                    clamped = self.tts.telegram.synthesis_response_timeout_ms,
                    min = SYNTHESIS_RESPONSE_TIMEOUT_MIN_MS,
                    "clamped synthesis_response_timeout_ms"
                );
            }
        }
        {
            let old = self.tts.telegram.download_retry_delay_ms;
            self.tts.telegram.download_retry_delay_ms =
                old.clamp(DOWNLOAD_RETRY_DELAY_MIN_MS, DOWNLOAD_RETRY_DELAY_MAX_MS);
            if self.tts.telegram.download_retry_delay_ms != old {
                warn!(
                    download_retry_delay_ms = old,
                    clamped = self.tts.telegram.download_retry_delay_ms,
                    min = DOWNLOAD_RETRY_DELAY_MIN_MS,
                    max = DOWNLOAD_RETRY_DELAY_MAX_MS,
                    "clamped download_retry_delay_ms"
                );
            }
        }
    }
}

// ==================== Settings Manager ====================

/// Manager for application settings with in-memory caching
///
/// This implementation uses RwLock for efficient read-heavy workloads.
/// Settings are loaded once into memory and cached, with cache invalidation
/// only when settings are modified.
pub struct SettingsManager {
    config_dir: PathBuf,
    /// In-memory cache of settings protected by RwLock for read-heavy access
    cache: Arc<RwLock<AppSettings>>,
    /// In-memory cache of the ElevenLabs catalog so repeated reads never hit
    /// the disk. `None` until first loaded (or after an API-key change).
    elevenlabs_catalog_cache: Arc<std::sync::RwLock<Option<ElevenLabsCatalogCache>>>,
}

impl Clone for SettingsManager {
    fn clone(&self) -> Self {
        Self {
            config_dir: self.config_dir.clone(),
            cache: Arc::clone(&self.cache),
            elevenlabs_catalog_cache: Arc::clone(&self.elevenlabs_catalog_cache),
        }
    }
}

impl SettingsManager {
    /// Create a new SettingsManager with initialized cache
    pub fn new() -> Result<Self> {
        let config_dir = crate::paths::config_root().context("Failed to get config dir")?;

        fs::create_dir_all(&config_dir).context("Failed to create config dir")?;

        // Load settings initially and cache them
        let settings = Self::load_from_disk(&config_dir)?;

        Ok(Self {
            config_dir,
            cache: Arc::new(RwLock::new(settings)),
            elevenlabs_catalog_cache: Arc::new(std::sync::RwLock::new(None)),
        })
    }

    /// Create a SettingsManager with a custom config directory (for testing).
    #[cfg(test)]
    pub fn with_config_dir(config_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&config_dir).context("Failed to create config dir")?;
        let settings = Self::load_from_disk(&config_dir)?;
        Ok(Self {
            config_dir,
            cache: Arc::new(RwLock::new(settings)),
            elevenlabs_catalog_cache: Arc::new(std::sync::RwLock::new(None)),
        })
    }

    /// Get the path to settings.json
    fn settings_path(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    /// Get the path to the ElevenLabs catalog cache file.
    fn elevenlabs_catalog_path(&self) -> PathBuf {
        self.config_dir.join("elevenlabs-catalog.json")
    }

    /// Read-only access to the config directory (for locale catalog integration).
    #[allow(dead_code)] // Ready for integration when the locale catalog lands
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Ensure the input server has a LAN access token, generating one when the
    /// persisted value is missing or empty. Returns whether a token was
    /// generated so the caller persists it in the same atomic write.
    fn ensure_input_server_access_token(settings: &mut AppSettings) -> bool {
        let needs_token = settings
            .input_server
            .access_token
            .as_deref()
            .map(str::is_empty)
            .unwrap_or(true);
        if needs_token {
            settings.input_server.access_token = Some(uuid::Uuid::new_v4().to_string());
        }
        needs_token
    }

    /// Load settings from disk (internal method)
    ///
    /// A failure to read, parse, or deserialize an existing `settings.json`
    /// never replaces the file with defaults: the error carries structured
    /// [`SettingsLoadFailure`] diagnostics, including the outcome of a
    /// byte-exact backup copy of the untouched source (ROADMAP-123).
    fn load_from_disk(config_dir: &Path) -> Result<AppSettings> {
        let path = config_dir.join("settings.json");

        if path.exists() {
            let content = fs::read_to_string(&path).map_err(|error| {
                load_failure(
                    &path,
                    SettingsFailureStage::Read,
                    error.to_string(),
                    None,
                    None,
                    None,
                )
            })?;

            let json_value = match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(value) => value,
                Err(e) => {
                    warn!(error = %e, "settings.json is corrupted, starting recovery dialog");
                    return Err(load_failure(
                        &path,
                        SettingsFailureStage::Syntax,
                        e.to_string(),
                        Some(e.line().max(1)),
                        Some(e.column().max(1)),
                        None,
                    ));
                }
            };

            let mut settings = match serde_json::from_value::<AppSettings>(json_value.clone()) {
                Ok(parsed) => parsed,
                Err(e) => {
                    warn!(error = %e, "settings.json does not match the settings schema, starting recovery dialog");
                    // The Value-based error carries no position; probe the
                    // streaming parser for the line/column of the offending
                    // value. The message itself may embed file content and is
                    // therefore replaced by a secret-safe reason.
                    let (line, column) = match serde_json::from_str::<AppSettings>(&content) {
                        Err(probe) if probe.line() > 0 => {
                            (Some(probe.line()), Some(probe.column()))
                        }
                        _ => (None, None),
                    };
                    return Err(load_failure(
                        &path,
                        SettingsFailureStage::Deserialize,
                        recovery::sanitized_deserialize_reason(&e),
                        line,
                        column,
                        Some(&content),
                    ));
                }
            };

            // Migrate from old settings missing hotkey fields.
            // Новые playback-поля (pause/stop/repeat) уже заполнены дефолтом
            // при десериализации благодаря #[serde(default)] на HotkeySettings,
            // но старый файл нужно дописать, чтобы он стал консистентным.
            let needs_hotkey_migration = settings.hotkeys.main_window.key.is_empty()
                || settings.hotkeys.sound_panel.key.is_empty()
                || settings.hotkeys.playback_pause.key.is_empty()
                || settings.hotkeys.playback_stop.key.is_empty()
                || settings.hotkeys.playback_repeat.key.is_empty()
                || settings.hotkeys.playback_control_window.key.is_empty()
                || settings.hotkeys.return_previous_window.key.is_empty();

            if needs_hotkey_migration {
                info!("Migrating hotkey settings from defaults");
                settings.hotkeys = HotkeySettings::default();
            }

            // Migrate legacy `input_server.auto_play` into the source-neutral
            // top-level `incoming` section when the latter is absent. A boolean
            // legacy value is preserved exactly; a missing/invalid value uses the
            // Incoming default. When both the new `incoming` and the legacy value
            // exist, the new value wins (no migration runs).
            let needs_incoming_migration = json_value.get("incoming").is_none();
            if needs_incoming_migration {
                let legacy_auto_play = match json_value.pointer("/input_server/auto_play") {
                    Some(serde_json::Value::Bool(value)) => {
                        settings.incoming.auto_play = *value;
                        *value
                    }
                    _ => settings.incoming.auto_play,
                };
                info!(
                    auto_play = legacy_auto_play,
                    "Migrating legacy input_server.auto_play into top-level incoming"
                );
            }

            // Migrate old settings missing the `ui_language` field: an absent
            // field deserializes to the legacy default "ru" via
            // #[serde(default)], but the canonical field must be written back
            // so the file stays consistent. All other settings are preserved.
            let needs_ui_language_migration = json_value.get("ui_language").is_none();
            if needs_ui_language_migration {
                info!("Migrating ui_language from legacy default");
            }

            // Generate the input-server LAN access token when missing or empty:
            // the token is the only gate for non-loopback clients, so an absent
            // field must not leave LAN access unusable after an upgrade.
            let needs_input_token_migration = Self::ensure_input_server_access_token(&mut settings);

            // Persist the migrated canonical file at most once: a single atomic
            // write covers the hotkey, incoming, ui_language and input-server
            // token migrations.
            if needs_hotkey_migration
                || needs_incoming_migration
                || needs_ui_language_migration
                || needs_input_token_migration
            {
                // Save migrated settings
                let migrated_content = serde_json::to_string_pretty(&settings)?;
                let write_result = {
                    let _guard = persistence::config_write_lock().lock();
                    persistence::write_json_atomically(&path, &migrated_content)
                };
                // Failure diagnostics create a backup under the same mutex.
                // Release the write guard before entering that path.
                write_result.map_err(|error| {
                    load_failure(
                        &path,
                        SettingsFailureStage::Write,
                        error.to_string(),
                        None,
                        None,
                        Some(&content),
                    )
                })?;
            }

            settings.validate();
            Ok(settings)
        } else {
            info!("Settings file not found, creating with defaults");
            write_default_settings(&path).map_err(|error| {
                load_failure(
                    &path,
                    SettingsFailureStage::Write,
                    error.to_string(),
                    None,
                    None,
                    None,
                )
            })
        }
    }

    /// Load settings from cache (fast, no disk I/O)
    ///
    /// This method reads from the in-memory cache protected by RwLock.
    /// Multiple readers can access this concurrently without blocking.
    /// Returns Ok with settings since cache reads cannot fail.
    #[inline]
    pub fn load(&self) -> Result<AppSettings> {
        Ok(self.cache.read().clone())
    }

    /// Get a clone of the internal cache Arc for sharing with AppState
    ///
    /// This allows the hot path to read cached settings without constructing
    /// a new SettingsManager. The returned Arc points to the same RwLock
    /// that save/update_field write to, so cache consistency is guaranteed.
    pub fn cache_arc(&self) -> Arc<RwLock<AppSettings>> {
        Arc::clone(&self.cache)
    }

    /// Save settings to both disk and cache
    ///
    /// This method writes to disk and updates the in-memory cache.
    /// Uses write lock to ensure exclusive access during updates.
    pub fn save(&self, settings: &AppSettings) -> Result<()> {
        let path = self.settings_path();

        let content =
            serde_json::to_string_pretty(settings).context("Failed to serialize settings")?;

        let _guard = persistence::config_write_lock().lock();
        persistence::write_json_atomically(&path, &content)
            .context("Failed to write settings file")?;

        // Update cache after successful disk write
        *self.cache.write() = settings.clone();

        info!("Settings saved and cache updated");
        Ok(())
    }

    /// Read, mutate, persist, and refresh the cache while holding the shared
    /// config writer lock for the complete read-modify-write transaction.
    fn update_settings_atomically(&self, update: impl FnOnce(&mut AppSettings)) -> Result<()> {
        let path = self.settings_path();
        let _guard = persistence::config_write_lock().lock();

        let mut settings = if path.exists() {
            let content = fs::read_to_string(&path).context("Failed to read settings file")?;
            serde_json::from_str(&content).context("Failed to parse settings JSON")?
        } else {
            AppSettings::default()
        };
        update(&mut settings);

        let content =
            serde_json::to_string_pretty(&settings).context("Failed to serialize settings")?;
        persistence::write_json_atomically(&path, &content)
            .context("Failed to write settings file")?;
        *self.cache.write() = settings;

        Ok(())
    }

    /// Atomically update a single field in the settings JSON file
    ///
    /// This method reads the JSON file, updates a specific field using JSON pointer,
    /// and writes it back without full deserialization/serialization of AppSettings.
    /// This is more efficient for single-field updates.
    ///
    /// # Arguments
    /// * `json_pointer` - JSON pointer path (e.g., "/audio/speaker_volume")
    /// * `value` - New value to set (must be serializable to JSON)
    ///
    /// # Example
    /// ```text
    /// settings_manager.update_field("/audio/speaker_volume", &80)?;
    /// ```
    fn update_field<T>(&self, json_pointer: &str, value: &T) -> Result<()>
    where
        T: serde::Serialize,
    {
        let path = self.settings_path();
        let _guard = persistence::config_write_lock().lock();

        // Read existing JSON or create default
        let mut json_value = if path.exists() {
            let content = fs::read_to_string(&path).context("Failed to read settings file")?;
            serde_json::from_str(&content).context("Failed to parse settings JSON")?
        } else {
            // Create default settings as JSON
            serde_json::to_value(AppSettings::default())
                .context("Failed to create default settings")?
        };

        // Parse JSON pointer and navigate to the target field
        let parts: Vec<&str> = json_pointer
            .split('/')
            .skip(1) // Skip empty first element from leading '/'
            .collect();

        if parts.is_empty() {
            return Err(anyhow::anyhow!("Invalid JSON pointer: {}", json_pointer));
        }

        // Navigate through the JSON structure
        let mut current = &mut json_value;
        for (i, &part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;

            if is_last {
                // Update the final field
                let json_val = serde_json::to_value(value).context("Failed to serialize value")?;
                match current {
                    Value::Object(map) => {
                        map.insert(part.to_string(), json_val);
                    }
                    _ => {
                        return Err(anyhow::anyhow!(
                            "Cannot set field '{}' on non-object at path",
                            part
                        ));
                    }
                }
            } else {
                // Navigate deeper
                current = match current {
                    Value::Object(map) => map
                        .entry(part.to_string())
                        .or_insert_with(|| Value::Object(serde_json::Map::new())),
                    _ => {
                        return Err(anyhow::anyhow!(
                            "Cannot navigate through non-object at field '{}'",
                            part
                        ));
                    }
                };
            }
        }

        // Write updated JSON back to file
        let content = serde_json::to_string_pretty(&json_value)
            .context("Failed to serialize updated settings")?;

        persistence::write_json_atomically(&path, &content)
            .context("Failed to write settings file")?;

        // Update cache after successful disk write
        let settings: AppSettings =
            serde_json::from_str(&content).context("Failed to parse updated settings")?;
        *self.cache.write() = settings;

        Ok(())
    }

    // ========== Audio Settings ==========

    pub fn set_audio_output_format(&self, format: AudioOutputFormat) -> Result<()> {
        self.update_field("/audio/output_format", &format)
    }

    /// Set speaker device
    pub fn set_speaker_device(&self, device_id: Option<String>) -> Result<()> {
        self.update_field("/audio/speaker_device", &device_id)
    }

    /// Set speaker enabled
    pub fn set_speaker_enabled(&self, enabled: bool) -> Result<()> {
        self.update_field("/audio/speaker_enabled", &enabled)
    }

    /// Set speaker volume
    pub fn set_speaker_volume(&self, volume: u8) -> Result<()> {
        let validated = validate_volume(volume);
        self.update_field("/audio/speaker_volume", &validated)
    }

    /// Set virtual mic device
    pub fn set_virtual_mic_device(&self, device_id: Option<String>) -> Result<()> {
        self.update_field("/audio/virtual_mic_device", &device_id)
    }

    /// Set virtual mic volume
    pub fn set_virtual_mic_volume(&self, volume: u8) -> Result<()> {
        let validated = validate_volume(volume);
        self.update_field("/audio/virtual_mic_volume", &validated)
    }

    // ========== TTS Settings ==========

    /// Get TTS provider
    pub fn get_tts_provider(&self) -> TtsProviderType {
        self.cache.read().tts.provider
    }

    /// Get TTS provider ID
    pub fn get_tts_provider_id(&self) -> Option<String> {
        self.cache.read().tts.provider_id.clone()
    }

    /// Atomically replace and persist the complete visible-provider-ID vector.
    ///
    /// Presentation-only preference: does not touch the active provider, runtime
    /// registry, API keys, or any other TTS setting. The frontend owns the
    /// current runtime-derived list and sends the full desired set (no dedup,
    /// no validation against registered providers).
    pub fn set_visible_tts_provider_ids(&self, provider_ids: Vec<String>) -> Result<()> {
        self.update_settings_atomically(move |settings| {
            settings.tts.visible_provider_ids = provider_ids;
        })
    }

    /// Set OpenAI API key
    pub fn set_openai_api_key(&self, api_key: Option<String>) -> Result<()> {
        self.update_field("/tts/openai/api_key", &api_key)
    }

    /// Get OpenAI API key
    pub fn get_openai_api_key(&self) -> Option<String> {
        self.cache.read().tts.openai.api_key.clone()
    }

    /// Set OpenAI voice
    pub fn set_openai_voice(&self, voice: String) -> Result<()> {
        self.update_field("/tts/openai/voice", &voice)
    }

    /// Get OpenAI voice
    pub fn get_openai_voice(&self) -> String {
        self.cache.read().tts.openai.voice.clone()
    }

    /// Set local TTS URL
    pub fn set_local_tts_url(&self, url: String) -> Result<()> {
        self.update_field("/tts/local/url", &url)
    }

    /// Get local TTS URL
    pub fn get_local_tts_url(&self) -> String {
        self.cache.read().tts.local.url.clone()
    }

    /// Set Telegram API ID
    pub fn set_telegram_api_id(&self, api_id: Option<i64>) -> Result<()> {
        self.update_field("/tts/telegram/api_id", &api_id)
    }

    /// Get Telegram API ID
    pub fn get_telegram_api_id(&self) -> Option<i64> {
        self.cache.read().tts.telegram.api_id
    }

    // ========== Fish Audio Settings ==========

    pub fn set_fish_audio_api_key(&self, api_key: Option<String>) -> Result<()> {
        self.update_field("/tts/fish/api_key", &api_key)
    }

    /// Persist the fields edited together in the Fish Audio form as one
    /// settings transaction. The cache is updated only after the durable write.
    pub fn set_fish_audio_connection_settings(
        &self,
        api_key: String,
        format: String,
        temperature: f32,
        sample_rate: u32,
    ) -> Result<()> {
        self.update_settings_atomically(move |settings| {
            let fish = &mut settings.tts.fish;
            fish.api_key = Some(api_key);
            fish.format = format;
            fish.temperature = temperature;
            fish.sample_rate = sample_rate;
        })
    }

    pub fn get_fish_audio_api_key(&self) -> Option<String> {
        self.cache.read().tts.fish.api_key.clone()
    }

    pub fn set_fish_audio_reference_id(&self, reference_id: String) -> Result<()> {
        self.update_field("/tts/fish/reference_id", &reference_id)
    }

    pub fn get_fish_audio_reference_id(&self) -> String {
        self.cache.read().tts.fish.reference_id.clone()
    }

    pub fn add_fish_audio_voice(&self, voice: crate::tts::VoiceModel) -> Result<()> {
        let mut settings = self.load()?;
        if !settings.tts.fish.voices.iter().any(|v| v.id == voice.id) {
            settings.tts.fish.voices.push(voice);
            self.save(&settings)?;
        }
        Ok(())
    }

    pub fn remove_fish_audio_voice(&self, voice_id: &str) -> Result<()> {
        let mut settings = self.load()?;
        settings.tts.fish.voices.retain(|v| v.id != voice_id);
        if settings.tts.fish.reference_id == voice_id {
            settings.tts.fish.reference_id.clear();
        }
        self.save(&settings)
    }

    pub fn get_fish_audio_voices(&self) -> Vec<crate::tts::VoiceModel> {
        self.cache.read().tts.fish.voices.clone()
    }

    pub fn set_fish_audio_format(&self, format: String) -> Result<()> {
        self.update_field("/tts/fish/format", &format)
    }

    pub fn set_fish_audio_temperature(&self, temperature: f32) -> Result<()> {
        self.update_field("/tts/fish/temperature", &temperature)
    }

    pub fn set_fish_audio_sample_rate(&self, sample_rate: u32) -> Result<()> {
        self.update_field("/tts/fish/sample_rate", &sample_rate)
    }

    pub fn set_fish_audio_use_proxy(&self, enabled: bool) -> Result<()> {
        self.update_field("/tts/fish/use_proxy", &enabled)
    }

    pub fn get_fish_audio_use_proxy(&self) -> bool {
        self.cache.read().tts.fish.use_proxy
    }

    // ========== ElevenLabs Settings ==========

    /// Read the persisted ElevenLabs catalog cache (voices + models).
    ///
    /// Missing or corrupt cache safely yields an empty catalog; the caller
    /// never receives a startup-breaking error. The result is cached in memory
    /// so repeated reads do not touch the disk.
    fn get_elevenlabs_catalog(&self) -> ElevenLabsCatalogCache {
        if let Some(cache) = self
            .elevenlabs_catalog_cache
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            return cache;
        }
        let cache = ElevenLabsCatalogCache::load_from_disk(&self.elevenlabs_catalog_path());
        *self
            .elevenlabs_catalog_cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(cache.clone());
        cache
    }

    /// Atomically replace the voices portion of the catalog cache, preserving
    /// the current models. Only the catalog file is rewritten — `settings.json`
    /// is untouched. A failed write leaves the previous catalog file intact.
    pub fn set_elevenlabs_voices(&self, voices: Vec<ElevenLabsVoice>) -> Result<()> {
        let path = self.elevenlabs_catalog_path();
        let _guard = persistence::config_write_lock().lock();
        let mut cache = ElevenLabsCatalogCache::load_from_disk(&path);
        cache.voices = voices;
        let content = serde_json::to_string_pretty(&cache)
            .context("Failed to serialize ElevenLabs catalog")?;
        persistence::write_json_atomically(&path, &content)
            .context("Failed to write ElevenLabs catalog file")?;
        *self
            .elevenlabs_catalog_cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(cache);
        Ok(())
    }

    /// Atomically replace the models portion of the catalog cache, preserving
    /// the current voices. Only the catalog file is rewritten — `settings.json`
    /// is untouched. A failed write leaves the previous catalog file intact.
    pub fn set_elevenlabs_models(&self, models: Vec<ElevenLabsModel>) -> Result<()> {
        let path = self.elevenlabs_catalog_path();
        let _guard = persistence::config_write_lock().lock();
        let mut cache = ElevenLabsCatalogCache::load_from_disk(&path);
        cache.models = models;
        let content = serde_json::to_string_pretty(&cache)
            .context("Failed to serialize ElevenLabs catalog")?;
        persistence::write_json_atomically(&path, &content)
            .context("Failed to write ElevenLabs catalog file")?;
        *self
            .elevenlabs_catalog_cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(cache);
        Ok(())
    }

    /// Persist only the ElevenLabs API key, trimmed of surrounding whitespace.
    ///
    /// Returns `Ok(true)` when the normalized key changed (and the cached
    /// catalogs plus `voice_id`/`model_id` were cleared so data from a previous
    /// account cannot be reused) and `Ok(false)` when the normalized key is
    /// unchanged (preserving the cached catalogs and selection).
    pub fn set_elevenlabs_api_key(&self, key: String) -> Result<bool> {
        let trimmed = key.trim().to_string();
        let settings_path = self.settings_path();
        let catalog_path = self.elevenlabs_catalog_path();
        let _guard = persistence::config_write_lock().lock();

        let mut settings = if settings_path.exists() {
            let content =
                fs::read_to_string(&settings_path).context("Failed to read settings file")?;
            serde_json::from_str::<AppSettings>(&content)
                .context("Failed to parse settings JSON")?
        } else {
            AppSettings::default()
        };

        let current = settings
            .tts
            .elevenlabs
            .api_key
            .as_deref()
            .map(str::trim)
            .map(str::to_string);
        if current.as_deref() == Some(trimmed.as_str()) {
            return Ok(false);
        }

        settings.tts.elevenlabs.api_key = Some(trimmed);
        settings.tts.elevenlabs.voice_id.clear();
        settings.tts.elevenlabs.model_id.clear();

        let content =
            serde_json::to_string_pretty(&settings).context("Failed to serialize settings")?;
        persistence::write_json_atomically(&settings_path, &content)
            .context("Failed to write settings file")?;
        *self.cache.write() = settings;

        let empty_content = serde_json::to_string_pretty(&ElevenLabsCatalogCache::default())
            .context("Failed to serialize empty ElevenLabs catalog")?;
        persistence::write_json_atomically(&catalog_path, &empty_content)
            .context("Failed to write empty ElevenLabs catalog file")?;
        *self
            .elevenlabs_catalog_cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;

        Ok(true)
    }

    /// Atomically validate, persist and normalize the ElevenLabs generation
    /// settings. The model must exist in the cached model catalog; unsupported
    /// style / speaker-boost values are normalized to safe defaults.
    pub fn set_elevenlabs_generation_settings(
        &self,
        model_id: String,
        output_format: String,
        stability: f32,
        similarity_boost: f32,
        style: f32,
        use_speaker_boost: bool,
    ) -> Result<()> {
        crate::tts::elevenlabs::validate_connection_settings(
            &output_format,
            stability,
            similarity_boost,
            style,
        )
        .map_err(anyhow::Error::msg)?;

        let catalog = self.get_elevenlabs_catalog();
        let model = catalog
            .models
            .iter()
            .find(|m| m.model_id == model_id)
            .ok_or_else(|| {
                anyhow::anyhow!("ElevenLabs model is not available in the cached catalog.")
            })?;
        let style = if model.can_use_style { style } else { 0.0 };
        let use_speaker_boost = if model.can_use_speaker_boost {
            use_speaker_boost
        } else {
            false
        };

        self.update_settings_atomically(move |settings| {
            let el = &mut settings.tts.elevenlabs;
            el.model_id = model_id;
            el.output_format = output_format;
            el.stability = stability;
            el.similarity_boost = similarity_boost;
            el.style = style;
            el.use_speaker_boost = use_speaker_boost;
        })
    }

    /// Atomically persist the selected model and its normalized capability
    /// flags. Used by the model refresh to preserve/fall back the selection.
    pub fn set_elevenlabs_model_selection(
        &self,
        model_id: String,
        style: f32,
        use_speaker_boost: bool,
    ) -> Result<()> {
        self.update_settings_atomically(move |settings| {
            let el = &mut settings.tts.elevenlabs;
            el.model_id = model_id;
            el.style = style;
            el.use_speaker_boost = use_speaker_boost;
        })
    }

    pub fn get_elevenlabs_api_key(&self) -> Option<String> {
        self.cache.read().tts.elevenlabs.api_key.clone()
    }

    pub fn set_elevenlabs_voice_id(&self, voice_id: String) -> Result<()> {
        self.update_field("/tts/elevenlabs/voice_id", &voice_id)
    }

    pub fn get_elevenlabs_voice_id(&self) -> String {
        self.cache.read().tts.elevenlabs.voice_id.clone()
    }

    pub fn get_elevenlabs_voices(&self) -> Vec<ElevenLabsVoice> {
        self.get_elevenlabs_catalog().voices
    }

    pub fn get_elevenlabs_models(&self) -> Vec<ElevenLabsModel> {
        self.get_elevenlabs_catalog().models
    }

    pub fn get_elevenlabs_model_id(&self) -> String {
        self.cache.read().tts.elevenlabs.model_id.clone()
    }

    pub fn set_elevenlabs_use_proxy(&self, enabled: bool) -> Result<()> {
        self.update_field("/tts/elevenlabs/use_proxy", &enabled)
    }

    pub fn get_elevenlabs_use_proxy(&self) -> bool {
        self.cache.read().tts.elevenlabs.use_proxy
    }

    pub fn get_elevenlabs_settings(&self) -> ElevenLabsSettings {
        self.cache.read().tts.elevenlabs.clone()
    }

    /// Combined view for the UI: persisted settings plus the cached catalogs,
    /// without persisting the catalogs into `settings.json`.
    pub fn get_elevenlabs_settings_dto(&self) -> crate::config::dto::ElevenLabsSettingsDto {
        let mut dto: crate::config::dto::ElevenLabsSettingsDto =
            self.cache.read().tts.elevenlabs.clone().into();
        let catalog = self.get_elevenlabs_catalog();
        dto.voices = catalog.voices.into_iter().map(Into::into).collect();
        dto.models = catalog.models.into_iter().map(Into::into).collect();
        dto
    }

    // ========== Hotkey Settings ==========

    /// Set hotkey enabled
    pub fn set_hotkey_enabled(&self, enabled: bool) -> Result<()> {
        self.update_field("/hotkey_enabled", &enabled)
    }

    /// Get hotkey enabled
    pub fn get_hotkey_enabled(&self) -> bool {
        self.cache.read().hotkey_enabled
    }

    // ========== Twitch Settings ==========

    /// Set Twitch settings
    pub fn set_twitch_settings(&self, settings: &TwitchSettings) -> Result<()> {
        let settings = settings.clone();
        self.update_settings_atomically(move |app_settings| {
            app_settings.twitch = settings;
        })
    }

    // ========== WebView Settings ==========

    /// Set WebView access token
    pub fn set_webview_access_token(&self, token: Option<String>) -> Result<()> {
        self.update_field("/webview/access_token", &token)
    }

    /// Set WebView UPnP enabled
    pub fn set_webview_upnp_enabled(&self, enabled: bool) -> Result<()> {
        self.update_field("/webview/upnp_enabled", &enabled)
    }

    /// Atomically replace four mutable fields of the WebView section.
    ///
    /// Preserves `access_token` and `enabled` from the current config.
    /// One load → mutate → save cycle; validated before call.
    pub fn set_webview_section(
        &self,
        start_on_boot: bool,
        port: u16,
        upnp_enabled: bool,
        send_original_text: bool,
    ) -> Result<()> {
        self.update_settings_atomically(move |app_settings| {
            app_settings.webview.start_on_boot = start_on_boot;
            app_settings.webview.port = port;
            app_settings.webview.upnp_enabled = upnp_enabled;
            app_settings.webview.send_original_text = send_original_text;
        })
    }

    // ========== Input Server Settings ==========

    /// Atomically replace the two desired input-server settings.
    ///
    /// One load → mutate → save cycle; the port must be validated by the caller
    /// before this is invoked. `auto_play` is intentionally not part of the
    /// server settings anymore — it lives in the source-neutral `incoming`
    /// policy and is only touched by [`Self::set_incoming_auto_play`].
    pub fn set_input_server_section(&self, start_on_boot: bool, port: u16) -> Result<()> {
        self.update_settings_atomically(move |app_settings| {
            app_settings.input_server.start_on_boot = start_on_boot;
            app_settings.input_server.port = port;
        })
    }

    /// Set the input-server LAN access token (non-loopback client auth).
    ///
    /// `None` removes the token; the next `load_from_disk` generates a fresh
    /// one, while the live auth middleware fails closed for non-loopback
    /// requests in the meantime.
    pub fn set_input_server_access_token(&self, token: Option<String>) -> Result<()> {
        self.update_field("/input_server/access_token", &token)
    }

    // ========== Incoming Settings ==========

    /// Atomically persist the source-neutral Incoming policy (`auto_play` and
    /// delivery route) as one section update.
    ///
    /// One load → mutate → save cycle that never touches the input-server
    /// section (or any other settings). Used by `save_incoming_settings` so a
    /// route change and an auto-play change land in a single durable write.
    pub fn set_incoming_section(&self, auto_play: bool, route: IncomingRoute) -> Result<()> {
        self.update_settings_atomically(move |app_settings| {
            app_settings.incoming.auto_play = auto_play;
            app_settings.incoming.route = route;
        })
    }

    // ========== OCR Settings ==========

    /// Atomically replace the two desired OCR settings.
    ///
    /// One load → mutate → save cycle; validation of the model id against
    /// discovered packs is performed by the caller (commands layer).
    pub fn set_ocr_section(&self, enabled: bool, model_id: Option<String>) -> Result<()> {
        self.update_settings_atomically(move |app_settings| {
            app_settings.ocr.enabled = enabled;
            app_settings.ocr.model_id = model_id;
        })
    }

    /// Atomically replace the full desired OCR settings (enabled, model id and
    /// capture target) as one section update for the user save path.
    ///
    /// One load → mutate → save cycle. Unlike [`Self::set_ocr_section`] — kept
    /// for model refresh and the failed-start untick, which must retain the
    /// persisted capture target — this persists the complete desired settings.
    pub fn set_ocr_settings(&self, settings: OcrSettings) -> Result<()> {
        self.update_settings_atomically(move |app_settings| {
            app_settings.ocr = settings;
        })
    }

    // ========== Logging Settings ==========

    /// Update logging settings atomically
    ///
    /// This method loads settings, updates logging configuration, and saves
    /// in a single operation to prevent race conditions.
    ///
    /// # Arguments
    /// * `updater` - Function that receives mutable reference to LoggingSettings
    pub fn update_logging<F>(&self, updater: F) -> Result<()>
    where
        F: FnOnce(&mut LoggingSettings),
    {
        let mut settings = self.load()?;
        updater(&mut settings.logging);
        self.save(&settings)
    }

    /// Get logging settings
    pub fn get_logging_settings(&self) -> LoggingSettings {
        self.cache.read().logging.clone()
    }

    // ========== Proxy Settings ==========

    /// Set SOCKS5 proxy URL
    ///
    /// Updates the /tts/network/proxy/proxy_url field.
    ///
    /// # Arguments
    /// * `url` - SOCKS5 proxy URL (e.g., socks5://host:port, socks5://user:pass@host:port)
    pub fn set_socks5_proxy_url(&self, url: String) -> Result<()> {
        self.update_field("/tts/network/proxy/proxy_url", &Some(url))
    }

    /// Get SOCKS5 proxy URL
    ///
    /// Returns the cached SOCKS5 proxy URL.
    pub fn get_socks5_proxy_url(&self) -> Option<String> {
        self.cache.read().tts.network.proxy.proxy_url.clone()
    }

    // ========== Legacy methods (deprecated) ==========

    /// Set proxy URL (legacy, use set_socks5_proxy_url instead)
    ///
    /// # Arguments
    /// * `url` - Proxy URL (e.g., socks5://host:port)
    pub fn set_proxy_url(&self, url: String) -> Result<()> {
        // Migrate to new structure
        self.set_socks5_proxy_url(url)
    }

    /// Get proxy URL (legacy, use get_socks5_proxy_url instead)
    pub fn get_proxy_url(&self) -> Option<String> {
        self.get_socks5_proxy_url()
    }

    /// Get proxy type (legacy, always returns Socks5)
    pub fn get_proxy_type(&self) -> ProxyType {
        ProxyType::Socks5
    }

    /// Set OpenAI use proxy flag
    ///
    /// Updates the /tts/openai/use_proxy field.
    ///
    /// # Arguments
    /// * `enabled` - Whether OpenAI should use the unified proxy
    pub fn set_openai_use_proxy(&self, enabled: bool) -> Result<()> {
        self.update_field("/tts/openai/use_proxy", &enabled)
    }

    /// Set Telegram proxy mode
    ///
    /// Updates the /tts/telegram/proxy_mode field.
    ///
    /// # Arguments
    /// * `mode` - Proxy mode for Telegram (None, Socks5, MtProxy)
    pub fn set_telegram_proxy_mode(&self, mode: ProxyMode) -> Result<()> {
        self.update_field("/tts/telegram/proxy_mode", &mode)
    }

    /// Set Silero synthesis response timeout in milliseconds (ROADMAP-133)
    pub fn set_silero_synthesis_response_timeout_ms(&self, timeout_ms: u32) -> Result<()> {
        let normalized = timeout_ms.max(SYNTHESIS_RESPONSE_TIMEOUT_MIN_MS);
        self.update_field("/tts/telegram/synthesis_response_timeout_ms", &normalized)
    }

    // ========== MTProxy Settings ==========

    /// Set MTProxy settings
    ///
    /// Updates all MTProxy fields atomically.
    ///
    /// # Arguments
    /// * `host` - MTProxy server host (IP or domain)
    /// * `port` - MTProxy server port
    /// * `secret` - MTProxy secret key (hex or base64 encoded)
    /// * `dc_id` - Optional DC ID (data center ID)
    pub fn set_mtproxy_settings(
        &self,
        host: Option<String>,
        port: u16,
        secret: Option<String>,
        dc_id: Option<i32>,
    ) -> Result<()> {
        let mut settings = self.load()?;
        settings.tts.network.mtproxy = MtProxySettings {
            host,
            port,
            secret,
            dc_id,
        };
        self.save(&settings)
    }

    // ========== Theme Settings ==========

    /// Set theme
    pub fn set_theme(&self, theme: Theme) -> Result<()> {
        self.update_field("/theme", &theme)
    }

    // ========== Interface Font Settings ==========

    /// Set the interface font family.
    pub fn set_ui_font_family(&self, family: String) -> Result<()> {
        if family.trim().is_empty() {
            return Err(anyhow::anyhow!("UI font family must not be blank"));
        }
        self.update_field("/ui_font_family", &family)
    }

    /// Get the interface font family.
    pub fn get_ui_font_family(&self) -> String {
        self.cache.read().ui_font_family.clone()
    }

    /// Set the interface font size (px), strict: rejects out-of-range values
    /// without writing.
    pub fn set_ui_font_size_px(&self, size_px: u32) -> Result<()> {
        if !(UI_FONT_SIZE_MIN_PX..=UI_FONT_SIZE_MAX_PX).contains(&size_px) {
            return Err(anyhow::anyhow!("Invalid UI font size: {}", size_px));
        }
        self.update_field("/ui_font_size_px", &size_px)
    }

    /// Get the interface font size (px).
    pub fn get_ui_font_size_px(&self) -> u32 {
        self.cache.read().ui_font_size_px
    }

    // ========== UI Language Settings ==========

    /// Get the persisted UI language tag.
    pub fn get_ui_language(&self) -> String {
        self.cache.read().ui_language.clone()
    }

    /// Set the UI language tag.
    ///
    /// Rejects invalid input (empty, non-ASCII, or containing path/separator
    /// characters) without touching cache or disk.
    pub fn set_ui_language(&self, language: String) -> Result<()> {
        if !is_valid_ui_language(&language) {
            return Err(anyhow::anyhow!("Invalid UI language tag: {:?}", language));
        }
        self.update_field("/ui_language", &language)
    }

    // ========== Storage Settings ==========

    /// Пометить одноразовый перенос legacy-кеша выполненным (ROADMAP-117).
    ///
    /// Атомарно читает текущие настройки с диска, ставит
    /// `storage.legacy_audio_cache_migrated = true` и сохраняет их без потери
    /// других полей. Провал записи возвращает ошибку, не меняя ни диск, ни кеш, —
    /// маркер можно повторить на следующем старте.
    pub fn set_legacy_audio_cache_migrated(&self) -> Result<()> {
        self.update_settings_atomically(move |settings| {
            settings.storage.legacy_audio_cache_migrated = true;
        })
    }

    /// Пометить одноразовый перенос legacy-моделей выполненным.
    ///
    /// Атомарно читает текущие настройки с диска, ставит
    /// `storage.legacy_models_migrated = true` и сохраняет их без потери
    /// других полей (включая независимый маркер audio-кеша). Провал записи
    /// возвращает ошибку, не меняя ни диск, ни кеш, — маркер можно повторить
    /// на следующем старте.
    pub fn set_legacy_models_migrated(&self) -> Result<()> {
        self.update_settings_atomically(move |settings| {
            settings.storage.legacy_models_migrated = true;
        })
    }

    /// Atomically persist the program data root and the legacy audio-cache
    /// override together (data transfer completion). A successful transfer
    /// migrates the audio cache into the new root, so the legacy override is
    /// cleared via `audio_cache_dir = None`.
    pub fn set_storage_data_dir(
        &self,
        data_dir: Option<String>,
        audio_cache_dir: Option<String>,
    ) -> Result<()> {
        self.update_settings_atomically(move |settings| {
            settings.storage.data_dir = data_dir;
            settings.storage.audio_cache_dir = audio_cache_dir;
        })
    }

    // ========== Editor Settings ==========

    /// Set quick editor behavior mode
    pub fn set_editor_quick(&self, mode: QuickEditorMode) -> Result<()> {
        self.update_field("/editor/quick", &mode)
    }

    /// Get quick editor behavior mode
    pub fn get_editor_quick(&self) -> QuickEditorMode {
        self.cache.read().editor.quick
    }

    /// Set AI correction in editor enabled state
    pub fn set_editor_ai(&self, enabled: bool) -> Result<()> {
        self.update_field("/editor/ai", &enabled)
    }

    /// Get AI correction in editor enabled state
    pub fn get_editor_ai(&self) -> bool {
        self.cache.read().editor.ai
    }

    /// Set AI completion in editor enabled state
    pub fn set_editor_ai_completion(&self, enabled: bool) -> Result<()> {
        self.update_field("/editor/ai_completion", &enabled)
    }

    /// Get AI completion in editor enabled state
    pub fn get_editor_ai_completion(&self) -> bool {
        self.cache.read().editor.ai_completion
    }

    /// Set editor autocomplete suggestions enabled state
    pub fn set_editor_autocomplete_enabled(&self, enabled: bool) -> Result<()> {
        self.update_field("/editor/autocomplete_enabled", &enabled)
    }

    /// Set built-in number conversion enabled state
    pub fn set_editor_number_conversion_enabled(&self, enabled: bool) -> Result<()> {
        self.update_field("/editor/number_conversion_enabled", &enabled)
    }

    /// Set spellcheck enabled state
    pub fn set_editor_spellcheck_enabled(&self, enabled: bool) -> Result<()> {
        self.update_field("/editor/spellcheck_enabled", &enabled)
    }

    /// Get spellcheck enabled state
    pub fn get_editor_spellcheck_enabled(&self) -> bool {
        self.cache.read().editor.spellcheck_enabled
    }

    /// Set spellcheck source
    pub fn set_editor_spellcheck_source(&self, source: SpellSource) -> Result<()> {
        self.update_field("/editor/spellcheck_source", &source)
    }

    /// Get spellcheck source
    pub fn get_editor_spellcheck_source(&self) -> SpellSource {
        self.cache.read().editor.spellcheck_source.clone()
    }

    /// Set editor height
    pub fn set_editor_height(&self, height: u32) -> Result<()> {
        let validated = height.clamp(200, 1200);
        self.update_field("/editor/editor_height", &validated)
    }

    /// Get editor height
    pub fn get_editor_height(&self) -> u32 {
        self.cache.read().editor.editor_height
    }

    /// Set VTS typing idle timeout (ms)
    pub fn set_editor_typing_idle_timeout_ms(&self, ms: u32) -> Result<()> {
        let validated = normalize_typing_idle_timeout_ms(ms);
        self.update_field("/editor/typing_idle_timeout_ms", &validated)
    }

    /// Get VTS typing idle timeout (ms)
    pub fn get_editor_typing_idle_timeout_ms(&self) -> u32 {
        self.cache.read().editor.typing_idle_timeout_ms
    }

    /// Set editor typing enabled state
    pub fn set_editor_typing_enabled(&self, enabled: bool) -> Result<()> {
        self.update_field("/editor/typing_enabled", &enabled)
    }

    /// Set keep-text-after-send state
    pub fn set_editor_keep_text(&self, enabled: bool) -> Result<()> {
        self.update_field("/editor/keep_text_after_send", &enabled)
    }

    /// Set default editor route
    pub fn set_editor_default_route(&self, route: EditorRoute) -> Result<()> {
        self.update_field("/editor/default_route", &route)
    }

    /// Set editor font family
    pub fn set_editor_font_family(&self, family: String) -> Result<()> {
        if family.trim().is_empty() {
            return Err(anyhow::anyhow!("Editor font family must not be blank"));
        }
        self.update_field("/editor/font_family", &family)
    }

    /// Get editor font family
    pub fn get_editor_font_family(&self) -> String {
        self.cache.read().editor.font_family.clone()
    }

    /// Set editor font size (px), strict: rejects out-of-range values without writing.
    pub fn set_editor_font_size_px(&self, size_px: u32) -> Result<()> {
        if !(EDITOR_FONT_SIZE_MIN_PX..=EDITOR_FONT_SIZE_MAX_PX).contains(&size_px) {
            return Err(anyhow::anyhow!("Invalid editor font size: {}", size_px));
        }
        self.update_field("/editor/font_size_px", &size_px)
    }

    /// Get editor font size (px)
    pub fn get_editor_font_size_px(&self) -> u32 {
        self.cache.read().editor.font_size_px
    }

    /// Set homograph/accentor (RUAccent) enabled state and selected pack id in
    /// one atomic transaction. Validation of the pack id against discovered
    /// packs is performed by the caller (commands layer).
    pub fn set_editor_homograph_accentor(
        &self,
        enabled: bool,
        accentor_pack_id: Option<String>,
    ) -> Result<()> {
        self.update_settings_atomically(move |settings| {
            settings.editor.homograph_accentor.enabled = enabled;
            settings.editor.homograph_accentor.accentor_pack_id = accentor_pack_id;
        })
    }

    /// Set whether the selected RUAccent model loads on startup. Validation of
    /// a selected model against discovered packs is performed by the caller
    /// (commands layer).
    pub fn set_editor_homograph_accentor_load_on_start(&self, load_on_start: bool) -> Result<()> {
        self.update_settings_atomically(move |settings| {
            settings.editor.homograph_accentor.load_on_start = load_on_start;
        })
    }

    // ========== AI Settings ==========

    /// Set AI provider
    pub fn set_ai_provider(&self, provider: AiProviderType) -> Result<()> {
        self.update_field("/ai/provider", &provider)
    }

    /// Set AI global prompt
    pub fn set_ai_prompt(&self, prompt: String) -> Result<()> {
        self.update_field("/ai/prompt", &prompt)
    }

    /// Set OpenAI API key for AI text correction
    pub fn set_ai_openai_api_key(&self, key: Option<String>) -> Result<()> {
        self.update_field("/ai/openai/api_key", &key)
    }

    /// Set OpenAI use proxy for AI text correction
    pub fn set_ai_openai_use_proxy(&self, enabled: bool) -> Result<()> {
        self.update_field("/ai/openai/use_proxy", &enabled)
    }

    /// Set Z.ai URL
    pub fn set_ai_zai_url(&self, url: Option<String>) -> Result<()> {
        self.update_field("/ai/zai/url", &url)
    }

    /// Set Z.ai API key
    pub fn set_ai_zai_api_key(&self, api_key: Option<String>) -> Result<()> {
        self.update_field("/ai/zai/api_key", &api_key)
    }

    /// Get Z.ai model
    pub fn get_ai_zai_model(&self) -> String {
        self.cache.read().ai.zai.model.clone()
    }

    /// Set OpenAI model for AI text correction
    pub fn set_ai_openai_model(&self, model: String) -> Result<()> {
        self.update_field("/ai/openai/model", &model)
    }

    /// Get OpenAI model for AI text correction
    pub fn get_ai_openai_model(&self) -> String {
        self.cache.read().ai.openai.model.clone()
    }

    /// Set Z.ai model for AI text correction
    pub fn set_ai_zai_model(&self, model: String) -> Result<()> {
        self.update_field("/ai/zai/model", &model)
    }

    /// Set DeepSeek API key for AI text correction
    pub fn set_ai_deepseek_api_key(&self, key: Option<String>) -> Result<()> {
        self.update_field("/ai/deepseek/api_key", &key)
    }

    /// Set DeepSeek use proxy for AI text correction
    pub fn set_ai_deepseek_use_proxy(&self, enabled: bool) -> Result<()> {
        self.update_field("/ai/deepseek/use_proxy", &enabled)
    }

    /// Get DeepSeek model
    pub fn get_ai_deepseek_model(&self) -> String {
        self.cache.read().ai.deepseek.model.clone()
    }

    /// Set DeepSeek model for AI text correction
    pub fn set_ai_deepseek_model(&self, model: String) -> Result<()> {
        self.update_field("/ai/deepseek/model", &model)
    }

    /// Set Custom API URL for AI text correction
    pub fn set_ai_custom_url(&self, url: Option<String>) -> Result<()> {
        self.update_field("/ai/custom/url", &url)
    }

    /// Set Custom API key for AI text correction
    pub fn set_ai_custom_api_key(&self, key: Option<String>) -> Result<()> {
        self.update_field("/ai/custom/api_key", &key)
    }

    /// Set Custom use proxy for AI text correction
    pub fn set_ai_custom_use_proxy(&self, enabled: bool) -> Result<()> {
        self.update_field("/ai/custom/use_proxy", &enabled)
    }

    /// Get Custom model for AI text correction
    pub fn get_ai_custom_model(&self) -> String {
        self.cache.read().ai.custom.model.clone()
    }

    /// Set Custom model for AI text correction
    pub fn set_ai_custom_model(&self, model: String) -> Result<()> {
        self.update_field("/ai/custom/model", &model)
    }

    // ========== Audio Effects Settings ==========

    /// Get audio effects settings
    pub fn get_audio_effects(&self) -> AudioEffectsSettings {
        self.cache.read().audio_effects.clone()
    }

    /// Set audio effects enabled
    pub fn set_audio_effects_enabled(&self, enabled: bool) -> Result<()> {
        self.update_field("/audio_effects/enabled", &enabled)
    }

    /// Set audio effects pitch
    pub fn set_audio_effects_pitch(&self, pitch: i16) -> Result<()> {
        let validated = pitch.clamp(-100, 100);
        self.update_field("/audio_effects/pitch", &validated)
    }

    /// Set audio effects speed
    pub fn set_audio_effects_speed(&self, speed: i16) -> Result<()> {
        let validated = speed.clamp(-100, 100);
        self.update_field("/audio_effects/speed", &validated)
    }

    /// Set audio effects volume
    pub fn set_audio_effects_volume(&self, volume: i16) -> Result<()> {
        let validated = volume.clamp(0, 200);
        self.update_field("/audio_effects/volume", &validated)
    }

    /// Set audio effects enhance (DeepFilterNet noise suppression) enabled
    pub fn set_audio_effects_enhance_enabled(&self, enabled: bool) -> Result<()> {
        self.update_field("/audio_effects/enhance_enabled", &enabled)
    }

    /// Set audio effects enhance attenuation limit (dB), clamped to 5..30
    pub fn set_audio_effects_enhance_atten_db(&self, atten_db: f32) -> Result<()> {
        let validated = atten_db.clamp(5.0, 30.0);
        self.update_field("/audio_effects/enhance_atten_db", &validated)
    }

    /// Set audio effects formant preservation (Signalsmith formant correction)
    pub fn set_audio_effects_formant_preserved(&self, preserved: bool) -> Result<()> {
        self.update_field("/audio_effects/formant_preserved", &preserved)
    }

    // ========== DSP Settings ==========

    /// Get DSP post-processing settings
    pub fn get_dsp_settings(&self) -> DspSettings {
        self.cache.read().dsp.clone()
    }

    /// Atomically save all DSP settings
    pub fn set_dsp_settings(&self, dsp: &DspSettings) -> Result<()> {
        let mut settings = self.load()?;
        settings.dsp = dsp.clone();
        self.save(&settings)
    }

    // ========== Hotkey Settings ==========

    /// Get all hotkey settings
    pub fn get_hotkey_settings(&self) -> Result<super::hotkeys::HotkeySettings> {
        Ok(self.cache.read().hotkeys.clone())
    }

    /// Set a specific hotkey
    ///
    /// # Arguments
    /// * `name` - Either "main_window" or "sound_panel"
    /// * `hotkey` - The new hotkey configuration
    pub fn set_hotkey(&self, name: &str, hotkey: &super::hotkeys::Hotkey) -> Result<()> {
        let mut settings = self.load()?;
        let field = settings
            .hotkeys
            .get_global_by_name_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Invalid hotkey name: {}", name))?;
        *field = hotkey.clone();
        self.save(&settings)
    }

    /// Flip the per-hotkey `enabled` switch without touching the stored
    /// combination. Conflict validation for a `false -> true` transition is the
    /// command layer's responsibility.
    pub fn set_hotkey_enabled_flag(&self, name: &str, enabled: bool) -> Result<()> {
        let mut settings = self.load()?;
        let field = settings
            .hotkeys
            .get_global_by_name_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Invalid hotkey name: {}", name))?;
        field.enabled = enabled;
        self.save(&settings)
    }

    /// Flip the per-editor-hotkey `enabled` switch without touching the stored
    /// combination.
    pub fn set_editor_hotkey_enabled_flag(&self, action_id: &str, enabled: bool) -> Result<()> {
        if !super::hotkeys::EditorHotkeySettings::is_valid_action_id(action_id) {
            return Err(anyhow::anyhow!("Invalid editor action: {}", action_id));
        }
        let mut settings = self.load()?;
        let field = settings
            .hotkeys
            .editor
            .get_mut_by_id(action_id)
            .ok_or_else(|| anyhow::anyhow!("Invalid editor action: {}", action_id))?;
        field.enabled = enabled;
        self.save(&settings)
    }

    /// Set an editor-scoped hotkey.
    ///
    /// Validates action id, checks for duplicates within the editor group and
    /// conflicts with global/main-window-local bindings. Saves settings and emits
    /// `settings-changed` but does NOT call `reregister_hotkeys` (editor hotkeys
    /// are never registered as global shortcuts).
    ///
    /// Empty binding is allowed and means disabled.
    pub fn set_editor_hotkey(
        &self,
        action_id: &str,
        hotkey: &super::hotkeys::Hotkey,
    ) -> Result<()> {
        use super::hotkeys::EditorHotkeySettings;
        if !EditorHotkeySettings::is_valid_action_id(action_id) {
            return Err(anyhow::anyhow!("Invalid editor action: {}", action_id));
        }

        let mut settings = self.load()?;

        // Check for duplicate within editor group
        if let Some(conflict) = settings.hotkeys.editor.find_duplicate(action_id, hotkey) {
            let label = editor_action_label(conflict);
            return Err(anyhow::anyhow!(
                "Этот хоткей уже используется для {}",
                label
            ));
        }

        // Check for conflicts with global / main-window-local bindings
        if hotkey.conflicts_with_global(&settings.hotkeys) {
            return Err(anyhow::anyhow!(
                "Этот хоткей конфликтует с назначенным глобальным хоткеем"
            ));
        }

        let field = settings
            .hotkeys
            .editor
            .get_mut_by_id(action_id)
            .ok_or_else(|| anyhow::anyhow!("Invalid editor action: {}", action_id))?;
        *field = hotkey.clone();

        self.save(&settings)
    }

    /// Reset an editor-scoped hotkey to its canonical default.
    ///
    /// The per-hotkey `enabled` switch is preserved: resetting a disabled
    /// hotkey restores its default combination but leaves it switched off.
    pub fn reset_editor_hotkey(&self, action_id: &str) -> Result<super::hotkeys::Hotkey> {
        use super::hotkeys::Hotkey;
        if !super::hotkeys::EditorHotkeySettings::is_valid_action_id(action_id) {
            return Err(anyhow::anyhow!("Invalid editor action: {}", action_id));
        }
        let mut default = match action_id {
            "edit_word" => Hotkey::default_edit_word(),
            "submit_continue" => Hotkey::default_submit_continue(),
            "submit_keep_text" => Hotkey::default_submit_keep_text(),
            "submit_keep_focus" => Hotkey::default_submit_keep_focus(),
            "next_spelling_error" => Hotkey::default_next_spelling_error(),
            "previous_spelling_error" => Hotkey::default_previous_spelling_error(),
            "next_tab" => Hotkey::default_next_tab(),
            "previous_tab" => Hotkey::default_previous_tab(),
            "cycle_route" => Hotkey::default_cycle_route(),
            "toggle_typing" => Hotkey::default_toggle_typing(),
            "cycle_quick_mode" => Hotkey::default_cycle_quick_mode(),
            "toggle_history" => Hotkey::default_toggle_history(),
            "accent_homographs" => Hotkey::default_accent_homographs(),
            "approve_next_incoming" => Hotkey::default_approve_next_incoming(),
            "edit_next_incoming" => Hotkey::default_edit_next_incoming(),
            _ => return Err(anyhow::anyhow!("Invalid editor action: {}", action_id)),
        };
        if let Some(current) = self.load()?.hotkeys.editor.get_by_id(action_id) {
            default.enabled = current.enabled;
        }
        self.set_editor_hotkey(action_id, &default)?;
        Ok(default)
    }

    /// Reset a hotkey to its default value.
    ///
    /// The per-hotkey `enabled` switch is preserved: resetting a disabled
    /// hotkey restores its default combination but leaves it switched off.
    ///
    /// # Arguments
    /// * `name` - Either "main_window" or "sound_panel"
    pub fn reset_hotkey_to_default(&self, name: &str) -> Result<super::hotkeys::Hotkey> {
        let mut default = match name {
            "main_window" => super::hotkeys::Hotkey::default_main_window(),
            "sound_panel" => super::hotkeys::Hotkey::default_sound_panel(),
            "playback_pause" => super::hotkeys::Hotkey::default_playback_pause(),
            "playback_stop" => super::hotkeys::Hotkey::default_playback_stop(),
            "playback_repeat" => super::hotkeys::Hotkey::default_playback_repeat(),
            "playback_control_window" => super::hotkeys::Hotkey::default_playback_control_window(),
            "return_previous_window" => super::hotkeys::Hotkey::default_return_previous_window(),
            "toggle_minimal_mode" => super::hotkeys::Hotkey::default_toggle_minimal_mode(),
            "ocr_capture" => super::hotkeys::Hotkey::default_ocr_capture(),
            _ => return Err(anyhow::anyhow!("Invalid hotkey name: {}", name)),
        };
        if let Some(current) = self.load()?.hotkeys.get_global_by_name(name) {
            default.enabled = current.enabled;
        }
        self.set_hotkey(name, &default)?;
        Ok(default)
    }

    // ========== Playback Control Window Settings ==========

    /// Get show playback control window on start
    pub fn get_show_playback_on_start(&self) -> bool {
        self.cache.read().show_playback_on_start
    }

    /// Set show playback control window on start
    pub fn set_show_playback_on_start(&self, value: bool) -> Result<()> {
        self.update_field("/show_playback_on_start", &value)
    }

    /// Set start in compact mode
    pub fn set_start_compact(&self, value: bool) -> Result<()> {
        self.update_field("/start_compact", &value)
    }

    /// Get hide main window on minimize
    pub fn get_hide_on_minimize(&self) -> bool {
        self.cache.read().hide_on_minimize
    }

    /// Set hide main window on minimize
    pub fn set_hide_on_minimize(&self, value: bool) -> Result<()> {
        self.update_field("/hide_on_minimize", &value)
    }

    // ========== VTube Studio Settings ==========

    pub fn get_vtube_studio_settings(&self) -> VTubeStudioSettings {
        self.cache.read().vtube_studio.clone()
    }

    pub fn set_vtube_studio_settings(&self, settings: &VTubeStudioSettings) -> Result<()> {
        let settings = settings.clone();
        self.update_settings_atomically(move |app_settings| {
            app_settings.vtube_studio = settings;
        })
    }

    pub fn set_vtube_studio_token(&self, token: Option<String>) -> Result<()> {
        self.update_field("/vtube_studio/token", &token)
    }

    // ========== VRChat Settings ==========

    pub fn set_vrchat_settings(&self, settings: &VrchatSettings) -> Result<()> {
        let settings = settings.clone();
        self.update_settings_atomically(move |app_settings| {
            app_settings.vrchat = settings;
        })
    }
}

/// Canonical settings for a fresh installation: defaults plus a fresh
/// input-server LAN access token.
///
/// The token is generated immediately even though the listener defaults to not
/// starting, so non-loopback access is ready when the user enables it.
pub(crate) fn fresh_default_settings() -> AppSettings {
    let mut settings = AppSettings::default();
    SettingsManager::ensure_input_server_access_token(&mut settings);
    settings
}

/// Atomically write canonical default settings to `path` and return them.
///
/// Used by the first-run path in `load_from_disk` and by the explicit
/// "restore standard settings" action of the recovery dialog (ROADMAP-123);
/// the caller must have already secured a backup when a previous file exists.
pub(crate) fn write_default_settings(path: &Path) -> Result<AppSettings> {
    let settings = fresh_default_settings();
    let content =
        serde_json::to_string_pretty(&settings).context("Failed to serialize settings")?;
    let _guard = persistence::config_write_lock().lock();
    persistence::write_json_atomically(path, &content).context("Failed to write settings file")?;
    Ok(settings)
}

/// Wrap a settings-load failure into structured recovery diagnostics.
///
/// The backup attempt happens here, once, at detection time: the source file
/// is copied byte-exactly and left in place, and the dialog reports the actual
/// outcome instead of an assumed success.
fn load_failure(
    path: &Path,
    stage: SettingsFailureStage,
    reason: String,
    line: Option<usize>,
    column: Option<usize>,
    raw_content: Option<&str>,
) -> anyhow::Error {
    anyhow::Error::new(SettingsLoadFailure::diagnose(
        path,
        stage,
        reason,
        line,
        column,
        raw_content,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twitch_settings_validation_returns_stable_field_codes() {
        let mut settings = TwitchSettings {
            username: "user".into(),
            token: "token".into(),
            channel: "channel".into(),
            ..TwitchSettings::default()
        };
        for login in [
            "",
            "@user",
            "user name",
            "юзер",
            "abcdefghijklmnopqrstuvwxyz",
        ] {
            settings.username = login.into();
            assert_eq!(settings.is_valid().unwrap_err(), "twitch.invalid_username");
        }
        settings.username = "Mixed_Case".into();
        settings.channel = "https://twitch.tv/channel".into();
        assert_eq!(settings.is_valid().unwrap_err(), "twitch.invalid_channel");
        settings.channel = "channel".into();
        settings.token = "   ".into();
        assert_eq!(settings.is_valid().unwrap_err(), "twitch.missing_token");
    }

    #[test]
    fn twitch_settings_validate_channel_name_without_url() {
        let mut settings = TwitchSettings {
            username: "user".into(),
            token: "token".into(),
            ..TwitchSettings::default()
        };
        for channel in ["user", "Mixed_Case123", "a", "abcdefghijklmnopqrstuvwxy"] {
            settings.channel = channel.into();
            assert!(settings.is_valid().is_ok(), "{channel}");
        }
        for channel in [
            "",
            "https://twitch.tv/user",
            "#user",
            " user",
            "user ",
            "a-b",
            "a\nb",
            "юзер",
            "abcdefghijklmnopqrstuvwxyz",
        ] {
            settings.channel = channel.into();
            assert!(settings.is_valid().is_err(), "{channel}");
        }
    }

    #[test]
    fn twitch_settings_mode_defaults_to_irc_and_validates_api_mode() {
        let legacy_json = r#"{"enabled": true}"#;
        let settings: TwitchSettings = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(settings.mode, TwitchMode::Irc);
        assert!(settings.is_valid().is_err());

        let api_json = r#"{"mode": "api", "enabled": true}"#;
        let api_settings: TwitchSettings = serde_json::from_str(api_json).unwrap();
        assert_eq!(api_settings.mode, TwitchMode::Api);
        assert!(api_settings.is_valid().is_ok());
    }

    #[test]
    fn audio_output_format_defaults_for_old_settings_and_rejects_unknown() {
        let mut value = serde_json::to_value(AudioSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("output_format");
        let old: AudioSettings = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(old.output_format, AudioOutputFormat::Default);
        value["output_format"] = serde_json::json!("f32");
        assert!(serde_json::from_value::<AudioSettings>(value).is_err());
    }

    #[test]
    fn audio_output_format_persists_and_updates_shared_cache() {
        let (manager, dir) = webview_section_tmp_manager("audio-output-format");
        let cache = manager.cache_arc();
        manager
            .set_audio_output_format(AudioOutputFormat::I32)
            .unwrap();
        assert_eq!(cache.read().audio.output_format, AudioOutputFormat::I32);
        let restarted = SettingsManager::with_config_dir(dir.clone()).unwrap();
        assert_eq!(
            restarted.load().unwrap().audio.output_format,
            AudioOutputFormat::I32
        );
        assert_eq!(
            read_disk_settings(&dir).audio.output_format,
            AudioOutputFormat::I32
        );
        assert_eq!(restarted.load().unwrap().audio.speaker_volume, 80);
        manager
            .set_audio_output_format(AudioOutputFormat::Default)
            .unwrap();
        assert_eq!(cache.read().audio.output_format, AudioOutputFormat::Default);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn audio_output_format_failed_write_preserves_cache() {
        let (manager, dir) = webview_section_tmp_manager("audio-output-format-failed");
        manager
            .set_audio_output_format(AudioOutputFormat::I32)
            .unwrap();
        let path = dir.join("settings.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(manager
            .set_audio_output_format(AudioOutputFormat::Default)
            .is_err());
        assert_eq!(
            manager.cache_arc().read().audio.output_format,
            AudioOutputFormat::I32
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Backwards-compatibility: an old settings.json written BEFORE the `deepseek`
    /// provider existed must still deserialize (the `deepseek` field is absent).
    /// This is the #[serde(default)] contract — same lesson as playback_pause /
    /// PhraseEntry. If this test fails, old configs panic on load.
    #[test]
    fn ai_settings_deserializes_without_deepseek_field() {
        // Old-format JSON: only openai/zai, no deepseek field.
        let old_json = r#"{
            "provider": "openai",
            "openai": { "api_key": "sk-test", "use_proxy": false, "model": "gpt-4o-mini" },
            "zai": { "url": null, "api_key": null, "model": "glm-4.5" },
            "prompt": "test prompt",
            "timeout": 20
        }"#;
        let settings: AiSettings = serde_json::from_str(old_json)
            .expect("old AiSettings (without deepseek) must deserialize");
        // deepseek falls back to default
        assert_eq!(settings.deepseek.model, "deepseek-chat");
        assert!(settings.deepseek.api_key.is_none());
        assert_eq!(settings.provider, AiProviderType::OpenAi);
    }

    /// Round-trip: AiSettings with deepseek set must serialize + deserialize back.
    #[test]
    fn ai_settings_deepseek_round_trip() {
        let original = AiSettings {
            provider: AiProviderType::DeepSeek,
            openai: AiOpenAiSettings::default(),
            zai: AiZAiSettings::default(),
            deepseek: AiDeepSeekSettings {
                api_key: Some("sk-deepseek".into()),
                use_proxy: true,
                model: "deepseek-chat".into(),
            },
            custom: AiCustomSettings::default(),
            prompt: default_ai_prompt(),
            timeout: default_ai_timeout(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: AiSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, AiProviderType::DeepSeek);
        assert_eq!(back.deepseek.api_key.as_deref(), Some("sk-deepseek"));
        assert!(back.deepseek.use_proxy);
    }

    /// Provider type serde must use lowercase (frontend sends "deepseek").
    #[test]
    fn ai_provider_type_serde_lowercase() {
        assert_eq!(
            serde_json::to_string(&AiProviderType::DeepSeek).unwrap(),
            "\"deepseek\""
        );
        let p: AiProviderType = serde_json::from_str("\"deepseek\"").unwrap();
        assert_eq!(p, AiProviderType::DeepSeek);
    }

    #[test]
    fn custom_ai_model_defaults_to_empty() {
        assert!(AiCustomSettings::default().model.is_empty());

        let settings: AiCustomSettings =
            serde_json::from_str(r#"{"url":null,"api_key":null,"use_proxy":false}"#)
                .expect("Custom AI settings without model must deserialize");
        assert!(settings.model.is_empty());
    }

    #[test]
    fn elevenlabs_settings_deserializes_from_empty_with_correct_defaults() {
        let el: ElevenLabsSettings = serde_json::from_str("{}").unwrap();
        assert!(el.api_key.is_none());
        assert!(el.voice_id.is_empty());
        assert!(el.model_id.is_empty());
        assert_eq!(el.output_format, "mp3_44100_128");
        assert_eq!(el.stability, 0.5);
        assert_eq!(el.similarity_boost, 0.75);
        assert_eq!(el.style, 0.0);
        assert!(el.use_speaker_boost);
        assert!(!el.use_proxy);
    }

    /// Old settings.json with `voices`/`models` inside `elevenlabs` must still
    /// deserialize (unknown fields are ignored) without migrating their values.
    #[test]
    fn elevenlabs_settings_ignores_legacy_voices_and_models() {
        let json = r#"{
            "api_key": "el-key",
            "voice_id": "voice-abc",
            "voices": [{ "voice_id": "voice-abc", "name": "Voice" }],
            "models": [{ "model_id": "eleven_multilingual_v2", "name": "Multilingual v2" }],
            "model_id": "eleven_multilingual_v2"
        }"#;
        let el: ElevenLabsSettings = serde_json::from_str(json)
            .expect("legacy ElevenLabsSettings with voices/models must deserialize");
        assert_eq!(el.api_key.as_deref(), Some("el-key"));
        assert_eq!(el.voice_id, "voice-abc");
        assert_eq!(el.model_id, "eleven_multilingual_v2");
    }

    #[test]
    fn tts_settings_deserializes_without_elevenlabs_field() {
        let json = r#"{
            "provider": "openai",
            "openai": { "api_key": null, "voice": "alloy", "proxy_host": null, "proxy_port": null, "use_proxy": false },
            "local": { "url": "http://127.0.0.1:8124" },
            "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" }
        }"#;
        let settings: TtsSettings = serde_json::from_str(json).unwrap();
        assert!(settings.elevenlabs.api_key.is_none());
        assert!(settings.elevenlabs.model_id.is_empty());
        assert_eq!(settings.elevenlabs.output_format, "mp3_44100_128");
    }

    #[test]
    fn elevenlabs_settings_round_trip() {
        let original = ElevenLabsSettings {
            api_key: Some("el-key".to_string()),
            voice_id: "voice-abc".to_string(),
            model_id: "eleven_multilingual_v2".to_string(),
            output_format: "mp3_44100_96".to_string(),
            stability: 0.3,
            similarity_boost: 0.8,
            style: 0.1,
            use_speaker_boost: false,
            use_proxy: true,
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: ElevenLabsSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original);
    }

    /// `ElevenLabsSettings` must never serialize `voices`/`models` into
    /// `settings.json`.
    #[test]
    fn elevenlabs_settings_serializes_without_catalogs() {
        let settings = ElevenLabsSettings::default();
        let json = serde_json::to_value(&settings).unwrap();
        assert!(json.get("voices").is_none());
        assert!(json.get("models").is_none());
    }

    // ==================== ElevenLabs catalog cache ====================

    fn el_voice(id: &str) -> ElevenLabsVoice {
        ElevenLabsVoice {
            voice_id: id.to_string(),
            name: id.to_string(),
            category: None,
            labels: vec![],
            preview_url: None,
            classification: None,
        }
    }

    fn el_model(id: &str, style: bool, speaker_boost: bool) -> ElevenLabsModel {
        ElevenLabsModel {
            model_id: id.to_string(),
            name: id.to_string(),
            can_use_style: style,
            can_use_speaker_boost: speaker_boost,
        }
    }

    fn elevenlabs_tmp_manager(label: &str) -> (SettingsManager, PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-elevenlabs-{}-{}-{}",
            label,
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        (manager, dir)
    }

    fn read_catalog_file(dir: &Path) -> ElevenLabsCatalogCache {
        let path = dir.join("elevenlabs-catalog.json");
        ElevenLabsCatalogCache::load_from_disk(&path)
    }

    /// Missing catalog file yields an empty cache without error.
    #[test]
    fn elevenlabs_catalog_missing_yields_empty() {
        let (manager, dir) = elevenlabs_tmp_manager("missing");
        assert!(manager.get_elevenlabs_catalog().voices.is_empty());
        assert!(manager.get_elevenlabs_catalog().models.is_empty());
        assert!(manager.get_elevenlabs_voices().is_empty());
        assert!(manager.get_elevenlabs_models().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Corrupt catalog file yields an empty cache without making startup fail.
    #[test]
    fn elevenlabs_catalog_corrupt_yields_empty() {
        let (manager, dir) = elevenlabs_tmp_manager("corrupt");
        std::fs::write(dir.join("elevenlabs-catalog.json"), "{{{not json").unwrap();
        assert!(manager.get_elevenlabs_catalog().voices.is_empty());
        assert!(manager.get_elevenlabs_catalog().models.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A missing cache file yields an empty cache carrying the current version,
    /// so a later write starts from the canonical format.
    #[test]
    fn elevenlabs_catalog_missing_yields_empty_with_current_version() {
        let (manager, dir) = elevenlabs_tmp_manager("missing-version");
        let cache = manager.get_elevenlabs_catalog();
        assert!(cache.voices.is_empty());
        assert!(cache.models.is_empty());
        assert_eq!(cache.version, ELEVENLABS_CATALOG_CURRENT_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A freshly written catalog cache is stamped with the current version on
    /// disk, regardless of which setter produced it.
    #[test]
    fn elevenlabs_catalog_newly_written_contains_current_version() {
        let (manager, dir) = elevenlabs_tmp_manager("write-version");

        manager.set_elevenlabs_voices(vec![el_voice("v1")]).unwrap();
        let cache = read_catalog_file(&dir);
        assert_eq!(cache.voices, vec![el_voice("v1")]);
        assert_eq!(cache.version, ELEVENLABS_CATALOG_CURRENT_VERSION);

        manager
            .set_elevenlabs_models(vec![el_model("m1", true, false)])
            .unwrap();
        let cache = read_catalog_file(&dir);
        assert_eq!(cache.models, vec![el_model("m1", true, false)]);
        assert_eq!(cache.version, ELEVENLABS_CATALOG_CURRENT_VERSION);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An on-disk cache stamped with an unsupported version (the historical
    /// derived-default `0` or any future version) is ignored: its payload is
    /// discarded and an empty current-version cache is returned, exactly as for
    /// corrupt content.
    #[test]
    fn elevenlabs_catalog_unsupported_version_is_ignored() {
        let (manager, dir) = elevenlabs_tmp_manager("unsupported-version");

        for unsupported in [0u32, ELEVENLABS_CATALOG_CURRENT_VERSION + 1, 999] {
            let legacy = ElevenLabsCatalogCache {
                version: unsupported,
                voices: vec![el_voice("v1")],
                models: vec![el_model("m1", true, false)],
            };
            std::fs::write(
                dir.join("elevenlabs-catalog.json"),
                serde_json::to_string_pretty(&legacy).unwrap(),
            )
            .unwrap();

            let cache = manager.get_elevenlabs_catalog();
            assert!(
                cache.voices.is_empty(),
                "voices of version {unsupported} cache must be discarded"
            );
            assert!(
                cache.models.is_empty(),
                "models of version {unsupported} cache must be discarded"
            );
            assert_eq!(
                cache.version, ELEVENLABS_CATALOG_CURRENT_VERSION,
                "fallback must carry the current version"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn elevenlabs_catalog_deserializes_voices_without_classification_field() {
        let json = r#"{
            "version": 1,
            "voices": [
                { "voice_id": "v1", "name": "Voice", "category": null, "labels": [], "preview_url": null }
            ],
            "models": []
        }"#;
        let cache: ElevenLabsCatalogCache = serde_json::from_str(json)
            .expect("legacy catalog without classification must deserialize");
        assert_eq!(cache.version, ELEVENLABS_CATALOG_CURRENT_VERSION);
        assert_eq!(cache.voices.len(), 1);
        assert_eq!(cache.voices[0].voice_id, "v1");
        assert!(cache.voices[0].classification.is_none());
    }

    #[test]
    fn elevenlabs_catalog_round_trips_classification() {
        use crate::tts::elevenlabs::ElevenLabsVoiceClassification;

        let mut voice = el_voice("v1");
        voice.classification = Some(ElevenLabsVoiceClassification::Library);
        let cache = ElevenLabsCatalogCache {
            version: ELEVENLABS_CATALOG_CURRENT_VERSION,
            voices: vec![voice],
            models: vec![],
        };
        let json = serde_json::to_string(&cache).unwrap();
        let back: ElevenLabsCatalogCache = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.voices[0].classification,
            Some(ElevenLabsVoiceClassification::Library)
        );

        let default = ElevenLabsCatalogCache {
            version: ELEVENLABS_CATALOG_CURRENT_VERSION,
            voices: vec![{
                let mut v = el_voice("v2");
                v.classification = Some(ElevenLabsVoiceClassification::Default);
                v
            }],
            models: vec![],
        };
        let back: ElevenLabsCatalogCache =
            serde_json::from_str(&serde_json::to_string(&default).unwrap()).unwrap();
        assert_eq!(
            back.voices[0].classification,
            Some(ElevenLabsVoiceClassification::Default)
        );
    }

    /// Setting voices must not affect models and vice versa.
    #[test]
    fn elevenlabs_catalog_persists_voices_and_models_independently() {
        let (manager, dir) = elevenlabs_tmp_manager("independent");

        manager.set_elevenlabs_voices(vec![el_voice("v1")]).unwrap();
        assert_eq!(manager.get_elevenlabs_voices(), vec![el_voice("v1")]);
        assert!(manager.get_elevenlabs_models().is_empty());

        manager
            .set_elevenlabs_models(vec![el_model("m1", true, false)])
            .unwrap();
        assert_eq!(manager.get_elevenlabs_voices(), vec![el_voice("v1")]);
        assert_eq!(
            manager.get_elevenlabs_models(),
            vec![el_model("m1", true, false)]
        );

        // Replacing voices keeps the models intact.
        manager
            .set_elevenlabs_voices(vec![el_voice("v2"), el_voice("v3")])
            .unwrap();
        assert_eq!(
            manager.get_elevenlabs_voices(),
            vec![el_voice("v2"), el_voice("v3")]
        );
        assert_eq!(
            manager.get_elevenlabs_models(),
            vec![el_model("m1", true, false)]
        );

        // The catalog file (not settings.json) holds the data.
        assert_eq!(
            read_catalog_file(&dir).voices,
            vec![el_voice("v2"), el_voice("v3")]
        );
        assert_eq!(
            read_catalog_file(&dir).models,
            vec![el_model("m1", true, false)]
        );

        let settings_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        let elevenlabs = settings_json
            .pointer("/tts/elevenlabs")
            .and_then(|v| v.as_object())
            .expect("elevenlabs object must exist in settings.json");
        assert!(
            !elevenlabs.contains_key("voices"),
            "settings.json must not contain the voices catalog"
        );
        assert!(
            !elevenlabs.contains_key("models"),
            "settings.json must not contain the models catalog"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Once loaded, the catalog is served from memory: mutating the file on
    /// disk after the first read must not change what subsequent reads return.
    #[test]
    fn elevenlabs_catalog_is_cached_after_first_read() {
        let (manager, dir) = elevenlabs_tmp_manager("cached-read");

        let initial = ElevenLabsCatalogCache {
            version: ELEVENLABS_CATALOG_CURRENT_VERSION,
            voices: vec![el_voice("v1")],
            models: vec![el_model("m1", true, false)],
        };
        std::fs::write(
            dir.join("elevenlabs-catalog.json"),
            serde_json::to_string_pretty(&initial).unwrap(),
        )
        .unwrap();

        assert_eq!(manager.get_elevenlabs_voices(), vec![el_voice("v1")]);
        assert_eq!(
            manager.get_elevenlabs_models(),
            vec![el_model("m1", true, false)]
        );

        // Mutate the file directly, bypassing the manager's setters.
        let changed = ElevenLabsCatalogCache {
            version: ELEVENLABS_CATALOG_CURRENT_VERSION,
            voices: vec![el_voice("v2")],
            models: vec![],
        };
        std::fs::write(
            dir.join("elevenlabs-catalog.json"),
            serde_json::to_string_pretty(&changed).unwrap(),
        )
        .unwrap();

        // The second read is served from the in-memory cache, not the disk.
        assert_eq!(manager.get_elevenlabs_voices(), vec![el_voice("v1")]);
        assert_eq!(
            manager.get_elevenlabs_models(),
            vec![el_model("m1", true, false)]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Catalog mutations performed through a cloned manager are immediately
    /// visible through the original manager: the catalog cache is shared across
    /// clones, matching the settings-cache sharing semantics.
    #[test]
    fn elevenlabs_catalog_cache_is_shared_with_cloned_managers() {
        let (manager, dir) = elevenlabs_tmp_manager("clone-shared");

        // Prime the catalog through the original manager.
        manager.set_elevenlabs_voices(vec![el_voice("v1")]).unwrap();
        manager
            .set_elevenlabs_models(vec![el_model("m1", true, false)])
            .unwrap();

        // Mutate both portions through a clone.
        let clone = manager.clone();
        clone
            .set_elevenlabs_voices(vec![el_voice("v2"), el_voice("v3")])
            .unwrap();
        clone
            .set_elevenlabs_models(vec![el_model("m2", false, true)])
            .unwrap();

        // The original observes the new catalog without restarting.
        assert_eq!(
            manager.get_elevenlabs_voices(),
            vec![el_voice("v2"), el_voice("v3")]
        );
        assert_eq!(
            manager.get_elevenlabs_models(),
            vec![el_model("m2", false, true)]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// API-key invalidation performed through a clone after the original has
    /// primed its catalog must be observed by the original as an empty catalog.
    #[test]
    fn elevenlabs_api_key_invalidation_through_clone_is_observed_by_original() {
        let (manager, dir) = elevenlabs_tmp_manager("clone-invalidate");

        manager.set_elevenlabs_voices(vec![el_voice("v1")]).unwrap();
        manager
            .set_elevenlabs_models(vec![el_model("m1", true, true)])
            .unwrap();
        assert_eq!(manager.get_elevenlabs_voices(), vec![el_voice("v1")]);

        let clone = manager.clone();
        let changed = clone.set_elevenlabs_api_key("new-key".to_string()).unwrap();
        assert!(changed, "a new key must report changed");

        assert!(manager.get_elevenlabs_voices().is_empty());
        assert!(manager.get_elevenlabs_models().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A changed (normalized) API key clears catalogs and selection.
    #[test]
    fn elevenlabs_api_key_change_invalidates_catalogs_and_selection() {
        let (manager, dir) = elevenlabs_tmp_manager("key-change");

        manager.set_elevenlabs_voices(vec![el_voice("v1")]).unwrap();
        manager
            .set_elevenlabs_models(vec![el_model("m1", true, true)])
            .unwrap();
        manager.set_elevenlabs_voice_id("v1".to_string()).unwrap();
        manager
            .set_elevenlabs_model_selection("m1".to_string(), 0.5, true)
            .unwrap();

        let changed = manager
            .set_elevenlabs_api_key("  new-key  ".to_string())
            .unwrap();
        assert!(changed, "a new key must report changed");

        assert_eq!(manager.get_elevenlabs_api_key().as_deref(), Some("new-key"));
        assert!(manager.get_elevenlabs_voices().is_empty());
        assert!(manager.get_elevenlabs_models().is_empty());
        assert_eq!(manager.get_elevenlabs_voice_id(), "");
        assert_eq!(manager.get_elevenlabs_model_id(), "");

        // settings.json must not carry catalogs.
        let settings: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert!(settings.tts.elevenlabs.voice_id.is_empty());
        assert!(settings.tts.elevenlabs.model_id.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An unchanged (normalized) key preserves catalogs and selection.
    #[test]
    fn elevenlabs_api_key_unchanged_preserves_catalogs_and_selection() {
        let (manager, dir) = elevenlabs_tmp_manager("key-unchanged");

        manager
            .set_elevenlabs_api_key("stable-key".to_string())
            .unwrap();
        manager.set_elevenlabs_voices(vec![el_voice("v1")]).unwrap();
        manager
            .set_elevenlabs_models(vec![el_model("m1", true, true)])
            .unwrap();
        manager.set_elevenlabs_voice_id("v1".to_string()).unwrap();
        manager
            .set_elevenlabs_model_selection("m1".to_string(), 0.5, true)
            .unwrap();

        // Same key with surrounding whitespace must be treated as unchanged.
        let changed = manager
            .set_elevenlabs_api_key("  stable-key  ".to_string())
            .unwrap();
        assert!(!changed, "an identical trimmed key must report unchanged");

        assert_eq!(manager.get_elevenlabs_voices(), vec![el_voice("v1")]);
        assert_eq!(
            manager.get_elevenlabs_models(),
            vec![el_model("m1", true, true)]
        );
        assert_eq!(manager.get_elevenlabs_voice_id(), "v1");
        assert_eq!(manager.get_elevenlabs_model_id(), "m1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Generation settings require a model present in the cached catalog.
    #[test]
    fn elevenlabs_generation_settings_require_known_model() {
        let (manager, dir) = elevenlabs_tmp_manager("gen-unknown-model");

        let result = manager.set_elevenlabs_generation_settings(
            "missing-model".to_string(),
            "mp3_44100_128".to_string(),
            0.5,
            0.75,
            0.0,
            true,
        );
        assert!(result.is_err(), "unknown model must be rejected");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Generation settings normalize style / speaker boost when the selected
    /// model does not support them.
    #[test]
    fn elevenlabs_generation_settings_normalize_capabilities() {
        let (manager, dir) = elevenlabs_tmp_manager("gen-normalize");

        manager
            .set_elevenlabs_models(vec![el_model("m1", false, false)])
            .unwrap();

        manager
            .set_elevenlabs_generation_settings(
                "m1".to_string(),
                "mp3_44100_128".to_string(),
                0.5,
                0.75,
                0.4,
                true,
            )
            .unwrap();

        let el = manager.get_elevenlabs_settings();
        assert_eq!(el.model_id, "m1");
        assert_eq!(el.output_format, "mp3_44100_128");
        assert_eq!(el.stability, 0.5);
        assert_eq!(el.similarity_boost, 0.75);
        assert_eq!(el.style, 0.0, "unsupported style must normalize to 0");
        assert!(
            !el.use_speaker_boost,
            "unsupported speaker boost must normalize to false"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A supported model keeps the submitted style / speaker boost values.
    #[test]
    fn elevenlabs_generation_settings_keep_supported_capabilities() {
        let (manager, dir) = elevenlabs_tmp_manager("gen-supported");

        manager
            .set_elevenlabs_models(vec![el_model("m1", true, true)])
            .unwrap();

        manager
            .set_elevenlabs_generation_settings(
                "m1".to_string(),
                "mp3_44100_96".to_string(),
                0.1,
                0.9,
                0.4,
                true,
            )
            .unwrap();

        let el = manager.get_elevenlabs_settings();
        assert_eq!(el.style, 0.4);
        assert!(el.use_speaker_boost);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The combined DTO exposes persisted settings plus cached catalogs.
    #[test]
    fn elevenlabs_settings_dto_combines_cached_catalogs() {
        let (manager, dir) = elevenlabs_tmp_manager("dto-combine");

        manager
            .set_elevenlabs_models(vec![el_model("m1", true, false)])
            .unwrap();
        manager.set_elevenlabs_voices(vec![el_voice("v1")]).unwrap();
        manager.set_elevenlabs_voice_id("v1".to_string()).unwrap();
        manager
            .set_elevenlabs_model_selection("m1".to_string(), 0.0, false)
            .unwrap();

        let dto = manager.get_elevenlabs_settings_dto();
        assert_eq!(dto.voice_id, "v1");
        assert_eq!(dto.model_id, "m1");
        assert_eq!(dto.voices.len(), 1);
        assert_eq!(dto.voices[0].voice_id, "v1");
        assert_eq!(dto.models.len(), 1);
        assert_eq!(dto.models[0].model_id, "m1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_updates_preserve_both_fields() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let config_dir = std::env::temp_dir().join(format!(
            "ttsbard-settings-test-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&config_dir).unwrap();

        let settings_path = config_dir.join("settings.json");
        let default_settings = AppSettings::default();
        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&default_settings).unwrap(),
        )
        .unwrap();

        let manager_a = SettingsManager {
            config_dir: config_dir.clone(),
            cache: Arc::new(RwLock::new(default_settings.clone())),
            elevenlabs_catalog_cache: Arc::new(std::sync::RwLock::new(None)),
        };
        let manager_b = SettingsManager {
            config_dir: config_dir.clone(),
            cache: Arc::new(RwLock::new(default_settings)),
            elevenlabs_catalog_cache: Arc::new(std::sync::RwLock::new(None)),
        };

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let barrier_a = barrier.clone();
        let barrier_b = barrier.clone();

        let handle_a = std::thread::spawn(move || {
            barrier_a.wait();
            manager_a.set_speaker_volume(33).unwrap();
        });
        let handle_b = std::thread::spawn(move || {
            barrier_b.wait();
            manager_b.set_show_playback_on_start(true).unwrap();
        });

        barrier.wait();
        handle_a.join().unwrap();
        handle_b.join().unwrap();

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: AppSettings = serde_json::from_str(&content).unwrap();

        assert_eq!(settings.audio.speaker_volume, 33);
        assert!(settings.show_playback_on_start);

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn openai_api_key_roundtrip() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let config_dir = std::env::temp_dir().join(format!(
            "ttsbard-openai-key-test-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&config_dir).unwrap();

        let settings_path = config_dir.join("settings.json");
        let default_settings = AppSettings::default();
        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&default_settings).unwrap(),
        )
        .unwrap();
        let manager = SettingsManager {
            config_dir: config_dir.clone(),
            cache: Arc::new(RwLock::new(default_settings.clone())),
            elevenlabs_catalog_cache: Arc::new(std::sync::RwLock::new(None)),
        };
        assert!(manager.get_openai_api_key().is_none());

        manager
            .set_openai_api_key(Some("sk-test-key-12345".to_string()))
            .unwrap();
        assert_eq!(
            manager.get_openai_api_key().as_deref(),
            Some("sk-test-key-12345")
        );

        manager.set_openai_api_key(None).unwrap();
        assert!(manager.get_openai_api_key().is_none());

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn malformed_settings_json_keeps_source_and_creates_backup() {
        let (config_dir, settings_path) = unique_test_config_dir("corrupt-settings");
        let corrupt = "{{{not valid json at all";
        std::fs::write(&settings_path, corrupt).unwrap();

        let error = SettingsManager::load_from_disk(&config_dir)
            .expect_err("corrupt settings must stop the ordinary load");

        let failure = error
            .downcast_ref::<SettingsLoadFailure>()
            .expect("failure must carry recovery diagnostics");
        assert_eq!(failure.stage, SettingsFailureStage::Syntax);
        assert_eq!(failure.line, Some(1));

        // The source file is untouched and no defaults replaced it.
        assert_eq!(
            std::fs::read_to_string(&settings_path).unwrap(),
            corrupt,
            "settings.json must stay exactly as the user left it"
        );

        // Exactly one backup exists and matches the source byte-for-byte.
        let backups = backup_files_in(&config_dir);
        assert_eq!(backups.len(), 1, "a single backup file should exist");
        assert_eq!(
            std::fs::read_to_string(&backups[0]).unwrap(),
            corrupt,
            "backup must preserve the corrupted bytes"
        );

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn empty_settings_json_keeps_source_and_creates_backup() {
        let (config_dir, settings_path) = unique_test_config_dir("empty-settings");
        std::fs::write(&settings_path, "").unwrap();

        let error = SettingsManager::load_from_disk(&config_dir)
            .expect_err("an empty settings file must stop the ordinary load");

        let failure = error
            .downcast_ref::<SettingsLoadFailure>()
            .expect("failure must carry recovery diagnostics");
        assert_eq!(failure.stage, SettingsFailureStage::Syntax);

        assert_eq!(
            std::fs::read_to_string(&settings_path).unwrap(),
            "",
            "settings.json must stay exactly as the user left it"
        );

        let backups = backup_files_in(&config_dir);
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), "");

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn incompatible_field_type_reports_sanitized_position() {
        let (config_dir, settings_path) = unique_test_config_dir("wrong-type");
        let secret = "sk-secret-token-value-77";
        // A known numeric field carrying a secret-looking string: the raw
        // serde message quotes that value, so the sanitized reason must not.
        let broken = format!(r#"{{"audio": {{"speaker_volume": "{secret}"}}}}"#);
        std::fs::write(&settings_path, &broken).unwrap();

        let error = SettingsManager::load_from_disk(&config_dir)
            .expect_err("an incompatible field type must stop the ordinary load");

        let failure = error
            .downcast_ref::<SettingsLoadFailure>()
            .expect("failure must carry recovery diagnostics");
        assert_eq!(failure.stage, SettingsFailureStage::Deserialize);
        assert!(
            !failure.reason.contains(secret),
            "diagnostics must not embed the offending value: {}",
            failure.reason
        );
        assert_eq!(
            failure.line,
            Some(1),
            "the streaming probe must report the offending position"
        );

        // The original file is preserved and backed up byte-for-byte.
        assert_eq!(std::fs::read_to_string(&settings_path).unwrap(), broken);
        let backups = backup_files_in(&config_dir);
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), broken);

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[cfg(windows)]
    #[test]
    fn migration_write_failure_returns_without_deadlock_and_preserves_source() {
        let (config_dir, settings_path) = unique_test_config_dir("migration-write-failure");
        let mut legacy = serde_json::to_value(fresh_default_settings()).unwrap();
        legacy.as_object_mut().unwrap().remove("ui_language");
        let original = serde_json::to_vec_pretty(&legacy).unwrap();
        std::fs::write(&settings_path, &original).unwrap();
        let original_permissions = std::fs::metadata(&settings_path).unwrap().permissions();
        let mut readonly = original_permissions.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&settings_path, readonly).unwrap();

        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_dir = config_dir.clone();
        let worker = std::thread::spawn(move || {
            let result = SettingsManager::load_from_disk(&worker_dir);
            let _ = sender.send(result);
        });
        let result = receiver.recv_timeout(std::time::Duration::from_secs(5));
        std::fs::set_permissions(&settings_path, original_permissions.clone()).unwrap();
        let backups = backup_files_in(&config_dir);
        // Windows copies the read-only attribute to the backup as well.
        for backup in &backups {
            std::fs::set_permissions(backup, original_permissions.clone()).unwrap();
        }
        let result = result.expect("migration failure must return instead of locking itself");
        worker.join().unwrap();
        let error = result.expect_err("read-only settings must reject migration writes");
        let failure = error.downcast_ref::<SettingsLoadFailure>().unwrap();
        assert_eq!(failure.stage, SettingsFailureStage::Write);
        assert!(matches!(
            failure.backup,
            crate::config::BackupOutcome::Created { .. }
        ));
        assert_eq!(std::fs::read(&settings_path).unwrap(), original);
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(&backups[0]).unwrap(), original);
        std::fs::remove_dir_all(&config_dir).unwrap();
    }

    #[test]
    fn missing_settings_file_first_run_writes_defaults() {
        let (config_dir, settings_path) = unique_test_config_dir("first-run");

        let settings = SettingsManager::load_from_disk(&config_dir)
            .expect("a missing settings file is the ordinary first-run path");

        // First-run settings are the defaults plus a fresh access token.
        let mut expected = AppSettings::default();
        let token = settings
            .input_server
            .access_token
            .clone()
            .expect("first run must provision the input-server access token");
        assert!(!token.is_empty());
        expected.input_server.access_token = Some(token);
        assert_eq!(settings, expected);

        let persisted: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap())
                .expect("first run must persist parseable defaults");
        assert_eq!(persisted, settings);
        assert!(backup_files_in(&config_dir).is_empty());

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn write_default_settings_produces_tokenized_parseable_file() {
        let (config_dir, settings_path) = unique_test_config_dir("write-defaults");

        let written = write_default_settings(&settings_path)
            .expect("default settings must write successfully");
        let parsed: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap())
                .expect("written defaults must parse back");
        assert_eq!(parsed, written);
        assert!(parsed
            .input_server
            .access_token
            .as_deref()
            .is_some_and(|token| !token.is_empty()));

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    fn unique_test_config_dir(tag: &str) -> (PathBuf, PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let config_dir = std::env::temp_dir().join(format!(
            "ttsbard-{}-test-{}-{}",
            tag,
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&config_dir).unwrap();
        let settings_path = config_dir.join("settings.json");
        (config_dir, settings_path)
    }

    fn backup_files_in(config_dir: &Path) -> Vec<PathBuf> {
        let mut backups: Vec<PathBuf> = std::fs::read_dir(config_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains(".bak.") && n.ends_with(".json"))
            })
            .collect();
        backups.sort();
        backups
    }

    #[test]
    fn persist_error_preserves_cache() {
        let (config_dir, settings_path) = unique_test_config_dir("persist-err");
        let default_settings = AppSettings::default();
        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&default_settings).unwrap(),
        )
        .unwrap();
        let cache = Arc::new(RwLock::new(default_settings));
        let manager = SettingsManager {
            config_dir: config_dir.clone(),
            cache: Arc::clone(&cache),
            elevenlabs_catalog_cache: Arc::new(std::sync::RwLock::new(None)),
        };

        manager.set_openai_voice("nova".to_string()).unwrap();
        assert_eq!(manager.get_openai_voice(), "nova");

        let bad_config_dir = config_dir.join("nonexistent_subdir");
        let bad_manager = SettingsManager {
            config_dir: bad_config_dir,
            cache,
            elevenlabs_catalog_cache: Arc::new(std::sync::RwLock::new(None)),
        };
        let result = bad_manager.set_openai_voice("alloy".to_string());
        assert!(result.is_err(), "persist to nonexistent dir must fail");
        assert_eq!(
            bad_manager.get_openai_voice(),
            "nova",
            "cache must retain previous value after persist error"
        );

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn set_webview_section_persist_error_preserves_cache_and_disk() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let config_dir = std::env::temp_dir().join(format!(
            "ttsbard-webview-persist-err-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&config_dir).unwrap();

        let settings_path = config_dir.join("settings.json");
        let mut settings = AppSettings::default();
        settings.webview.port = 8080;
        settings.webview.start_on_boot = true;
        settings.webview.upnp_enabled = false;
        settings.webview.send_original_text = false;
        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&settings).unwrap(),
        )
        .unwrap();

        let cache = Arc::new(RwLock::new(settings));

        // Write to a nonexistent subdir — rename will fail with "no such file"
        let bad_config_dir = config_dir.join("nonexistent_subdir");
        let bad_manager = SettingsManager {
            config_dir: bad_config_dir,
            cache: Arc::clone(&cache),
            elevenlabs_catalog_cache: Arc::new(std::sync::RwLock::new(None)),
        };

        let result = bad_manager.set_webview_section(false, 9999, true, true);
        assert!(result.is_err(), "persist to nonexistent dir must fail");

        let after_cache = bad_manager.load().unwrap();
        assert_eq!(
            after_cache.webview.port, 8080,
            "cache port must be unchanged"
        );
        assert!(
            after_cache.webview.start_on_boot,
            "cache start_on_boot must be unchanged"
        );
        assert!(
            !after_cache.webview.upnp_enabled,
            "cache upnp_enabled must be unchanged"
        );
        assert!(
            !after_cache.webview.send_original_text,
            "cache send_original_text must be unchanged"
        );

        let actual_disk = std::fs::read_to_string(&settings_path).unwrap();
        let disk_settings: AppSettings = serde_json::from_str(&actual_disk).unwrap();
        assert_eq!(
            disk_settings.webview.port, 8080,
            "disk port must be unchanged"
        );

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    /// Helper: build a SettingsManager over a fresh temp config dir.
    fn webview_section_tmp_manager(label: &str) -> (SettingsManager, PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-webview-section-{}-{}-{}",
            label,
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        (manager, dir)
    }

    fn read_disk_settings(config_dir: &Path) -> AppSettings {
        let content = std::fs::read_to_string(config_dir.join("settings.json")).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    #[test]
    fn set_webview_section_saves_four_fields_and_keeps_cache_in_sync() {
        let (manager, dir) = webview_section_tmp_manager("save-four");

        let result = manager.set_webview_section(false, 9090, false, true);
        assert!(
            result.is_ok(),
            "set_webview_section failed: {:?}",
            result.err()
        );

        let disk = read_disk_settings(&dir);
        assert!(!disk.webview.start_on_boot);
        assert_eq!(disk.webview.port, 9090);
        assert!(!disk.webview.upnp_enabled);
        assert!(disk.webview.send_original_text);

        let cache = manager.load().unwrap();
        assert_eq!(cache, disk, "in-memory cache and disk must be identical");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_webview_section_preserves_access_token_and_enabled() {
        let (manager, dir) = webview_section_tmp_manager("preserve");

        // Seed unrelated fields via the still-public point setters.
        manager
            .set_webview_access_token(Some("secret-token-123".to_string()))
            .unwrap();
        let before = manager.load().unwrap();
        assert!(!before.webview.enabled, "default enabled must be false");
        assert_eq!(
            before.webview.access_token,
            Some("secret-token-123".to_string())
        );

        manager
            .set_webview_section(false, 9090, true, true)
            .unwrap();

        let after = manager.load().unwrap();
        assert_eq!(
            after.webview.access_token,
            Some("secret-token-123".to_string()),
            "access_token must survive section save"
        );
        assert!(!after.webview.enabled, "enabled must survive section save");
        assert_eq!(after.webview.port, 9090);
        assert!(after.webview.upnp_enabled);
        assert!(after.webview.send_original_text);

        assert_eq!(read_disk_settings(&dir), after, "disk and cache must agree");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_webview_section_reads_snapshot_after_writer_lock() {
        let (manager, dir) = webview_section_tmp_manager("fresh-snapshot");
        manager
            .set_webview_access_token(Some("cached-token".to_string()))
            .unwrap();

        // Model a writer that has committed its disk value while its cache
        // publication is still pending under config_write_lock. A following
        // section transaction must use the serialized disk snapshot, not the
        // stale cache clone it had before acquiring the writer lock.
        let mut disk = read_disk_settings(&dir);
        disk.webview.access_token = Some("newer-disk-token".to_string());
        std::fs::write(
            dir.join("settings.json"),
            serde_json::to_string_pretty(&disk).unwrap(),
        )
        .unwrap();

        manager
            .set_webview_section(false, 9090, false, true)
            .unwrap();

        let after = read_disk_settings(&dir);
        assert_eq!(
            after.webview.access_token,
            Some("newer-disk-token".to_string())
        );
        assert_eq!(manager.load().unwrap(), after);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_webview_section_leaves_no_residual_temp_file() {
        let (manager, dir) = webview_section_tmp_manager("atomic-write");

        manager
            .set_webview_section(false, 2020, false, true)
            .unwrap();

        let tmp_files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".tmp")))
            .collect();
        assert!(
            tmp_files.is_empty(),
            "atomic write must not leave a .tmp file behind"
        );

        assert!(
            dir.join("settings.json").exists(),
            "settings.json must exist"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_webview_section_repeated_saves_keep_cache_and_disk_in_sync() {
        let (manager, dir) = webview_section_tmp_manager("consistency");

        for (port, upnp) in [(8080u16, false), (9090, false), (7070, true), (1024, true)] {
            manager.set_webview_section(true, port, upnp, true).unwrap();
            assert_eq!(
                manager.load().unwrap(),
                read_disk_settings(&dir),
                "cache and disk must match after save with port={}",
                port
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Backward-compat: old settings.json without `dsp` field must deserialize.
    #[test]
    fn settings_deserializes_without_dsp_field() {
        let old_json = r#"{
            "audio": { "speaker_device": null, "speaker_enabled": true, "speaker_volume": 80, "virtual_mic_device": null, "virtual_mic_volume": 100 },
            "tts": { "provider": "openai", "openai": { "api_key": "sk-test", "voice": "alloy" }, "local": { "url": "http://127.0.0.1:8124" }, "fish": { "api_key": null, "voices": [], "reference_id": "", "format": "mp3", "temperature": 0.7, "sample_rate": 44100, "use_proxy": false }, "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" }, "network": { "proxy": { "proxy_url": null }, "mtproxy": { "host": null, "port": 8888, "secret": null, "dc_id": null } } },
            "audio_effects": { "enabled": false, "pitch": 0, "speed": 0, "volume": 100, "enhance_enabled": false, "enhance_atten_db": 12.0, "formant_preserved": true },
            "hotkey_enabled": true,
            "editor": { "quick": false, "ai": false, "ai_completion": false, "spellcheck_enabled": true, "spellcheck_source": "offline", "editor_height": 340 },
            "theme": "dark",
            "twitch": { "enabled": false, "username": "", "token": "", "channel": "", "start_on_boot": false },
            "webview": { "enabled": false, "start_on_boot": false, "port": 10100, "bind_address": "0.0.0.0", "access_token": null, "upnp_enabled": false },
            "logging": { "enabled": false, "level": "info", "module_levels": {} },
            "ai": { "provider": "openai", "openai": { "api_key": null, "use_proxy": false, "model": "gpt-4o-mini" }, "zai": { "url": null, "api_key": null, "model": "glm-4.5" }, "deepseek": { "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "custom": { "url": null, "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "prompt": "test", "timeout": 20 },
            "hotkeys": { "main_window": { "modifiers": ["ctrl"], "key": "F12" }, "sound_panel": { "modifiers": ["alt"], "key": "F12" }, "playback_pause": { "modifiers": [], "key": "" }, "playback_stop": { "modifiers": [], "key": "" }, "playback_repeat": { "modifiers": [], "key": "" }, "playback_control_window": { "modifiers": [], "key": "" } },
            "show_playback_on_start": false
        }"#;
        let settings: AppSettings = serde_json::from_str(old_json)
            .expect("old AppSettings (without dsp field) must deserialize");
        // DSP falls back to default (all disabled)
        assert!(!settings.dsp.eq.enabled);
        assert!(!settings.dsp.compressor.enabled);
        assert!(!settings.dsp.limiter.enabled);
        assert_eq!(settings.dsp.eq.bands.len(), 3);
    }

    /// Backward-compat: old settings.json without `hide_on_minimize` field
    /// must deserialize with the field defaulting to `false`.
    #[test]
    fn settings_deserializes_without_hide_on_minimize_field() {
        let old_json = r#"{
            "audio": { "speaker_device": null, "speaker_enabled": true, "speaker_volume": 80, "virtual_mic_device": null, "virtual_mic_volume": 100 },
            "tts": { "provider": "openai", "openai": { "api_key": "sk-test", "voice": "alloy" }, "local": { "url": "http://127.0.0.1:8124" }, "fish": { "api_key": null, "voices": [], "reference_id": "", "format": "mp3", "temperature": 0.7, "sample_rate": 44100, "use_proxy": false }, "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" }, "network": { "proxy": { "proxy_url": null }, "mtproxy": { "host": null, "port": 8888, "secret": null, "dc_id": null } } },
            "audio_effects": { "enabled": false, "pitch": 0, "speed": 0, "volume": 100, "enhance_enabled": false, "enhance_atten_db": 12.0, "formant_preserved": true },
            "hotkey_enabled": true,
            "editor": { "quick": false, "ai": false, "ai_completion": false, "spellcheck_enabled": true, "spellcheck_source": "offline", "editor_height": 340 },
            "theme": "dark",
            "twitch": { "enabled": false, "username": "", "token": "", "channel": "", "start_on_boot": false },
            "webview": { "enabled": false, "start_on_boot": false, "port": 10100, "bind_address": "0.0.0.0", "access_token": null, "upnp_enabled": false },
            "logging": { "enabled": false, "level": "info", "module_levels": {} },
            "ai": { "provider": "openai", "openai": { "api_key": null, "use_proxy": false, "model": "gpt-4o-mini" }, "zai": { "url": null, "api_key": null, "model": "glm-4.5" }, "deepseek": { "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "custom": { "url": null, "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "prompt": "test", "timeout": 20 },
            "hotkeys": { "main_window": { "modifiers": ["ctrl"], "key": "F12" }, "sound_panel": { "modifiers": ["alt"], "key": "F12" }, "playback_pause": { "modifiers": [], "key": "" }, "playback_stop": { "modifiers": [], "key": "" }, "playback_repeat": { "modifiers": [], "key": "" }, "playback_control_window": { "modifiers": [], "key": "" } },
            "show_playback_on_start": false
        }"#;
        let settings: AppSettings = serde_json::from_str(old_json)
            .expect("old AppSettings (without hide_on_minimize field) must deserialize");
        assert!(!settings.hide_on_minimize);
    }

    #[test]
    fn twitch_settings_missing_send_original_text_defaults_to_true() {
        let parsed: super::TwitchSettings = serde_json::from_str(
            r#"{"enabled": false, "username": "", "token": "", "channel": "", "start_on_boot": false}"#,
        )
        .unwrap();
        assert!(parsed.send_original_text);
        assert!(super::TwitchSettings::default().send_original_text);
    }

    /// Focused contract for the renamed setting: the legacy experimental
    /// `minimize_to_taskbar` value is ignored (no migration), and an explicit
    /// `hide_on_minimize: true` survives a serde round-trip while unrelated
    /// fields are preserved.
    #[test]
    fn hide_on_minimize_ignores_legacy_field_and_round_trips() {
        // Legacy field present, new field absent -> new field defaults to false.
        let mut legacy = serde_json::to_value(AppSettings::default()).expect("serialize default");
        let obj = legacy.as_object_mut().expect("default must be an object");
        obj.remove("hide_on_minimize");
        obj.insert("minimize_to_taskbar".into(), serde_json::json!(false));
        let settings: AppSettings =
            serde_json::from_value(legacy).expect("legacy settings must deserialize");
        assert!(
            !settings.hide_on_minimize,
            "legacy minimize_to_taskbar=false must not enable hide_on_minimize"
        );

        // Explicit true survives a serde round-trip; unrelated field preserved.
        let explicit = AppSettings {
            hide_on_minimize: true,
            show_playback_on_start: true,
            ..AppSettings::default()
        };
        let round: AppSettings =
            serde_json::from_value(serde_json::to_value(&explicit).expect("serialize explicit"))
                .expect("round-trip deserialize");
        assert!(round.hide_on_minimize);
        assert!(round.show_playback_on_start);
    }

    // ==================== UI Language tests ====================

    /// Fresh default must be English ("en").
    #[test]
    fn ui_language_default_is_en() {
        assert_eq!(AppSettings::default().ui_language, "en");
    }

    /// An old settings.json missing `ui_language` deserializes to "ru".
    #[test]
    fn ui_language_deserializes_missing_field_to_ru() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value
            .as_object_mut()
            .expect("default settings must be an object")
            .remove("ui_language");
        let settings: AppSettings =
            serde_json::from_value(value).expect("missing ui_language must deserialize");
        assert_eq!(settings.ui_language, "ru");
    }

    /// Explicit valid tags survive a serde round-trip.
    #[test]
    fn ui_language_round_trips_explicit_tags() {
        for tag in ["en", "ru", "de", "pt-BR", "zh-Hant"] {
            let settings = AppSettings {
                ui_language: tag.to_string(),
                ..AppSettings::default()
            };
            let json = serde_json::to_string(&settings).unwrap();
            let back: AppSettings = serde_json::from_str(&json).unwrap();
            assert_eq!(back.ui_language, tag, "round-trip failed for {}", tag);
        }
    }

    /// A present-but-invalid value normalizes to "en" without discarding
    /// unrelated settings.
    #[test]
    fn ui_language_invalid_normalizes_to_en_without_resetting_other_settings() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        let obj = value.as_object_mut().unwrap();
        obj.insert("ui_language".into(), serde_json::json!("bad/../tag\\"));
        obj.insert("hide_on_minimize".into(), serde_json::json!(true));
        obj.insert("show_playback_on_start".into(), serde_json::json!(true));

        let settings: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(settings.ui_language, "en");
        assert!(settings.hide_on_minimize, "unrelated setting must survive");
        assert!(
            settings.show_playback_on_start,
            "unrelated setting must survive"
        );
    }

    /// Old file missing `ui_language` gets the canonical field written back to
    /// disk while preserving other settings.
    #[test]
    fn ui_language_migration_writes_canonical_field_to_disk() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-ui-lang-migration-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("ui_language");
        std::fs::write(
            dir.join("settings.json"),
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();

        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        assert_eq!(manager.get_ui_language(), "ru");

        let raw = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        let disk: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            disk.get("ui_language"),
            Some(&serde_json::json!("ru")),
            "canonical ui_language must be written back"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// set_ui_language persists a valid tag and rejects invalid input without
    /// mutating the cache.
    #[test]
    fn set_ui_language_persists_and_rejects_invalid() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-ui-lang-set-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();

        assert_eq!(manager.get_ui_language(), "en");

        manager.set_ui_language("pt-BR".to_string()).unwrap();
        assert_eq!(manager.get_ui_language(), "pt-BR");

        let disk: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(disk.ui_language, "pt-BR", "disk and cache must agree");

        let result = manager.set_ui_language("bad/../tag".to_string());
        assert!(result.is_err(), "invalid tag must be rejected");
        assert_eq!(
            manager.get_ui_language(),
            "pt-BR",
            "cache must be unchanged"
        );

        let result = manager.set_ui_language(String::new());
        assert!(result.is_err(), "empty tag must be rejected");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// set_ui_language follows the catalog tag rule: tags with empty segments or
    /// tags that cannot name a catalog file are rejected without persisting.
    #[test]
    fn set_ui_language_rejects_empty_segments_and_uncatalogable_tags() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-ui-lang-tags-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();

        for tag in ["en--BR", "-en", "en-", "a", "en_US", "en/a"] {
            let result = manager.set_ui_language(tag.to_string());
            assert!(result.is_err(), "tag {tag:?} must be rejected");
            assert_eq!(
                manager.get_ui_language(),
                "en",
                "cache must stay 'en' after rejecting {tag:?}"
            );
        }

        manager.set_ui_language("pt-BR".to_string()).unwrap();
        assert_eq!(manager.get_ui_language(), "pt-BR");
        manager.set_ui_language("zh-Hant".to_string()).unwrap();
        assert_eq!(manager.get_ui_language(), "zh-Hant");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A persist failure must leave both cache and disk consistent (no partial
    /// update).
    #[test]
    fn set_ui_language_persist_error_preserves_cache_and_disk() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let config_dir = std::env::temp_dir().join(format!(
            "ttsbard-ui-lang-persist-err-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&config_dir).unwrap();

        let settings_path = config_dir.join("settings.json");
        let default_settings = AppSettings::default();
        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&default_settings).unwrap(),
        )
        .unwrap();
        let cache = Arc::new(RwLock::new(default_settings));
        let manager = SettingsManager {
            config_dir: config_dir.clone(),
            cache: Arc::clone(&cache),
            elevenlabs_catalog_cache: Arc::new(std::sync::RwLock::new(None)),
        };

        manager.set_ui_language("de".to_string()).unwrap();
        assert_eq!(manager.get_ui_language(), "de");

        let bad_config_dir = config_dir.join("nonexistent_subdir");
        let bad_manager = SettingsManager {
            config_dir: bad_config_dir,
            cache,
            elevenlabs_catalog_cache: Arc::new(std::sync::RwLock::new(None)),
        };
        let result = bad_manager.set_ui_language("fr".to_string());
        assert!(result.is_err(), "persist to nonexistent dir must fail");
        assert_eq!(
            bad_manager.get_ui_language(),
            "de",
            "cache must retain previous value after persist error"
        );

        let disk: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(disk.ui_language, "de", "disk must retain previous value");

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    /// Backward-compat: old settings.json without `boundary_cleanup_enabled`
    /// must deserialize with the field defaulting to `true`.
    #[test]
    fn audio_effects_deserializes_without_boundary_cleanup_field() {
        let old_json = r#"{
            "enabled": false,
            "pitch": 0,
            "speed": 0,
            "volume": 100,
            "enhance_enabled": false,
            "enhance_atten_db": 12.0,
            "formant_preserved": true
        }"#;
        let settings: AudioEffectsSettings = serde_json::from_str(old_json)
            .expect("old AudioEffectsSettings (without boundary_cleanup_enabled) must deserialize");
        assert!(settings.boundary_cleanup_enabled);
    }

    /// Round-trip: AudioEffectsSettings with boundary_cleanup_enabled = true/false
    /// must serialize + deserialize correctly.
    #[test]
    fn audio_effects_boundary_cleanup_round_trip() {
        let original = AudioEffectsSettings {
            enabled: false,
            pitch: 10,
            speed: -20,
            volume: 100,
            enhance_enabled: true,
            enhance_atten_db: 15.0,
            formant_preserved: false,
            boundary_cleanup_enabled: false,
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: AudioEffectsSettings = serde_json::from_str(&json).unwrap();
        assert!(!back.boundary_cleanup_enabled);
        assert_eq!(back.pitch, 10);
        assert_eq!(back.enhance_atten_db, 15.0);

        let original_true = AudioEffectsSettings {
            boundary_cleanup_enabled: true,
            ..AudioEffectsSettings::default()
        };
        let json_true = serde_json::to_string(&original_true).unwrap();
        let back_true: AudioEffectsSettings = serde_json::from_str(&json_true).unwrap();
        assert!(back_true.boundary_cleanup_enabled);
    }

    /// Default AudioEffectsSettings must have boundary_cleanup_enabled = true.
    #[test]
    fn default_audio_effects_boundary_cleanup_enabled() {
        let s = AudioEffectsSettings::default();
        assert!(s.boundary_cleanup_enabled);
    }

    /// Backward-compat: old settings.json without `provider_id` field in tts
    /// must deserialize with provider_id defaulting to None.
    #[test]
    fn tts_settings_deserializes_without_provider_id() {
        let old_json = r#"{
            "provider": "openai",
            "openai": { "api_key": null, "voice": "alloy" },
            "local": { "url": "http://127.0.0.1:8124" },
            "fish": { "api_key": null, "voices": [], "reference_id": "", "format": "mp3", "temperature": 0.7, "sample_rate": 44100, "use_proxy": false },
            "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" },
            "network": { "proxy": { "proxy_url": null }, "mtproxy": { "host": null, "port": 8888, "secret": null, "dc_id": null } }
        }"#;
        let settings: TtsSettings = serde_json::from_str(old_json)
            .expect("TtsSettings without provider_id must deserialize");
        assert_eq!(settings.provider_id, None);
        assert_eq!(settings.provider, TtsProviderType::OpenAi);
    }

    /// Backward-compat: old settings.json without `visible_provider_ids` field
    /// must deserialize with the default `["silero"]`.
    #[test]
    fn tts_settings_deserializes_without_visible_provider_ids() {
        let old_json = r#"{
            "provider": "openai",
            "openai": { "api_key": null, "voice": "alloy" },
            "local": { "url": "http://127.0.0.1:8124" },
            "fish": { "api_key": null, "voices": [], "reference_id": "", "format": "mp3", "temperature": 0.7, "sample_rate": 44100, "use_proxy": false },
            "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" },
            "network": { "proxy": { "proxy_url": null }, "mtproxy": { "host": null, "port": 8888, "secret": null, "dc_id": null } }
        }"#;
        let settings: TtsSettings = serde_json::from_str(old_json)
            .expect("TtsSettings without visible_provider_ids must deserialize");
        assert_eq!(settings.visible_provider_ids, vec!["silero".to_string()]);
    }

    /// Fresh default TtsSettings contains exactly `silero` as the visible provider.
    #[test]
    fn tts_settings_default_visible_provider_ids_is_silero() {
        assert_eq!(
            TtsSettings::default().visible_provider_ids,
            vec!["silero".to_string()]
        );
    }

    /// Round-trip: visible_provider_ids persists through serialize/deserialize.
    #[test]
    fn tts_settings_visible_provider_ids_round_trip() {
        let settings = TtsSettings {
            visible_provider_ids: vec![
                "silero".to_string(),
                "openai".to_string(),
                "fish".to_string(),
            ],
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: TtsSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.visible_provider_ids,
            vec![
                "silero".to_string(),
                "openai".to_string(),
                "fish".to_string()
            ]
        );
    }

    /// set_visible_tts_provider_ids atomically replaces the complete list and
    /// keeps disk/cache in sync without touching any other TTS setting.
    #[test]
    fn set_visible_tts_provider_ids_replaces_and_persists() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let config_dir = std::env::temp_dir().join(format!(
            "ttsbard-visible-provider-ids-test-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&config_dir).unwrap();
        let manager = SettingsManager::with_config_dir(config_dir.clone()).unwrap();

        let before = manager.load().unwrap();
        assert_eq!(before.tts.visible_provider_ids, vec!["silero".to_string()]);
        let before_provider = before.tts.provider;
        let before_provider_id = before.tts.provider_id.clone();

        manager
            .set_visible_tts_provider_ids(vec!["fish".to_string(), "local-http".to_string()])
            .unwrap();

        let after = manager.load().unwrap();
        assert_eq!(
            after.tts.visible_provider_ids,
            vec!["fish".to_string(), "local-http".to_string()]
        );
        assert_eq!(after.tts.provider, before_provider);
        assert_eq!(after.tts.provider_id, before_provider_id);

        let disk: AppSettings = serde_json::from_str(
            &std::fs::read_to_string(config_dir.join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(disk, after, "disk and cache must agree");

        // Complete replacement, not append.
        manager
            .set_visible_tts_provider_ids(vec!["silero".to_string()])
            .unwrap();
        assert_eq!(
            manager.load().unwrap().tts.visible_provider_ids,
            vec!["silero".to_string()]
        );

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    // ==================== HomographAccentorSettings migration ====================

    /// Backward-compat: an old settings.json that carried the Python sidecar
    /// fields (`contextual`) must still deserialize. Unknown fields are ignored,
    /// so removing `ContextualRuAccentSettings` never blocks startup.
    #[test]
    fn homograph_accentor_deserializes_without_sidecar_fields() {
        let old_json = r#"{
            "enabled": true,
            "accentor_pack_id": "com.example.ruaccent",
            "contextual": {
                "enabled": true,
                "python_executable": "C:/python/python.exe",
                "model_dir": "C:/models/ruaccent",
                "model": "tiny"
            }
        }"#;
        let settings: HomographAccentorSettings = serde_json::from_str(old_json)
            .expect("old HomographAccentorSettings with sidecar fields must deserialize");
        assert!(settings.enabled);
        assert_eq!(
            settings.accentor_pack_id.as_deref(),
            Some("com.example.ruaccent")
        );
    }

    /// The new shape no longer serializes the removed Python sidecar fields.
    #[test]
    fn homograph_accentor_serializes_without_sidecar_fields() {
        let settings = HomographAccentorSettings {
            enabled: true,
            accentor_pack_id: Some("com.example.ruaccent".to_string()),
            load_on_start: false,
        };
        let json = serde_json::to_value(&settings).unwrap();
        assert!(json.get("contextual").is_none());
        assert!(json.get("python_executable").is_none());
        assert!(json.get("model_dir").is_none());
        assert!(json.get("model").is_none());
        assert_eq!(
            json.get("load_on_start"),
            Some(&serde_json::Value::Bool(false))
        );
    }

    /// `load_on_start` defaults to false when absent from the JSON.
    #[test]
    fn homograph_accentor_load_on_start_defaults_false() {
        let settings: HomographAccentorSettings = serde_json::from_str(
            r#"{ "enabled": true, "accentor_pack_id": "com.example.ruaccent" }"#,
        )
        .expect("must deserialize without load_on_start");
        assert!(!settings.load_on_start);
    }

    /// `load_on_start` round-trips through serialization.
    #[test]
    fn homograph_accentor_load_on_start_round_trips() {
        let settings = HomographAccentorSettings {
            enabled: true,
            accentor_pack_id: Some("com.example.ruaccent".to_string()),
            load_on_start: true,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let parsed: HomographAccentorSettings = serde_json::from_str(&json).expect("round-trip");
        assert!(parsed.load_on_start);
    }

    // ==================== QuickEditorMode tests ====================

    /// Backward-compat: old settings.json with `quick: false` → Disabled.
    #[test]
    fn quick_editor_mode_deserializes_bool_false() {
        let json = r#"{"quick":false,"ai":false,"ai_completion":false,"spellcheck_enabled":true,"spellcheck_source":"offline","editor_height":340}"#;
        let settings: EditorSettings =
            serde_json::from_str(json).expect("bool false must deserialize");
        assert_eq!(settings.quick, QuickEditorMode::Disabled);
    }

    /// Backward-compat: old settings.json with `quick: true` → Collapse.
    #[test]
    fn quick_editor_mode_deserializes_bool_true() {
        let json = r#"{"quick":true,"ai":false,"ai_completion":false,"spellcheck_enabled":true,"spellcheck_source":"offline","editor_height":340}"#;
        let settings: EditorSettings =
            serde_json::from_str(json).expect("bool true must deserialize");
        assert_eq!(settings.quick, QuickEditorMode::Collapse);
    }

    /// New format: deserialize each string variant.
    #[test]
    fn quick_editor_mode_deserializes_string_variants() {
        for (json_str, expected) in [
            (r#""disabled""#, QuickEditorMode::Disabled),
            (r#""collapse""#, QuickEditorMode::Collapse),
            (r#""return_focus""#, QuickEditorMode::ReturnFocus),
        ] {
            let mode: QuickEditorMode =
                serde_json::from_str(json_str).unwrap_or_else(|e| panic!("{}: {}", json_str, e));
            assert_eq!(mode, expected, "mismatch for {}", json_str);
        }
    }

    /// Serialize all variants to correct lowercase strings.
    #[test]
    fn quick_editor_mode_serializes_string() {
        assert_eq!(
            serde_json::to_string(&QuickEditorMode::Disabled).unwrap(),
            r#""disabled""#
        );
        assert_eq!(
            serde_json::to_string(&QuickEditorMode::Collapse).unwrap(),
            r#""collapse""#
        );
        assert_eq!(
            serde_json::to_string(&QuickEditorMode::ReturnFocus).unwrap(),
            r#""return_focus""#
        );
    }

    /// Default is Disabled.
    #[test]
    fn quick_editor_mode_default_is_disabled() {
        assert_eq!(QuickEditorMode::default(), QuickEditorMode::Disabled);
        let settings = EditorSettings::default();
        assert_eq!(settings.quick, QuickEditorMode::Disabled);
    }

    /// Round-trip: serialize EditorSettings with the new enum, deserialize back.
    #[test]
    fn editor_settings_quick_mode_round_trip() {
        let original = EditorSettings {
            quick: QuickEditorMode::ReturnFocus,
            ..EditorSettings::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: EditorSettings = serde_json::from_str(&json).expect("round-trip deserialization");
        assert_eq!(back.quick, QuickEditorMode::ReturnFocus);
        assert!(json.contains(r#""quick":"return_focus""#));
    }

    /// as_str / from_str helpers.
    #[test]
    fn quick_editor_mode_str_helpers() {
        for mode in [
            QuickEditorMode::Disabled,
            QuickEditorMode::Collapse,
            QuickEditorMode::ReturnFocus,
        ] {
            let s = mode.as_str();
            let parsed = QuickEditorMode::from_str(s).expect("from_str round-trip");
            assert_eq!(parsed, mode);
        }
        assert!(QuickEditorMode::from_str("bogus").is_none());
    }

    /// Backward-compat: EditorSettings without typing_idle_timeout_ms field defaults to 800.
    #[test]
    fn editor_settings_deserializes_without_typing_idle_timeout() {
        let json = r#"{"quick":false,"ai":false,"ai_completion":false,"spellcheck_enabled":true,"spellcheck_source":"offline","editor_height":340}"#;
        let settings: EditorSettings =
            serde_json::from_str(json).expect("must deserialize without typing_idle_timeout_ms");
        assert_eq!(settings.typing_idle_timeout_ms, 800);
    }

    /// EditorSettings default is 800.
    #[test]
    fn editor_settings_default_typing_timeout_is_800() {
        let s = EditorSettings::default();
        assert_eq!(s.typing_idle_timeout_ms, 800);
    }

    /// EditorSettings serializes typing_idle_timeout_ms.
    #[test]
    fn editor_settings_serializes_typing_idle_timeout() {
        let s = EditorSettings::default();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("typing_idle_timeout_ms"), "json: {}", json);
    }

    /// EditorSettings default has typing_enabled == true.
    #[test]
    fn editor_settings_default_typing_enabled_is_true() {
        assert!(EditorSettings::default().typing_enabled);
    }

    /// Backward-compat: EditorSettings without typing_enabled field defaults to true.
    #[test]
    fn editor_settings_deserializes_without_typing_enabled() {
        let json = r#"{"quick":false,"ai":false,"ai_completion":false,"spellcheck_enabled":true,"spellcheck_source":"offline","editor_height":340,"typing_idle_timeout_ms":800}"#;
        let settings: EditorSettings =
            serde_json::from_str(json).expect("must deserialize without typing_enabled");
        assert!(settings.typing_enabled);
    }

    /// EditorSettings default has autocomplete_enabled == true.
    #[test]
    fn editor_settings_default_autocomplete_enabled_is_true() {
        assert!(EditorSettings::default().autocomplete_enabled);
    }

    /// Backward-compat: EditorSettings without autocomplete_enabled field defaults to true.
    #[test]
    fn editor_settings_deserializes_without_autocomplete_enabled() {
        let json = r#"{"quick":false,"ai":false,"ai_completion":false,"spellcheck_enabled":true,"spellcheck_source":"offline","editor_height":340,"typing_idle_timeout_ms":800,"typing_enabled":true}"#;
        let settings: EditorSettings =
            serde_json::from_str(json).expect("must deserialize without autocomplete_enabled");
        assert!(settings.autocomplete_enabled);
    }

    /// EditorSettings: autocomplete_enabled round-trip.
    #[test]
    fn editor_settings_autocomplete_enabled_round_trip() {
        let original = EditorSettings {
            autocomplete_enabled: false,
            ..EditorSettings::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: EditorSettings = serde_json::from_str(&json).expect("round-trip deserialization");
        assert!(!back.autocomplete_enabled);
        assert!(json.contains(r#""autocomplete_enabled":false"#));
    }

    /// EditorSettings default converts numbers.
    #[test]
    fn editor_settings_default_number_conversion_enabled_is_true() {
        assert!(EditorSettings::default().number_conversion_enabled);
    }

    /// Backward-compat: legacy EditorSettings without number_conversion_enabled
    /// (or any newer field) defaults to enabled.
    #[test]
    fn editor_settings_deserializes_without_number_conversion_enabled() {
        let json = r#"{"quick":false,"ai":false,"ai_completion":false,"spellcheck_enabled":true,"spellcheck_source":"offline","editor_height":340}"#;
        let settings: EditorSettings = serde_json::from_str(json)
            .expect("must deserialize legacy settings without number_conversion_enabled");
        assert!(settings.number_conversion_enabled);
    }

    /// EditorSettings: number_conversion_enabled=false round-trips.
    #[test]
    fn editor_settings_number_conversion_enabled_round_trip() {
        let original = EditorSettings {
            number_conversion_enabled: false,
            ..EditorSettings::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: EditorSettings = serde_json::from_str(&json).expect("round-trip deserialization");
        assert!(!back.number_conversion_enabled);
        assert!(json.contains(r#""number_conversion_enabled":false"#));
    }

    /// EditorSettings default has keep_text_after_send == false.
    #[test]
    fn editor_settings_default_keep_text_is_false() {
        assert!(!EditorSettings::default().keep_text_after_send);
    }

    /// Backward-compat: EditorSettings without keep_text_after_send field defaults to false.
    #[test]
    fn editor_settings_deserializes_without_keep_text() {
        let json = r#"{"quick":false,"ai":false,"ai_completion":false,"spellcheck_enabled":true,"spellcheck_source":"offline","editor_height":340,"typing_idle_timeout_ms":800,"typing_enabled":true}"#;
        let settings: EditorSettings =
            serde_json::from_str(json).expect("must deserialize without keep_text_after_send");
        assert!(!settings.keep_text_after_send);
    }

    /// EditorSettings: keep_text_after_send round-trip.
    #[test]
    fn editor_settings_keep_text_round_trip() {
        let original = EditorSettings {
            keep_text_after_send: true,
            ..EditorSettings::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: EditorSettings = serde_json::from_str(&json).expect("round-trip deserialization");
        assert!(back.keep_text_after_send);
        assert!(json.contains(r#""keep_text_after_send":true"#));
    }

    /// Backward-compat: EditorSettings without homograph_accentor field defaults
    /// to disabled + no selected pack id.
    #[test]
    fn editor_settings_deserializes_without_homograph_accentor() {
        let json = r#"{"quick":false,"ai":false,"ai_completion":false,"spellcheck_enabled":true,"spellcheck_source":"offline","editor_height":340}"#;
        let settings: EditorSettings =
            serde_json::from_str(json).expect("must deserialize without homograph_accentor");
        assert!(!settings.homograph_accentor.enabled);
        assert_eq!(settings.homograph_accentor.accentor_pack_id, None);
    }

    /// EditorSettings default has homograph_accentor disabled with no pack id.
    #[test]
    fn editor_settings_default_homograph_accentor_disabled() {
        let s = EditorSettings::default();
        assert!(!s.homograph_accentor.enabled);
        assert!(s.homograph_accentor.accentor_pack_id.is_none());
    }

    /// set_editor_homograph_accentor persists both fields and keeps cache/disk in sync.
    #[test]
    fn set_editor_homograph_accentor_persists_and_round_trips() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let config_dir = std::env::temp_dir().join(format!(
            "ttsbard-homograph-accentor-test-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&config_dir).unwrap();
        let manager = SettingsManager::with_config_dir(config_dir.clone()).unwrap();

        assert!(!manager.load().unwrap().editor.homograph_accentor.enabled);

        manager
            .set_editor_homograph_accentor(true, Some("com.example.ruaccent".to_string()))
            .unwrap();
        let after = manager.load().unwrap();
        assert!(after.editor.homograph_accentor.enabled);
        assert_eq!(
            after.editor.homograph_accentor.accentor_pack_id.as_deref(),
            Some("com.example.ruaccent")
        );

        // Disable but preserve the selected id for a later re-enable.
        manager
            .set_editor_homograph_accentor(false, Some("com.example.ruaccent".to_string()))
            .unwrap();
        let after = manager.load().unwrap();
        assert!(!after.editor.homograph_accentor.enabled);
        assert_eq!(
            after.editor.homograph_accentor.accentor_pack_id.as_deref(),
            Some("com.example.ruaccent")
        );

        let _ = std::fs::remove_dir_all(&config_dir);
    }

    /// normalize_typing_idle_timeout_ms: default value 800 passes through.
    #[test]
    fn normalize_typing_idle_timeout_default() {
        assert_eq!(normalize_typing_idle_timeout_ms(800), 800);
    }

    /// normalize_typing_idle_timeout_ms: below min is clamped to 200.
    #[test]
    fn normalize_typing_idle_timeout_below_min() {
        assert_eq!(normalize_typing_idle_timeout_ms(100), 200);
        assert_eq!(normalize_typing_idle_timeout_ms(0), 200);
    }

    /// normalize_typing_idle_timeout_ms: above max is clamped to 5000.
    #[test]
    fn normalize_typing_idle_timeout_above_max() {
        assert_eq!(normalize_typing_idle_timeout_ms(10000), 5000);
        assert_eq!(normalize_typing_idle_timeout_ms(u32::MAX), 5000);
    }

    /// normalize_typing_idle_timeout_ms: boundary values pass through.
    #[test]
    fn normalize_typing_idle_timeout_boundaries() {
        assert_eq!(
            normalize_typing_idle_timeout_ms(TYPING_IDLE_TIMEOUT_MIN_MS),
            200
        );
        assert_eq!(
            normalize_typing_idle_timeout_ms(TYPING_IDLE_TIMEOUT_MAX_MS),
            5000
        );
    }

    /// AppSettings without typing_idle_timeout_ms in editor defaults to 800.
    #[test]
    fn app_settings_deserializes_old_editor_without_typing_timeout() {
        let old_json = r#"{
            "audio": { "speaker_device": null, "speaker_enabled": true, "speaker_volume": 80, "virtual_mic_device": null, "virtual_mic_volume": 100 },
            "tts": { "provider": "openai", "openai": { "api_key": null, "voice": "alloy" }, "local": { "url": "http://127.0.0.1:8124" }, "fish": { "api_key": null, "voices": [], "reference_id": "", "format": "mp3", "temperature": 0.7, "sample_rate": 44100, "use_proxy": false }, "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" }, "network": { "proxy": { "proxy_url": null }, "mtproxy": { "host": null, "port": 8888, "secret": null, "dc_id": null } } },
            "audio_effects": { "enabled": false, "pitch": 0, "speed": 0, "volume": 100, "enhance_enabled": false, "enhance_atten_db": 12.0, "formant_preserved": true },
            "hotkey_enabled": true,
            "editor": { "quick": false, "ai": false, "ai_completion": false, "spellcheck_enabled": true, "spellcheck_source": "offline", "editor_height": 340 },
            "theme": "dark",
            "twitch": { "enabled": false, "username": "", "token": "", "channel": "", "start_on_boot": false },
            "webview": { "enabled": false, "start_on_boot": false, "port": 10100, "bind_address": "0.0.0.0", "access_token": null, "upnp_enabled": false },
            "logging": { "enabled": false, "level": "info", "module_levels": {} },
            "ai": { "provider": "openai", "openai": { "api_key": null, "use_proxy": false, "model": "gpt-4o-mini" }, "zai": { "url": null, "api_key": null, "model": "glm-4.5" }, "deepseek": { "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "custom": { "url": null, "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "prompt": "test", "timeout": 20 },
            "hotkeys": { "main_window": { "modifiers": ["ctrl"], "key": "F12" }, "sound_panel": { "modifiers": ["alt"], "key": "F12" }, "playback_pause": { "modifiers": [], "key": "" }, "playback_stop": { "modifiers": [], "key": "" }, "playback_repeat": { "modifiers": [], "key": "" }, "playback_control_window": { "modifiers": [], "key": "" } },
            "show_playback_on_start": false
        }"#;
        let settings: AppSettings = serde_json::from_str(old_json)
            .expect("old AppSettings without typing_idle_timeout_ms must deserialize");
        assert_eq!(settings.editor.typing_idle_timeout_ms, 800);
    }

    // ==================== EditorRoute tests ====================

    /// EditorRoute: unknown value normalizes to Everywhere.
    #[test]
    fn editor_route_deserializes_unknown_to_everywhere() {
        let route: EditorRoute =
            serde_json::from_str(r#""bogus""#).expect("unknown route must deserialize");
        assert_eq!(route, EditorRoute::Everywhere);
        let route2: EditorRoute =
            serde_json::from_str(r#""some_future_value""#).expect("unknown route must deserialize");
        assert_eq!(route2, EditorRoute::Everywhere);
    }

    /// EditorRoute: all variants round-trip through snake_case strings.
    #[test]
    fn editor_route_round_trip() {
        for route in [
            EditorRoute::Everywhere,
            EditorRoute::NoTwitch,
            EditorRoute::VoiceOnly,
            EditorRoute::TwitchOnly,
        ] {
            let json = serde_json::to_string(&route).unwrap();
            let back: EditorRoute = serde_json::from_str(&json).unwrap();
            assert_eq!(back, route, "round-trip failed for {}", json);
        }
    }

    /// EditorRoute: default is Everywhere.
    #[test]
    fn editor_route_default_is_everywhere() {
        assert_eq!(EditorRoute::default(), EditorRoute::Everywhere);
    }

    /// EditorRoute: as_str / from_str helpers round-trip and reject unknowns.
    #[test]
    fn editor_route_str_helpers() {
        for route in [
            EditorRoute::Everywhere,
            EditorRoute::NoTwitch,
            EditorRoute::VoiceOnly,
            EditorRoute::TwitchOnly,
        ] {
            let s = route.as_str();
            let parsed = EditorRoute::from_str(s).expect("from_str round-trip");
            assert_eq!(parsed, route);
        }
        assert!(EditorRoute::from_str("bogus").is_none());
    }

    /// EditorSettings: missing default_route defaults to Everywhere.
    #[test]
    fn editor_settings_deserializes_without_default_route() {
        let json = r#"{"quick":false,"ai":false,"ai_completion":false,"spellcheck_enabled":true,"spellcheck_source":"offline","editor_height":340}"#;
        let settings: EditorSettings =
            serde_json::from_str(json).expect("must deserialize without default_route");
        assert_eq!(settings.default_route, EditorRoute::Everywhere);
    }

    /// EditorSettings: default_route round-trip.
    #[test]
    fn editor_settings_default_route_round_trip() {
        let original = EditorSettings {
            default_route: EditorRoute::TwitchOnly,
            ..EditorSettings::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: EditorSettings = serde_json::from_str(&json).expect("round-trip deserialization");
        assert_eq!(back.default_route, EditorRoute::TwitchOnly);
        assert!(json.contains(r#""default_route":"twitch_only""#));
    }

    // ==================== Editor hotkey action labels ====================

    /// Conflict/validation messages must describe the accent_homographs action
    /// as «расстановки ударений».
    #[test]
    fn editor_action_label_accent_homographs() {
        assert_eq!(
            editor_action_label("accent_homographs"),
            "расстановки ударений"
        );
        assert_eq!(editor_action_label("edit_word"), "редактирования слова");
        assert_eq!(
            editor_action_label("toggle_history"),
            "показа/скрытия истории"
        );
    }

    /// Conflict/validation messages must describe the approve_next_incoming
    /// action as «подтверждения отправки входящего».
    #[test]
    fn editor_action_label_approve_next_incoming() {
        assert_eq!(
            editor_action_label("approve_next_incoming"),
            "подтверждения отправки входящего"
        );
        assert_eq!(
            editor_action_label("edit_next_incoming"),
            "редактирования входящего"
        );
    }

    /// reset_editor_hotkey must return the canonical Ctrl+U default for
    /// accent_homographs and persist it.
    #[test]
    fn reset_editor_hotkey_accent_homographs_returns_default() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-accent-hk-reset-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mgr = SettingsManager::with_config_dir(dir.clone()).unwrap();

        let custom = crate::config::hotkeys::Hotkey {
            modifiers: vec![],
            key: "F11".to_string(),
            enabled: true,
        };
        mgr.set_editor_hotkey("accent_homographs", &custom).unwrap();

        let default = mgr.reset_editor_hotkey("accent_homographs").unwrap();
        assert_eq!(default.key, "U");
        assert_eq!(
            default.modifiers,
            vec![crate::config::hotkeys::HotkeyModifier::Ctrl]
        );

        let settings = mgr.load().unwrap();
        assert_eq!(settings.hotkeys.editor.accent_homographs.key, "U");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// reset_editor_hotkey must return the canonical Ctrl+K default for
    /// approve_next_incoming and persist it.
    #[test]
    fn reset_editor_hotkey_approve_next_incoming_returns_default() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-approve-hk-reset-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mgr = SettingsManager::with_config_dir(dir.clone()).unwrap();

        let custom = crate::config::hotkeys::Hotkey {
            modifiers: vec![],
            key: "F12".to_string(),
            enabled: true,
        };
        mgr.set_editor_hotkey("approve_next_incoming", &custom)
            .unwrap();

        let default = mgr.reset_editor_hotkey("approve_next_incoming").unwrap();
        assert_eq!(default.key, "K");
        assert_eq!(
            default.modifiers,
            vec![crate::config::hotkeys::HotkeyModifier::Ctrl]
        );

        let settings = mgr.load().unwrap();
        assert_eq!(settings.hotkeys.editor.approve_next_incoming.key, "K");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Backward-compat: old settings.json without `vtube_studio` field must deserialize.
    #[test]
    fn app_settings_deserializes_without_vtube_studio_field() {
        let old_json = r#"{
            "audio": { "speaker_device": null, "speaker_enabled": true, "speaker_volume": 80, "virtual_mic_device": null, "virtual_mic_volume": 100 },
            "tts": { "provider": "openai", "openai": { "api_key": null, "voice": "alloy" }, "local": { "url": "http://127.0.0.1:8124" }, "fish": { "api_key": null, "voices": [], "reference_id": "", "format": "mp3", "temperature": 0.7, "sample_rate": 44100, "use_proxy": false }, "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" }, "network": { "proxy": { "proxy_url": null }, "mtproxy": { "host": null, "port": 8888, "secret": null, "dc_id": null } } },
            "audio_effects": { "enabled": false, "pitch": 0, "speed": 0, "volume": 100, "enhance_enabled": false, "enhance_atten_db": 12.0, "formant_preserved": true },
            "hotkey_enabled": true,
            "editor": { "quick": false, "ai": false, "ai_completion": false, "spellcheck_enabled": true, "spellcheck_source": "offline", "editor_height": 340 },
            "theme": "dark",
            "twitch": { "enabled": false, "username": "", "token": "", "channel": "", "start_on_boot": false },
            "webview": { "enabled": false, "start_on_boot": false, "port": 10100, "bind_address": "0.0.0.0", "access_token": null, "upnp_enabled": false },
            "logging": { "enabled": false, "level": "info", "module_levels": {} },
            "ai": { "provider": "openai", "openai": { "api_key": null, "use_proxy": false, "model": "gpt-4o-mini" }, "zai": { "url": null, "api_key": null, "model": "glm-4.5" }, "deepseek": { "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "custom": { "url": null, "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "prompt": "test", "timeout": 20 },
            "hotkeys": { "main_window": { "modifiers": ["ctrl"], "key": "F12" }, "sound_panel": { "modifiers": ["alt"], "key": "F12" }, "playback_pause": { "modifiers": [], "key": "" }, "playback_stop": { "modifiers": [], "key": "" }, "playback_repeat": { "modifiers": [], "key": "" }, "playback_control_window": { "modifiers": [], "key": "" } },
            "show_playback_on_start": false
        }"#;
        let settings: AppSettings = serde_json::from_str(old_json)
            .expect("old AppSettings (without vtube_studio field) must deserialize");
        assert!(!settings.vtube_studio.enabled);
        assert_eq!(settings.vtube_studio.host, "127.0.0.1");
        assert_eq!(settings.vtube_studio.port, 8001);
        assert!(settings.vtube_studio.token.is_none());
        assert!(!settings.vtube_studio.start_on_boot);
    }

    /// Backward-compat (ROADMAP-120): old settings.json where vtube_studio exists
    /// but has no `host` field must load host as 127.0.0.1.
    #[test]
    fn vtube_studio_settings_without_host_field_defaults_to_localhost() {
        let old_json = r#"{
            "enabled": true,
            "port": 8001,
            "token": null,
            "start_on_boot": false
        }"#;
        let settings: VTubeStudioSettings = serde_json::from_str(old_json)
            .expect("old VTubeStudioSettings (without host field) must deserialize");
        assert!(settings.enabled);
        assert_eq!(settings.host, "127.0.0.1");
        assert_eq!(settings.port, 8001);
    }

    /// VTubeStudioSettings defaults: disabled, host 127.0.0.1, port 8001, no token,
    /// start_on_boot false, typing mode Event, param name TTSBardTyping, empty
    /// hotkey IDs, empty item metadata.
    #[test]
    fn vtube_studio_settings_defaults() {
        let s = VTubeStudioSettings::default();
        assert!(!s.enabled);
        assert_eq!(s.host, "127.0.0.1");
        assert_eq!(s.port, 8001);
        assert!(s.token.is_none());
        assert!(!s.start_on_boot);
        assert_eq!(s.typing_action.output_mode, VTubeStudioTypingMode::Event);
        assert_eq!(s.typing_action.parameter_name, "TTSBardTyping");
        assert!(s.typing_action.start_hotkey_id.is_empty());
        assert!(s.typing_action.stop_hotkey_id.is_empty());
        assert!(s.typing_action.item_file_name.is_empty());
        assert!(s.typing_action.item_type.is_empty());
    }

    /// VTubeStudioSettings round-trip: all fields including typing_action are persisted.
    #[test]
    fn vtube_studio_settings_token_round_trip() {
        let original = VTubeStudioSettings {
            enabled: true,
            host: "192.168.1.50".to_string(),
            port: 8002,
            token: Some("secret-token".to_string()),
            start_on_boot: true,
            typing_action: VTubeStudioTypingAction {
                output_mode: VTubeStudioTypingMode::Hotkeys,
                parameter_name: "CustomParam".to_string(),
                start_hotkey_id: "hotkey-start-1".to_string(),
                stop_hotkey_id: "hotkey-stop-1".to_string(),
                start_hotkey_name: "Start".to_string(),
                stop_hotkey_name: "Stop".to_string(),
                item_file_name: String::new(),
                item_type: String::new(),
            },
        };
        let json = serde_json::to_string(&original).unwrap();
        // Token must NOT be skip_serialized on the settings struct (it's persisted to disk)
        assert!(json.contains("secret-token"));
        let back: VTubeStudioSettings = serde_json::from_str(&json).unwrap();
        assert!(back.enabled);
        assert_eq!(back.host, "192.168.1.50");
        assert_eq!(back.port, 8002);
        assert_eq!(back.token.as_deref(), Some("secret-token"));
        assert!(back.start_on_boot);
        assert_eq!(
            back.typing_action.output_mode,
            VTubeStudioTypingMode::Hotkeys
        );
        assert_eq!(back.typing_action.parameter_name, "CustomParam");
        assert_eq!(back.typing_action.start_hotkey_id, "hotkey-start-1");
        assert_eq!(back.typing_action.stop_hotkey_id, "hotkey-stop-1");
        assert_eq!(back.typing_action.start_hotkey_name, "Start");
        assert_eq!(back.typing_action.stop_hotkey_name, "Stop");
    }

    /// Host validation (ROADMAP-120): IPv4 and hostnames are accepted; schemes,
    /// paths, ports, whitespace and malformed labels are rejected.
    #[test]
    fn validate_vtube_host_accepts_ipv4_and_hostnames() {
        for ok in [
            "127.0.0.1",
            "192.168.1.50",
            "0.0.0.0",
            "DESKTOP-ABC123",
            "stream-pc",
            "my.pc.local",
            "a.b",
        ] {
            assert!(
                validate_vtube_host(ok).is_ok(),
                "expected '{ok}' to be a valid host"
            );
        }
    }

    #[test]
    fn validate_vtube_host_rejects_scheme_path_port_and_bad_labels() {
        for bad in [
            "999.1.1.1",
            "192.168.001.50",
            "",
            "   ",
            "ws://127.0.0.1",
            "http://pc:8001",
            "192.168.1.50:8001",
            "127.0.0.1/",
            "127.0.0.1\\",
            "two words",
            "-leading",
            "trailing-",
            ".dot",
            "dot.",
            "double..dot",
            "-",
            "very-long-label-that-exceeds-sixty-three-characters-aaaaaaaaaaaaaaaaa",
        ] {
            assert!(
                validate_vtube_host(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn validate_vtube_host_trims_surrounding_whitespace() {
        assert!(validate_vtube_host("  127.0.0.1  ").is_ok());
    }

    /// VTubeStudioTypingMode is serialized as Event/Hotkeys/Item.
    #[test]
    fn vtube_studio_typing_mode_serde() {
        assert_eq!(
            serde_json::to_string(&VTubeStudioTypingMode::Event).unwrap(),
            "\"Event\""
        );
        assert_eq!(
            serde_json::to_string(&VTubeStudioTypingMode::Hotkeys).unwrap(),
            "\"Hotkeys\""
        );
        assert_eq!(
            serde_json::to_string(&VTubeStudioTypingMode::Item).unwrap(),
            "\"Item\""
        );
        let m: VTubeStudioTypingMode = serde_json::from_str("\"Hotkeys\"").unwrap();
        assert_eq!(m, VTubeStudioTypingMode::Hotkeys);
        let m: VTubeStudioTypingMode = serde_json::from_str("\"Item\"").unwrap();
        assert_eq!(m, VTubeStudioTypingMode::Item);
    }

    /// VTubeStudioTypingAction fresh defaults include empty item metadata.
    #[test]
    fn vtube_studio_typing_action_defaults() {
        let a = VTubeStudioTypingAction::default();
        assert_eq!(a.output_mode, VTubeStudioTypingMode::Event);
        assert_eq!(a.parameter_name, "TTSBardTyping");
        assert!(a.start_hotkey_id.is_empty());
        assert!(a.stop_hotkey_id.is_empty());
        assert!(a.item_file_name.is_empty());
        assert!(a.item_type.is_empty());
    }

    /// Existing settings without the later typing action use the safe Event default.
    #[test]
    fn vtube_studio_settings_defaults_without_typing_action() {
        let json = r#"{
            "enabled": true,
            "port": 8001,
            "token": null,
            "start_on_boot": false
        }"#;
        let settings: VTubeStudioSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.typing_action, VTubeStudioTypingAction::default());
    }

    /// JSON with `vtube_studio` present and correct `typing_action` with snake_case
    /// field names must deserialize successfully.
    #[test]
    fn vtube_studio_settings_deserializes_with_typing_action_snake_case() {
        let json = r#"{
            "enabled": true,
            "port": 8002,
            "token": "tok",
            "start_on_boot": false,
            "typing_action": {
                "output_mode": "Hotkeys",
                "parameter_name": "Custom",
                "start_hotkey_id": "s1",
                "stop_hotkey_id": "s2"
            }
        }"#;
        let s: VTubeStudioSettings =
            serde_json::from_str(json).expect("snake_case typing_action must deserialize");
        assert!(s.enabled);
        assert_eq!(s.port, 8002);
        assert_eq!(s.typing_action.output_mode, VTubeStudioTypingMode::Hotkeys);
        assert_eq!(s.typing_action.parameter_name, "Custom");
        assert_eq!(s.typing_action.start_hotkey_id, "s1");
        assert_eq!(s.typing_action.stop_hotkey_id, "s2");
    }

    /// JSON with camelCase field names in typing_action must FAIL (schema is snake_case).
    #[test]
    fn vtube_studio_settings_fails_with_camelcase_typing_action() {
        let json = r#"{
            "enabled": true,
            "port": 8001,
            "token": null,
            "start_on_boot": false,
            "typing_action": {
                "outputMode": "Event",
                "parameterName": "X",
                "startHotkeyId": "",
                "stopHotkeyId": ""
            }
        }"#;
        let result: Result<VTubeStudioSettings, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "camelCase typing_action must fail to deserialize with snake_case schema"
        );
    }

    /// VTubeStudioTypingAction serializes with snake_case field names.
    #[test]
    fn vtube_studio_typing_action_serializes_snake_case() {
        let a = VTubeStudioTypingAction::default();
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("output_mode"), "json: {}", json);
        assert!(json.contains("parameter_name"), "json: {}", json);
        assert!(json.contains("start_hotkey_id"), "json: {}", json);
        assert!(json.contains("stop_hotkey_id"), "json: {}", json);
        assert!(json.contains("item_file_name"), "json: {}", json);
        assert!(json.contains("item_type"), "json: {}", json);
    }

    // ==================== VRChat Tests ====================

    #[test]
    fn old_settings_deserializes_without_vrchat() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("vrchat");
        let json = serde_json::to_string(&value).unwrap();
        assert!(!json.contains("\"vrchat\""));

        let settings: AppSettings = serde_json::from_str(&json)
            .expect("old AppSettings (without vrchat field) must deserialize");
        assert!(!settings.vrchat.enabled);
        assert_eq!(settings.vrchat.host, "127.0.0.1");
        assert_eq!(settings.vrchat.port, 9000);
    }

    #[test]
    fn vrchat_settings_defaults() {
        let s = VrchatSettings::default();
        assert!(!s.enabled);
        assert!(!s.start_on_boot);
        assert_eq!(s.host, "127.0.0.1");
        assert_eq!(s.port, 9000);
    }

    #[test]
    fn vrchat_settings_round_trip() {
        let original = VrchatSettings {
            enabled: true,
            start_on_boot: true,
            host: "192.168.1.100".to_string(),
            port: 9005,
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: VrchatSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn vrchat_autostart_is_independent_of_last_runtime_state() {
        let mut saved = VrchatSettings {
            enabled: true,
            ..VrchatSettings::default()
        };
        assert!(!saved.runtime_at_startup().enabled);

        saved.enabled = false;
        saved.start_on_boot = true;
        assert!(saved.runtime_at_startup().enabled);
    }

    #[test]
    fn legacy_vrchat_settings_do_not_autostart() {
        let legacy = r#"{"enabled":true,"host":"127.0.0.1","port":9000}"#;
        let saved: VrchatSettings = serde_json::from_str(legacy).unwrap();
        assert!(saved.enabled);
        assert!(!saved.start_on_boot);
        assert!(!saved.runtime_at_startup().enabled);
    }

    #[test]
    fn validate_vrchat_host_accepts_valid() {
        for ok in [
            "127.0.0.1",
            "192.168.1.50",
            "::1",
            "localhost",
            "DESKTOP-ABC123",
            "my.pc.local",
        ] {
            assert!(
                validate_vrchat_host(ok).is_ok(),
                "expected '{ok}' to be a valid host"
            );
        }
    }

    #[test]
    fn validate_vrchat_host_rejects_invalid() {
        for bad in [
            "",
            "   ",
            "999.1.1.1",
            "ws://127.0.0.1",
            "127.0.0.1:9000",
            "127.0.0.1/",
            "two words",
            "-leading",
            "trailing-",
        ] {
            assert!(
                validate_vrchat_host(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    /// Old persisted typing action with no item fields deserializes to empty item metadata.
    #[test]
    fn vtube_studio_typing_action_deserializes_without_item_fields() {
        let old_json = r#"{
            "output_mode": "Event",
            "parameter_name": "TTSBardTyping",
            "start_hotkey_id": "",
            "stop_hotkey_id": ""
        }"#;
        let a: VTubeStudioTypingAction = serde_json::from_str(old_json)
            .expect("old typing action (without item fields) must deserialize");
        assert_eq!(a.output_mode, VTubeStudioTypingMode::Event);
        assert_eq!(a.parameter_name, "TTSBardTyping");
        assert!(a.item_file_name.is_empty());
        assert!(a.item_type.is_empty());
    }

    /// Old persisted typing action with hotkey names but no item fields deserializes to empty item metadata.
    #[test]
    fn vtube_studio_typing_action_hotkeys_deserializes_without_item_fields() {
        let old_json = r#"{
            "output_mode": "Hotkeys",
            "parameter_name": "Custom",
            "start_hotkey_id": "s1",
            "stop_hotkey_id": "s2",
            "start_hotkey_name": "Start",
            "stop_hotkey_name": "Stop"
        }"#;
        let a: VTubeStudioTypingAction = serde_json::from_str(old_json)
            .expect("old typing action (Hotkeys, without item fields) must deserialize");
        assert_eq!(a.output_mode, VTubeStudioTypingMode::Hotkeys);
        assert_eq!(a.start_hotkey_id, "s1");
        assert_eq!(a.stop_hotkey_id, "s2");
        assert!(a.item_file_name.is_empty());
        assert!(a.item_type.is_empty());
    }

    /// Item action round-trips exact mixed-case/non-ASCII filename and item type
    /// without an instanceID field appearing in persisted JSON.
    #[test]
    fn vtube_studio_typing_action_item_round_trip() {
        let original = VTubeStudioTypingAction {
            output_mode: VTubeStudioTypingMode::Item,
            parameter_name: String::new(),
            start_hotkey_id: String::new(),
            stop_hotkey_id: String::new(),
            start_hotkey_name: String::new(),
            stop_hotkey_name: String::new(),
            item_file_name: "Über_Cat_GIF_123.gif".to_string(),
            item_type: "GIF".to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        assert!(
            !json.contains("instanceID"),
            "json must not contain instanceID: {}",
            json
        );
        assert!(
            !json.contains("instance_id"),
            "json must not contain instance_id: {}",
            json
        );
        assert!(json.contains("Über_Cat_GIF_123.gif"), "json: {}", json);
        assert!(json.contains("GIF"), "json: {}", json);
        assert!(json.contains("\"Item\""), "json: {}", json);

        let back: VTubeStudioTypingAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back.output_mode, VTubeStudioTypingMode::Item);
        assert_eq!(back.item_file_name, "Über_Cat_GIF_123.gif");
        assert_eq!(back.item_type, "GIF");
    }

    /// Safe DTO JSON uses outputMode, itemFileName, itemType, preserves exact metadata,
    /// and contains no token or instanceID.
    #[test]
    fn vtube_studio_typing_action_dto_camelcase_no_secrets() {
        use crate::config::dto::VTubeStudioTypingActionDto;
        let action = VTubeStudioTypingAction {
            output_mode: VTubeStudioTypingMode::Item,
            parameter_name: String::new(),
            start_hotkey_id: String::new(),
            stop_hotkey_id: String::new(),
            start_hotkey_name: String::new(),
            stop_hotkey_name: String::new(),
            item_file_name: "My Item.PNG".to_string(),
            item_type: "PNG".to_string(),
        };
        let dto: VTubeStudioTypingActionDto = (&action).into();
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("outputMode"), "json: {}", json);
        assert!(json.contains("itemFileName"), "json: {}", json);
        assert!(json.contains("itemType"), "json: {}", json);
        assert!(json.contains("My Item.PNG"), "json: {}", json);
        assert!(json.contains("PNG"), "json: {}", json);
        assert!(!json.contains("token"), "json: {}", json);
        assert!(!json.contains("instanceID"), "json: {}", json);
        assert!(!json.contains("instance_id"), "json: {}", json);
    }

    // ==================== Silero timing settings tests ====================

    /// TelegramTtsSettings::default() must have correct default timing values.
    #[test]
    fn telegram_tts_settings_default_timing() {
        let s = TelegramTtsSettings::default();
        assert_eq!(s.synthesis_response_timeout_ms, 60000);
        assert_eq!(s.download_retry_delay_ms, 1000);
    }

    /// Old JSON without synthesis_response_timeout_ms or download_retry_delay_ms
    /// must deserialize to defaults.
    #[test]
    fn telegram_tts_settings_deserializes_without_timing_fields() {
        let old_json = r#"{
            "api_id": null,
            "proxy_mode": "none",
            "voices": [],
            "current_voice_id": ""
        }"#;
        let settings: TelegramTtsSettings = serde_json::from_str(old_json)
            .expect("old TelegramTtsSettings (without timing fields) must deserialize");
        assert_eq!(settings.synthesis_response_timeout_ms, 60000);
        assert_eq!(settings.download_retry_delay_ms, 1000);
    }

    /// Clamping: synthesis_response_timeout_ms at exact min boundary passes through.
    #[test]
    fn validate_silero_timeout_exact_min() {
        let mut app = AppSettings::default();
        app.tts.telegram.synthesis_response_timeout_ms = SYNTHESIS_RESPONSE_TIMEOUT_MIN_MS;
        app.validate();
        assert_eq!(
            app.tts.telegram.synthesis_response_timeout_ms,
            SYNTHESIS_RESPONSE_TIMEOUT_MIN_MS
        );
    }

    /// Clamping: synthesis_response_timeout_ms below min is clamped up.
    #[test]
    fn validate_silero_timeout_below_min() {
        let mut app = AppSettings::default();
        app.tts.telegram.synthesis_response_timeout_ms = 500;
        app.validate();
        assert_eq!(
            app.tts.telegram.synthesis_response_timeout_ms,
            SYNTHESIS_RESPONSE_TIMEOUT_MIN_MS
        );
        // zero also clamped
        let mut app2 = AppSettings::default();
        app2.tts.telegram.synthesis_response_timeout_ms = 0;
        app2.validate();
        assert_eq!(
            app2.tts.telegram.synthesis_response_timeout_ms,
            SYNTHESIS_RESPONSE_TIMEOUT_MIN_MS
        );
    }

    /// Unbounded above: synthesis_response_timeout_ms above 120_000 is preserved (ROADMAP-133).
    #[test]
    fn validate_silero_timeout_above_max_preserved() {
        let mut app = AppSettings::default();
        app.tts.telegram.synthesis_response_timeout_ms = 999_999;
        app.validate();
        assert_eq!(app.tts.telegram.synthesis_response_timeout_ms, 999_999);
    }

    /// Clamping: download_retry_delay_ms at exact min boundary passes through.
    #[test]
    fn validate_silero_delay_exact_min() {
        let mut app = AppSettings::default();
        app.tts.telegram.download_retry_delay_ms = DOWNLOAD_RETRY_DELAY_MIN_MS;
        app.validate();
        assert_eq!(
            app.tts.telegram.download_retry_delay_ms,
            DOWNLOAD_RETRY_DELAY_MIN_MS
        );
    }

    /// Clamping: download_retry_delay_ms at exact max boundary passes through.
    #[test]
    fn validate_silero_delay_exact_max() {
        let mut app = AppSettings::default();
        app.tts.telegram.download_retry_delay_ms = DOWNLOAD_RETRY_DELAY_MAX_MS;
        app.validate();
        assert_eq!(
            app.tts.telegram.download_retry_delay_ms,
            DOWNLOAD_RETRY_DELAY_MAX_MS
        );
    }

    #[test]
    fn set_silero_synthesis_response_timeout_persists_and_clamps_min() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-silero-timeout-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mgr = SettingsManager::with_config_dir(dir.clone()).unwrap();

        mgr.set_silero_synthesis_response_timeout_ms(500).unwrap();
        let loaded = mgr.load().unwrap();
        assert_eq!(
            loaded.tts.telegram.synthesis_response_timeout_ms,
            SYNTHESIS_RESPONSE_TIMEOUT_MIN_MS
        );

        mgr.set_silero_synthesis_response_timeout_ms(45000).unwrap();
        let loaded2 = mgr.load().unwrap();
        assert_eq!(loaded2.tts.telegram.synthesis_response_timeout_ms, 45000);

        // Unbounded above: large value preserved
        mgr.set_silero_synthesis_response_timeout_ms(300_000)
            .unwrap();
        let loaded3 = mgr.load().unwrap();
        assert_eq!(loaded3.tts.telegram.synthesis_response_timeout_ms, 300_000);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Clamping: download_retry_delay_ms below min is clamped up.
    #[test]
    fn validate_silero_delay_below_min() {
        let mut app = AppSettings::default();
        app.tts.telegram.download_retry_delay_ms = 50;
        app.validate();
        assert_eq!(
            app.tts.telegram.download_retry_delay_ms,
            DOWNLOAD_RETRY_DELAY_MIN_MS
        );
        // zero also clamped
        let mut app2 = AppSettings::default();
        app2.tts.telegram.download_retry_delay_ms = 0;
        app2.validate();
        assert_eq!(
            app2.tts.telegram.download_retry_delay_ms,
            DOWNLOAD_RETRY_DELAY_MIN_MS
        );
    }

    /// Clamping: download_retry_delay_ms above max is clamped down.
    #[test]
    fn validate_silero_delay_above_max() {
        let mut app = AppSettings::default();
        app.tts.telegram.download_retry_delay_ms = 99_999;
        app.validate();
        assert_eq!(
            app.tts.telegram.download_retry_delay_ms,
            DOWNLOAD_RETRY_DELAY_MAX_MS
        );
    }

    /// Settings -> DTO -> settings round-trip preserves both hidden timing values.
    #[test]
    fn telegram_tts_timing_dto_round_trip() {
        use crate::config::dto::TtsSettingsDto;
        let mut settings = AppSettings::default();
        settings.tts.telegram.synthesis_response_timeout_ms = 25000;
        settings.tts.telegram.download_retry_delay_ms = 3000;
        let dto: TtsSettingsDto = settings.tts.clone().into();
        assert_eq!(dto.telegram.synthesis_response_timeout_ms, 25000);
        assert_eq!(dto.telegram.download_retry_delay_ms, 3000);
        let back: TtsSettings = dto.into();
        assert_eq!(back.telegram.synthesis_response_timeout_ms, 25000);
        assert_eq!(back.telegram.download_retry_delay_ms, 3000);
    }

    /// DTO from old frontend (missing timing fields) deserializes to defaults.
    #[test]
    fn telegram_tts_dto_deserializes_without_timing_fields() {
        let old_json = r#"{
            "api_id": null,
            "proxy_mode": "none",
            "voices": [],
            "current_voice_id": ""
        }"#;
        let dto: crate::config::dto::TelegramTtsSettingsDto = serde_json::from_str(old_json)
            .expect("old DTO (without timing fields) must deserialize");
        assert_eq!(dto.synthesis_response_timeout_ms, 60000);
        assert_eq!(dto.download_retry_delay_ms, 1000);
    }

    // ==================== Input server settings tests ====================

    /// Default `AppSettings` carries the canonical `InputServerSettings` default
    /// and a source-neutral Incoming default with auto-play enabled.
    #[test]
    fn input_server_defaults_match_domain_default() {
        let settings = AppSettings::default();
        assert_eq!(settings.input_server, InputServerSettings::default());
        assert!(!settings.input_server.start_on_boot);
        assert_eq!(settings.input_server.port, 10101);
        assert_eq!(settings.incoming, IncomingSettings::default());
        assert!(settings.incoming.auto_play);
    }

    /// Backward-compat: old settings.json without the `input_server` section
    /// must deserialize with the domain default.
    #[test]
    fn app_settings_deserializes_without_input_server_section() {
        let old_json = r#"{
            "audio": { "speaker_device": null, "speaker_enabled": true, "speaker_volume": 80, "virtual_mic_device": null, "virtual_mic_volume": 100 },
            "tts": { "provider": "openai", "openai": { "api_key": null, "voice": "alloy" }, "local": { "url": "http://127.0.0.1:8124" }, "fish": { "api_key": null, "voices": [], "reference_id": "", "format": "mp3", "temperature": 0.7, "sample_rate": 44100, "use_proxy": false }, "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" }, "network": { "proxy": { "proxy_url": null }, "mtproxy": { "host": null, "port": 8888, "secret": null, "dc_id": null } } },
            "audio_effects": { "enabled": false, "pitch": 0, "speed": 0, "volume": 100, "enhance_enabled": false, "enhance_atten_db": 12.0, "formant_preserved": true },
            "hotkey_enabled": true,
            "editor": { "quick": false, "ai": false, "ai_completion": false, "spellcheck_enabled": true, "spellcheck_source": "offline", "editor_height": 340 },
            "theme": "dark",
            "twitch": { "enabled": false, "username": "", "token": "", "channel": "", "start_on_boot": false },
            "webview": { "enabled": false, "start_on_boot": false, "port": 10100, "bind_address": "0.0.0.0", "access_token": null, "upnp_enabled": false },
            "logging": { "enabled": false, "level": "info", "module_levels": {} },
            "ai": { "provider": "openai", "openai": { "api_key": null, "use_proxy": false, "model": "gpt-4o-mini" }, "zai": { "url": null, "api_key": null, "model": "glm-4.5" }, "deepseek": { "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "custom": { "url": null, "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "prompt": "test", "timeout": 20 },
            "hotkeys": { "main_window": { "modifiers": ["ctrl"], "key": "F12" }, "sound_panel": { "modifiers": ["alt"], "key": "F12" }, "playback_pause": { "modifiers": [], "key": "" }, "playback_stop": { "modifiers": [], "key": "" }, "playback_repeat": { "modifiers": [], "key": "" }, "playback_control_window": { "modifiers": [], "key": "" } },
            "show_playback_on_start": false
        }"#;
        let settings: AppSettings = serde_json::from_str(old_json)
            .expect("old AppSettings (without input_server field) must deserialize");
        assert_eq!(settings.input_server, InputServerSettings::default());
        assert_eq!(settings.input_server.port, 10101);
    }

    /// An out-of-range persisted input-server port is reset to the fallback.
    #[test]
    fn input_server_invalid_port_falls_back_to_default() {
        let mut settings = AppSettings::default();
        settings.input_server.port = 80;
        settings.validate();
        assert_eq!(settings.input_server.port, 10101);

        let mut zero = AppSettings::default();
        zero.input_server.port = 0;
        zero.validate();
        assert_eq!(zero.input_server.port, 10101);

        let mut valid = AppSettings::default();
        valid.input_server.port = 20202;
        valid.validate();
        assert_eq!(valid.input_server.port, 20202);
    }

    /// Helper: build a SettingsManager over a fresh temp config dir.
    fn input_server_section_tmp_manager(label: &str) -> (SettingsManager, PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-input-server-section-{}-{}-{}",
            label,
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        (manager, dir)
    }

    /// The atomic setter persists both fields and keeps cache/disk in sync.
    #[test]
    fn set_input_server_section_persists_two_fields_and_keeps_cache_in_sync() {
        let (manager, dir) = input_server_section_tmp_manager("save-two");

        manager.set_input_server_section(true, 20202).unwrap();

        let disk: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert!(disk.input_server.start_on_boot);
        assert_eq!(disk.input_server.port, 20202);

        assert_eq!(manager.load().unwrap(), disk, "cache and disk must agree");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The atomic setter preserves unrelated settings.
    #[test]
    fn set_input_server_section_preserves_unrelated_settings() {
        let (manager, dir) = input_server_section_tmp_manager("preserve-unrelated");

        manager.set_speaker_volume(33).unwrap();
        manager
            .set_webview_access_token(Some("secret-token".to_string()))
            .unwrap();
        let before = manager.load().unwrap();
        assert_eq!(before.audio.speaker_volume, 33);

        manager.set_input_server_section(true, 20202).unwrap();

        let after = manager.load().unwrap();
        assert_eq!(after.audio.speaker_volume, 33);
        assert_eq!(after.webview.access_token, Some("secret-token".to_string()));
        assert!(after.input_server.start_on_boot);
        assert_eq!(after.input_server.port, 20202);

        let disk: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(disk, after, "disk and cache must agree");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The token setter persists via the JSON pointer and keeps cache/disk in
    /// sync without touching any other input-server field.
    #[test]
    fn set_input_server_access_token_persists_and_keeps_cache_in_sync() {
        let (manager, dir) = input_server_section_tmp_manager("token-save");

        manager
            .set_input_server_access_token(Some("token-a".to_string()))
            .unwrap();
        manager.set_input_server_section(true, 20202).unwrap();
        manager.set_input_server_access_token(None).unwrap();

        let cached = manager.load().unwrap();
        assert_eq!(cached.input_server.access_token, None);
        assert!(cached.input_server.start_on_boot, "section must survive");
        assert_eq!(cached.input_server.port, 20202, "section must survive");

        let disk: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(disk, cached, "disk and cache must agree");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Serialize a canonical settings.json without the input-server access
    /// token, simulating a pre-ROADMAP-118 file.
    fn legacy_input_server_token_json() -> String {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value
            .get_mut("input_server")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("access_token");
        serde_json::to_string_pretty(&value).unwrap()
    }

    fn legacy_token_tmp_dir(label: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-input-server-token-{}-{}-{}",
            label,
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A legacy settings.json without an input-server access token gets one
    /// generated on load, persisted to disk and reflected in the cache.
    #[test]
    fn legacy_settings_without_input_token_generate_one_on_load() {
        let dir = legacy_token_tmp_dir("migrate");
        std::fs::write(dir.join("settings.json"), legacy_input_server_token_json()).unwrap();

        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let token = manager
            .load()
            .unwrap()
            .input_server
            .access_token
            .expect("legacy load must generate a token");
        assert!(!token.is_empty());

        let disk_value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            disk_value.pointer("/input_server/access_token"),
            Some(&serde_json::Value::String(token)),
            "generated token must be persisted"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A persisted token is never regenerated on load.
    #[test]
    fn existing_input_token_survives_load() {
        let dir = legacy_token_tmp_dir("preserve");
        let mut value: serde_json::Value =
            serde_json::from_str(&legacy_input_server_token_json()).unwrap();
        value
            .get_mut("input_server")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(
                "access_token".to_string(),
                serde_json::Value::String("keep-me".to_string()),
            );
        std::fs::write(
            dir.join("settings.json"),
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();

        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        assert_eq!(
            manager.load().unwrap().input_server.access_token,
            Some("keep-me".to_string())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fresh settings file is created with a generated token.
    #[test]
    fn fresh_settings_file_gets_input_token() {
        let dir = legacy_token_tmp_dir("fresh");

        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let token = manager
            .load()
            .unwrap()
            .input_server
            .access_token
            .expect("fresh settings must carry a generated token");
        assert!(!token.is_empty());

        let disk_value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            disk_value.pointer("/input_server/access_token"),
            Some(&serde_json::Value::String(token)),
            "fresh file must persist the token"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== Incoming policy tests ====================

    /// Serialize a legacy settings.json: canonical defaults with the top-level
    /// `incoming` section removed and (optionally) a legacy
    /// `input_server.auto_play` value re-added.
    fn legacy_incoming_settings_json(auto_play: Option<serde_json::Value>) -> String {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        let obj = value.as_object_mut().unwrap();
        obj.remove("incoming");
        if let Some(auto_play) = auto_play {
            obj.get_mut("input_server")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("auto_play".to_string(), auto_play);
        }
        serde_json::to_string_pretty(&value).unwrap()
    }

    fn read_disk_value(config_dir: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(config_dir.join("settings.json")).unwrap())
            .unwrap()
    }

    /// IncomingSettings::default must auto-play (the historical default), and a
    /// canonical settings.json missing the `incoming` section carries it.
    #[test]
    fn incoming_settings_default_auto_play_is_true() {
        assert!(IncomingSettings::default().auto_play);
        assert!(AppSettings::default().incoming.auto_play);

        let parsed: AppSettings =
            serde_json::from_str(&legacy_incoming_settings_json(None)).unwrap();
        assert!(
            parsed.incoming.auto_play,
            "missing incoming must use default"
        );
    }

    /// Legacy `input_server.auto_play: false` migrates exactly to false.
    #[test]
    fn incoming_migrates_legacy_auto_play_false_exactly() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-incoming-migrate-false-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            legacy_incoming_settings_json(Some(serde_json::json!(false))),
        )
        .unwrap();

        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let loaded = manager.load().unwrap();
        assert!(
            !loaded.incoming.auto_play,
            "legacy false must migrate exactly, never flip to true"
        );

        let disk = read_disk_value(&dir);
        assert_eq!(disk["incoming"]["auto_play"], serde_json::json!(false));
        assert!(
            disk["input_server"].get("auto_play").is_none(),
            "legacy auto_play must be removed from input_server"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Legacy `input_server.auto_play: true` migrates exactly to true.
    #[test]
    fn incoming_migrates_legacy_auto_play_true_exactly() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-incoming-migrate-true-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            legacy_incoming_settings_json(Some(serde_json::json!(true))),
        )
        .unwrap();

        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        assert!(manager.load().unwrap().incoming.auto_play);

        let disk = read_disk_value(&dir);
        assert_eq!(disk["incoming"]["auto_play"], serde_json::json!(true));
        assert!(
            disk["input_server"].get("auto_play").is_none(),
            "legacy auto_play must be removed from input_server"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A missing or invalid legacy value falls back to the Incoming default
    /// (true) and is still persisted as a canonical `incoming` section.
    #[test]
    fn incoming_missing_or_invalid_legacy_value_uses_default() {
        for (label, legacy) in [
            ("missing", None),
            ("non-bool", Some(serde_json::json!("yes"))),
            ("null", Some(serde_json::Value::Null)),
        ] {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "ttsbard-incoming-migrate-{label}-{}-{}",
                std::process::id(),
                unique
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("settings.json"),
                legacy_incoming_settings_json(legacy.clone()),
            )
            .unwrap();

            let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
            assert!(
                manager.load().unwrap().incoming.auto_play,
                "{label} legacy value must fall back to the default"
            );

            let disk = read_disk_value(&dir);
            assert_eq!(disk["incoming"]["auto_play"], serde_json::json!(true));
            assert!(
                disk["input_server"].get("auto_play").is_none(),
                "{label}: legacy auto_play must be removed"
            );

            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// When both the new `incoming` section and a legacy `input_server.auto_play`
    /// coexist, the new value wins and no migration write occurs.
    #[test]
    fn incoming_new_value_wins_when_both_exist() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-incoming-both-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value["incoming"]["auto_play"] = serde_json::json!(false);
        value["input_server"]["auto_play"] = serde_json::json!(true);
        std::fs::write(
            dir.join("settings.json"),
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();

        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let loaded = manager.load().unwrap();
        assert!(
            !loaded.incoming.auto_play,
            "existing incoming section must win over the legacy value"
        );

        // The persisted input-server section still carries the leftover legacy
        // field until the next section save — but it is ignored on load.
        let disk = read_disk_value(&dir);
        assert_eq!(disk["incoming"]["auto_play"], serde_json::json!(false));
        assert_eq!(
            disk["input_server"]["start_on_boot"],
            serde_json::json!(false)
        );
        assert_eq!(disk["input_server"]["port"], serde_json::json!(10101));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A single load of a legacy file must persist the canonical `incoming`
    /// section once and remove the legacy `input_server.auto_play` field.
    #[test]
    fn incoming_migration_persists_canonical_incoming_once() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-incoming-canonical-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            legacy_incoming_settings_json(Some(serde_json::json!(false))),
        )
        .unwrap();

        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let first = manager.load().unwrap();
        assert!(!first.incoming.auto_play);

        // Reloading the now-canonical file must be a no-op for the migration.
        let disk_before = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        let manager2 = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let second = manager2.load().unwrap();
        assert_eq!(second, first);
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.json")).unwrap(),
            disk_before,
            "reloading a canonical file must not rewrite it"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The source-neutral Incoming setter persists `incoming.auto_play` and
    /// `incoming.route` together without touching the input-server section (or
    /// any other settings).
    #[test]
    fn set_incoming_section_preserves_unrelated_settings() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-incoming-setter-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();

        manager.set_input_server_section(true, 20202).unwrap();
        manager.set_speaker_volume(33).unwrap();

        manager
            .set_incoming_section(false, IncomingRoute::AudioTwitch)
            .unwrap();

        let after = manager.load().unwrap();
        assert!(!after.incoming.auto_play);
        assert_eq!(after.incoming.route, IncomingRoute::AudioTwitch);
        assert!(after.input_server.start_on_boot, "server section preserved");
        assert_eq!(after.input_server.port, 20202, "server port preserved");
        assert_eq!(
            after.audio.speaker_volume, 33,
            "unrelated setting preserved"
        );

        let disk = read_disk_value(&dir);
        assert_eq!(disk["incoming"]["auto_play"], serde_json::json!(false));
        assert_eq!(disk["incoming"]["route"], serde_json::json!("audio_twitch"));
        assert!(
            disk["input_server"].get("auto_play").is_none(),
            "server section must never carry auto_play after a section save"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Saving the input-server section no longer mutates the Incoming policy:
    /// `auto_play` and `route` keep whatever the source-neutral `incoming`
    /// section holds.
    #[test]
    fn set_input_server_section_does_not_mutate_incoming_policy() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-incoming-save-server-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();

        manager
            .set_incoming_section(false, IncomingRoute::AudioTwitch)
            .unwrap();

        manager.set_input_server_section(true, 20202).unwrap();

        let after = manager.load().unwrap();
        assert!(
            !after.incoming.auto_play,
            "input-server save must not flip auto-play"
        );
        assert_eq!(
            after.incoming.route,
            IncomingRoute::AudioTwitch,
            "input-server save must not change the incoming route"
        );
        assert!(after.input_server.start_on_boot);
        assert_eq!(after.input_server.port, 20202);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== Incoming route tests ====================

    /// An `incoming` section that predates the route field (only `auto_play`)
    /// deserializes with `route = AudioOnly`.
    #[test]
    fn incoming_route_missing_field_defaults_to_audio_only() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value["incoming"].as_object_mut().unwrap().remove("route");
        value["incoming"]["auto_play"] = serde_json::json!(false);
        let settings: AppSettings = serde_json::from_value(value).unwrap();
        assert!(!settings.incoming.auto_play);
        assert_eq!(settings.incoming.route, IncomingRoute::AudioOnly);
    }

    /// An unknown persisted route value normalizes to audio-only at the whole
    /// `AppSettings` boundary.
    #[test]
    fn incoming_unknown_route_normalizes_to_audio_only() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value["incoming"]["route"] = serde_json::json!("nonsense");
        let settings: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(settings.incoming.route, IncomingRoute::AudioOnly);
    }

    /// The default Incoming policy is audio-only.
    #[test]
    fn incoming_default_route_is_audio_only() {
        assert_eq!(IncomingSettings::default().route, IncomingRoute::AudioOnly);
        assert_eq!(
            AppSettings::default().incoming.route,
            IncomingRoute::AudioOnly
        );
    }

    // ==================== OCR settings tests ====================

    /// Default `AppSettings` carries the canonical `OcrSettings` default.
    #[test]
    fn ocr_defaults_match_domain_default() {
        let settings = AppSettings::default();
        assert_eq!(settings.ocr, OcrSettings::default());
        assert!(!settings.ocr.enabled);
        assert_eq!(settings.ocr.model_id, None);
    }

    /// Backward-compat: old settings.json without the `ocr` section must
    /// deserialize with the domain default.
    #[test]
    fn app_settings_deserializes_without_ocr_section() {
        let old_json = r#"{
            "audio": { "speaker_device": null, "speaker_enabled": true, "speaker_volume": 80, "virtual_mic_device": null, "virtual_mic_volume": 100 },
            "tts": { "provider": "openai", "openai": { "api_key": null, "voice": "alloy" }, "local": { "url": "http://127.0.0.1:8124" }, "fish": { "api_key": null, "voices": [], "reference_id": "", "format": "mp3", "temperature": 0.7, "sample_rate": 44100, "use_proxy": false }, "telegram": { "api_id": null, "proxy_mode": "none", "voices": [], "current_voice_id": "" }, "network": { "proxy": { "proxy_url": null }, "mtproxy": { "host": null, "port": 8888, "secret": null, "dc_id": null } } },
            "audio_effects": { "enabled": false, "pitch": 0, "speed": 0, "volume": 100, "enhance_enabled": false, "enhance_atten_db": 12.0, "formant_preserved": true },
            "hotkey_enabled": true,
            "editor": { "quick": false, "ai": false, "ai_completion": false, "spellcheck_enabled": true, "spellcheck_source": "offline", "editor_height": 340 },
            "theme": "dark",
            "twitch": { "enabled": false, "username": "", "token": "", "channel": "", "start_on_boot": false },
            "webview": { "enabled": false, "start_on_boot": false, "port": 10100, "bind_address": "0.0.0.0", "access_token": null, "upnp_enabled": false },
            "logging": { "enabled": false, "level": "info", "module_levels": {} },
            "ai": { "provider": "openai", "openai": { "api_key": null, "use_proxy": false, "model": "gpt-4o-mini" }, "zai": { "url": null, "api_key": null, "model": "glm-4.5" }, "deepseek": { "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "custom": { "url": null, "api_key": null, "use_proxy": false, "model": "deepseek-chat" }, "prompt": "test", "timeout": 20 },
            "hotkeys": { "main_window": { "modifiers": ["ctrl"], "key": "F12" }, "sound_panel": { "modifiers": ["alt"], "key": "F12" }, "playback_pause": { "modifiers": [], "key": "" }, "playback_stop": { "modifiers": [], "key": "" }, "playback_repeat": { "modifiers": [], "key": "" }, "playback_control_window": { "modifiers": [], "key": "" } },
            "show_playback_on_start": false
        }"#;
        let settings: AppSettings = serde_json::from_str(old_json)
            .expect("old AppSettings (without ocr field) must deserialize");
        assert_eq!(settings.ocr, OcrSettings::default());
        assert!(!settings.ocr.enabled);
        assert_eq!(settings.ocr.model_id, None);
    }

    /// The atomic setter persists both fields and keeps cache/disk in sync.
    #[test]
    fn set_ocr_section_persists_two_fields_and_keeps_cache_in_sync() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-ocr-section-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();

        assert!(!manager.load().unwrap().ocr.enabled);

        manager
            .set_ocr_section(true, Some("com.example.ocr".to_string()))
            .unwrap();

        let after = manager.load().unwrap();
        assert!(after.ocr.enabled);
        assert_eq!(after.ocr.model_id.as_deref(), Some("com.example.ocr"));

        let disk: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(disk, after, "cache and disk must agree");

        // Disable but preserve the selected id for a later re-enable.
        manager
            .set_ocr_section(false, Some("com.example.ocr".to_string()))
            .unwrap();
        let after = manager.load().unwrap();
        assert!(!after.ocr.enabled);
        assert_eq!(after.ocr.model_id.as_deref(), Some("com.example.ocr"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `set_ocr_section` (model refresh / failed-start untick) must retain a
    /// previously persisted specific capture target.
    #[test]
    fn set_ocr_section_retains_capture_target() {
        use crate::ocr::monitors::CaptureTarget;

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-ocr-section-target-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();

        let target = CaptureTarget::Monitor {
            device_path: r"\\.\DISPLAY2\Monitor0".to_string(),
        };
        manager
            .set_ocr_settings(OcrSettings {
                enabled: false,
                model_id: Some("com.example.ocr".to_string()),
                capture_target: target.clone(),
            })
            .unwrap();

        // Model refresh / failed-start untick only flip enabled/model_id.
        manager
            .set_ocr_section(true, Some("com.example.other".to_string()))
            .unwrap();
        let after = manager.load().unwrap();
        assert!(after.ocr.enabled);
        assert_eq!(after.ocr.model_id.as_deref(), Some("com.example.other"));
        assert_eq!(after.ocr.capture_target, target);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The full setter persists the complete desired settings and keeps the
    /// cache and disk in agreement.
    #[test]
    fn set_ocr_settings_persists_full_section_and_cache() {
        use crate::ocr::monitors::CaptureTarget;

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-ocr-full-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();

        let desired = OcrSettings {
            enabled: true,
            model_id: Some("com.example.ocr".to_string()),
            capture_target: CaptureTarget::Primary,
        };
        manager.set_ocr_settings(desired.clone()).unwrap();

        let after = manager.load().unwrap();
        assert_eq!(after.ocr, desired);

        let disk: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(disk, after, "cache and disk must agree");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== Editor font tests ====================

    /// EditorSettings default font family is `default` and size is 16.
    #[test]
    fn editor_font_defaults() {
        let s = EditorSettings::default();
        assert_eq!(s.font_family, "default");
        assert_eq!(s.font_size_px, 16);
    }

    /// Old settings missing both font fields deserialize to the ID default and 16.
    #[test]
    fn editor_font_deserializes_without_fields() {
        let json = r#"{"quick":false,"ai":false,"ai_completion":false,"spellcheck_enabled":true,"spellcheck_source":"offline","editor_height":340}"#;
        let settings: EditorSettings =
            serde_json::from_str(json).expect("must deserialize without font fields");
        assert_eq!(settings.font_family, "default");
        assert_eq!(settings.font_size_px, 16);
    }

    /// Old stable ids and arbitrary installed-family names round-trip through
    /// serde, so a custom font survives config reads and writes.
    #[test]
    fn editor_font_family_round_trip_old_ids_and_custom_names() {
        for family in [
            "default", "system", "arial", "georgia", "consolas", "PT Sans",
        ] {
            let s = EditorSettings {
                font_family: family.to_owned(),
                ..EditorSettings::default()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: EditorSettings = serde_json::from_str(&json).unwrap();
            assert_eq!(back.font_family, family, "round-trip failed for {}", json);
        }
    }

    /// Font size boundaries (12 and 96) round-trip through serde.
    #[test]
    fn editor_font_size_boundaries_round_trip() {
        for size in [EDITOR_FONT_SIZE_MIN_PX, 64, EDITOR_FONT_SIZE_MAX_PX] {
            let s = EditorSettings {
                font_size_px: size,
                ..EditorSettings::default()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: EditorSettings = serde_json::from_str(&json).unwrap();
            assert_eq!(back.font_size_px, size);
        }
    }

    /// Malformed font values fall back individually without losing unrelated
    /// editor values or failing the whole deserialization.
    #[test]
    fn editor_font_malformed_values_fall_back_individually() {
        // Arbitrary family string is kept; out-of-range size -> 16.
        let json = r#"{
            "quick":"collapse","ai":true,"spellcheck_enabled":true,"spellcheck_source":"offline",
            "editor_height":420,"typing_idle_timeout_ms":800,"typing_enabled":true,
            "font_family":"PT Sans","font_size_px":99
        }"#;
        let settings: EditorSettings = serde_json::from_str(json)
            .expect("custom family / out-of-range size must not fail deserialization");
        assert_eq!(settings.font_family, "PT Sans");
        assert_eq!(settings.font_size_px, 16);
        assert_eq!(
            settings.editor_height, 420,
            "unrelated editor value preserved"
        );
        assert!(settings.ai);

        // Blank, whitespace-only and wrong-type family -> default; wrong-type
        // size (string) -> 16.
        for raw in ["\"   \"", "42", "null", "true"] {
            let json2 = format!(
                r#"{{
                    "quick":"collapse","ai":false,"spellcheck_enabled":true,"spellcheck_source":"offline",
                    "editor_height":420,"font_family":{raw},"font_size_px":"big"
                }}"#
            );
            let settings2: EditorSettings = serde_json::from_str(&json2)
                .expect("blank or wrong-type family must not fail deserialization");
            assert_eq!(
                settings2.font_family, "default",
                "font_family={} must fall back",
                raw
            );
            assert_eq!(settings2.font_size_px, 16);
            assert_eq!(settings2.editor_height, 420);
        }

        // Below-min, above-max, negative and float sizes all fall back to 16.
        for raw in ["11", "97", "-5", "16.5", "null"] {
            let json3 = format!(
                r#"{{"quick":"disabled","spellcheck_enabled":true,"spellcheck_source":"offline","font_size_px":{}}}"#,
                raw
            );
            let settings3: EditorSettings = serde_json::from_str(&json3)
                .unwrap_or_else(|e| panic!("font_size_px={} must deserialize: {}", raw, e));
            assert_eq!(
                settings3.font_size_px, 16,
                "font_size_px={} must fall back to 16",
                raw
            );
        }
    }

    /// Helper: build a SettingsManager over a fresh temp config dir.
    fn editor_font_tmp_manager(label: &str) -> (SettingsManager, PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-editor-font-{}-{}-{}",
            label,
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        (manager, dir)
    }

    /// An invalid size setter is rejected and changes neither cache nor disk.
    #[test]
    fn editor_font_invalid_size_setter_changes_nothing() {
        let (manager, dir) = editor_font_tmp_manager("invalid-size");

        manager.set_editor_font_size_px(20).unwrap();
        assert_eq!(manager.get_editor_font_size_px(), 20);

        for invalid in [11, 97, 0, u32::MAX] {
            let result = manager.set_editor_font_size_px(invalid);
            assert!(result.is_err(), "size {} must be rejected", invalid);
            assert_eq!(
                manager.get_editor_font_size_px(),
                20,
                "cache must keep 20 after rejected size {}",
                invalid
            );
            assert_eq!(
                read_disk_settings(&dir).editor.font_size_px,
                20,
                "disk must keep 20 after rejected size {}",
                invalid
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A blank family is rejected and changes neither cache nor disk, while
    /// any non-empty name (including fonts later uninstalled) is accepted.
    #[test]
    fn editor_font_blank_family_setter_changes_nothing() {
        let (manager, dir) = editor_font_tmp_manager("blank-family");

        manager
            .set_editor_font_family("Georgia".to_owned())
            .unwrap();
        assert_eq!(manager.get_editor_font_family(), "Georgia");

        for blank in ["", "   ", "\t"] {
            let result = manager.set_editor_font_family(blank.to_owned());
            assert!(result.is_err(), "family {:?} must be rejected", blank);
            assert_eq!(
                manager.get_editor_font_family(),
                "Georgia",
                "cache must keep Georgia after rejected family {:?}",
                blank
            );
            assert_eq!(
                read_disk_settings(&dir).editor.font_family,
                "Georgia",
                "disk must keep Georgia after rejected family {:?}",
                blank
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Valid font writes survive a reload (new manager over the same dir),
    /// including a family that is a free-form Windows font name.
    #[test]
    fn editor_font_valid_writes_survive_reload() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-editor-font-reload-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();

        {
            let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
            manager
                .set_editor_font_family("consolas".to_owned())
                .unwrap();
            manager
                .set_editor_font_family("PT Sans".to_owned())
                .unwrap();
            manager.set_editor_font_size_px(96).unwrap();
        }

        let manager2 = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let after = manager2.load().unwrap();
        assert_eq!(after.editor.font_family, "PT Sans");
        assert_eq!(after.editor.font_size_px, 96);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Each setter only touches its own field, so concurrent updates never
    /// overwrite the other.
    #[test]
    fn editor_font_setters_preserve_other_field() {
        let (manager, dir) = editor_font_tmp_manager("independent");

        manager
            .set_editor_font_family("georgia".to_owned())
            .unwrap();
        manager.set_editor_font_size_px(24).unwrap();

        let after = manager.load().unwrap();
        assert_eq!(after.editor.font_family, "georgia");
        assert_eq!(after.editor.font_size_px, 24);

        manager.set_editor_font_size_px(18).unwrap();
        assert_eq!(
            manager.load().unwrap().editor.font_family,
            "georgia",
            "size setter must not clobber family"
        );

        manager.set_editor_font_family("arial".to_owned()).unwrap();
        assert_eq!(
            manager.load().unwrap().editor.font_size_px,
            18,
            "family setter must not clobber size"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== UI font tests ====================

    /// AppSettings default UI font is the built-in stack selector and 16 px.
    #[test]
    fn ui_font_defaults() {
        let s = AppSettings::default();
        assert_eq!(s.ui_font_family, "default");
        assert_eq!(s.ui_font_size_px, 16);
    }

    /// Old settings missing both UI font fields deserialize to the default
    /// family and 16 px.
    #[test]
    fn ui_font_deserializes_without_fields() {
        let mut json = serde_json::to_value(AppSettings::default()).unwrap();
        if let Some(obj) = json.as_object_mut() {
            obj.remove("ui_font_family");
            obj.remove("ui_font_size_px");
        }
        let back: AppSettings =
            serde_json::from_value(json).expect("must deserialize without UI font fields");
        assert_eq!(back.ui_font_family, "default");
        assert_eq!(back.ui_font_size_px, 16);
    }

    /// The stable default id and arbitrary installed-family names round-trip
    /// through serde, so a custom font survives config reads and writes.
    #[test]
    fn ui_font_family_round_trip() {
        for family in ["default", "system", "Segoe UI", "PT Sans"] {
            let s = AppSettings {
                ui_font_family: family.to_owned(),
                ..AppSettings::default()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: AppSettings = serde_json::from_str(&json).unwrap();
            assert_eq!(
                back.ui_font_family, family,
                "round-trip failed for {}",
                json
            );
        }
    }

    /// UI font size boundaries (14 and 20) round-trip through serde.
    #[test]
    fn ui_font_size_boundaries_round_trip() {
        for size in [UI_FONT_SIZE_MIN_PX, 17, UI_FONT_SIZE_MAX_PX] {
            let s = AppSettings {
                ui_font_size_px: size,
                ..AppSettings::default()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: AppSettings = serde_json::from_str(&json).unwrap();
            assert_eq!(back.ui_font_size_px, size);
        }
    }

    /// Malformed UI font values fall back individually without losing
    /// unrelated settings or failing the whole deserialization.
    #[test]
    fn ui_font_malformed_values_fall_back_individually() {
        let base = serde_json::to_value(AppSettings::default()).unwrap();

        // Custom family is kept; out-of-range size falls back to 16.
        let mut json = base.clone();
        json["theme"] = serde_json::json!("light");
        json["ui_font_family"] = serde_json::json!("PT Sans");
        json["ui_font_size_px"] = serde_json::json!(99);
        let settings: AppSettings = serde_json::from_value(json)
            .expect("custom family / out-of-range size must not fail deserialization");
        assert_eq!(settings.ui_font_family, "PT Sans");
        assert_eq!(settings.ui_font_size_px, 16);
        assert_eq!(settings.theme, Theme::Light, "unrelated value preserved");

        // Blank, whitespace-only and wrong-type family -> default; wrong-type
        // size (string) -> 16.
        for raw in [
            serde_json::json!("   "),
            serde_json::json!(42),
            serde_json::json!(null),
            serde_json::json!(true),
        ] {
            let mut json = base.clone();
            json["ui_font_family"] = raw;
            json["ui_font_size_px"] = serde_json::json!("big");
            let settings: AppSettings = serde_json::from_value(json)
                .expect("blank or wrong-type family must not fail deserialization");
            assert_eq!(settings.ui_font_family, "default", "family must fall back");
            assert_eq!(settings.ui_font_size_px, 16);
        }

        // Below-min, above-max, negative, float and null sizes fall back to 16.
        for raw in [
            serde_json::json!(13),
            serde_json::json!(21),
            serde_json::json!(-5),
            serde_json::json!(16.5),
            serde_json::json!(null),
        ] {
            let mut json = base.clone();
            json["ui_font_size_px"] = raw;
            let settings: AppSettings = serde_json::from_value(json)
                .expect("out-of-range or wrong-type size must not fail deserialization");
            assert_eq!(settings.ui_font_size_px, 16, "size must fall back to 16");
        }
    }

    /// An invalid UI size setter is rejected and changes neither cache nor
    /// disk. Reuses the shared temp-manager helper.
    #[test]
    fn ui_font_invalid_size_setter_changes_nothing() {
        let (manager, dir) = editor_font_tmp_manager("ui-invalid-size");

        manager.set_ui_font_size_px(18).unwrap();
        assert_eq!(manager.get_ui_font_size_px(), 18);

        for invalid in [13, 21, 0, u32::MAX] {
            let result = manager.set_ui_font_size_px(invalid);
            assert!(result.is_err(), "size {} must be rejected", invalid);
            assert_eq!(
                manager.get_ui_font_size_px(),
                18,
                "cache must keep 18 after rejected size {}",
                invalid
            );
            assert_eq!(
                read_disk_settings(&dir).ui_font_size_px,
                18,
                "disk must keep 18 after rejected size {}",
                invalid
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A blank UI family is rejected and changes neither cache nor disk, while
    /// any non-empty name (including fonts later uninstalled) is accepted.
    #[test]
    fn ui_font_blank_family_setter_changes_nothing() {
        let (manager, dir) = editor_font_tmp_manager("ui-blank-family");

        manager.set_ui_font_family("Georgia".to_owned()).unwrap();
        assert_eq!(manager.get_ui_font_family(), "Georgia");

        for blank in ["", "   ", "\t"] {
            let result = manager.set_ui_font_family(blank.to_owned());
            assert!(result.is_err(), "family {:?} must be rejected", blank);
            assert_eq!(
                manager.get_ui_font_family(),
                "Georgia",
                "cache must keep Georgia after rejected family {:?}",
                blank
            );
            assert_eq!(
                read_disk_settings(&dir).ui_font_family,
                "Georgia",
                "disk must keep Georgia after rejected family {:?}",
                blank
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Valid UI font writes survive a reload (new manager over the same dir).
    #[test]
    fn ui_font_valid_writes_survive_reload() {
        let (manager, dir) = editor_font_tmp_manager("ui-reload");

        manager.set_ui_font_family("Segoe UI".to_owned()).unwrap();
        manager.set_ui_font_size_px(20).unwrap();

        let manager2 = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let after = manager2.load().unwrap();
        assert_eq!(after.ui_font_family, "Segoe UI");
        assert_eq!(after.ui_font_size_px, 20);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Each UI font setter only touches its own field, and neither clobbers
    /// the editor font fields.
    #[test]
    fn ui_font_setters_preserve_other_fields() {
        let (manager, dir) = editor_font_tmp_manager("ui-independent");

        manager.set_ui_font_family("georgia".to_owned()).unwrap();
        manager.set_ui_font_size_px(18).unwrap();

        manager
            .set_editor_font_family("consolas".to_owned())
            .unwrap();
        manager.set_editor_font_size_px(24).unwrap();

        let after = manager.load().unwrap();
        assert_eq!(after.ui_font_family, "georgia");
        assert_eq!(after.ui_font_size_px, 18);
        assert_eq!(after.editor.font_family, "consolas");
        assert_eq!(after.editor.font_size_px, 24);

        manager.set_editor_font_family("arial".to_owned()).unwrap();
        assert_eq!(
            manager.load().unwrap().ui_font_size_px,
            18,
            "editor family setter must not clobber UI size"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== Storage legacy audio cache marker ====================

    fn storage_tmp_manager(label: &str) -> (SettingsManager, PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-storage-{}-{}-{}",
            label,
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        (manager, dir)
    }

    /// The marker defaults to false, both for a fresh struct and for an empty
    /// (or legacy) JSON that predates the field.
    #[test]
    fn storage_legacy_audio_cache_migrated_defaults_false() {
        assert!(!StorageSettings::default().legacy_audio_cache_migrated);
        let s: StorageSettings = serde_json::from_str("{}").unwrap();
        assert!(!s.legacy_audio_cache_migrated);
    }

    /// The marker survives a serde round-trip.
    #[test]
    fn storage_legacy_audio_cache_migrated_round_trip() {
        let s = StorageSettings {
            data_dir: None,
            audio_cache_dir: None,
            legacy_audio_cache_migrated: true,
            legacy_models_migrated: false,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: StorageSettings = serde_json::from_str(&json).unwrap();
        assert!(back.legacy_audio_cache_migrated);
    }

    /// set_legacy_audio_cache_migrated persists `true` to disk and cache, and a
    /// restarted manager over the same dir still reads `true`.
    #[test]
    fn storage_legacy_audio_cache_migrated_persists_and_updates_cache() {
        let (manager, dir) = storage_tmp_manager("migrated-persist");
        assert!(!manager.load().unwrap().storage.legacy_audio_cache_migrated);

        manager.set_legacy_audio_cache_migrated().unwrap();

        assert!(manager.load().unwrap().storage.legacy_audio_cache_migrated);
        assert!(read_disk_settings(&dir).storage.legacy_audio_cache_migrated);

        let restarted = SettingsManager::with_config_dir(dir.clone()).unwrap();
        assert!(
            restarted
                .load()
                .unwrap()
                .storage
                .legacy_audio_cache_migrated
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Later settings updates (field-level and section-level) must preserve the
    /// saved marker on disk and in the cache.
    #[test]
    fn storage_legacy_audio_cache_migrated_preserved_across_updates() {
        let (manager, dir) = storage_tmp_manager("migrated-preserved");
        manager.set_legacy_audio_cache_migrated().unwrap();

        manager.set_speaker_volume(42).unwrap();
        manager
            .set_webview_section(false, 9090, false, true)
            .unwrap();

        assert!(manager.load().unwrap().storage.legacy_audio_cache_migrated);
        assert!(read_disk_settings(&dir).storage.legacy_audio_cache_migrated);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed marker write must not flip the in-memory or on-disk state, so a
    /// later startup retries.
    #[test]
    fn storage_legacy_audio_cache_migrated_failed_write_preserves_false() {
        let (manager, dir) = storage_tmp_manager("migrated-failed");
        let path = dir.join("settings.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        assert!(manager.set_legacy_audio_cache_migrated().is_err());
        assert!(!manager.load().unwrap().storage.legacy_audio_cache_migrated);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== Storage legacy models marker ====================

    /// The marker defaults to false, both for a fresh struct and for an empty
    /// (or legacy) JSON that predates the field.
    #[test]
    fn legacy_model_migrated_defaults_false() {
        assert!(!StorageSettings::default().legacy_models_migrated);
        let s: StorageSettings = serde_json::from_str("{}").unwrap();
        assert!(!s.legacy_models_migrated);
    }

    /// The marker survives a serde round-trip.
    #[test]
    fn legacy_model_migrated_round_trip() {
        let s = StorageSettings {
            data_dir: None,
            audio_cache_dir: None,
            legacy_audio_cache_migrated: false,
            legacy_models_migrated: true,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: StorageSettings = serde_json::from_str(&json).unwrap();
        assert!(back.legacy_models_migrated);
    }

    /// set_legacy_models_migrated persists `true` to disk and cache, and a
    /// restarted manager over the same dir still reads `true`.
    #[test]
    fn legacy_model_migrated_persists_and_updates_cache() {
        let (manager, dir) = storage_tmp_manager("legacy-model-persist");
        assert!(!manager.load().unwrap().storage.legacy_models_migrated);

        manager.set_legacy_models_migrated().unwrap();

        assert!(manager.load().unwrap().storage.legacy_models_migrated);
        assert!(read_disk_settings(&dir).storage.legacy_models_migrated);

        let restarted = SettingsManager::with_config_dir(dir.clone()).unwrap();
        assert!(restarted.load().unwrap().storage.legacy_models_migrated);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Setting the models marker must preserve the independent audio-cache
    /// marker and unrelated settings on disk and in the cache.
    #[test]
    fn legacy_model_migrated_preserves_audio_marker_and_unrelated_settings() {
        let (manager, dir) = storage_tmp_manager("legacy-model-preserved");
        manager.set_legacy_audio_cache_migrated().unwrap();
        manager.set_speaker_volume(42).unwrap();

        manager.set_legacy_models_migrated().unwrap();

        let disk = read_disk_settings(&dir);
        assert!(disk.storage.legacy_models_migrated);
        assert!(
            disk.storage.legacy_audio_cache_migrated,
            "audio-cache marker must survive the models-marker save"
        );
        assert_eq!(disk.audio.speaker_volume, 42);

        let cache = manager.load().unwrap();
        assert!(cache.storage.legacy_models_migrated);
        assert!(cache.storage.legacy_audio_cache_migrated);
        assert_eq!(cache.audio.speaker_volume, 42);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed marker write must not flip the in-memory or on-disk state, so a
    /// later startup retries.
    #[test]
    fn legacy_model_migrated_failed_write_preserves_false() {
        let (manager, dir) = storage_tmp_manager("legacy-model-failed");
        let path = dir.join("settings.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        assert!(manager.set_legacy_models_migrated().is_err());
        assert!(!manager.load().unwrap().storage.legacy_models_migrated);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== Per-hotkey enabled switch ====================

    fn hotkey_tmp_manager(label: &str) -> (SettingsManager, std::path::PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-hotkey-enabled-{}-{}-{}",
            std::process::id(),
            unique,
            label
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        (manager, dir)
    }

    #[test]
    fn set_hotkey_enabled_flag_preserves_combination_and_persists() {
        let (manager, dir) = hotkey_tmp_manager("global-preserve");
        let before = manager.load().unwrap().hotkeys.main_window;

        manager
            .set_hotkey_enabled_flag("main_window", false)
            .unwrap();

        let after = manager.load().unwrap().hotkeys.main_window;
        assert!(!after.enabled);
        assert_eq!(after.key, before.key);
        assert_eq!(after.modifiers, before.modifiers);
        assert!(!read_disk_settings(&dir).hotkeys.main_window.enabled);

        manager
            .set_hotkey_enabled_flag("main_window", true)
            .unwrap();
        assert!(manager.load().unwrap().hotkeys.main_window.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_hotkey_enabled_flag_rejects_unknown_name() {
        let (manager, dir) = hotkey_tmp_manager("global-invalid");
        let error = manager.set_hotkey_enabled_flag("bogus", false).unwrap_err();
        assert!(error.to_string().contains("Invalid hotkey name"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_editor_hotkey_enabled_flag_preserves_combination_and_persists() {
        let (manager, dir) = hotkey_tmp_manager("editor-preserve");
        let before = manager.load().unwrap().hotkeys.editor.edit_word;

        manager
            .set_editor_hotkey_enabled_flag("edit_word", false)
            .unwrap();

        let after = manager.load().unwrap().hotkeys.editor.edit_word;
        assert!(!after.enabled);
        assert_eq!(after.key, before.key);
        assert_eq!(after.modifiers, before.modifiers);
        assert!(!read_disk_settings(&dir).hotkeys.editor.edit_word.enabled);

        manager
            .set_editor_hotkey_enabled_flag("edit_word", true)
            .unwrap();
        assert!(manager.load().unwrap().hotkeys.editor.edit_word.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reset_hotkey_to_default_preserves_disabled_switch() {
        let (manager, dir) = hotkey_tmp_manager("reset-disabled");
        manager
            .set_hotkey_enabled_flag("main_window", false)
            .unwrap();

        let default = manager.reset_hotkey_to_default("main_window").unwrap();

        assert_eq!(default.key, "F3");
        assert!(!default.enabled);
        let stored = manager.load().unwrap().hotkeys.main_window;
        assert_eq!(stored.key, "F3");
        assert!(!stored.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reset_editor_hotkey_preserves_disabled_switch() {
        let (manager, dir) = hotkey_tmp_manager("reset-editor-disabled");
        manager
            .set_editor_hotkey_enabled_flag("accent_homographs", false)
            .unwrap();

        let default = manager.reset_editor_hotkey("accent_homographs").unwrap();

        assert_eq!(default.key, "U");
        assert!(!default.enabled);
        let stored = manager.load().unwrap().hotkeys.editor.accent_homographs;
        assert_eq!(stored.key, "U");
        assert!(!stored.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A disabled editor binding no longer occupies its combination, so another
    /// action may use it. Enabling the first action again is rejected by the
    /// transition check in the command layer (covered by `hotkeys` unit tests).
    #[test]
    fn disabled_editor_binding_is_not_occupied_for_other_actions() {
        let (manager, dir) = hotkey_tmp_manager("disabled-not-occupied");
        manager
            .set_editor_hotkey_enabled_flag("edit_word", false)
            .unwrap();

        let ctrl_e = manager.load().unwrap().hotkeys.editor.edit_word;
        assert!(!ctrl_e.enabled);
        let mut enabled_candidate = ctrl_e.clone();
        enabled_candidate.enabled = true;
        manager
            .set_editor_hotkey("next_tab", &enabled_candidate)
            .unwrap();

        let stored = manager.load().unwrap().hotkeys.editor.next_tab;
        assert_eq!(stored.key, "E");
        assert_eq!(stored.modifiers, ctrl_e.modifiers);
        assert!(stored.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
