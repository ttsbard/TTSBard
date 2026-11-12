use crate::ai::AiProvider;
use crate::events::{AppEvent, TwitchEvent};
use crate::secret_log;
use crate::stress::packs::RuAccentPackDescriptor;
use crate::stress::runtime::RuAccentRuntimeSlot;
use crate::telegram::TelegramClient;
use crate::tts::{
    elevenlabs::ElevenLabsTts,
    fish::FishTts,
    local_http_server::LocalHttpServerTts,
    openai::OpenAiTts,
    piper::runtime::LocalModelTts,
    piper::scanner::{discover_piper_models, merge_piper_roots, scan_piper_models_root},
    pocket::{
        descriptor::PocketModelDescriptor, discover_pocket_models,
        merge_pocket_roots as merge_pocket_model_roots,
        resolved_selection as resolved_pocket_selection, scan_pocket_models_root, with_voice_store,
        PocketTts, PocketVoiceStore,
    },
    registry::TtsProviderEntry,
    registry::TtsProviderRegistry,
    silero::SileroTts,
    TtsProvider, TtsProviderType,
};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// NOTE: Lock ordering hierarchy is no longer needed with unified TtsConfig.
/// The RwLock on tts_config provides efficient concurrent access.
///
/// NOTE: Window settings (opacity, colors, etc.) are now stored in config/windows.json
/// Audio settings are now stored in config/settings.json
/// This AppState only holds runtime state, not configuration.
///
/// Активное плавающее окно
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveWindow {
    /// Нет активного окна
    #[default]
    None,
    /// SoundPanel окно (звуковая панель)
    SoundPanel,
}

/// Унифицированная конфигурация TTS
#[derive(Clone, Debug)]
pub struct TtsConfig {
    pub provider_type: TtsProviderType,
    pub openai_key: Option<String>,
    pub openai_voice: String,
    /// Unified proxy URL (socks5://, socks4://, http://user:pass@host:port)
    pub openai_proxy_url: Option<String>,
    pub fish_api_key: Option<String>,
    pub fish_reference_id: String,
    pub fish_proxy_url: Option<String>,
    pub fish_format: String,
    pub fish_temperature: f32,
    pub fish_sample_rate: u32,
    pub elevenlabs_api_key: Option<String>,
    pub elevenlabs_voice_id: String,
    pub elevenlabs_model_id: String,
    pub elevenlabs_output_format: String,
    pub elevenlabs_stability: f32,
    pub elevenlabs_similarity_boost: f32,
    pub elevenlabs_style: f32,
    pub elevenlabs_use_speaker_boost: bool,
    pub elevenlabs_proxy_url: Option<String>,
    pub local_url: String,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            provider_type: TtsProviderType::OpenAi,
            openai_key: None,
            openai_voice: "alloy".to_string(),
            openai_proxy_url: None,
            fish_api_key: None,
            fish_reference_id: String::new(),
            fish_proxy_url: None,
            fish_format: "mp3".to_string(),
            fish_temperature: 0.7,
            fish_sample_rate: 44100,
            elevenlabs_api_key: None,
            elevenlabs_voice_id: String::new(),
            elevenlabs_model_id: String::new(),
            elevenlabs_output_format: crate::tts::elevenlabs::DEFAULT_OUTPUT_FORMAT.to_string(),
            elevenlabs_stability: crate::tts::elevenlabs::DEFAULT_STABILITY,
            elevenlabs_similarity_boost: crate::tts::elevenlabs::DEFAULT_SIMILARITY_BOOST,
            elevenlabs_style: crate::tts::elevenlabs::DEFAULT_STYLE,
            elevenlabs_use_speaker_boost: crate::tts::elevenlabs::DEFAULT_USE_SPEAKER_BOOST,
            elevenlabs_proxy_url: None,
            local_url: "http://127.0.0.1:8124".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    /// Отправитель событий для MPSC канала
    pub event_sender: Arc<Mutex<Option<Sender<AppEvent>>>>,

    /// WebView service (settings + event sender)
    pub webview: Arc<crate::webview::service::WebViewService>,

    /// Input server service (settings, pending-review inbox, runtime status)
    pub input_server: Arc<crate::input_server::InputServerService>,

    /// OCR service (desired settings, runtime lifecycle status, capture hotkey)
    pub ocr: Arc<crate::ocr::OcrService>,

    /// Включены ли хоткеи (runtime only, synced with settings.json)
    pub hotkey_enabled: Arc<Mutex<bool>>,

    /// Унифицированная конфигурация TTS ( RwLock для эффективного чтения)
    pub tts_config: Arc<RwLock<TtsConfig>>,

    /// TTS провайдеры (registry)
    pub tts_registry: Arc<Mutex<TtsProviderRegistry>>,

    /// Thread-safe snapshot of discovered local RUAccent packs.
    ///
    /// Produced purely from filesystem/metadata discovery: no ONNX session is
    /// created and no model files are loaded here.
    pub ruaccent_packs: Arc<RwLock<Vec<RuAccentPackDescriptor>>>,

    /// Lazy native RUAccent runtime slots keyed by pack ID.
    ///
    /// Slots never create sessions or read model bytes. Refreshing discovery
    /// replaces an obsolete slot and preserves the existing one only when a
    /// same-ID descriptor still points at the same pack root and omograph
    /// model.
    pub ruaccent_runtime_slots: Arc<RwLock<HashMap<String, RuAccentRuntimeSlot>>>,

    /// Editor service (preprocessor, history, spellcheck)
    pub editor: Arc<crate::editor::EditorService>,

    /// Активное плавающее окно (для взаимного исключения хоткеев)
    pub active_window: Arc<Mutex<ActiveWindow>>,

    /// Twitch service (settings, connection status, event sender)
    pub twitch: Arc<crate::twitch::TwitchService>,

    /// VTube Studio service (settings, connection, typing state)
    pub vtube_studio: Arc<crate::vtube_studio::VTubeStudioService>,

    /// VRChat OSC service (settings, typing state, output datagrams)
    pub vrchat: Arc<crate::vrchat::VrchatService>,

    /// Backend ready flag - set to true when all initialization is complete
    pub backend_ready: Arc<AtomicBool>,

    /// Hotkey recording flag - set to true when user is recording a new hotkey
    /// When true, hotkey handlers should ignore their triggers
    pub hotkey_recording_in_progress: Arc<AtomicBool>,

    /// Tokio runtime для async operations
    /// Arc позволяет клонировать AppState и сохраняет runtime живым
    pub runtime: Arc<tokio::runtime::Runtime>,

    /// Кэшированные аудио устройства (device_id -> Device)
    pub cached_devices: Arc<RwLock<HashMap<String, cpal::Device>>>,

    /// Cached AI client for text correction
    pub ai_client: Arc<Mutex<Option<Arc<AiProvider>>>>,

    /// Hash of current AI settings (for cache invalidation)
    pub ai_settings_hash: Arc<AtomicU64>,

    /// Playback manager for queue/pause/resume
    pub playback_manager: Arc<Mutex<Option<Arc<crate::playback::PlaybackManager>>>>,

    /// Shared settings cache (same Arc as SettingsManager.cache) for hot-path reads
    pub settings_cache: Arc<RwLock<crate::config::AppSettings>>,

    /// Keyboard hook lifecycle manager (Windows only; no-op on other platforms)
    pub soundpanel_hook: Arc<Mutex<Option<crate::soundpanel::HookManager>>>,

    /// Токен отмены для всех фоновых серверов
    pub shutdown: CancellationToken,

    /// Флаг однократного запуска graceful shutdown (первый вызов выигрывает)
    pub shutdown_started: Arc<AtomicBool>,

    /// Сохранённый HWND внешнего окна, бывшего на переднем плане перед активацией TTSBard
    pub previous_foreground_hwnd: Arc<Mutex<Option<isize>>>,

