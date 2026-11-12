// Setup module - Application initialization
//
// This module handles initialization of application components including:
// - Settings loading
// - Window initialization
// - System tray setup
// - Event system setup
// - WebView and Twitch server initialization
//
// Refactored from lib.rs run() setup callback (2026-03-11)

use anyhow::Context;
use std::sync::mpsc;
use std::thread;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager};
use tracing::{error, info, warn};

use crate::commands::playback::PlaybackState;
use crate::commands::speech_queue::SpeechQueueState;
use crate::commands::telegram::TelegramState;
use crate::config::{AppSettings, Hotkey, SettingsManager, WindowsManager, WindowsSettings};
use crate::event_loop::EventHandler;
use crate::events::AppEvent;
use crate::soundpanel::SoundPanelState;
use crate::speech_queue::JobStatus;
use crate::state::AppState;
use crate::tts::TtsProviderType;
use crate::vtube_studio::ConnectOrigin;
use std::sync::Arc;

/// Initialize the application (called from Tauri's setup callback)
///
/// Settings are passed from lib.rs to avoid race condition from double loading.
/// Logger is initialized before this function with the same settings.
pub fn init_app(app: &App, mut settings: AppSettings) -> Result<(), Box<dyn std::error::Error>> {
    info!("=== Application setup started ===");

    // Get state managers
    let settings_manager = app.state::<SettingsManager>();
    crate::audio::init_output_settings(settings_manager.cache_arc())?;
    let windows_manager = app.state::<WindowsManager>();
    let windows = windows_manager.load()?;

    // Windows are declared in tauri.conf.json with "create": false and are
    // built here so that the settings-recovery flow can boot a minimal
    // process without them (ROADMAP-123).
    create_configured_windows(app, settings.start_compact, windows.main.compact_view)?;

    let app_state = app.state::<AppState>();
    let telegram_state = app.state::<TelegramState>();
    let soundpanel_state = app.state::<SoundPanelState>();

    // Initialize locale runtime: bundled resource locales first, config locales as fallback
    let resource_path = match app.path().resource_dir() {
        Ok(dir) => Some(dir.join("locales")),
        Err(e) => {
            warn!(error = %e, "resource_dir() failed, no resource locale directory");
            None
        }
    };
    let config_path = settings_manager.config_dir().join("locales");
    let catalog = crate::localization::LocaleCatalog::load(
        resource_path.as_deref(),
        Some(config_path.as_path()),
    );
    app.manage(crate::commands::localization::LocalizationState::new(
        catalog,
        &settings_manager.get_ui_language(),
    ));

    info!(tts_provider = ?settings.tts.provider, hotkey_enabled = settings.hotkey_enabled, "Settings loaded");

    // Load Twitch settings into AppState
    info!("Loading Twitch settings...");
    *app_state.inner().twitch.settings.blocking_write() = settings.twitch.clone();

    // Load VTube Studio settings into AppState
    info!("Loading VTube Studio settings...");
    *app_state.inner().vtube_studio.settings.blocking_write() = settings.vtube_studio.clone();

    // Load WebView settings into AppState
    info!("Loading WebView settings...");
    *app_state.inner().webview.settings.blocking_write() = crate::webview::WebViewSettings {
        enabled: settings.webview.enabled,
        start_on_boot: settings.webview.start_on_boot,
        port: settings.webview.port,
        access_token: settings.webview.access_token.clone(),
        upnp_enabled: settings.webview.upnp_enabled,
        send_original_text: settings.webview.send_original_text,
    };

    // Load input server settings into AppState
    info!("Loading input server settings...");
    *app_state.inner().input_server.settings.blocking_write() = settings.input_server.clone();

    // Load the source-neutral Incoming policy into the shared runtime snapshot.
    // Both the input server supervisor and OCR intake read auto-play from here.
    info!("Loading Incoming settings...");
    *app_state.inner().input_server.incoming.blocking_write() = settings.incoming.clone();

    // Load OCR settings into AppState
    info!("Loading OCR settings...");
    *app_state.inner().ocr.settings.blocking_write() = settings.ocr.clone();

    // Load VRChat settings into AppState
    info!("Loading VRChat settings...");
    app_state
        .inner()
        .vrchat
        .update_settings_internal(settings.vrchat.runtime_at_startup());

    // Load hotkey_enabled setting into AppState
    info!("Loading hotkey_enabled setting...");
    *app_state.inner().hotkey_enabled.lock() = settings.hotkey_enabled;

    // Setup event system (must be before PlaybackManager)
    let app_handle = app.handle().clone();
    let app_state_for_events = app_state.inner().clone();
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>();

    app_state_for_events.set_event_sender(event_tx.clone());

    // Initialize PlaybackManager
    let app_handle_pb = app.handle().clone();
    let pb_manager = Arc::new(crate::playback::PlaybackManager::new(
        app_handle_pb,
        event_tx.clone(),
        Some(app_state.inner().cached_devices.clone()),
    ));
    *app_state.inner().playback_manager.lock() = Some(pb_manager.clone());
    app.manage(PlaybackState(pb_manager));
    info!("Event sender configured in AppState");

    // Start speech queue worker after PlaybackManager is installed
    {
        let sq_state = app.state::<SpeechQueueState>();
        let worker_queue = sq_state.queue_arc();
        let worker_notify = sq_state.notifier();
        let worker_handle = app.handle().clone();
        let worker_shutdown = app_state.inner().shutdown.clone();
        let worker_editor = app_state.inner().editor.clone();
        let worker_playback = app_state.inner().playback_manager.clone();
        let worker_webview = app_state.inner().webview.clone();
        let worker_twitch = app_state.inner().twitch.clone();
        let worker_vrchat = app_state.inner().vrchat.clone();
        app_state.inner().runtime.spawn(async move {
            speech_worker(
                worker_queue,
                worker_notify,
                worker_handle,
                worker_shutdown,
                worker_editor,
                worker_playback,
                worker_webview,
                worker_twitch,
                worker_vrchat,
            )
            .await;
        });
        info!("Speech queue worker started");
    }

    thread::spawn(move || {
        info!("Event thread started, waiting for events...");
        for event in event_rx {
            info!(event = ?std::mem::discriminant(&event), "Received from channel");
            let event_name = event.to_tauri_event();
            let _ = app_handle.emit(event_name, &event);

            let handler = EventHandler::new(app_state_for_events.clone(), app_handle.clone());
            handler.handle(event);
        }
    });

    // Setup SoundPanel event system
    use crate::playback_window::{hide_playback_window, show_playback_window};
    use crate::soundpanel::{load_appearance, load_bindings};
    use crate::soundpanel_window::{
        emit_soundpanel_no_binding, hide_soundpanel_window, show_soundpanel_window,
        update_soundpanel_appearance,
    };

    let (soundpanel_tx, soundpanel_rx) = mpsc::channel::<AppEvent>();
    soundpanel_state.inner().set_event_sender(soundpanel_tx);
    info!("[SOUNDPANEL] Event sender configured");

    let app_handle_for_soundpanel = app.handle().clone();
    let app_state_for_soundpanel = app_state.inner().clone();
    thread::spawn(move || {
        for event in soundpanel_rx {
            let event_name = event.to_tauri_event();
            let _ = app_handle_for_soundpanel.emit(event_name, &event);

            match event {
                AppEvent::ShowSoundPanelWindow => {
                    info!("[SOUNDPANEL] Show soundpanel window");
                    let _ = show_soundpanel_window(&app_handle_for_soundpanel);
                }
                AppEvent::HideSoundPanelWindow => {
                    info!("[SOUNDPANEL] Hide soundpanel window");
                    let _ = hide_soundpanel_window(
                        &app_handle_for_soundpanel,
                        &app_state_for_soundpanel,
                    );
                }
                AppEvent::SoundPanelNoBinding(key) => {
                    info!(key = ?key, "No binding for key");
                    let _ = emit_soundpanel_no_binding(&app_handle_for_soundpanel, key);
                }
                AppEvent::SoundPanelAppearanceChanged => {
                    info!("[SOUNDPANEL] === Appearance changed event received ===");
                    let _ = update_soundpanel_appearance(&app_handle_for_soundpanel);
                }
                AppEvent::TtsProviderChanged(_) => {}
                AppEvent::ShowPlaybackControlWindow => {
                    info!("[PLAYBACK] Show playback control window");
                    let _ = show_playback_window(&app_handle_for_soundpanel);
                }
                AppEvent::HidePlaybackControlWindow => {
                    info!("[PLAYBACK] Hide playback control window");
                    let _ = hide_playback_window(&app_handle_for_soundpanel);
                }
                _ => {}
            }
        }
    });

    // Load soundpanel bindings
    match load_bindings(&soundpanel_state) {
        Ok(bindings) => {
            info!(count = bindings.len(), "Loaded bindings on startup");
        }
        Err(e) => {
            error!(error = %e, "Failed to load bindings");
        }
    }

    match load_appearance(&soundpanel_state, &windows_manager) {
        Ok(appearance) => {
            info!(opacity = appearance.opacity, bg_color = %appearance.bg_color, "[SOUNDPANEL] Loaded appearance");
        }
        Err(e) => {
            error!(error = %e, "Failed to load appearance");
        }
    }

    // Start the single SoundPanel FIFO playback worker. It owns all SoundPanel
    // playback and stops cleanly when the shared shutdown token is cancelled.
    {
        let sp_worker_shutdown = app_state.inner().shutdown.clone();
        if let Err(error) = soundpanel_state.start_queue_worker(sp_worker_shutdown) {
            error!(error = %error, "Failed to start SoundPanel queue worker");
        }
    }

    // Initialize TTS provider
    init_tts_provider(
        &app_state,
        &telegram_state,
        settings.clone(),
        &settings_manager,
    );

    // Register discovered Piper providers (no ONNX session created yet)
    app_state.register_piper_providers();
    // Register discovered Pocket TTS providers (no ONNX session created yet).
    app_state.register_pocket_providers();

    // Discover local RUAccent packs (no model/ONNX session loaded). The retry
    // wrapper keeps a transient boot-time filesystem failure from reading as
    // "models removed" (which would persist-disable the autoload feature).
    {
        let ruaccent_roots = crate::commands::ruaccent_search_roots(app.handle());
        app_state.refresh_ruaccent_packs_with_retry(&ruaccent_roots);
    }

    // Initialize espeak-ng data path for Piper phonemization
    {
        let resource_dir = match app.path().resource_dir() {
            Ok(dir) => Some(dir),
            Err(e) => {
                warn!(error = %e, "resource_dir() failed, espeak-ng data not initialised");
                None
            }
        };
        crate::tts::piper::runtime::LocalModelTts::init_espeak_data(resource_dir);
        // Probe every startup, even when Piper is not selected. Failure stays
        // silent until a saved or manually requested Piper needs the runtime.
        crate::tts::piper::runtime::PiperReadiness::assess_cached();
    }

    // Restore saved concrete provider ID. A deleted Piper model falls back to
    // Silero, persists that choice and queues a one-shot frontend notification.
    // Other missing IDs keep the registry's generic safe fallback behavior.
    let saved_id = settings_manager.get_tts_provider_id();
    let missing_piper = {
        let registry = app_state.tts_registry.lock();
        missing_piper_model_name(saved_id.as_deref(), |id| registry.get(id).is_some())
    };

    if let Some(model_name) = missing_piper {
        app_state.init_silero_tts(Arc::clone(&telegram_state.client));
        app_state
            .tts_registry
            .lock()
            .select("silero")
            .expect("Silero is registered before Piper fallback selection");
        app_state.set_tts_provider_type(TtsProviderType::Silero);

        settings.tts.provider = TtsProviderType::Silero;
        settings.tts.provider_id = Some("silero".to_string());
        if let Err(error) = settings_manager.save(&settings) {
            warn!(error = %error, "Failed to persist Silero fallback for missing Piper model");
        }

        app_state.push_notification(missing_piper_notification(&model_name));
    } else {
        let (restored_piper, restored_pocket) = {
            let mut registry = app_state.tts_registry.lock();
            registry.restore_saved_or_first(saved_id.as_deref());
            let active = registry
                .active()
                .map(|entry| (entry.id.clone(), entry.provider.clone()));
            match active {
                Some((id, crate::tts::TtsProvider::Piper(provider))) => {
                    (Some((id, provider)), None)
                }
                Some((id, crate::tts::TtsProvider::Pocket(provider))) => {
                    (None, Some((id, provider)))
                }
                _ => (None, None),
            }
        };

        // A persisted Pocket model keeps the saved selection even when its
        // voice is not ready; synthesis will surface the specific readiness
        // error and other providers stay available.
        if let Some((provider_id, provider)) = restored_pocket {
            info!(provider_id, "Preparing restored Pocket TTS provider");
            if let Err(error) = provider.prepare() {
                warn!(
                    provider_id,
                    error = %error,
                    "Restored Pocket TTS provider is not ready"
                );
            }
        }

        // A persisted Piper choice is already selected from the user's point
        // of view. Verify eSpeak phonemization readiness before loading the
        // model: an unusable runtime keeps the saved selection (no silent
        // provider rewrite) and reports a single app-level notification
        // instead of leaving a "Ready"-looking model that fails at synthesis.
        if let Some((provider_id, provider)) = restored_piper {
            let readiness = provider.espeak_readiness();
            if readiness.is_ready() {
                info!(provider_id, "Preparing restored Piper provider");
                match provider.check_phonemization() {
                    Ok(()) => {
                        if let Err(error) = provider.prepare() {
                            warn!(provider_id, error = %error, "Failed to prepare restored Piper provider");
                        }
                    }
                    Err(cause) => {
                        warn!(provider_id, error = %cause, "Restored Piper phonemization unavailable");
                        if let Some(key) =
                            crate::tts::piper::runtime::phonemization_error_key(&cause)
                        {
                            let snapshot = app
                                .state::<crate::commands::localization::LocalizationState>()
                                .snapshot();
                            app_state.push_startup_error(
                                snapshot.messages.get(key).cloned().unwrap_or_else(||
                                    "Piper model voice is unavailable in eSpeak NG. Check the model configuration or select another model.".to_string()),
                            );
                        }
                    }
                }
            } else {
                warn!(
                    provider_id,
                    readiness = ?readiness,
                    "Restored Piper provider skipped: eSpeak NG data unavailable"
                );
                let localization = app.state::<crate::commands::localization::LocalizationState>();
                let snapshot = localization.snapshot();
                let message =
                    crate::tts::piper::runtime::not_ready_message(&snapshot.messages, readiness);
                app_state.push_startup_error(message);
            }
        }
    }

    // Initialize offline spellcheck
    init_spellcheck(app, &app_state);

    // Initialize windows
    init_windows(app, &windows, &windows_manager, &settings)?;

    // Initialize system tray
    init_tray(app)?;

    // Initialize hooks
    init_hooks(&app_state, &soundpanel_state, app.handle().clone())?;

    // Initialize WebView server
    init_webview_server(&app_state, app.handle().clone());

    // Initialize Twitch API authorization coordinator before Twitch client (ROADMAP-132)
    // so any start_on_boot API client finds the coordinator published and does not fail.
    init_twitch_api_auth(&app_state, settings_manager.inner());

    // Initialize Twitch client
    init_twitch_client(&app_state, app.handle().clone());

    // Initialize input server (after persisted input settings are loaded above)
    init_input_server(&app_state, app.handle().clone());

    // Initialize OCR (after persisted OCR settings are loaded above)
    init_ocr(
        &app_state,
        app.handle().clone(),
        settings.ocr.enabled,
        settings.hotkeys.ocr_capture.clone(),
    );

    // Initialize VTube Studio autostart
    init_vtube_studio(&app_state, app.handle().clone());

    // Initialize window protection (Windows only)
    #[cfg(windows)]
    init_window_protection(app, &windows_manager);

    // Set backend ready flag - all initialization complete
    app_state
        .backend_ready
        .store(true, std::sync::atomic::Ordering::SeqCst);
    info!("Backend ready flag set");

    // Show main window after backend is fully initialized
    if let Some(main_window) = app.get_webview_window("main") {
        if settings.start_compact {
            // Start directly in compact mode: enforce compact bounds and resize
            // to the saved compact dimensions before the window becomes visible
            // (no animation/guards needed since the window is still hidden).
            //
            // Compact dimensions are physical INNER pixels; the maximum is the
            // logical 800×630 scaled by the current scale factor. Saved values
            // are clamped here but NOT written back to disk (the stored number
            // is preserved until the user saves a new size).
            let scale = match main_window.scale_factor() {
                Ok(scale) => scale,
                Err(error) => {
                    warn!(error = %error, "Failed to read scale factor, using 1.0 for compact bounds");
                    1.0
                }
            };
            let (min_w, min_h, max_w, max_h) = crate::config::compact_physical_bounds(scale);
            let width = windows.main.compact_width.clamp(min_w, max_w);
            let height = windows.main.compact_height.clamp(min_h, max_h);
            let min_size = tauri::Size::Physical(tauri::PhysicalSize {
                width: min_w,
                height: min_h,
            });
            let max_size = tauri::Size::Physical(tauri::PhysicalSize {
                width: max_w,
                height: max_h,
            });
            let _ = main_window.set_min_size(Some(min_size));
            let _ = main_window.set_max_size(Some(max_size));
            let _ =
                main_window.set_size(tauri::Size::Physical(tauri::PhysicalSize { width, height }));
            info!(width, height, "Main window started in compact mode");
        }
        let _ = main_window.show();
        info!("Main window shown");
    }

    // Auto-show playback control window on start if setting enabled
    if settings.show_playback_on_start {
        let _ = crate::playback_window::show_playback_window(app.handle());
    }

    // Register the second-instance show callback (Windows only). The hidden
    // receiver window is created at the very start of run(); this callback
    // fires when another process asks this instance to surface its main window.
    #[cfg(windows)]
    {
        let app_handle = app.handle().clone();
        crate::single_instance::register_show_callback(move || {
            crate::setup::show_main_window(&app_handle, "second-instance");
        });
    }

    info!("Setup complete - hotkeys will be registered when window gains focus");
    Ok(())
}

/// Initialize TTS provider based on settings
fn init_tts_provider(
    app_state: &AppState,
    telegram_state: &TelegramState,
    settings: AppSettings,
    settings_manager: &SettingsManager,
) {
    info!(provider = ?settings.tts.provider, "Initializing TTS provider");
    app_state.set_tts_provider_type(settings.tts.provider);

    // Always load OpenAI API key if available (for UI display)
    if let Some(ref api_key) = settings.tts.openai.api_key {
        app_state.set_openai_api_key(Some(api_key.clone()));
        info!("OpenAI API key loaded for UI");
    }

    // Always load Fish Audio API key if available (for UI display)
    if let Some(ref api_key) = settings.tts.fish.api_key {
        app_state.set_fish_audio_api_key(Some(api_key.clone()));
        app_state.set_fish_audio_format(settings.tts.fish.format.clone());
        app_state.set_fish_audio_temperature(settings.tts.fish.temperature);
        app_state.set_fish_audio_sample_rate(settings.tts.fish.sample_rate);
        info!("Fish Audio API key loaded for UI");
    }

    // Always load ElevenLabs API key if available (for UI display). The provider
    // itself is registered after the active provider below so the startup
    // fallback selection never prefers a non-active ElevenLabs entry.
    if let Some(ref api_key) = settings.tts.elevenlabs.api_key {
        if !api_key.is_empty() {
            app_state.set_elevenlabs_api_key(Some(api_key.clone()));
            app_state.set_elevenlabs_connection_settings(
                api_key.clone(),
                settings.tts.elevenlabs.model_id.clone(),
                settings.tts.elevenlabs.output_format.clone(),
                settings.tts.elevenlabs.stability,
                settings.tts.elevenlabs.similarity_boost,
                settings.tts.elevenlabs.style,
                settings.tts.elevenlabs.use_speaker_boost,
            );
            app_state.set_elevenlabs_voice_id(settings.tts.elevenlabs.voice_id.clone());
            if settings.tts.elevenlabs.use_proxy {
                if let Some(ref proxy_url) = settings.tts.network.proxy.proxy_url {
                    app_state.set_elevenlabs_proxy(Some(proxy_url.clone()));
                }
            }
            info!("ElevenLabs API key loaded for UI");
        }
    }

    match settings.tts.provider {
        TtsProviderType::OpenAi => {
            if let Some(ref api_key) = settings.tts.openai.api_key {
                let api_key_str: String = api_key.clone();
                app_state.set_openai_api_key(Some(api_key_str.clone()));
                info!("OpenAI TTS initialized with API key");
                app_state.init_openai_tts(api_key_str.clone());
                app_state.set_openai_voice(settings.tts.openai.voice.clone());
                // Apply proxy settings respecting use_proxy flag
                let proxy_url = if settings.tts.openai.use_proxy {
                    settings.tts.network.proxy.proxy_url.clone()
                } else {
                    None
                };
                app_state.set_openai_proxy(proxy_url);
            } else {
                warn!("OpenAI selected but no API key found");
            }
        }
        TtsProviderType::Silero => {
            info!("Initializing Silero TTS on startup");
            let client_arc = std::sync::Arc::clone(&telegram_state.client);
            app_state.init_silero_tts(client_arc);
            info!("Silero TTS initialized");
        }
        TtsProviderType::Local => {
            let url = settings.tts.local.url.clone();
            app_state.set_local_tts_url(url.clone());
            app_state.init_local_tts(url);
            info!("Local TTS initialized");
        }
        TtsProviderType::Fish => {
            if let Some(ref api_key) = settings.tts.fish.api_key {
                let api_key_str: String = api_key.clone();
                app_state.set_fish_audio_api_key(Some(api_key_str.clone()));
                app_state.set_fish_audio_format(settings.tts.fish.format.clone());
                app_state.set_fish_audio_temperature(settings.tts.fish.temperature);
                app_state.set_fish_audio_sample_rate(settings.tts.fish.sample_rate);
                info!("Fish Audio API key loaded");
                app_state.init_fish_audio_tts(api_key_str.clone());
                app_state.set_fish_audio_reference_id(settings.tts.fish.reference_id.clone());

                if settings.tts.fish.use_proxy {
                    if let Some(ref proxy_url) = settings.tts.network.proxy.proxy_url {
                        app_state.set_fish_audio_proxy(Some(proxy_url.clone()));
                    }
                }
            } else {
                warn!("Fish Audio selected but no API key found");
            }
        }
        TtsProviderType::ElevenLabs => {
            let models = settings_manager.get_elevenlabs_models();
            let voices = settings_manager.get_elevenlabs_voices();
            let has_model = !settings.tts.elevenlabs.model_id.trim().is_empty()
                && models
                    .iter()
                    .any(|model| model.model_id == settings.tts.elevenlabs.model_id);
            let has_voice = !settings.tts.elevenlabs.voice_id.trim().is_empty()
                && voices
                    .iter()
                    .any(|voice| voice.voice_id == settings.tts.elevenlabs.voice_id);
            match &settings.tts.elevenlabs.api_key {
                Some(key) if !key.trim().is_empty() => {
                    app_state.init_elevenlabs_tts(key.trim().to_string());
                    info!("ElevenLabs TTS initialized as active provider");
                    if models.is_empty() || voices.is_empty() {
                        warn!("ElevenLabs catalog cache is unavailable; keeping persisted model/voice selection");
                    } else if !has_model || !has_voice {
                        warn!(
                            "ElevenLabs selected model or voice is missing from the cached catalog"
                        );
                    }
                }
                _ => {
                    warn!("ElevenLabs selected but its API key is unavailable");
                }
            }
        }
    }

    // Register ElevenLabs when a key exists but it is not the active provider,
    // so the card is available for later selection without shifting the startup
    // fallback order.
    if settings.tts.provider != TtsProviderType::ElevenLabs {
        if let Some(ref api_key) = settings.tts.elevenlabs.api_key {
            if !api_key.is_empty() {
                app_state.init_elevenlabs_tts(api_key.clone());
                info!("ElevenLabs TTS provider registered (inactive)");
            }
        }
    }
}