    /// Сохранённый HWND внешнего окна, бывшего на переднем плане перед активацией SoundPanel.
    ///
    /// Отдельный слот: не конфликтует с `previous_foreground_hwnd` (главное окно),
    /// потоки не перезаписывают друг друга.
    pub soundpanel_previous_foreground_hwnd: Arc<Mutex<Option<isize>>>,

    /// Поколение отложенного скрытия SoundPanel по потере фокуса.
    ///
    /// Инкрементируется при каждом планировании/отмене отложенного скрытия.
    /// Отложенный callback сравнивает своё захваченное поколение с текущим и
    /// скрывает панель только если оно осталось актуальным.
    pub soundpanel_blur_hide_generation: Arc<AtomicU64>,

    /// Async mutex serialising concurrent provider selection operations.
    /// Selected by concrete provider ID.
    pub selection_mutex: Arc<tokio::sync::Mutex<()>>,

    /// One-shot messages produced during backend startup before the WebView is ready.
    pub pending_notifications: Arc<Mutex<Vec<String>>>,
    pub pending_startup_errors: Arc<Mutex<Vec<String>>>,
}

impl AppState {
    pub fn new() -> Self {
        let (twitch_event_tx, _) = broadcast::channel::<TwitchEvent>(100);
        let twitch = Arc::new(crate::twitch::TwitchService::new(twitch_event_tx));

        // Создаём runtime один раз при инициализации AppState
        // Arc сохраняет runtime живым пока живёт AppState
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .thread_stack_size(8 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime"),
        );

        let vtube_studio = Arc::new(crate::vtube_studio::VTubeStudioService::new());

        let vrchat = Arc::new(crate::vrchat::VrchatService::new());

        let editor = Arc::new(crate::editor::EditorService::new());

        let webview = Arc::new(crate::webview::service::WebViewService::new());

        let input_server = Arc::new(crate::input_server::InputServerService::new());

        let ocr = Arc::new(crate::ocr::OcrService::new());

        Self {
            event_sender: Arc::new(Mutex::new(None)),
            webview,
            input_server,
            ocr,
            hotkey_enabled: Arc::new(Mutex::new(true)), // default true
            tts_config: Arc::new(RwLock::new(TtsConfig::default())),
            tts_registry: Arc::new(Mutex::new(TtsProviderRegistry::new())),
            ruaccent_packs: Arc::new(RwLock::new(Vec::new())),
            ruaccent_runtime_slots: Arc::new(RwLock::new(HashMap::new())),
            editor,
            active_window: Arc::new(Mutex::new(ActiveWindow::None)),
            twitch,
            vtube_studio,
            vrchat,
            backend_ready: Arc::new(AtomicBool::new(false)),
            playback_manager: Arc::new(Mutex::new(None)),
            hotkey_recording_in_progress: Arc::new(AtomicBool::new(false)),
            runtime,
            cached_devices: Arc::new(RwLock::new(HashMap::new())),
            ai_client: Arc::new(Mutex::new(None)),
            ai_settings_hash: Arc::new(AtomicU64::new(0)),
            settings_cache: Arc::new(RwLock::new(Default::default())),
            soundpanel_hook: Arc::new(Mutex::new(None)),
            shutdown: CancellationToken::new(),
            shutdown_started: Arc::new(AtomicBool::new(false)),
            previous_foreground_hwnd: Arc::new(Mutex::new(None)),
            soundpanel_previous_foreground_hwnd: Arc::new(Mutex::new(None)),
            soundpanel_blur_hide_generation: Arc::new(AtomicU64::new(0)),
            selection_mutex: Arc::new(tokio::sync::Mutex::new(())),
            pending_notifications: Arc::new(Mutex::new(Vec::new())),
            pending_startup_errors: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn push_notification(&self, message: String) {
        self.pending_notifications.lock().push(message);
    }

    pub fn take_notifications(&self) -> Vec<String> {
        std::mem::take(&mut *self.pending_notifications.lock())
    }

    pub fn push_startup_error(&self, message: String) {
        self.pending_startup_errors.lock().push(message);
    }

    pub fn take_startup_errors(&self) -> Vec<String> {
        std::mem::take(&mut *self.pending_startup_errors.lock())
    }

    pub fn set_event_sender(&self, sender: Sender<AppEvent>) {
        *self.event_sender.lock() = Some(sender);
    }

    pub fn emit_event(&self, event: AppEvent) {
        debug!(event = ?std::mem::discriminant(&event), "Called with");
        // Send to main event channel
        if let Some(ref sender) = *self.event_sender.lock() {
            match sender.send(event.clone()) {
                Ok(_) => {}
                Err(e) => warn!(error = %e, "Failed to send event to main channel"),
            }
        }
    }

    pub fn get_event_sender(&self) -> Option<Sender<AppEvent>> {
        self.event_sender.lock().clone()
    }

    pub fn is_hotkey_enabled(&self) -> bool {
        *self.hotkey_enabled.lock()
    }

    pub fn set_hotkey_enabled(&self, enabled: bool) {
        *self.hotkey_enabled.lock() = enabled;
    }

    pub fn is_hotkey_recording(&self) -> bool {
        self.hotkey_recording_in_progress
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_hotkey_recording(&self, recording: bool) {
        self.hotkey_recording_in_progress
            .store(recording, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn begin_shutdown(&self) -> bool {
        !self.shutdown_started.swap(true, Ordering::SeqCst)
    }

    #[allow(dead_code)]
    pub fn get_tts_provider_type(&self) -> TtsProviderType {
        self.tts_config.read().provider_type
    }

    pub fn set_tts_provider_type(&self, provider: TtsProviderType) {
        self.tts_config.write().provider_type = provider;
        self.emit_event(AppEvent::TtsProviderChanged(provider));
    }

    #[allow(dead_code)]
    pub fn get_openai_api_key(&self) -> Option<String> {
        self.tts_config.read().openai_key.clone()
    }

    pub fn set_openai_api_key(&self, key: Option<String>) {
        self.tts_config.write().openai_key = key;
    }

    pub fn init_openai_tts(&self, api_key: String) {
        info!(has_api_key = !api_key.is_empty(), "init_openai_tts called");
        let mut tts = OpenAiTts::new(api_key);
        let config = self.tts_config.read();
        let voice = config.openai_voice.clone();
        tts.set_voice(voice.clone());
        if let Some(proxy_url) = &config.openai_proxy_url {
            tts.set_proxy(Some(proxy_url.clone()));
        }
        drop(config);

        // Add event sender if available
        if let Some(event_tx) = self.get_event_sender() {
            tts = tts.with_event_tx(event_tx);
        }

        info!(
            voice,
            has_proxy = tts.get_proxy_url().is_some(),
            "Created OpenAiTts"
        );

        let mut registry = self.tts_registry.lock();
        registry.add_or_replace(TtsProviderEntry {
            id: "openai".to_string(),
            display_name: "OpenAI TTS".to_string(),
            provider: TtsProvider::OpenAi(tts),
        });
        info!("OpenAI TTS provider registered");
    }

    pub fn init_local_tts(&self, url: String) {
        info!(safe_url = %secret_log::safe_url_for_log(&url), "Initializing Local TTS");

        let mut tts = LocalHttpServerTts::new();
        tts.set_url(url);

        // Add event sender if available
        if let Some(event_tx) = self.get_event_sender() {
            tts = tts.with_event_tx(event_tx);
        }

        info!(safe_url = %secret_log::safe_url_for_log(tts.get_url()), "Created LocalHttpServerTts");

        let mut registry = self.tts_registry.lock();
        registry.add_or_replace(TtsProviderEntry {
            id: "local-http".to_string(),
            display_name: "Local HTTP TTS".to_string(),
            provider: TtsProvider::Local(tts),
        });
        info!("Local TTS provider registered");
    }

    pub fn init_silero_tts(
        &self,
        telegram_client_arc: Arc<tokio::sync::Mutex<Option<TelegramClient>>>,
    ) {
        info!("Initializing Silero TTS...");

        // Создаём SileroTts с Arc на Telegram клиент
        // SileroTts будет извлекать клиент при необходимости
        let mut tts = SileroTts::with_telegram_client(telegram_client_arc);

        // Add event sender if available
        if let Some(event_tx) = self.get_event_sender() {
            info!("Adding event_tx to SileroTts");
            tts = tts.with_event_tx(event_tx);
        } else {
            warn!("No event_tx available, SileroTts will not send events");
        }

        info!("Created SileroTts with Telegram client Arc");
        let mut registry = self.tts_registry.lock();
        registry.add_or_replace(TtsProviderEntry {
            id: "silero".to_string(),
            display_name: "Silero TTS".to_string(),
            provider: TtsProvider::Silero(tts),
        });
        info!("Silero TTS provider registered");
    }

    pub fn init_fish_audio_tts(&self, api_key: String) {
        let mut tts = FishTts::new(api_key);
        let config = self.tts_config.read();
        tts.set_reference_id(config.fish_reference_id.clone());
        tts.set_format(config.fish_format.clone());
        tts.set_temperature(config.fish_temperature);
        tts.set_sample_rate(config.fish_sample_rate);
        if let Some(proxy_url) = &config.fish_proxy_url {
            tts.set_proxy(Some(proxy_url.clone()));
        }
        drop(config);

        if let Some(event_tx) = self.get_event_sender() {
            tts = tts.with_event_tx(event_tx);
        }

        let mut registry = self.tts_registry.lock();
        registry.add_or_replace(TtsProviderEntry {
            id: "fish".to_string(),
            display_name: "Fish Audio TTS".to_string(),
            provider: TtsProvider::Fish(tts),
        });
        info!("Fish Audio TTS provider registered");
    }

    #[allow(dead_code)]
    pub fn get_fish_audio_api_key(&self) -> Option<String> {
        self.tts_config.read().fish_api_key.clone()
    }

    pub fn set_fish_audio_api_key(&self, key: Option<String>) {
        self.tts_config.write().fish_api_key = key;
    }

    /// Publish a Fish form submission as a single runtime configuration update.
    pub fn set_fish_audio_connection_settings(
        &self,
        key: String,
        format: String,
        temperature: f32,
        sample_rate: u32,
    ) {
        let mut config = self.tts_config.write();
        config.fish_api_key = Some(key);
        config.fish_format = format;
        config.fish_temperature = temperature;
        config.fish_sample_rate = sample_rate;
    }

    pub fn set_fish_audio_reference_id(&self, reference_id: String) {
        let mut registry = self.tts_registry.lock();
        if let Some(entry) = registry.get_mut("fish") {
            if let TtsProvider::Fish(ref mut tts) = &mut entry.provider {
                tts.set_reference_id(reference_id.clone());
            }
        }
        drop(registry);
        self.tts_config.write().fish_reference_id = reference_id;
    }

    pub fn set_fish_audio_proxy(&self, proxy_url: Option<String>) {
        let mut registry = self.tts_registry.lock();
        if let Some(entry) = registry.get_mut("fish") {
            if let TtsProvider::Fish(ref mut tts) = &mut entry.provider {
                tts.set_proxy(proxy_url.clone());
            }
        }
        drop(registry);
        self.tts_config.write().fish_proxy_url = proxy_url;
    }

    pub fn set_fish_audio_format(&self, format: String) {
        self.tts_config.write().fish_format = format;
    }

    pub fn set_fish_audio_temperature(&self, temperature: f32) {
        self.tts_config.write().fish_temperature = temperature;
    }

    pub fn set_fish_audio_sample_rate(&self, sample_rate: u32) {
        self.tts_config.write().fish_sample_rate = sample_rate;
    }

    // ========== ElevenLabs TTS ==========

    pub fn init_elevenlabs_tts(&self, api_key: String) {
        let mut tts = ElevenLabsTts::new(api_key);
        let config = self.tts_config.read();
        tts.set_voice_id(config.elevenlabs_voice_id.clone());
        tts.set_model_id(config.elevenlabs_model_id.clone());
        tts.set_output_format(config.elevenlabs_output_format.clone());
        tts.set_stability(config.elevenlabs_stability);
        tts.set_similarity_boost(config.elevenlabs_similarity_boost);
        tts.set_style(config.elevenlabs_style);
        tts.set_use_speaker_boost(config.elevenlabs_use_speaker_boost);
        if let Some(proxy_url) = &config.elevenlabs_proxy_url {
            tts.set_proxy(Some(proxy_url.clone()));
        }
        drop(config);

        if let Some(event_tx) = self.get_event_sender() {
            tts = tts.with_event_tx(event_tx);
        }

        let mut registry = self.tts_registry.lock();
        registry.add_or_replace(TtsProviderEntry {
            id: "elevenlabs".to_string(),
            display_name: "ElevenLabs TTS".to_string(),
            provider: TtsProvider::ElevenLabs(tts),
        });
        info!("ElevenLabs TTS provider registered");
    }

    #[allow(dead_code)]
    pub fn get_elevenlabs_api_key(&self) -> Option<String> {
        self.tts_config.read().elevenlabs_api_key.clone()
    }

    pub fn set_elevenlabs_api_key(&self, key: Option<String>) {
        self.tts_config.write().elevenlabs_api_key = key;
    }

    /// Publish an ElevenLabs form submission as a single runtime config update.
    #[allow(clippy::too_many_arguments)] // Mirrors the persisted ElevenLabs settings field set.
    pub fn set_elevenlabs_connection_settings(
        &self,
        api_key: String,
        model_id: String,
        output_format: String,
        stability: f32,
        similarity_boost: f32,
        style: f32,
        use_speaker_boost: bool,
    ) {
        let mut config = self.tts_config.write();
        config.elevenlabs_api_key = Some(api_key);
        config.elevenlabs_model_id = model_id;
        config.elevenlabs_output_format = output_format;
        config.elevenlabs_stability = stability;
        config.elevenlabs_similarity_boost = similarity_boost;
        config.elevenlabs_style = style;
        config.elevenlabs_use_speaker_boost = use_speaker_boost;
    }

    pub fn set_elevenlabs_voice_id(&self, voice_id: String) {
        let mut registry = self.tts_registry.lock();
        if let Some(entry) = registry.get_mut("elevenlabs") {
            if let TtsProvider::ElevenLabs(ref mut tts) = &mut entry.provider {
                tts.set_voice_id(voice_id.clone());
            }
        }
        drop(registry);
        self.tts_config.write().elevenlabs_voice_id = voice_id;
    }

    pub fn set_elevenlabs_model_settings(
        &self,
        model_id: String,
        style: f32,
        use_speaker_boost: bool,
    ) {
        let mut registry = self.tts_registry.lock();
        if let Some(entry) = registry.get_mut("elevenlabs") {
            if let TtsProvider::ElevenLabs(ref mut tts) = &mut entry.provider {
                tts.set_model_id(model_id.clone());
                tts.set_style(style);
                tts.set_use_speaker_boost(use_speaker_boost);
            }
        }
        drop(registry);
        let mut config = self.tts_config.write();
        config.elevenlabs_model_id = model_id;
        config.elevenlabs_style = style;
        config.elevenlabs_use_speaker_boost = use_speaker_boost;
    }

    pub fn set_elevenlabs_proxy(&self, proxy_url: Option<String>) {
        let mut registry = self.tts_registry.lock();
        if let Some(entry) = registry.get_mut("elevenlabs") {
            if let TtsProvider::ElevenLabs(ref mut tts) = &mut entry.provider {
                tts.set_proxy(proxy_url.clone());
            }
        }
        drop(registry);
        self.tts_config.write().elevenlabs_proxy_url = proxy_url;
    }

    /// Set OpenAI voice (simplified with unified TtsConfig)
    pub fn set_openai_voice(&self, voice: String) {
        let mut registry = self.tts_registry.lock();
        if let Some(entry) = registry.get_mut("openai") {
            if let TtsProvider::OpenAi(ref mut tts) = &mut entry.provider {
                tts.set_voice(voice.clone());
            }
        }
        drop(registry);
        self.tts_config.write().openai_voice = voice;
    }

    /// Set OpenAI proxy URL (simplified with unified TtsConfig)
    pub fn set_openai_proxy(&self, proxy_url: Option<String>) {
        let mut registry = self.tts_registry.lock();
        if let Some(entry) = registry.get_mut("openai") {
            if let TtsProvider::OpenAi(ref mut tts) = &mut entry.provider {
                tts.set_proxy(proxy_url.clone());
            }
        }
        drop(registry);
        self.tts_config.write().openai_proxy_url = proxy_url;
    }

    pub fn get_active_provider(&self) -> Option<TtsProvider> {
        let registry = self.tts_registry.lock();
        registry.active().map(|e| e.provider.clone())
    }

    #[allow(dead_code)]
    pub fn get_local_tts_url(&self) -> String {
        self.tts_config.read().local_url.clone()
    }

    pub fn set_local_tts_url(&self, url: String) {
        self.tts_config.write().local_url = url;
    }

    // ========== Active Window Management (взаимное исключение хоткеев) ==========

    /// Установить активное окно
    pub fn set_active_window(&self, window: ActiveWindow) {
        *self.active_window.lock() = window;
    }

    // ========== Twitch Event Management ==========

    /// Отправить событие Twitch
    pub fn send_twitch_event(&self, event: TwitchEvent) {
        self.twitch.send_event(event);
    }

    // ========== AI Client Caching ==========

    /// Get cached AI client or create if needed/invalidated
    ///
    /// This method checks if the cached client is still valid by comparing
    /// the hash of current AI settings with the stored hash. If they match,
    /// the cached client is returned. Otherwise, a new client is created.
    ///
    /// # Arguments
    /// * `ai_settings` - Current AI settings
    /// * `network_settings` - Current network settings (for proxy configuration)
    ///
    /// # Returns
    /// Arc<AiProvider> - The cached or newly created AI client
    ///
    /// # Errors
    /// Returns String if client creation fails
    pub fn get_or_create_ai_client(
        &self,
        ai_settings: &crate::config::AiSettings,
        network_settings: &crate::config::NetworkSettings,
    ) -> Result<Arc<AiProvider>, String> {
        let current_hash = crate::ai::hash_ai_settings(ai_settings);

        // Check if cache is valid
        if self
            .ai_settings_hash
            .load(std::sync::atomic::Ordering::Relaxed)
            == current_hash
        {
            if let Some(client) = self.ai_client.lock().as_ref() {
                debug!("Using cached AI client (hash: {})", current_hash);
                return Ok(client.clone());
            }
        }

        // Create new client
        debug!("Creating new AI client (hash: {})", current_hash);
        let client = crate::ai::create_ai_client(ai_settings, network_settings)
            .map_err(|e| format!("Failed to create AI client: {}", e))?;
        let client = Arc::new(client);

        // Update cache
        *self.ai_client.lock() = Some(client.clone());
        self.ai_settings_hash
            .store(current_hash, std::sync::atomic::Ordering::Relaxed);

        Ok(client)
    }

    /// Invalidate AI client cache
    ///
    /// Call this when AI settings change to force recreation of the client
    /// on the next request.
    pub fn invalidate_ai_client(&self) {
        debug!("Invalidating AI client cache");
        self.ai_client.lock().take();
        self.ai_settings_hash
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Discover and register Piper providers across the shared model search
    /// roots (ROADMAP-117): the process cwd first, then the effective data
    /// root (`storage.data_dir` override or the default). There is no separate
    /// exe/resource root: models next to the exe are found whenever the cwd
    /// equals the exe directory.
    ///
    /// The cwd root is scanned read-only; only the managed data root may have
    /// its `models/piper` directory created. Descriptors are deduplicated by
    /// ID after per-root validation, so a broken candidate in the cwd never
    /// blocks a valid same-ID model in the data root, while a valid cwd model
    /// wins over the data-root duplicate.
    /// Does NOT select any Piper provider — the current built-in provider is preserved.
    /// Does NOT create ONNX sessions (they are lazily initialized on first use).
    pub fn register_piper_providers(&self) {
        let per_root: Vec<Vec<_>> = crate::paths::model_search_roots()
            .iter()
            .map(|root| {
                if root.managed {
                    discover_piper_models(&root.path)
                } else {
                    scan_piper_models_root(&root.path)
                }
            })
            .collect();
        let descriptors = merge_piper_roots(per_root);

        let count = descriptors.len();
        let mut registry = self.tts_registry.lock();

        for desc in &descriptors {
            let tts = LocalModelTts::from_descriptor(desc);
            registry.add_or_replace(TtsProviderEntry {
                id: desc.id.clone(),
                display_name: desc.display_name.clone(),
                provider: TtsProvider::Piper(Arc::new(tts)),
            });
        }

        info!(count = count, "Piper provider registration complete");
    }

    /// Discover and register Pocket TTS packages across the same shared model
    /// roots as Piper. Voice selections are loaded from `pocket-voices.json`
    /// and applied to the provider instances. Repeated scans replace entries
    /// for changed packages and drop entries whose directory disappeared.
    ///
    /// No ONNX session is created here: models stay lazy until selection,
    /// readiness preparation, or synthesis.
    pub fn register_pocket_providers(&self) -> usize {
        let per_root: Vec<Vec<_>> = crate::paths::model_search_roots()
            .iter()
            .map(|root| {
                if root.managed {
                    discover_pocket_models(&root.path)
                } else {
                    scan_pocket_models_root(&root.path)
                }
            })
            .collect();
        let descriptors = merge_pocket_model_roots(per_root);
        let count = with_voice_store(|store| {
            let mut registry = self.tts_registry.lock();
            let saved_id = self.settings_cache.read().tts.provider_id.clone();
            reconcile_pocket_registry(&mut registry, &descriptors, store, saved_id.as_deref())
        });

        info!(count = count, "Pocket TTS provider registration complete");
        count
    }

    /// Snapshot the currently discovered RUAccent packs.
    #[allow(dead_code)]
    pub fn get_ruaccent_packs(&self) -> Vec<RuAccentPackDescriptor> {
        self.ruaccent_packs.read().clone()
    }

    /// Get the lazy native runtime slot for a pack ID, if the pack exists with
    /// a native-ready capability. Never loads models or creates sessions.
    #[allow(dead_code)]
    pub fn get_ruaccent_runtime_slot(&self, pack_id: &str) -> Option<RuAccentRuntimeSlot> {
        self.ruaccent_runtime_slots.read().get(pack_id).cloned()
    }

    /// Snapshot every RUAccent runtime slot except one selected ID. The caller
    /// can unload the returned slots on a blocking worker without holding the
    /// registry lock or blocking an async command thread.
    pub fn ruaccent_runtime_slots_except(&self, keep_id: Option<&str>) -> Vec<RuAccentRuntimeSlot> {
        self.ruaccent_runtime_slots
            .read()
            .iter()
            .filter(|(id, _)| Some(id.as_str()) != keep_id)
            .map(|(_, slot)| slot.clone())
            .collect()
    }

    /// Re-discover RUAccent packs across the given search roots and replace the
    /// stored snapshot. Returns the number of packs found; does not load any
    /// model or create an ONNX session.
    ///
    /// Runtime slots are reconciled with the new snapshot: slots for packs that
    /// disappeared are dropped, a slot is preserved only when its same-ID
    /// descriptor still points at the same pack root and omograph model, and a
    /// fresh slot is created for new/changed packs.
    pub fn refresh_ruaccent_packs(&self, search_roots: &[std::path::PathBuf]) -> usize {
        let descriptors = crate::stress::packs::discover_ruaccent_packs(search_roots);
        let count = descriptors.len();
        *self.ruaccent_packs.write() = descriptors.clone();

        let by_id: HashMap<&str, &RuAccentPackDescriptor> =
            descriptors.iter().map(|d| (d.id.as_str(), d)).collect();

        let mut slots = self.ruaccent_runtime_slots.write();
        slots.retain(|id, slot| {
            by_id
                .get(id.as_str())
                .map(|desc| {
                    slot.descriptor().pack_root == desc.pack_root
                        && slot.descriptor().omograph_model_id == desc.omograph_model_id
                })
                .unwrap_or_else(|| slot.is_live())
        });
        for (id, desc) in &by_id {
            if !slots.contains_key(*id) {
                slots.insert((*id).to_string(), RuAccentRuntimeSlot::new((*desc).clone()));
            }
        }

        info!(count = count, "RUAccent pack discovery complete");
        count
    }

    /// Re-scan RUAccent packs with a bounded retry when any search root has an
    /// existing `models/ruaccent` directory but the scan finds nothing.
    ///
    /// A transient boot-time filesystem failure must not read as "models
    /// removed": that would make the startup autoload fail and persist-disable
    /// the feature (ROADMAP-090). Only this suspicious combination is retried;
    /// a genuinely absent models directory in every root and a persistently
    /// empty scan keep the "no models" meaning.
    pub fn refresh_ruaccent_packs_with_retry(&self, search_roots: &[std::path::PathBuf]) -> usize {
        refresh_ruaccent_packs_with_retry_core(
            search_roots,
            |roots| self.refresh_ruaccent_packs(roots),
            || std::thread::sleep(std::time::Duration::from_millis(300)),
        )
    }

    /// Prepare, persist and publish one concrete provider selection.
    /// Runtime state changes only after preparation and persistence succeed.
    pub(crate) async fn select_tts_provider<F>(&self, id: String, persist: F) -> Result<(), String>
    where
        F: FnOnce(String, Option<TtsProviderType>) -> Result<(), String> + Send + 'static,
    {
        let _guard = self.selection_mutex.lock().await;

        let provider = {
            let registry = self.tts_registry.lock();
            registry
                .get(&id)
                .map(|e| e.provider.clone())
                .ok_or_else(|| format!("Unknown provider ID: {}", id))?
        };

        tokio::task::spawn_blocking(move || provider.prepare())
            .await
            .map_err(|e| format!("Selection task panicked: {}", e))?
            .map_err(|e| format!("Provider preparation failed: {}", e))?;

        let legacy_type = builtin_type_for_id(&id);
        let persist_id = id.clone();
        tokio::task::spawn_blocking(move || persist(persist_id, legacy_type))
            .await
            .map_err(|e| format!("Persist task panicked: {}", e))?
            .map_err(|e| format!("Failed to persist selection: {}", e))?;

        {
            let mut registry = self.tts_registry.lock();
            registry
                .select(&id)
                .expect("provider remains registered during selection transaction");
        }

        if let Some(tp) = legacy_type {
            self.tts_config.write().provider_type = tp;
        }

        info!(id, legacy = ?legacy_type, "TTS provider selected");
        Ok(())
    }
}

/// Reconcile Pocket provider entries with a freshly scanned descriptor list.
///
/// The caller must hold the Pocket voice-store transaction lock and pass the
/// store loaded inside it. This keeps a concurrent voice change that already
/// updated the registry from being overwritten by a refresh using an older
/// preference snapshot.
fn reconcile_pocket_registry(
    registry: &mut TtsProviderRegistry,
    descriptors: &[PocketModelDescriptor],
    store: &PocketVoiceStore,
    saved_id: Option<&str>,
) -> usize {
    let discovered: std::collections::HashSet<String> = descriptors
        .iter()
        .map(|descriptor| descriptor.id.clone())
        .collect();
    let stale: Vec<String> = registry
        .iter()
        .filter(|entry| {
            matches!(entry.provider, TtsProvider::Pocket(_)) && !discovered.contains(&entry.id)
        })
        .map(|entry| entry.id.clone())
        .collect();
    for id in stale {
        registry.remove(&id);
    }

    for desc in descriptors {
        let selection = resolved_pocket_selection(desc, store);
        let tts = PocketTts::new(desc.clone(), selection);
        registry.add_or_replace(TtsProviderEntry {
            id: desc.id.clone(),
            display_name: desc.display_name.clone(),
            provider: TtsProvider::Pocket(tts),
        });
    }

    if registry.active_id().is_none() {
        registry.restore_saved_or_first(saved_id);
    }
    descriptors.len()
}

/// Bounded RUAccent retry policy with injectable scan and pause.
///
/// Production ([`AppState::refresh_ruaccent_packs_with_retry`]) passes the real
/// discovery scan and a 300 ms pause; tests inject counters to assert the exact
/// number of attempts without sleeping.
fn refresh_ruaccent_packs_with_retry_core<S, P>(
    search_roots: &[std::path::PathBuf],
    mut scan: S,
    mut pause: P,
) -> usize
where
    S: FnMut(&[std::path::PathBuf]) -> usize,
    P: FnMut(),
{
    let mut count = scan(search_roots);
    if count > 0 {
        return count;
    }
    let models_dir_exists = search_roots.iter().any(|root| {
        root.join(crate::stress::packs::PRIMARY_MODELS_SUBDIR)
            .is_dir()
    });
    if !models_dir_exists {
        return count;
    }
    for attempt in 1..=2 {
        tracing::warn!(
            attempt,
            "RUAccent scan found no packs in an existing models directory; retrying"
        );
        pause();
        count = scan(search_roots);
        if count > 0 {
            return count;
        }
    }
    count
}

/// Map a concrete provider ID to its built-in TtsProviderType, if any.
pub(crate) fn builtin_type_for_id(id: &str) -> Option<TtsProviderType> {
    match id {
        "openai" => Some(TtsProviderType::OpenAi),
        "silero" => Some(TtsProviderType::Silero),
        "local-http" => Some(TtsProviderType::Local),
        "fish" => Some(TtsProviderType::Fish),
        "elevenlabs" => Some(TtsProviderType::ElevenLabs),
        _ => None,
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::piper::runtime::LocalModelTts;

    fn dummy_local() -> TtsProvider {
        TtsProvider::Local(LocalHttpServerTts::new())
    }

    fn failing_piper() -> TtsProvider {
        TtsProvider::Piper(Arc::new(LocalModelTts::new(
            "/nonexistent/model.onnx",
            "/nonexistent/model.onnx.json",
        )))
    }

    fn entry(id: &str, name: &str, provider: TtsProvider) -> TtsProviderEntry {
        TtsProviderEntry {
            id: id.to_string(),
            display_name: name.to_string(),
            provider,
        }
    }

    #[test]
    fn builtin_type_for_id_openai() {
        assert_eq!(builtin_type_for_id("openai"), Some(TtsProviderType::OpenAi));
    }

    #[test]
    fn builtin_type_for_id_silero() {
        assert_eq!(builtin_type_for_id("silero"), Some(TtsProviderType::Silero));
    }

    #[test]
    fn builtin_type_for_id_local_http() {
        assert_eq!(
            builtin_type_for_id("local-http"),
            Some(TtsProviderType::Local)
        );
    }

    #[test]
    fn builtin_type_for_id_fish() {
        assert_eq!(builtin_type_for_id("fish"), Some(TtsProviderType::Fish));
    }

    #[test]
    fn builtin_type_for_id_elevenlabs() {
        assert_eq!(
            builtin_type_for_id("elevenlabs"),
            Some(TtsProviderType::ElevenLabs)
        );
    }

    #[test]
    fn builtin_type_for_id_piper_is_none() {
        assert_eq!(builtin_type_for_id("local-piper:en_US-lessac-medium"), None);
    }

    #[test]
    fn builtin_type_for_id_unknown_is_none() {
        assert_eq!(builtin_type_for_id("nonexistent"), None);
    }

    // ── reconfigure inactive provider tests ──

    #[test]
    fn init_local_tts_registers_but_does_not_activate_when_another_is_active() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI", dummy_local()));
            reg.select("openai").unwrap();
        }

        state.init_local_tts("http://127.0.0.1:9999".to_string());

        let reg = state.tts_registry.lock();
        assert_eq!(reg.active_id(), Some("openai"));
        assert!(reg.get("local-http").is_some());
    }

    #[test]
    fn init_openai_tts_registers_but_does_not_activate_when_another_is_active() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("local-http", "Local", dummy_local()));
            reg.select("local-http").unwrap();
        }

        state.init_openai_tts("sk-test12345678901234567890".to_string());

        let reg = state.tts_registry.lock();
        assert_eq!(reg.active_id(), Some("local-http"));
        assert!(reg.get("openai").is_some());
    }

    #[test]
    fn add_or_replace_preserves_existing_active_id() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI v1", dummy_local()));
            reg.select("openai").unwrap();
        }

        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI v2", dummy_local()));
        }