/// Initialize offline spellcheck (spellbook + embedded Hunspell dictionary)
fn init_spellcheck(app: &App, app_state: &AppState) {
    let manager = Arc::new(crate::spellcheck::SpellcheckManager::new());
    let spellcheck_state = crate::commands::spellcheck::SpellcheckState(manager.clone());
    *app_state.editor.spellcheck_manager.lock() = Some(manager);
    app.manage(spellcheck_state);
    info!("[spellcheck] initialized");
}

/// Build the windows of the ordinary startup from their `tauri.conf.json`
/// entries. The `settings-recovery` entry is intentionally absent from this
/// list: that window is created only by the recovery process.
fn create_configured_windows(
    app: &App,
    start_compact: bool,
    compact_view: crate::config::CompactView,
) -> Result<(), Box<dyn std::error::Error>> {
    for label in ["main", "soundpanel", "playback-control", "ocr-selection"] {
        let config = app
            .config()
            .app
            .windows
            .iter()
            .find(|window| window.label == label)
            .cloned()
            .ok_or_else(|| format!("window config for {label} is missing"))?;
        let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)
            .with_context(|| format!("failed to prepare window {label}"))?;
        if label == "main" {
            builder = builder.initialization_script(build_main_window_initialization_script(
                start_compact,
                compact_view,
            ));
        }
        builder
            .build()
            .with_context(|| format!("failed to create window {label}"))?;
        info!(label, "Window created from config");
    }
    Ok(())
}