        let reg = state.tts_registry.lock();
        assert_eq!(reg.active_id(), Some("openai"));
        assert_eq!(reg.get("openai").unwrap().display_name, "OpenAI v2");
    }

    // ── owner flow transactional tests (core logic without persist/emit) ──

    /// Exercise the production transaction with an in-memory successful persist.
    async fn selection_core(state: &AppState, id: &str) -> Result<Option<TtsProviderType>, String> {
        let legacy = builtin_type_for_id(id);
        state
            .select_tts_provider(id.to_string(), |_, _| Ok(()))
            .await?;
        Ok(legacy)
    }

    #[test]
    fn selection_unknown_id_preserves_active() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI", dummy_local()));
            reg.select("openai").unwrap();
        }

        let result = state.runtime.block_on(selection_core(&state, "unknown-id"));
        assert!(result.is_err());

        let reg = state.tts_registry.lock();
        assert_eq!(reg.active_id(), Some("openai"));
    }

    #[test]
    fn prepare_failure_preserves_active_and_does_not_emit_success() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI", dummy_local()));
            reg.add_or_replace(entry("bad-piper", "Bad Piper", failing_piper()));
            reg.select("openai").unwrap();
        }

        let result = state.runtime.block_on(selection_core(&state, "bad-piper"));

        assert!(result.is_err());
        // registry must be unchanged
        let reg = state.tts_registry.lock();
        assert_eq!(reg.active_id(), Some("openai"));
    }

    #[test]
    fn persistence_failure_preserves_runtime_selection_and_legacy_type() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI", dummy_local()));
            reg.add_or_replace(entry("fish", "Fish", dummy_local()));
            reg.select("openai").unwrap();
        }
        let old_type = state.tts_config.read().provider_type;

        let result = state
            .runtime
            .block_on(state.select_tts_provider("fish".into(), |_, _| Err("disk full".into())));

        assert!(result.unwrap_err().contains("disk full"));
        assert_eq!(state.tts_registry.lock().active_id(), Some("openai"));
        assert_eq!(state.tts_config.read().provider_type, old_type);
    }

    #[test]
    fn successful_builtin_selection_agrees_concrete_id_and_legacy_type() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI", dummy_local()));
            reg.add_or_replace(entry("fish", "Fish", dummy_local()));
            reg.select("openai").unwrap();
        }

        let registry = Arc::clone(&state.tts_registry);
        let result =
            state
                .runtime
                .block_on(state.select_tts_provider("fish".into(), move |id, legacy| {
                    assert_eq!(id, "fish");
                    assert_eq!(legacy, Some(TtsProviderType::Fish));
                    assert_eq!(registry.lock().active_id(), Some("openai"));
                    Ok(())
                }));
        assert!(result.is_ok());

        let reg = state.tts_registry.lock();
        assert_eq!(reg.active_id(), Some("fish"));
        assert_eq!(state.tts_config.read().provider_type, TtsProviderType::Fish);
    }

    #[test]
    fn successful_piper_selection_has_none_legacy_type() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI", dummy_local()));
            reg.add_or_replace(entry("piper-en-us-amy", "Amy", dummy_local()));
            reg.select("openai").unwrap();
        }

        let result = state
            .runtime
            .block_on(selection_core(&state, "piper-en-us-amy"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);

        let reg = state.tts_registry.lock();
        assert_eq!(reg.active_id(), Some("piper-en-us-amy"));
    }

    #[test]
    fn builtin_selection_updates_config_provider_type() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI", dummy_local()));
            reg.add_or_replace(entry("silero", "Silero", dummy_local()));
        }

        state
            .runtime
            .block_on(selection_core(&state, "silero"))
            .unwrap();

        assert_eq!(
            state.tts_config.read().provider_type,
            TtsProviderType::Silero
        );
    }

    #[test]
    fn piper_selection_does_not_overwrite_legacy_provider_type() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI", dummy_local()));
            reg.add_or_replace(entry("piper-en-us-amy", "Amy", dummy_local()));
            reg.select("openai").unwrap();
        }
        // legacy config is implicitly whatever was set — for Piper selection, provider_type stays unchanged
        let prev_type = state.tts_config.read().provider_type;

        state
            .runtime
            .block_on(selection_core(&state, "piper-en-us-amy"))
            .unwrap();

        assert_eq!(state.tts_config.read().provider_type, prev_type);
    }

    #[test]
    fn concurrent_selections_are_serialized() {
        let state = AppState::new();
        {
            let mut reg = state.tts_registry.lock();
            reg.add_or_replace(entry("openai", "OpenAI", dummy_local()));
            reg.add_or_replace(entry("fish", "Fish", dummy_local()));
        }

        let (ra, rb) = state.runtime.block_on(async {
            tokio::join!(
                selection_core(&state, "openai"),
                selection_core(&state, "fish")
            )
        });

        assert!(ra.is_ok());
        assert!(rb.is_ok());

        let reg = state.tts_registry.lock();
        let active = reg.active_id();
        assert!(
            active == Some("openai") || active == Some("fish"),
            "Final active must be one of the two selected IDs, got {:?}",
            active
        );
    }

    #[test]
    fn selection_mutex_serialises_concurrent_calls() {
        let state = AppState::new();

        {
            let mut registry = state.tts_registry.lock();
            registry.add_or_replace(TtsProviderEntry {
                id: "a".to_string(),
                display_name: "A".to_string(),
                provider: TtsProvider::OpenAi(crate::tts::openai::OpenAiTts::new("sk-test".into())),
            });
            registry.add_or_replace(TtsProviderEntry {
                id: "b".to_string(),
                display_name: "B".to_string(),
                provider: TtsProvider::OpenAi(crate::tts::openai::OpenAiTts::new("sk-test".into())),
            });
            registry.select("a").unwrap();
        }
        assert_eq!(state.tts_registry.lock().active_id(), Some("a"));

        let result = state.runtime.block_on(async {
            let mutex_for_task = Arc::clone(&state.selection_mutex);
            let locked = state.selection_mutex.lock().await;
            let task = tokio::spawn(async move {
                let _guard = mutex_for_task.lock().await;
                "b-done"
            });

            tokio::task::yield_now().await;
            assert!(!task.is_finished());
            drop(locked);
            task.await.unwrap()
        });
        assert_eq!(result, "b-done");
    }

    #[test]
    fn startup_notifications_are_delivered_once() {
        let state = AppState::new();
        state.push_notification("message".to_string());
        state.push_startup_error("error".to_string());

        assert_eq!(state.take_notifications(), vec!["message"]);
        assert!(state.take_notifications().is_empty());
        assert_eq!(state.take_startup_errors(), vec!["error"]);
        assert!(state.take_startup_errors().is_empty());
    }

    #[test]
    fn input_server_service_is_initialized() {
        let state = AppState::new();
        assert_eq!(
            state.input_server.status(),
            crate::input_server::InputServerStatus::Stopped
        );
    }

    #[test]
    fn begin_shutdown_returns_true_only_once() {
        let state = AppState::new();
        assert!(state.begin_shutdown());
        assert!(!state.begin_shutdown());
    }

    // ── RUAccent pack discovery snapshot tests ──

    #[test]
    fn ruaccent_snapshot_starts_empty() {
        let state = AppState::new();
        assert!(state.get_ruaccent_packs().is_empty());
    }

    #[test]
    fn ruaccent_refresh_is_visible_through_getter() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("visible");
        tu::write_upstream_pack(&root, &["one"]);

        let count = state.refresh_ruaccent_packs(std::slice::from_ref(&root));
        assert_eq!(count, 1);

        let snapshot = state.get_ruaccent_packs();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, "ruaccent.upstream.one");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ruaccent_second_refresh_replaces_snapshot() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("replace-a");
        let root2 = tu::unique_test_root("replace-b");
        tu::write_upstream_pack(&root, &["first"]);
        tu::write_upstream_pack(&root2, &["second"]);

        assert_eq!(state.refresh_ruaccent_packs(std::slice::from_ref(&root)), 1);

        let count = state.refresh_ruaccent_packs(std::slice::from_ref(&root2));
        assert_eq!(count, 1);

        let snapshot = state.get_ruaccent_packs();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, "ruaccent.upstream.second");

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&root2).ok();
    }

    // ── RUAccent lazy runtime slot reconciliation tests ──

    #[test]
    fn ruaccent_slots_created_for_complete_packs() {
        use crate::stress::packs::test_util as tu;
        use crate::stress::runtime::RuAccentRuntimeStatus;

        let state = AppState::new();
        let root = tu::unique_test_root("slots-complete");

        let pack = tu::write_upstream_pack(&root, &["tiny"]);

        let count = state.refresh_ruaccent_packs(std::slice::from_ref(&root));
        assert_eq!(count, 1);

        let slots = state.ruaccent_runtime_slots.read().clone();
        assert_eq!(slots.len(), 1);
        assert!(slots.contains_key("ruaccent.upstream.tiny"));
        assert_eq!(
            slots["ruaccent.upstream.tiny"].status(),
            RuAccentRuntimeStatus::NotLoaded
        );
        assert_eq!(slots["ruaccent.upstream.tiny"].descriptor().pack_root, pack);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ruaccent_refresh_preserves_slot_for_same_pack_root() {
        use crate::stress::packs::test_util as tu;
        use crate::stress::runtime::RuAccentRuntimeStatus;

        let state = AppState::new();
        let root = tu::unique_test_root("preserve");

        tu::write_upstream_pack(&root, &["same"]);
        assert_eq!(state.refresh_ruaccent_packs(std::slice::from_ref(&root)), 1);

        let slot = state
            .get_ruaccent_runtime_slot("ruaccent.upstream.same")
            .expect("slot exists");
        slot.mark_ready();
        assert_eq!(slot.status(), RuAccentRuntimeStatus::Ready);

        // Same pack root and model: the live slot (and its status) is preserved.
        assert_eq!(state.refresh_ruaccent_packs(std::slice::from_ref(&root)), 1);
        let slot = state
            .get_ruaccent_runtime_slot("ruaccent.upstream.same")
            .expect("slot preserved");
        assert_eq!(slot.status(), RuAccentRuntimeStatus::Ready);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ruaccent_refresh_replaces_slot_when_pack_root_changes() {
        use crate::stress::packs::test_util as tu;
        use crate::stress::runtime::RuAccentRuntimeStatus;

        let state = AppState::new();
        let root_a = tu::unique_test_root("replace-a");
        let root_b = tu::unique_test_root("replace-b");

        let pack_a = tu::write_upstream_pack(&root_a, &["move"]);
        assert_eq!(
            state.refresh_ruaccent_packs(std::slice::from_ref(&root_a)),
            1
        );
        let slot = state
            .get_ruaccent_runtime_slot("ruaccent.upstream.move")
            .expect("slot");
        slot.mark_ready();

        let pack_b = tu::write_upstream_pack(&root_b, &["move"]);
        assert_eq!(
            state.refresh_ruaccent_packs(std::slice::from_ref(&root_b)),
            1
        );

        let slot = state
            .get_ruaccent_runtime_slot("ruaccent.upstream.move")
            .expect("slot replaced");
        assert_ne!(slot.descriptor().pack_root, pack_a);
        assert_eq!(slot.descriptor().pack_root, pack_b);
        assert_eq!(slot.status(), RuAccentRuntimeStatus::NotLoaded);

        std::fs::remove_dir_all(&root_a).ok();
        std::fs::remove_dir_all(&root_b).ok();
    }

    #[test]
    fn ruaccent_refresh_removes_slots_for_missing_packs() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("remove");

        tu::write_upstream_pack(&root, &["gone"]);
        assert_eq!(state.refresh_ruaccent_packs(std::slice::from_ref(&root)), 1);
        assert!(state
            .get_ruaccent_runtime_slot("ruaccent.upstream.gone")
            .is_some());

        let empty_root = tu::unique_test_root("empty");
        assert_eq!(
            state.refresh_ruaccent_packs(std::slice::from_ref(&empty_root)),
            0
        );
        assert!(state
            .get_ruaccent_runtime_slot("ruaccent.upstream.gone")
            .is_none());
        assert!(state.get_ruaccent_packs().is_empty());

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&empty_root).ok();
    }

    #[test]
    fn ruaccent_refresh_preserves_ready_slot_for_missing_pack() {
        use crate::stress::packs::test_util as tu;
        use crate::stress::runtime::RuAccentRuntimeStatus;

        let state = AppState::new();
        let root = tu::unique_test_root("preserve-ready-missing");
        tu::write_upstream_pack(&root, &["gone"]);
        assert_eq!(state.refresh_ruaccent_packs(std::slice::from_ref(&root)), 1);

        let slot = state
            .get_ruaccent_runtime_slot("ruaccent.upstream.gone")
            .expect("slot exists");
        slot.mark_ready();

        let empty_root = tu::unique_test_root("preserve-ready-empty");
        assert_eq!(
            state.refresh_ruaccent_packs(std::slice::from_ref(&empty_root)),
            0
        );

        let slot = state
            .get_ruaccent_runtime_slot("ruaccent.upstream.gone")
            .expect("ready slot preserved for a missing pack");
        assert_eq!(slot.status(), RuAccentRuntimeStatus::Ready);

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&empty_root).ok();
    }

    #[test]
    fn ruaccent_refresh_drops_failed_slot_for_missing_pack() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("drop-failed-missing");
        tu::write_upstream_pack(&root, &["gone"]);
        assert_eq!(state.refresh_ruaccent_packs(std::slice::from_ref(&root)), 1);

        let slot = state
            .get_ruaccent_runtime_slot("ruaccent.upstream.gone")
            .expect("slot exists");
        slot.mark_failed("load failed".to_string());

        let empty_root = tu::unique_test_root("drop-failed-empty");
        assert_eq!(
            state.refresh_ruaccent_packs(std::slice::from_ref(&empty_root)),
            0
        );

        assert!(state
            .get_ruaccent_runtime_slot("ruaccent.upstream.gone")
            .is_none());

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&empty_root).ok();
    }

    #[test]
    fn ruaccent_refresh_retry_returns_packs_without_retry_when_complete() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("retry-complete");
        tu::write_upstream_pack(&root, &["tiny"]);
        assert_eq!(
            state.refresh_ruaccent_packs_with_retry(std::slice::from_ref(&root)),
            1
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ruaccent_refresh_retry_exhausts_retries_for_existing_incomplete_dir() {
        use crate::stress::packs::test_util as tu;

        let state = AppState::new();
        let root = tu::unique_test_root("retry-incomplete");
        tu::write_incomplete_root(&root, &["tiny"]);
        assert_eq!(
            state.refresh_ruaccent_packs_with_retry(std::slice::from_ref(&root)),
            0
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ruaccent_refresh_retry_skips_retry_when_models_dir_absent() {
        let state = AppState::new();
        let root = crate::stress::packs::test_util::unique_test_root("retry-absent");
        // No pack written: the models directory does not exist at all.
        assert_eq!(
            state.refresh_ruaccent_packs_with_retry(std::slice::from_ref(&root)),
            0
        );
        std::fs::remove_dir_all(&root).ok();
    }

    // ── RUAccent retry policy seam tests (attempt counts, no sleeping) ──

    #[test]
    fn ruaccent_refresh_retry_retries_when_second_root_has_models_dir() {
        use crate::stress::packs::test_util as tu;

        let first = tu::unique_test_root("retry-multi-first-absent");
        let second = tu::unique_test_root("retry-multi-second-present");
        std::fs::create_dir_all(tu::upstream_pack_root(&second)).unwrap();
        let roots = vec![first.clone(), second.clone()];

        let scans = std::cell::Cell::new(0usize);
        let pauses = std::cell::Cell::new(0usize);
        let count = refresh_ruaccent_packs_with_retry_core(
            &roots,
            |_| {
                scans.set(scans.get() + 1);
                if scans.get() == 1 {
                    0
                } else {
                    1
                }
            },
            || pauses.set(pauses.get() + 1),
        );

        assert_eq!(count, 1);
        assert_eq!(scans.get(), 2);
        assert_eq!(pauses.get(), 1);

        std::fs::remove_dir_all(&first).ok();
        std::fs::remove_dir_all(&second).ok();
    }

    #[test]
    fn ruaccent_refresh_retry_exhausts_two_retries_with_seam() {
        use crate::stress::packs::test_util as tu;

        let root = tu::unique_test_root("retry-multi-exhaust");
        std::fs::create_dir_all(tu::upstream_pack_root(&root)).unwrap();
        let roots = vec![root.clone()];

        let scans = std::cell::Cell::new(0usize);
        let pauses = std::cell::Cell::new(0usize);
        let count = refresh_ruaccent_packs_with_retry_core(
            &roots,
            |_| {
                scans.set(scans.get() + 1);
                0usize
            },
            || pauses.set(pauses.get() + 1),
        );

        assert_eq!(count, 0);
        assert_eq!(scans.get(), 3);
        assert_eq!(pauses.get(), 2);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ruaccent_refresh_retry_absent_models_dir_in_all_roots_scans_once() {
        use crate::stress::packs::test_util as tu;

        let first = tu::unique_test_root("retry-multi-absent-a");
        let second = tu::unique_test_root("retry-multi-absent-b");
        let roots = vec![first.clone(), second.clone()];

        let scans = std::cell::Cell::new(0usize);
        let pauses = std::cell::Cell::new(0usize);
        let count = refresh_ruaccent_packs_with_retry_core(
            &roots,
            |_| {
                scans.set(scans.get() + 1);
                0usize
            },
            || pauses.set(pauses.get() + 1),
        );

        assert_eq!(count, 0);
        assert_eq!(scans.get(), 1);
        assert_eq!(pauses.get(), 0);

        std::fs::remove_dir_all(&first).ok();
        std::fs::remove_dir_all(&second).ok();
    }

    #[test]
    fn ruaccent_refresh_retry_successful_first_scan_is_immediate() {
        use crate::stress::packs::test_util as tu;

        let root = tu::unique_test_root("retry-multi-immediate");
        let roots = vec![root.clone()];

        let scans = std::cell::Cell::new(0usize);
        let pauses = std::cell::Cell::new(0usize);
        let count = refresh_ruaccent_packs_with_retry_core(
            &roots,
            |_| {
                scans.set(scans.get() + 1);
                1usize
            },
            || pauses.set(pauses.get() + 1),
        );

        assert_eq!(count, 1);
        assert_eq!(scans.get(), 1);
        assert_eq!(pauses.get(), 0);

        std::fs::remove_dir_all(&root).ok();
    }

    fn pocket_test_descriptor(root: &std::path::Path) -> PocketModelDescriptor {
        PocketModelDescriptor {
            id: "local-pocket:serialized".to_string(),
            package_id: "serialized".to_string(),
            display_name: "Serialized".to_string(),
            language: "en".to_string(),
            format: "pocket-tts-onnx".to_string(),
            format_version: 1,
            package_root: root.to_path_buf(),
            config_path: root.join("config.yaml"),
            weights_path: root.join("model.safetensors"),
            tokenizer_path: root.join("tokenizer.json"),
            tokenizer_kind: crate::tts::pocket::PocketTokenizerKind::Json,
            n_bins: 10,
            sample_rate: 24_000,
            frame_rate: 12.5,
            num_layers: 1,
            latent_dim: 32,
            insert_bos_before_voice: true,
            temperature: 0.3,
            eos_threshold: -4.0,
            voice_modes: vec![crate::tts::pocket::PocketVoiceMode::ReferenceWav],
            voices: Vec::new(),
            graphs: crate::tts::pocket::descriptor::PocketGraphs {
                encoder: root.join("mimi_encoder.onnx"),
                conditioner: root.join("text_conditioner.onnx"),
                main: root.join("flow_lm_main.onnx"),
                flow: root.join("flow_lm_flow.onnx"),
                decoder: root.join("mimi_decoder.onnx"),
                quantized: false,
            },
            asset_paths: Vec::new(),
        }
    }

    #[test]
    fn pocket_refresh_and_voice_change_serialize_registry_reconciliation() {
        use crate::tts::pocket::voice::with_voice_store_at;
        use crate::tts::pocket::PocketVoiceSelection;

        let root = crate::stress::packs::test_util::unique_test_root("pocket-reconcile");
        std::fs::create_dir_all(&root).unwrap();
        let descriptor = pocket_test_descriptor(&root);
        let store_path = root.join("pocket-voices.json");
        let first = PocketVoiceSelection::ReferenceWav(root.join("first.wav"));
        let second = PocketVoiceSelection::ReferenceWav(root.join("second.wav"));
        let registry = Arc::new(Mutex::new(TtsProviderRegistry::new()));

        with_voice_store_at(store_path.clone(), |store| {
            store.set(&descriptor.package_id, &first).unwrap();
            let mut registry = registry.lock();
            reconcile_pocket_registry(
                &mut registry,
                std::slice::from_ref(&descriptor),
                store,
                None,
            );
        });

        let barrier = Arc::new(std::sync::Barrier::new(3));
        let second_for_setter = second.clone();
        let setter = {
            let path = store_path.clone();
            let registry = Arc::clone(&registry);
            let descriptor = descriptor.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                with_voice_store_at(path, |store| {
                    store
                        .set(&descriptor.package_id, &second_for_setter)
                        .unwrap();
                    let mut registry = registry.lock();
                    reconcile_pocket_registry(
                        &mut registry,
                        std::slice::from_ref(&descriptor),
                        store,
                        None,
                    );
                });
            })
        };
        let refresher = {
            let path = store_path.clone();
            let registry = Arc::clone(&registry);
            let descriptor = descriptor.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                with_voice_store_at(path, |store| {
                    let mut registry = registry.lock();
                    reconcile_pocket_registry(
                        &mut registry,
                        std::slice::from_ref(&descriptor),
                        store,
                        None,
                    );
                });
            })
        };
        barrier.wait();
        setter.join().unwrap();
        refresher.join().unwrap();

        let registry = registry.lock();
        let entry = registry.get(&descriptor.id).expect("Pocket provider");
        let TtsProvider::Pocket(provider) = &entry.provider else {
            panic!("expected a Pocket provider");
        };
        assert_eq!(provider.selection(), Some(&second));
        let store = PocketVoiceStore::load(store_path);
        assert_eq!(store.selection_for(&descriptor.package_id), Some(second));

        drop(registry);
        std::fs::remove_dir_all(&root).ok();
    }
}