/// Initialize application windows
fn init_windows(
    app: &App,
    windows: &WindowsSettings,
    windows_manager: &WindowsManager,
    settings: &AppSettings,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("State initialized");

    // Apply saved main window position (window will be shown after backend is ready)
    if let Some(main_window) = app.get_webview_window("main") {
        if let (Some(x), Some(y)) = (windows.main.x, windows.main.y) {
            if saved_main_position_is_visible(&main_window, x, y) {
                info!(x, y, "Restoring main window position");
                let _ = main_window
                    .set_position(tauri::Position::Physical(tauri::PhysicalPosition { x, y }));
            } else {
                warn!(
                    x,
                    y, "Saved main window position is outside available work areas; recovering"
                );
                recover_main_window_to_primary(&main_window, windows_manager, "startup");
            }
        }

        // Apply theme to the Tauri window itself to ensure titlebar and OS frames match
        let tauri_theme = match settings.theme {
            crate::config::Theme::Light => tauri::Theme::Light,
            crate::config::Theme::Dark => tauri::Theme::Dark,
        };
        let _ = main_window.set_theme(Some(tauri_theme));
        info!(?tauri_theme, "Applied initial window theme");
    }

    if let Some(pb_window) = app.get_webview_window("playback-control") {
        let tauri_theme = match settings.theme {
            crate::config::Theme::Light => tauri::Theme::Light,
            crate::config::Theme::Dark => tauri::Theme::Dark,
        };
        let _ = pb_window.set_theme(Some(tauri_theme));
    }

    // Set the application-controlled native window titles from the startup
    // localization snapshot so every window matches the selected language.
    apply_localized_window_titles(app);

    Ok(())
}

/// Set native titles for the app-controlled windows using the localization
/// snapshot captured at startup. A key missing from the snapshot falls back
/// to the static title declared for the window in `tauri.conf.json`.
fn apply_localized_window_titles(app: &App) {
    let snapshot = app
        .state::<crate::commands::localization::LocalizationState>()
        .snapshot();
    let title_text = |key: &str, fallback: &str| {
        snapshot
            .messages
            .get(key)
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    };

    let configured: [(&str, &str, &str); 4] = [
        ("main", "window.main", "TTSBard"),
        ("playback-control", "window.playback", "Управление"),
        ("soundpanel", "window.soundpanel", ""),
        ("ocr-selection", "window.ocr_selection", ""),
    ];

    for (label, key, fallback) in configured {
        if let Some(window) = app.get_webview_window(label) {
            let title = title_text(key, fallback);
            if let Err(error) = window.set_title(&title) {
                warn!(label, error = %error, "Failed to set localized window title");
            }
        }
    }
}

const MIN_VISIBLE_WINDOW_WIDTH: i32 = 160;
const MIN_VISIBLE_WINDOW_HEIGHT: i32 = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PhysicalRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl PhysicalRect {
    fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }
}

fn has_sufficient_visible_area(window: PhysicalRect, work_areas: &[PhysicalRect]) -> bool {
    work_areas.iter().any(|area| {
        let overlap_width =
            (window.right().min(area.right()) - i64::from(window.x).max(i64::from(area.x))).max(0);
        let overlap_height = (window.bottom().min(area.bottom())
            - i64::from(window.y).max(i64::from(area.y)))
        .max(0);
        overlap_width >= i64::from(MIN_VISIBLE_WINDOW_WIDTH)
            && overlap_height >= i64::from(MIN_VISIBLE_WINDOW_HEIGHT)
    })
}

fn monitor_work_areas(window: &tauri::WebviewWindow) -> Result<Vec<PhysicalRect>, tauri::Error> {
    window.available_monitors().map(|monitors| {
        monitors
            .into_iter()
            .map(|monitor| {
                let area = monitor.work_area();
                PhysicalRect {
                    x: area.position.x,
                    y: area.position.y,
                    width: area.size.width,
                    height: area.size.height,
                }
            })
            .collect()
    })
}

fn saved_main_position_is_visible(window: &tauri::WebviewWindow, x: i32, y: i32) -> bool {
    let size = match window.outer_size() {
        Ok(size) => size,
        Err(error) => {
            warn!(error = %error, "Could not read main window size; keeping saved position");
            return true;
        }
    };
    let work_areas = match monitor_work_areas(window) {
        Ok(work_areas) if !work_areas.is_empty() => work_areas,
        Ok(_) => {
            warn!("No monitors available; keeping saved main window position");
            return true;
        }
        Err(error) => {
            warn!(error = %error, "Could not enumerate monitors; keeping saved main window position");
            return true;
        }
    };
    has_sufficient_visible_area(
        PhysicalRect {
            x,
            y,
            width: size.width,
            height: size.height,
        },
        &work_areas,
    )
}

fn recover_main_window_to_primary(
    window: &tauri::WebviewWindow,
    windows_manager: &WindowsManager,
    action: &str,
) {
    let size = match window.outer_size() {
        Ok(size) => size,
        Err(error) => {
            warn!(action, error = %error, "Could not read main window size for recovery");
            return;
        }
    };
    let primary = match window.primary_monitor() {
        Ok(Some(monitor)) => monitor,
        Ok(None) => {
            warn!(
                action,
                "No primary monitor available for main window recovery"
            );
            return;
        }
        Err(error) => {
            warn!(action, error = %error, "Could not read primary monitor for main window recovery");
            return;
        }
    };
    let area = primary.work_area();
    let width = i32::try_from(size.width).unwrap_or(i32::MAX);
    let height = i32::try_from(size.height).unwrap_or(i32::MAX);
    let area_width = i32::try_from(area.size.width).unwrap_or(i32::MAX);
    let area_height = i32::try_from(area.size.height).unwrap_or(i32::MAX);
    let x = area
        .position
        .x
        .saturating_add(area_width.saturating_sub(width) / 2);
    let y = area
        .position
        .y
        .saturating_add(area_height.saturating_sub(height) / 2);
    if let Err(error) =
        window.set_position(tauri::Position::Physical(tauri::PhysicalPosition { x, y }))
    {
        warn!(action, error = %error, "Could not recover main window position");
        return;
    }
    match window.outer_position() {
        Ok(position) => {
            if let Err(error) =
                windows_manager.set_main_position(Some(position.x), Some(position.y))
            {
                warn!(action, error = %error, "Could not persist recovered main window position");
            }
        }
        Err(error) => {
            warn!(action, error = %error, "Could not read recovered main window position")
        }
    }
}

fn ensure_main_window_visible(
    window: &tauri::WebviewWindow,
    windows_manager: &WindowsManager,
    action: &str,
) {
    let position = match window.outer_position() {
        Ok(position) => position,
        Err(error) => {
            warn!(action, error = %error, "Could not read main window position before showing");
            return;
        }
    };
    if !saved_main_position_is_visible(window, position.x, position.y) {
        warn!(
            action,
            x = position.x,
            y = position.y,
            "Main window is outside available work areas; recovering before show"
        );
        recover_main_window_to_primary(window, windows_manager, action);
    }
}

/// Show (restore) the main webview window: show, unminimize, and focus.
///
/// Logs failures with `warn!` and the provided `action` context, without panicking.
pub(crate) fn show_main_window(app_handle: &AppHandle, action: &str) {
    let window = match app_handle.get_webview_window("main") {
        Some(w) => w,
        None => {
            warn!(action, "show_main_window: main window not found");
            return;
        }
    };
    let windows_manager = app_handle.state::<WindowsManager>();
    ensure_main_window_visible(&window, windows_manager.inner(), action);
    if let Err(e) = window.show() {
        warn!(action, error = %e, "show_main_window: show failed");
    }
    if let Err(e) = window.unminimize() {
        warn!(action, error = %e, "show_main_window: unminimize failed");
    }
    if let Err(e) = window.set_focus() {
        warn!(action, error = %e, "show_main_window: set_focus failed");
    }
}

/// Initialize system tray
fn init_tray(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let app_handle = app.handle().clone();

    let localization_state = app.state::<crate::commands::localization::LocalizationState>();
    let snapshot = localization_state.snapshot();
    let tray_text = |key: &str, fallback: &str| {
        snapshot
            .messages
            .get(key)
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    };

    // Load icon.png (512x512) for tray
    let png_bytes = include_bytes!("../icons/icon.png");
    let decoded_image = image::load_from_memory(png_bytes)
        .map_err(|e| format!("Failed to decode tray icon: {}", e))?;
    let rgba_image = decoded_image.to_rgba8();
    let (width, height) = (rgba_image.width(), rgba_image.height());

    // Resize to 32x32 for tray
    let resized =
        image::imageops::resize(&rgba_image, 32, 32, image::imageops::FilterType::Lanczos3);
    let tray_icon = Image::new_owned(resized.into_raw(), 32, 32);

    info!(
        width,
        height, "Initializing system tray with icon (resized to 32x32 from original)"
    );

    // Create context menu
    let show_main = MenuItem::with_id(
        &app_handle,
        "show_main",
        tray_text("tray.show_main", "Show main window"),
        true,
        None as Option<&str>,
    )
    .map_err(|e| format!("Failed to create 'show_main' menu item: {}", e))?;
    let sp_toggle = MenuItem::with_id(
        &app_handle,
        "toggle_soundpanel",
        tray_text("tray.soundpanel", "Sound panel"),
        true,
        None as Option<&str>,
    )
    .map_err(|e| format!("Failed to create 'toggle_soundpanel' menu item: {}", e))?;
    let pc_toggle = MenuItem::with_id(
        &app_handle,
        "toggle_playback",
        tray_text("tray.playback", "Playback"),
        true,
        None as Option<&str>,
    )
    .map_err(|e| format!("Failed to create 'toggle_playback' menu item: {}", e))?;
    let separator = PredefinedMenuItem::separator(&app_handle)
        .map_err(|e| format!("Failed to create separator: {}", e))?;
    let quit_item = MenuItem::with_id(
        &app_handle,
        "quit",
        tray_text("tray.quit", "Quit"),
        true,
        None as Option<&str>,
    )
    .map_err(|e| format!("Failed to create 'quit' menu item: {}", e))?;

    let menu = Menu::with_items(
        &app_handle,
        &[&show_main, &sp_toggle, &pc_toggle, &separator, &quit_item],
    )
    .map_err(|e| format!("Failed to build menu: {}", e))?;

    // Create tray icon
    info!("[TRAY] Creating tray icon...");
    TrayIconBuilder::with_id("main")
        .icon(tray_icon)
        .tooltip("TTSBard")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                if matches!(button, tauri::tray::MouseButton::Left)
                    && matches!(button_state, tauri::tray::MouseButtonState::Up)
                {
                    show_main_window(tray.app_handle(), "tray-left-click");
                }
            }
        })
        .on_menu_event(|tray, event| match event.id.as_ref() {
            "show_main" => {
                show_main_window(tray.app_handle(), "tray-menu-show_main");
            }
            "toggle_soundpanel" => {
                if let Err(e) =
                    crate::soundpanel_window::toggle_soundpanel_window(tray.app_handle())
                {
                    warn!(error = %e, "Tray toggle_soundpanel failed");
                }
            }
            "toggle_playback" => {
                if let Err(e) = crate::playback_window::toggle_playback_window(tray.app_handle()) {
                    warn!(error = %e, "Tray toggle_playback failed");
                }
            }
            "quit" => {
                tauri::async_runtime::spawn(crate::commands::coordinate_shutdown(
                    tray.app_handle().clone(),
                ));
            }
            _ => {}
        })
        .menu(&menu)
        .build(&app_handle)
        .map_err(|e| format!("Failed to build tray icon: {}", e))?;
    info!("[TRAY] Tray icon created successfully");

    Ok(())
}

/// Initialize keyboard hooks
fn init_hooks(
    app_state: &AppState,
    soundpanel_state: &SoundPanelState,
    app_handle: AppHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::soundpanel::initialize_soundpanel_hook;

    let hook_manager = initialize_soundpanel_hook(soundpanel_state.clone(), app_handle);
    *app_state.soundpanel_hook.lock() = Some(hook_manager);
    info!("[SOUNDPANEL] Keyboard hook initialized");

    Ok(())
}

/// Initialize WebView server
fn init_webview_server(app_state: &AppState, app_handle: AppHandle) {
    let webview_settings = app_state.webview.settings.clone();
    let (webview_tx, webview_rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let shutdown = app_state.shutdown.clone();

    app_state.webview.set_event_sender(webview_tx);

    app_state.runtime.spawn(async move {
        crate::servers::run_webview_server(webview_settings, app_handle, webview_rx, shutdown)
            .await;
    });
}

/// Initialize Twitch client
fn init_twitch_client(app_state: &AppState, app_handle: AppHandle) {
    let app_state_clone = app_state.clone();
    let twitch_rx = app_state.twitch.event_tx.subscribe();
    let shutdown = app_state.shutdown.clone();

    app_state.runtime.spawn(async move {
        crate::servers::run_twitch_client(app_state_clone, app_handle, twitch_rx, shutdown).await;
    });
}

/// Initialize the Twitch API authorization coordinator (ROADMAP-132).
///
/// The credential path is derived from the settings config directory here and
/// never supplied by the frontend. An active OAuth session is cancelled when the
/// shared shutdown token fires.
fn init_twitch_api_auth(app_state: &AppState, settings_manager: &SettingsManager) {
    let path = settings_manager.config_dir().join("secrets.dat");
    let store = crate::twitch::credentials::CredentialsStore::new(path);
    let shutdown = app_state.shutdown.clone();
    let coordinator = Arc::new(crate::twitch::auth::TwitchAuthCoordinator::new(
        store,
        crate::twitch::api::DEFAULT_TIMEOUT,
        shutdown.clone(),
    ));

    *app_state.twitch.auth.blocking_write() = Some(coordinator.clone());

    app_state.runtime.spawn(async move {
        shutdown.cancelled().await;
        coordinator.cancel_active().await;
    });
    info!("Twitch API auth coordinator initialized");
}

/// Initialize the input server loopback supervisor.
fn init_input_server(app_state: &AppState, app_handle: AppHandle) {
    // Seed the in-memory run request from the persisted boot preference before
    // the supervisor begins. The run request is not persisted.
    let start_on_boot = {
        let settings = app_state.input_server.settings.blocking_read();
        settings.start_on_boot
    };
    app_state.input_server.set_run_request(start_on_boot);

    let shutdown = app_state.shutdown.clone();

    app_state.runtime.spawn(async move {
        crate::servers::run_input_server(app_handle, shutdown).await;
    });
    info!("Input server supervisor started");
}

/// Initialize the OCR runtime from the persisted `enabled` setting.
fn init_ocr(app_state: &AppState, app_handle: AppHandle, enabled: bool, hotkey: Hotkey) {
    if !enabled {
        info!("OCR disabled on boot (enabled=false)");
        return;
    }

    let app_handle_clone = app_handle.clone();
    let app_state_clone = app_state.clone();
    app_state.runtime.spawn(async move {
        crate::commands::ocr::start_ocr_runtime(&app_handle_clone, &app_state_clone, &hotkey).await;
    });
    info!("OCR boot start scheduled (enabled=true)");
}

/// Initialize window protection (Windows only)
#[cfg(windows)]
fn init_window_protection(app: &App, windows_manager: &WindowsManager) {
    use crate::window::set_window_exclude_from_capture;

    let exclude_from_capture = windows_manager.get_global_exclude_from_capture();
    info!(
        exclude_from_capture,
        "Applying global exclude from capture to main window"
    );

    if let Some(main_window) = app.get_webview_window("main") {
        if let Ok(hwnd) = main_window.hwnd() {
            match set_window_exclude_from_capture(hwnd.0 as isize, exclude_from_capture) {
                Ok(_) => info!(
                    exclude_from_capture,
                    "Main window exclude from capture applied"
                ),
                Err(e) => error!(error = %e, "Failed to apply exclude from capture to main window"),
            }
        }
    }
}

fn init_vtube_studio(app_state: &AppState, app_handle: AppHandle) {
    // Подключаем AppHandle ДО всего: connection-actor должен уметь эмитить статус
    // при transport-failure из idle независимо от того, включён ли автозапуск.
    app_state.vtube_studio.attach_app_handle(app_handle.clone());

    let start_on_boot = {
        let settings = app_state.vtube_studio.settings.blocking_read();
        settings.start_on_boot
    };

    if !start_on_boot {
        info!("VTube Studio autostart disabled (start_on_boot=false)");
        return;
    }

    info!("VTube Studio: auto-start on boot");

    let app_handle_clone = app_handle.clone();
    let app_state_clone = app_state.clone();

    app_state.runtime.spawn(async move {
        let (host, port, stored_token) = {
            let settings = app_state_clone.vtube_studio.settings.read().await;
            (settings.host.clone(), settings.port, settings.token.clone())
        };

        info!(
            host = %host,
            port,
            has_token = stored_token.is_some(),
            "VTube Studio: attempting autostart connection"
        );

        let result = app_state_clone
            .vtube_studio
            .connect(
                &host,
                port,
                stored_token.as_deref(),
                ConnectOrigin::Autostart,
            )
            .await;

        let status = app_state_clone.vtube_studio.get_connection_status();
        let _ = app_handle_clone.emit("vtube-studio-status-changed", &status);

        match result {
            Ok(new_token) => {
                info!("VTube Studio: autostart connected successfully");
                if let Some(ref tok) = new_token {
                    let mut s = app_state_clone.vtube_studio.settings.write().await;
                    s.token = Some(tok.clone());
                    drop(s);

                    let mgr_opt = app_handle_clone.try_state::<SettingsManager>();
                    if let Some(mgr) = mgr_opt {
                        let inner = mgr.inner().clone();
                        let tok_clone = tok.clone();
                        let _ = crate::commands::persist_blocking(&inner, move |m| {
                            let mut vts = m.get_vtube_studio_settings();
                            vts.token = Some(tok_clone);
                            m.set_vtube_studio_settings(&vts)
                        })
                        .await;
                    }
                }
            }
            Err(e) => {
                info!(error = %e, "VTube Studio: autostart connection failed (non-fatal)");
            }
        }
    });
}

fn q_state_dto(
    q: &Arc<parking_lot::Mutex<crate::speech_queue::SpeechQueue>>,
) -> crate::speech_queue::SpeechQueueStateDto {
    q.lock().state()
}

// ── Speech worker ──

#[allow(clippy::too_many_arguments)] // Worker dependencies are explicit ownership boundaries.
async fn speech_worker(
    queue: Arc<parking_lot::Mutex<crate::speech_queue::SpeechQueue>>,
    notify: Arc<tokio::sync::Notify>,
    app_handle: AppHandle,
    shutdown: tokio_util::sync::CancellationToken,
    editor: Arc<crate::editor::EditorService>,
    playback_manager: Arc<parking_lot::Mutex<Option<Arc<crate::playback::PlaybackManager>>>>,
    webview: Arc<crate::webview::service::WebViewService>,
    twitch: Arc<crate::twitch::TwitchService>,
    vrchat: Arc<crate::vrchat::VrchatService>,
) {
    use crate::commands::tts_pipeline;
    use crate::speech_queue::PreparationWait;

    loop {
        let work_item = {
            let mut q = queue.lock();
            match q.claim_next_generation() {
                Ok(Some(item)) => Some(item),
                Ok(None) => {
                    drop(q);
                    None
                }
                Err(e) => {
                    warn!(error = %e, "claim_next_generation error, worker continuing");
                    drop(q);
                    None
                }
            }
        };

        let work_item = match work_item {
            Some(item) => item,
            None => {
                tokio::select! {
                    _ = notify.notified() => {
                        continue;
                    }
                    _ = shutdown.cancelled() => {
                        info!("Speech worker exiting on shutdown (idle wait)");
                        return;
                    }
                }
            }
        };

        {
            let q = queue.lock();
            let dto = q.state();
            drop(q);
            let _ = app_handle.emit("speech-queue-changed", dto);
        }

        let job_id = work_item.job_id;
        let snapshot = work_item.snapshot;
        let original_text = work_item.original_text;
        let cancellation = work_item.cancel_token;

        let preparation = tts_pipeline::prepare_speech(&snapshot, &original_text);
        let prepared =
            match crate::speech_queue::await_preparation(preparation, &cancellation, &shutdown)
                .await
            {
                PreparationWait::Completed(result) => result,
                PreparationWait::Cancelled => {
                    info!(
                        job_id = %job_id,
                        "Speech preparation cancelled; dropping pending provider work"
                    );
                    continue;
                }
                PreparationWait::Shutdown => {
                    let mut q = queue.lock();
                    let _ = q.fail_generation(
                        job_id,
                        "Speech preparation cancelled (shutdown)".to_string(),
                    );
                    let dto = q.state();
                    drop(q);
                    let _ = app_handle.emit("speech-queue-changed", dto);
                    info!("Speech worker exiting on shutdown (during preparation)");
                    return;
                }
            };

        match prepared {
            Ok(prepared) => {
                let (speaker, mic) =
                    tts_pipeline::compute_output_configs(&snapshot.audio, &snapshot.audio_effects);

                if speaker.is_none() && mic.is_none() {
                    let mut q = queue.lock();
                    let _ = q.fail_generation(
                        job_id,
                        "All outputs disabled (speaker and mic both off)".to_string(),
                    );
                    let dto = q.state();
                    drop(q);
                    let _ = app_handle.emit("speech-queue-changed", dto);
                    continue;
                }

                let provider_text = prepared.provider_text.clone();
                let insert_text = prepared.insert_text.clone();

                let handoff_guard = {
                    let mut q = queue.lock();
                    let status = q.get_status(job_id);
                    if status != Some(JobStatus::Generating) {
                        drop(q);
                        info!(job_id = %job_id, observed_status = ?status, "worker discard: not Generating after preparation");
                        let _ = app_handle.emit("speech-queue-changed", q_state_dto(&queue));
                        continue;
                    }
                    let _ = q.mark_ready(job_id, insert_text.clone());
                    let guard = q
                        .get_handoff_guard(job_id)
                        .expect("handoff_guard missing after mark_ready");
                    let dto = q.state();
                    drop(q);
                    let _ = app_handle.emit("speech-queue-changed", dto);
                    guard
                };

                let handoff_accepted = {
                    let _g = handoff_guard.lock();

                    {
                        let q = queue.lock();
                        let status = q.get_status(job_id);
                        if status != Some(JobStatus::Ready) {
                            drop(q);
                            info!(job_id = %job_id, observed_status = ?status, "worker discard: cancelled after mark_ready");
                            false
                        } else {
                            drop(q);
                            let pb_guard = playback_manager.lock();
                            if let Some(pb) = pb_guard.as_ref() {
                                pb.enqueue_with_outputs(
                                    job_id.to_string(),
                                    insert_text.clone(),
                                    prepared.audio,
                                    speaker,
                                    mic,
                                )
                            } else {
                                false
                            }
                        }
                    }
                };

                if !handoff_accepted {
                    let mut q = queue.lock();
                    if q.get_status(job_id) == Some(JobStatus::Ready) {
                        let _ = q.fail_playback(
                            job_id,
                            "Playback handoff rejected (queue full or no manager)".to_string(),
                        );
                        let dto = q.state();
                        drop(q);
                        let _ = app_handle.emit("speech-queue-changed", dto);
                    }
                    continue;
                }

                {
                    let text = insert_text.clone();
                    let delivery_text = prepared.delivery_text.clone();
                    let webview_svc = webview.clone();
                    let twitch_svc = twitch.clone();
                    let vrchat_svc = vrchat.clone();
                    let skip_twitch = snapshot.delivery.skip_twitch();
                    let skip_webview = snapshot.delivery.skip_webview();
                    let skip_vrchat = snapshot.delivery.skip_vrchat();
                    let join_handle = tokio::task::spawn_blocking(move || {
                        route_external_text_from_handles(
                            &webview_svc,
                            &twitch_svc,
                            &vrchat_svc,
                            &delivery_text,
                            &text,
                            skip_twitch,
                            skip_webview,
                            skip_vrchat,
                        );
                    });
                    if let Err(e) = join_handle.await {
                        warn!(error = %e, "route_external_text join failed");
                    }
                }

                {
                    // Owned handle: the history slot guard must not be held
                    // across blocking I/O or awaits. The write stays on the
                    // blocking pool and is awaited before the phrase event is
                    // published (persist-before-publish).
                    if let Some(history) = editor.history_handle() {
                        let (provider, voice, cache_key) =
                            if prepared.cache_saved || prepared.cache_hit {
                                (
                                    prepared.provider_name.clone(),
                                    prepared.voice_name.clone(),
                                    prepared.cache_key.clone(),
                                )
                            } else {
                                (String::new(), String::new(), String::new())
                            };
                        let provider_text = provider_text.clone();
                        let insert_text_for_history = insert_text.clone();
                        let write = move || {
                            history.record_phrase_with_meta(
                                &provider_text,
                                &insert_text_for_history,
                                &provider,
                                &voice,
                                &cache_key,
                            )
                        };
                        if let Err(error) = crate::history::write_phrase_blocking(write).await {
                            warn!(error = %error, "Failed to persist phrase history");
                        }
                    }
                }

                let history_event = crate::events::AppEvent::TextSentToTts(
                    crate::events::RoutedText::broadcast(insert_text.clone()),
                );
                let event_name = history_event.to_tauri_event();
                let _ = app_handle.emit(event_name, &history_event);

                loop {
                    let status = {
                        let q = queue.lock();
                        q.get_status(job_id)
                    };

                    match status {
                        Some(JobStatus::Playing) => break,
                        Some(JobStatus::Failed) => break,
                        Some(JobStatus::Ready) => {
                            tokio::select! {
                                _ = notify.notified() => {
                                    continue;
                                }
                                _ = shutdown.cancelled() => {
                                    info!("Speech worker exiting on shutdown (waiting for Playing)");
                                    return;
                                }
                            }
                        }
                        _ => {
                            warn!(?status, job_id = %job_id, "Unexpected status waiting for Playing");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                let mut q = queue.lock();
                let _ = q.fail_generation(job_id, e.clone());
                let dto = q.state();
                drop(q);
                let _ = app_handle.emit("speech-queue-changed", dto);
                warn!(error = %e, job_id = %job_id, "Speech preparation failed, worker waiting for retry/skip");
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn route_external_text_from_handles(
    webview: &crate::webview::service::WebViewService,
    twitch: &crate::twitch::TwitchService,
    vrchat: &crate::vrchat::VrchatService,
    delivery_text: &str,
    insert_text: &str,
    skip_twitch: bool,
    skip_webview: bool,
    skip_vrchat: bool,
) {
    if !skip_webview {
        // ROADMAP-107: per-server choice between the user's original text and
        // the TTS-processed representation, read at delivery time.
        let send_original = webview.settings.blocking_read().send_original_text;
        let text = if send_original {
            delivery_text
        } else {
            insert_text
        };
        webview.send_event(crate::events::AppEvent::TextSentToTts(
            crate::events::RoutedText::broadcast(text.to_string()),
        ));
    }
    if !skip_twitch {
        let settings = twitch.settings.blocking_read();
        if settings.enabled {
            let send_original = settings.send_original_text;
            drop(settings);
            let text = if send_original {
                delivery_text
            } else {
                insert_text
            };
            twitch.send_event(crate::events::TwitchEvent::SendMessage(text.to_string()));
        }
    }
    if !skip_vrchat && vrchat.is_enabled() {
        let vrchat_svc = vrchat.clone();
        let text = delivery_text.to_string();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(e) = vrchat_svc.send_text(&text).await {
                    tracing::warn!(error = %e, "Failed to deliver text to VRChat chatbox");
                }
            });
        }
    }
}

fn missing_piper_model_name<F>(saved_id: Option<&str>, is_registered: F) -> Option<String>
where
    F: FnOnce(&str) -> bool,
{
    let id = saved_id?;
    let model_name = id.strip_prefix("local-piper:")?;
    if model_name.is_empty() || is_registered(id) {
        return None;
    }
    Some(model_name.to_string())
}

fn missing_piper_notification(model_name: &str) -> String {
    format!(
        "Piper-модель «{}» не найдена. Выбран провайдер Silero",
        model_name
    )
}

/// Builds the initialization script injected into the main window before any frontend script runs.
///
/// Injects `window.__TTSBARD_START_COMPACT__` and `window.__TTSBARD_COMPACT_VIEW__`
/// to guarantee no-flash startup window mode and view resolution.
pub fn build_main_window_initialization_script(
    start_compact: bool,
    compact_view: crate::config::CompactView,
) -> String {
    let view_str = match compact_view {
        crate::config::CompactView::Mono => "mono",
        crate::config::CompactView::Compact => "compact",
    };
    format!(
        "window.__TTSBARD_START_COMPACT__ = {start_compact};\nwindow.__TTSBARD_COMPACT_VIEW__ = \"{view_str}\";"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        has_sufficient_visible_area, missing_piper_model_name, missing_piper_notification,
        route_external_text_from_handles, PhysicalRect,
    };

    fn rect(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect {
        PhysicalRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn visible_area_accepts_window_on_primary_monitor() {
        assert!(has_sufficient_visible_area(
            rect(100, 100, 800, 630),
            &[rect(0, 0, 1920, 1040)],
        ));
    }

    #[test]
    fn visible_area_accepts_window_on_negative_coordinate_monitor() {
        assert!(has_sufficient_visible_area(
            rect(-1800, 100, 800, 630),
            &[rect(-1920, 0, 1920, 1040), rect(0, 0, 1920, 1040)],
        ));
    }

    #[test]
    fn visible_area_rejects_window_lost_after_monitor_disconnect() {
        assert!(!has_sufficient_visible_area(
            rect(2200, 100, 800, 630),
            &[rect(0, 0, 1920, 1040)],
        ));
    }

    #[test]
    fn visible_area_rejects_narrow_strip() {
        assert!(!has_sufficient_visible_area(
            rect(1800, 100, 800, 630),
            &[rect(0, 0, 1920, 1040)],
        ));
    }

    #[test]
    fn visible_area_accepts_sufficient_partial_overlap() {
        assert!(has_sufficient_visible_area(
            rect(1760, 950, 800, 630),
            &[rect(0, 0, 1920, 1040)],
        ));
    }

    #[test]
    fn missing_saved_piper_returns_safe_model_name() {
        assert_eq!(
            missing_piper_model_name(Some("local-piper:irina"), |_| false),
            Some("irina".to_string())
        );
    }

    #[test]
    fn registered_piper_and_builtin_ids_do_not_trigger_fallback() {
        assert_eq!(
            missing_piper_model_name(Some("local-piper:irina"), |_| true),
            None
        );
        assert_eq!(missing_piper_model_name(Some("silero"), |_| false), None);
        assert_eq!(missing_piper_model_name(None, |_| false), None);
    }

    #[test]
    fn missing_piper_notification_matches_user_facing_contract() {
        assert_eq!(
            missing_piper_notification("irina"),
            "Piper-модель «irina» не найдена. Выбран провайдер Silero"
        );
    }

    #[test]
    fn routed_text_to_webview_and_twitch_is_clean_without_provider_markers() {
        use crate::events::{AppEvent, TwitchEvent};

        let webview = crate::webview::service::WebViewService::new();
        let (webview_tx, mut webview_rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        webview.set_event_sender(webview_tx);

        let (twitch_tx, mut twitch_rx) = tokio::sync::broadcast::channel::<TwitchEvent>(16);
        let twitch = crate::twitch::TwitchService::new(twitch_tx);
        twitch.settings.blocking_write().enabled = true;

        let clean_text = "Привет, мир";
        let vrchat = crate::vrchat::VrchatService::new();
        route_external_text_from_handles(
            &webview, &twitch, &vrchat, clean_text, clean_text, false, false, false,
        );

        match webview_rx.try_recv() {
            Ok(AppEvent::TextSentToTts(routed)) => {
                assert_eq!(routed.text, clean_text);
                assert!(!routed.text.contains('+'));
                assert!(!routed.text.contains('\u{0301}'));
            }
            Ok(other) => panic!("unexpected webview event: {other:?}"),
            Err(error) => panic!("missing webview event: {error}"),
        }

        match twitch_rx.try_recv() {
            Ok(TwitchEvent::SendMessage(text)) => {
                assert_eq!(text, clean_text);
                assert!(!text.contains('+'));
                assert!(!text.contains('\u{0301}'));
            }
            Ok(other) => panic!("unexpected twitch event: {other:?}"),
            Err(error) => panic!("missing twitch event: {error}"),
        }
    }

    #[test]
    fn routed_text_is_skipped_when_both_consumers_are_off() {
        use crate::events::{AppEvent, TwitchEvent};

        let webview = crate::webview::service::WebViewService::new();
        let (webview_tx, mut webview_rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        webview.set_event_sender(webview_tx);

        let (twitch_tx, mut twitch_rx) = tokio::sync::broadcast::channel::<TwitchEvent>(16);
        let twitch = crate::twitch::TwitchService::new(twitch_tx);
        twitch.settings.blocking_write().enabled = true;

        let vrchat = crate::vrchat::VrchatService::new();
        route_external_text_from_handles(
            &webview, &twitch, &vrchat, "hello", "hello", true, true, true,
        );

        assert!(
            webview_rx.try_recv().is_err(),
            "webview must not receive a skipped message"
        );
        assert!(
            twitch_rx.try_recv().is_err(),
            "twitch must not receive a skipped message"
        );
    }

    #[test]
    fn routed_text_default_sends_original_text_to_both_channels() {
        use crate::events::{AppEvent, TwitchEvent};

        let webview = crate::webview::service::WebViewService::new();
        let (webview_tx, mut webview_rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        webview.set_event_sender(webview_tx);

        let (twitch_tx, mut twitch_rx) = tokio::sync::broadcast::channel::<TwitchEvent>(16);
        let twitch = crate::twitch::TwitchService::new(twitch_tx);
        twitch.settings.blocking_write().enabled = true;

        let vrchat = crate::vrchat::VrchatService::new();
        // Сервисы после Task B создаются с send_original_text = true.
        route_external_text_from_handles(
            &webview,
            &twitch,
            &vrchat,
            "67 бананов.",
            "шестьдесят семь бананов.",
            false,
            false,
            false,
        );

        match webview_rx.try_recv() {
            Ok(AppEvent::TextSentToTts(routed)) => {
                assert_eq!(routed.text, "67 бананов.");
            }
            other => panic!("unexpected webview event: {other:?}"),
        }

        match twitch_rx.try_recv() {
            Ok(TwitchEvent::SendMessage(text)) => {
                assert_eq!(text, "67 бананов.");
            }
            other => panic!("unexpected twitch event: {other:?}"),
        }
    }

    #[test]
    fn routed_text_webview_toggle_off_sends_processed_text_only_to_webview() {
        use crate::events::{AppEvent, TwitchEvent};

        let webview = crate::webview::service::WebViewService::new();
        webview.settings.blocking_write().send_original_text = false;
        let (webview_tx, mut webview_rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        webview.set_event_sender(webview_tx);

        let (twitch_tx, mut twitch_rx) = tokio::sync::broadcast::channel::<TwitchEvent>(16);
        let twitch = crate::twitch::TwitchService::new(twitch_tx);
        twitch.settings.blocking_write().enabled = true;

        let vrchat = crate::vrchat::VrchatService::new();
        route_external_text_from_handles(
            &webview,
            &twitch,
            &vrchat,
            "67 бананов.",
            "шестьдесят семь бананов.",
            false,
            false,
            false,
        );

        match webview_rx.try_recv() {
            Ok(AppEvent::TextSentToTts(routed)) => {
                assert_eq!(routed.text, "шестьдесят семь бананов.");
            }
            other => panic!("unexpected webview event: {other:?}"),
        }

        match twitch_rx.try_recv() {
            Ok(TwitchEvent::SendMessage(text)) => {
                // Twitch-галка не тронута — канал получает исходник.
                assert_eq!(text, "67 бананов.");
            }
            other => panic!("unexpected twitch event: {other:?}"),
        }
    }

    #[test]
    fn routed_text_twitch_toggle_off_sends_processed_text_only_to_twitch() {
        use crate::events::{AppEvent, TwitchEvent};

        let webview = crate::webview::service::WebViewService::new();
        let (webview_tx, mut webview_rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        webview.set_event_sender(webview_tx);

        let (twitch_tx, mut twitch_rx) = tokio::sync::broadcast::channel::<TwitchEvent>(16);
        let twitch = crate::twitch::TwitchService::new(twitch_tx);
        twitch.settings.blocking_write().enabled = true;
        twitch.settings.blocking_write().send_original_text = false;

        let vrchat = crate::vrchat::VrchatService::new();
        route_external_text_from_handles(
            &webview,
            &twitch,
            &vrchat,
            "67 бананов.",
            "шестьдесят семь бананов.",
            false,
            false,
            false,
        );

        match webview_rx.try_recv() {
            Ok(AppEvent::TextSentToTts(routed)) => {
                assert_eq!(routed.text, "67 бананов.");
            }
            other => panic!("unexpected webview event: {other:?}"),
        }

        match twitch_rx.try_recv() {
            Ok(TwitchEvent::SendMessage(text)) => {
                assert_eq!(text, "шестьдесят семь бананов.");
            }
            other => panic!("unexpected twitch event: {other:?}"),
        }
    }

    #[test]
    fn routed_text_delivers_to_vrchat_when_enabled() {
        use crate::config::VrchatSettings;
        use rosc::decoder::decode_udp;
        use rosc::{OscPacket, OscType};
        use std::net::UdpSocket;
        use std::time::Duration;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind socket");
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set timeout");
        let port = socket.local_addr().expect("port").port();

        let webview = crate::webview::service::WebViewService::new();
        let (twitch_tx, _) = tokio::sync::broadcast::channel(16);
        let twitch = crate::twitch::TwitchService::new(twitch_tx);
        let vrchat = crate::vrchat::VrchatService::with_settings(VrchatSettings {
            enabled: true,
            start_on_boot: false,
            host: "127.0.0.1".into(),
            port,
        });

        route_external_text_from_handles(
            &webview,
            &twitch,
            &vrchat,
            "Clean delivery text",
            "Markup text 123",
            true,
            true,
            false,
        );

        let mut buf = [0u8; 1024];
        let (len, _) = socket.recv_from(&mut buf).expect("recv ok");

        let (_, packet) = decode_udp(&buf[..len]).expect("valid osc packet");
        match packet {
            OscPacket::Message(msg) => {
                assert_eq!(msg.addr, "/chatbox/input");
                assert_eq!(msg.args[0], OscType::String("Clean delivery text".into()));
                assert_eq!(msg.args[1], OscType::Bool(true));
                assert_eq!(msg.args[2], OscType::Bool(true));
            }
            _ => panic!("expected OscMessage"),
        }
    }

    #[test]
    fn routed_text_skips_vrchat_when_flag_set() {
        use crate::config::VrchatSettings;
        use std::net::UdpSocket;
        use std::time::Duration;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind socket");
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("set timeout");
        let port = socket.local_addr().expect("port").port();

        let webview = crate::webview::service::WebViewService::new();
        let (twitch_tx, _) = tokio::sync::broadcast::channel(16);
        let twitch = crate::twitch::TwitchService::new(twitch_tx);
        let vrchat = crate::vrchat::VrchatService::with_settings(VrchatSettings {
            enabled: true,
            start_on_boot: false,
            host: "127.0.0.1".into(),
            port,
        });

        route_external_text_from_handles(
            &webview,
            &twitch,
            &vrchat,
            "Clean delivery text",
            "Markup text",
            false,
            false,
            true,
        );

        let mut buf = [0u8; 1024];
        let res = socket.recv_from(&mut buf);
        assert!(res.is_err(), "must not receive when skip_vrchat is true");
    }

    #[test]
    fn routed_text_skips_vrchat_when_disabled() {
        use crate::config::VrchatSettings;
        use std::net::UdpSocket;
        use std::time::Duration;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind socket");
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("set timeout");
        let port = socket.local_addr().expect("port").port();

        let webview = crate::webview::service::WebViewService::new();
        let (twitch_tx, _) = tokio::sync::broadcast::channel(16);
        let twitch = crate::twitch::TwitchService::new(twitch_tx);
        let vrchat = crate::vrchat::VrchatService::with_settings(VrchatSettings {
            enabled: false,
            start_on_boot: false,
            host: "127.0.0.1".into(),
            port,
        });

        route_external_text_from_handles(
            &webview,
            &twitch,
            &vrchat,
            "Clean delivery text",
            "Markup text",
            false,
            false,
            false,
        );

        let mut buf = [0u8; 1024];
        let res = socket.recv_from(&mut buf);
        assert!(res.is_err(), "must not receive when vrchat is disabled");
    }

    #[test]
    fn routed_text_vrchat_invalid_destination_does_not_break_webview_or_twitch() {
        use crate::config::VrchatSettings;
        use crate::events::{AppEvent, TwitchEvent};

        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        let webview = crate::webview::service::WebViewService::new();
        let (webview_tx, mut webview_rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        webview.set_event_sender(webview_tx);

        let (twitch_tx, mut twitch_rx) = tokio::sync::broadcast::channel::<TwitchEvent>(16);
        let twitch = crate::twitch::TwitchService::new(twitch_tx);
        twitch.settings.blocking_write().enabled = true;

        let vrchat = crate::vrchat::VrchatService::with_settings(VrchatSettings {
            enabled: true,
            start_on_boot: false,
            host: "127.0.0.1".into(),
            port: 1,
        });

        route_external_text_from_handles(
            &webview,
            &twitch,
            &vrchat,
            "Delivery text",
            "Insert text",
            false,
            false,
            false,
        );

        assert!(webview_rx.try_recv().is_ok());
        assert!(twitch_rx.try_recv().is_ok());
    }

    #[test]
    fn build_main_window_initialization_script_matrix() {
        use super::build_main_window_initialization_script;
        use crate::config::CompactView;

        assert_eq!(
            build_main_window_initialization_script(false, CompactView::Compact),
            "window.__TTSBARD_START_COMPACT__ = false;\nwindow.__TTSBARD_COMPACT_VIEW__ = \"compact\";"
        );
        assert_eq!(
            build_main_window_initialization_script(false, CompactView::Mono),
            "window.__TTSBARD_START_COMPACT__ = false;\nwindow.__TTSBARD_COMPACT_VIEW__ = \"mono\";"
        );
        assert_eq!(
            build_main_window_initialization_script(true, CompactView::Compact),
            "window.__TTSBARD_START_COMPACT__ = true;\nwindow.__TTSBARD_COMPACT_VIEW__ = \"compact\";"
        );
        assert_eq!(
            build_main_window_initialization_script(true, CompactView::Mono),
            "window.__TTSBARD_START_COMPACT__ = true;\nwindow.__TTSBARD_COMPACT_VIEW__ = \"mono\";"
        );
    }

    #[test]
    fn twitch_api_auth_initialized_publishes_coordinator_to_app_state() {
        let app_state = crate::state::AppState::new();
        let test_dir =
            std::env::temp_dir().join(format!("ttsbard-test-setup-{}", uuid::Uuid::new_v4()));
        let settings_manager = crate::SettingsManager::with_config_dir(test_dir.clone()).unwrap();

        assert!(app_state.twitch.auth.blocking_read().is_none());
        super::init_twitch_api_auth(&app_state, &settings_manager);
        assert!(app_state.twitch.auth.blocking_read().is_some());

        let _ = std::fs::remove_dir_all(&test_dir);
    }
}
