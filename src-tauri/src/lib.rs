mod ai;
mod assets;
pub mod audio;
mod commands;
mod config;
mod editor;
mod error;
mod event_loop;
mod events;
mod history;
mod hotkeys;
mod input_server;
pub mod ipc;
mod localization;
mod model_downloads;
mod ocr;
mod paths;
pub mod playback;
mod playback_window;
mod preprocessor;
mod rate_limiter;
mod secret_log;
pub mod secrets;
mod servers;
mod settings_recovery;
mod setup;
mod signalsmith;
#[cfg(windows)]
mod single_instance;
mod soundpanel;
mod soundpanel_window;
pub mod speech_queue;
mod spellcheck;
mod state;
mod storage_transfer;
pub mod stress;
mod system_fonts;
mod tabs;
mod telegram;
mod thread_manager;
mod tts;
mod twitch;
pub mod vrchat;
mod vtube_studio;
mod webview;
mod window;

use anyhow::Context;
use commands::ai::{
    ai_check_grammar, correct_text, get_ai_completion, get_ai_custom_model, get_ai_deepseek_model,
    get_ai_openai_model, get_ai_zai_model, get_editor_ai, get_editor_ai_completion,
    set_ai_custom_api_key, set_ai_custom_model, set_ai_custom_url, set_ai_custom_use_proxy,
    set_ai_deepseek_api_key, set_ai_deepseek_model, set_ai_deepseek_use_proxy,
    set_ai_openai_api_key, set_ai_openai_model, set_ai_openai_use_proxy, set_ai_prompt,
    set_ai_provider, set_ai_zai_api_key, set_ai_zai_model, set_ai_zai_url, set_editor_ai,
    set_editor_ai_completion, set_visible_tts_provider_ids,
};
use commands::logging::{get_logging_settings, save_logging_settings};
use commands::playback_window::{pc_get_appearance, pc_set_bg_color, pc_set_opacity};
use commands::telegram::TelegramState;
use commands::telegram::{
    telegram_auto_restore, telegram_check_password, telegram_disconnect, telegram_get_status,
    telegram_get_user, telegram_init, telegram_request_code, telegram_sign_in, telegram_sign_out,
};
use commands::{
    apply_openai_proxy_settings, close_playback_control_window, close_soundpanel_window,
    disable_virtual_mic, enable_virtual_mic, get_audio_effects, get_audio_output_format_pending,
    get_audio_settings, get_dsp_settings, get_editor_font_family, get_editor_font_size_px,
    get_editor_height, get_editor_hotkeys, get_editor_quick, get_editor_spellcheck_enabled,
    get_editor_spellcheck_source, get_editor_typing_idle_timeout_ms,
    get_global_exclude_from_capture, get_hotkey_enabled, get_hotkey_settings, get_local_tts_url,
    get_main_appearance, get_main_compact_dims, get_main_compact_view, get_openai_api_key,
    get_openai_voice, get_output_devices, get_playback_appearance_source,
    get_show_playback_on_start, get_soundpanel_appearance_source, get_system_font_families,
    get_tts_provider, get_ui_font_family, get_ui_font_size_px, get_virtual_mic_devices,
    get_visibility_snapshot, has_api_key, hide_main_window, list_homograph_accentor_packs,
    list_pocket_models, load_homograph_accentor_model, open_file_dialog,
    prepare_tts_provider_by_id, preview_audio_file, preview_contextual_ruaccent, quit_app,
    refresh_homograph_accentor_packs, refresh_pocket_models, reregister_hotkeys_cmd,
    reset_editor_hotkey, reset_hotkey_to_default, save_audio_effects, save_dsp_settings,
    select_tts_provider_by_id, set_audio_effects_enabled, set_audio_effects_enhance_atten_db,
    set_audio_effects_enhance_enabled, set_audio_effects_formant_preserved,
    set_audio_effects_pitch, set_audio_effects_speed, set_audio_effects_volume,
    set_audio_output_format, set_editor_autocomplete_enabled, set_editor_default_route,
    set_editor_font_family, set_editor_font_size, set_editor_height, set_editor_homograph_accentor,
    set_editor_homograph_accentor_load_on_start, set_editor_hotkey, set_editor_hotkey_enabled,
    set_editor_keep_text, set_editor_number_conversion_enabled, set_editor_quick,
    set_editor_spellcheck_enabled, set_editor_spellcheck_source, set_editor_typing_enabled,
    set_editor_typing_idle_timeout_ms, set_global_exclude_from_capture,
    set_hide_extra_window_buttons, set_hide_on_minimize, set_hotkey, set_hotkey_binding_enabled,
    set_hotkey_enabled, set_hotkey_recording, set_local_tts_url, set_main_bg_color,
    set_main_compact_dims, set_main_compact_view, set_main_custom_background,
    set_main_custom_opacity, set_main_opacity, set_main_opacity_compact_only, set_openai_api_key,
    set_openai_voice, set_playback_appearance_source, set_pocket_voice, set_show_playback_on_start,
    set_soundpanel_appearance_source, set_speaker_device, set_speaker_enabled, set_speaker_volume,
    set_start_compact, set_tts_provider, set_ui_font_family, set_ui_font_size,
    set_virtual_mic_device, set_virtual_mic_volume, speak_text_raw_export,
    start_homograph_accentor_startup_load, stop_preview, test_audio_device,
    toggle_playback_control_window, toggle_soundpanel_window, unregister_hotkeys, update_theme,
    validate_pocket_reference, window::remove_main_bounds, window::resize_main_window,
    window::return_to_previous_window, window::set_main_bounds,
};
use config::{SettingsManager, WindowsManager};
use soundpanel::{
    clear_intercept_binding, get_intercept_settings, set_intercept_allow_any_key,
    set_intercept_binding, set_intercept_enabled, sp_add_binding, sp_add_set, sp_escape_soundpanel,
    sp_get_active_set, sp_get_bindings, sp_get_floating_appearance, sp_get_sets,
    sp_get_stay_visible, sp_is_floating_clickthrough_enabled, sp_is_supported_format,
    sp_play_binding, sp_remove_binding, sp_remove_set, sp_rename_set, sp_set_active_set,
    sp_set_config_mode, sp_set_floating_bg_color, sp_set_floating_clickthrough,
    sp_set_floating_opacity, sp_set_stay_visible, sp_test_sound, sp_update_binding,
};
use state::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tracing::{error, info, warn, Level};
use tracing_appender::non_blocking;
use tracing_subscriber::{fmt, prelude::*, EnvFilter, Registry};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Determines whether the SoundPanel should hide on focus loss.
///
/// Hides only when `hide_on_blur` is enabled AND both `stay_visible` and
/// `config_mode` are disabled.
fn should_hide_soundpanel_on_blur(
    hide_on_blur: bool,
    stay_visible: bool,
    config_mode: bool,
) -> bool {
    hide_on_blur && !stay_visible && !config_mode
}

/// Выполнить одноразовый перенос legacy-кеша и сохранить маркер (ROADMAP-117).
///
/// Маркер `true` полностью отключает проверку legacy-каталога. Маркер ставится
/// только после подтверждения полного переноса; провал сохранения логируется и
/// оставляет возможность повторной попытки на следующем старте.
fn migrate_legacy_audio_cache_once(
    marker: bool,
    migrate: impl FnOnce() -> bool,
    persist_marker: impl FnOnce() -> anyhow::Result<()>,
) {
    if marker {
        return;
    }
    if !migrate() {
        return;
    }
    if let Err(e) = persist_marker() {
        warn!(
            error = %e,
            "Failed to persist legacy audio cache migration marker; will retry on next start"
        );
    }
}

/// Выполнить одноразовый перенос legacy-моделей и сохранить маркер.
///
/// Маркер `true` полностью отключает проверку legacy-каталога. Маркер ставится
/// только после подтверждения полного переноса (либо подтверждённого отсутствия
/// источника); провал сохранения логируется и оставляет попытку на следующем
/// старте. Независим от маркера audio-кеша.
fn migrate_legacy_models_once(
    marker: bool,
    migrate: impl FnOnce() -> bool,
    persist_marker: impl FnOnce() -> anyhow::Result<()>,
) {
    if marker {
        return;
    }
    if !migrate() {
        return;
    }
    if let Err(e) = persist_marker() {
        warn!(
            error = %e,
            "Failed to persist legacy models migration marker; will retry on next start"
        );
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(windows)]
    crate::single_instance::acquire_lock_or_exit();

    // Инициализируем менеджеры ДО setup. Ошибка загрузки settings.json больше
    // не сбрасывает конфиг молча: обычный запуск не продолжается, вместо него
    // открывается минимальный диалог восстановления (ROADMAP-123).
    let settings_manager = match SettingsManager::new() {
        Ok(manager) => manager,
        Err(error) => crate::settings_recovery::run_recovery_app(error),
    };

    let windows_manager = WindowsManager::new().expect("Failed to create windows manager");

    // Load settings to configure logger
    // These settings will be passed to init_app to avoid race condition
    let settings = settings_manager.load().expect("Failed to load settings");

    // Validate module levels (log a warning if invalid)
    if let Err(e) = commands::logging::validate_module_levels(&settings.logging.module_levels) {
        warn!(error = %e, "Invalid module_levels in settings.json. Module logging disabled.");
    }

    // Initialize tracing subscriber
    let log_dir = crate::paths::config_root()
        .map(|dir| dir.join("logs"))
        .unwrap_or_else(|_| PathBuf::from(".").join("logs"));

    // Ensure log directory exists with graceful fallback
    if let Err(e) = std::fs::create_dir_all(&log_dir)
        .with_context(|| format!("Failed to create log directory at {:?}", log_dir))
    {
        eprintln!(
            "Failed to create log directory: {}. Logging to stdout only.",
            e
        );
        // Continue with stdout-only logging
    }

    // Build env filter with per-module directives
    // In dev mode, default to TRACE level; in release mode, respect settings
    let default_level = if cfg!(debug_assertions) {
        Level::TRACE // Show all logs in dev mode
    } else if settings.logging.enabled {
        match settings.logging.level.as_str() {
            "error" => Level::ERROR,
            "warn" => Level::WARN,
            "info" => Level::INFO,
            "debug" => Level::DEBUG,
            "trace" => Level::TRACE,
            _ => Level::INFO,
        }
    } else {
        Level::ERROR
    };

    let mut env_filter = EnvFilter::builder()
        .with_default_directive(default_level.into())
        .from_env_lossy();

    // Apply per-module filters from settings.json
    for (module, level_str) in &settings.logging.module_levels {
        let module_level = match level_str.as_str() {
            "error" => Level::ERROR,
            "warn" => Level::WARN,
            "info" => Level::INFO,
            "debug" => Level::DEBUG,
            "trace" => Level::TRACE,
            _ => Level::INFO,
        };
        let directive = format!("{}={}", module, module_level);
        match directive.parse() {
            Ok(d) => env_filter = env_filter.add_directive(d),
            Err(e) => {
                warn!(directive = %directive, error = %e, "Invalid log directive in settings, skipping")
            }
        }
    }

    // WorkerGuard must live for the entire program duration.
    // We use Box::leak to prevent it from being dropped, which would stop logging.
    // This is a small memory leak (a few bytes) that is acceptable for a desktop app.
    let _guard: &'static mut non_blocking::WorkerGuard = if cfg!(debug_assertions) {
        // Debug mode: always log to console, optionally to file
        let (non_blocking_file, guard) = if settings.logging.enabled {
            let mut log_file: Box<dyn std::io::Write + Send> = match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_dir.join("ttsbard.log"))
            {
                Ok(f) => Box::new(f),
                Err(e) => {
                    eprintln!("Failed to open log file: {}. Logging to stdout only.", e);
                    Box::new(std::io::sink())
                }
            };

            // Add session separator for readability
            let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
            writeln!(
                &mut *log_file,
                "\n====== New session: {} | Version: {} ======\n",
                timestamp,
                env!("CARGO_PKG_VERSION")
            )
            .ok();

            non_blocking(log_file)
        } else {
            non_blocking(std::io::sink())
        };

        let leaked_guard = Box::leak(Box::new(guard));

        tracing::subscriber::set_global_default(
            Registry::default()
                .with(tracing_subscriber::filter::filter_fn(|metadata| {
                    crate::secret_log::allow_log_target(metadata.target())
                }))
                .with(env_filter)
                .with(fmt::layer().with_writer(std::io::stdout).with_ansi(true))
                .with(fmt::layer().with_writer(non_blocking_file).with_ansi(false)),
        )
        .expect("Failed to set tracing subscriber");

        leaked_guard
    } else if settings.logging.enabled {
        // Release mode + enabled: file only with non-blocking writer
        let mut log_file: Box<dyn std::io::Write + Send> = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("ttsbard.log"))
        {
            Ok(f) => Box::new(f),
            Err(e) => {
                eprintln!("Failed to open log file: {}. Logging to stdout only.", e);
                Box::new(std::io::sink())
            }
        };

        // Add session separator for readability
        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        writeln!(
            &mut *log_file,
            "\n====== New session: {} | Version: {} ======\n",
            timestamp,
            env!("CARGO_PKG_VERSION")
        )
        .ok();

        let (non_blocking_file, guard) = non_blocking(log_file);
        let leaked_guard = Box::leak(Box::new(guard));

        tracing::subscriber::set_global_default(
            Registry::default()
                .with(tracing_subscriber::filter::filter_fn(|metadata| {
                    crate::secret_log::allow_log_target(metadata.target())
                }))
                .with(env_filter)
                .with(fmt::layer().with_writer(non_blocking_file).with_ansi(false)),
        )
        .expect("Failed to set tracing subscriber");

        leaked_guard
    } else {
        // Logging disabled: errors only to console (no guard needed for stdout)
        tracing::subscriber::set_global_default(
            Registry::default()
                .with(tracing_subscriber::filter::filter_fn(|metadata| {
                    crate::secret_log::allow_log_target(metadata.target())
                }))
                .with(EnvFilter::new("error"))
                .with(fmt::layer().with_writer(std::io::stdout).with_ansi(true)),
        )
        .expect("Failed to set tracing subscriber");
        // Dummy guard to satisfy the type system - won't be used
        Box::leak(Box::new(non_blocking(std::io::sink()).1))
    };

    // Стартовые одноразовые очистки: после single-instance (инвариант отсутствия
    // живых владельцев temp-файлов) и после инициализации логгера.
    #[cfg(windows)]
    crate::paths::clean_temp_root();
    #[cfg(windows)]
    crate::paths::remove_legacy_roaming_temp();

    // Корень данных: пользовательский override из настроек должен быть
    // применён до model discovery (Piper/OCR/RUAccent) и переноса legacy-кеша.
    crate::paths::init_data_root(settings.storage.data_dir.as_deref());
    // Legacy-модели (piper/ocr/ruaccent) переносятся до model discovery, но
    // после применения корня данных и независимо от маркера audio-кеша.
    migrate_legacy_models_once(
        settings.storage.legacy_models_migrated,
        crate::storage_transfer::migrate_legacy_models_from_config,
        || settings_manager.set_legacy_models_migrated(),
    );
    // OCR and other services snapshot their model root at construction.
    let mut app_state = AppState::new();
    app_state.settings_cache = settings_manager.cache_arc();

    // Аудио-кеш: кастомный путь из настроек и одноразовый перенос legacy-кеша.
    crate::history::init_audio_cache_dir(settings.storage.audio_cache_dir.as_deref());
    migrate_legacy_audio_cache_once(
        settings.storage.legacy_audio_cache_migrated,
        crate::history::migrate_legacy_audio_cache,
        || settings_manager.set_legacy_audio_cache_migrated(),
    );

    let app_dir = crate::paths::config_root().expect("Failed to resolve app data directory");

    std::fs::create_dir_all(&app_dir).expect("Failed to create appdata directory");

    let soundpanel_state = soundpanel::SoundPanelState::new(app_dir.to_string_lossy().into_owned());

    let (history_path, ngram_path, phrase_path) =
        history::history_paths().expect("Failed to resolve history paths");
    let history_manager = Arc::new(history::HistoryManager::new(
        history_path,
        ngram_path,
        phrase_path,
    ));
    *app_state.editor.history_manager.lock() = Some(history_manager.clone());
    let history_state = commands::history::HistoryState(history_manager);

    let tabs_path = tabs::tabs_path().expect("Failed to resolve tabs path");
    let tab_manager = Arc::new(tabs::TabManager::new(tabs_path));
    let tabs_state = commands::tabs::TabsState(tab_manager);

    let speech_queue =
        commands::speech_queue::SpeechQueueState::new(speech_queue::SpeechQueue::new());
    // DirectWrite enumeration runs before the Tauri backend is available, so
    // opening editor settings never waits for the Windows font catalog.
    let system_font_catalog = system_fonts::SystemFontCatalog::load();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .manage(TelegramState::new())
        .manage(settings_manager)
        .manage(windows_manager)
        .manage(soundpanel_state)
        .manage(history_state)
        .manage(tabs_state)
        .manage(speech_queue)
        .manage(system_font_catalog)
        .manage(model_downloads::ModelDownloadsState::default())
        .invoke_handler(tauri::generate_handler![
            greet,
            speak_text_raw_export,
            get_tts_provider,
            set_tts_provider,
            select_tts_provider_by_id,
            prepare_tts_provider_by_id,
            list_pocket_models,
            refresh_pocket_models,
            set_pocket_voice,
            validate_pocket_reference,
            get_local_tts_url,
            set_local_tts_url,
            get_openai_api_key,
            set_openai_api_key,
            get_openai_voice,
            set_openai_voice,
            apply_openai_proxy_settings,
            has_api_key,
            quit_app,
            get_hotkey_enabled,
            set_hotkey_enabled,
            // Global settings commands
            get_global_exclude_from_capture,
            set_global_exclude_from_capture,
            // Editor commands
            get_editor_quick,
            set_editor_quick,
            set_editor_default_route,
            set_editor_spellcheck_enabled,
            get_editor_spellcheck_enabled,
            set_editor_spellcheck_source,
            get_editor_spellcheck_source,
            set_editor_height,
            get_editor_height,
            set_editor_typing_idle_timeout_ms,
            get_editor_typing_idle_timeout_ms,
            set_editor_typing_enabled,
            set_editor_autocomplete_enabled,
            set_editor_number_conversion_enabled,
            set_editor_keep_text,
            set_editor_font_family,
            get_editor_font_family,
            get_system_font_families,
            set_editor_font_size,
            get_editor_font_size_px,
            set_ui_font_family,
            set_ui_font_size,
            get_ui_font_family,
            get_ui_font_size_px,
            list_homograph_accentor_packs,
            refresh_homograph_accentor_packs,
            set_editor_homograph_accentor,
            set_editor_homograph_accentor_load_on_start,
            load_homograph_accentor_model,
            start_homograph_accentor_startup_load,
            preview_contextual_ruaccent,
            // Local model management commands
            model_downloads::model_downloads_status,
            model_downloads::model_downloads_install,
            model_downloads::model_downloads_cancel,
            model_downloads::model_downloads_refresh_catalog,
            model_downloads::model_downloads_open_folder,
            // History commands
            commands::history::get_history_suggestions,
            commands::history::record_history,
            commands::history::clear_history,
            commands::history::get_phrase_completion,
            commands::history::get_phrase_history,
            commands::history::delete_phrase_history,
            commands::history::clear_phrase_history,
            commands::history::replay_phrase_from_cache,
            // Tab persistence commands
            commands::tabs::get_tabs,
            commands::tabs::save_tabs,
            // Theme commands
            update_theme,
            hide_main_window,
            close_soundpanel_window,
            sp_get_bindings,
            sp_add_binding,
            sp_remove_binding,
            sp_update_binding,
            sp_test_sound,
            sp_is_supported_format,
            sp_get_floating_appearance,
            sp_set_floating_opacity,
            sp_set_floating_bg_color,
            sp_set_floating_clickthrough,
            sp_is_floating_clickthrough_enabled,
            sp_get_stay_visible,
            sp_set_stay_visible,
            sp_set_config_mode,
            sp_play_binding,
            sp_escape_soundpanel,
            sp_get_sets,
            sp_get_active_set,
            sp_set_active_set,
            sp_add_set,
            sp_rename_set,
            sp_remove_set,
            get_intercept_settings,
            set_intercept_enabled,
            set_intercept_binding,
            clear_intercept_binding,
            set_intercept_allow_any_key,
            open_file_dialog,
            commands::open_app_folder,
            commands::storage::open_local_data_folder,
            commands::storage::storage_get_data_info,
            commands::storage::storage_prepare_data_transfer,
            commands::storage::storage_transfer_data,
            // Audio commands
            get_output_devices,
            get_virtual_mic_devices,
            get_audio_settings,
            set_speaker_device,
            set_speaker_enabled,
            set_speaker_volume,
            set_virtual_mic_device,
            enable_virtual_mic,
            disable_virtual_mic,
            set_virtual_mic_volume,
            test_audio_device,
            get_audio_output_format_pending,
            set_audio_output_format,
            // Audio Effects commands
            get_audio_effects,
            set_audio_effects_enabled,
            set_audio_effects_pitch,
            set_audio_effects_speed,
            set_audio_effects_volume,
            set_audio_effects_enhance_enabled,
            set_audio_effects_enhance_atten_db,
            set_audio_effects_formant_preserved,
            preview_audio_file,
            stop_preview,
            save_audio_effects,
            get_dsp_settings,
            save_dsp_settings,
            // Preprocessor commands
            commands::preprocessor::get_replacements,
            commands::preprocessor::save_replacements,
            commands::preprocessor::get_usernames,
            commands::preprocessor::save_usernames,
            commands::preprocessor::preview_preprocessing,
            commands::preprocessor::load_preprocessor_data,
            // Telegram commands
            telegram_init,
            telegram_request_code,
            telegram_sign_in,
            telegram_check_password,
            telegram_disconnect,
            telegram_sign_out,
            telegram_get_status,
            telegram_get_user,
            telegram_auto_restore,
            commands::telegram::speak_text_silero,
            commands::telegram::telegram_get_current_voice,
            commands::telegram::telegram_get_limits,
            commands::telegram::telegram_set_speaker,
            commands::telegram::telegram_add_voice_code,
            commands::telegram::telegram_remove_voice_code,
            commands::telegram::telegram_select_voice,
            commands::telegram::set_silero_synthesis_timeout,
            // WebView commands
            commands::webview::get_webview_settings,
            commands::webview::get_webview_server_status,
            commands::webview::save_webview_settings,
            commands::webview::restart_webview_server,
            commands::webview::get_local_ip,
            commands::webview::get_webview_enabled,
            commands::webview::get_webview_start_on_boot,
            commands::webview::get_webview_port,
            commands::webview::open_template_folder,
            commands::webview::send_test_message,
            commands::webview::reload_templates,
            // WebView security commands
            commands::webview::generate_webview_token,
            commands::webview::get_webview_token,
            commands::webview::copy_webview_token,
            commands::webview::regenerate_webview_token,
            commands::webview::set_webview_upnp_enabled,
            commands::webview::get_webview_upnp_enabled,
            commands::webview::get_webview_upnp_status,
            commands::webview::set_webview_typing,
            commands::webview::get_external_ip,
            // Twitch commands
            commands::twitch::get_twitch_settings,
            commands::twitch::save_twitch_settings,
            commands::twitch::deliver_twitch_message,
            commands::twitch::connect_twitch,
            commands::twitch::disconnect_twitch,
            commands::twitch::restart_twitch,
            commands::twitch::get_twitch_status,
            // Twitch API authorization commands
            commands::twitch::get_twitch_api_auth_status,
            commands::twitch::save_twitch_api_client,
            commands::twitch::begin_twitch_api_auth,
            commands::twitch::begin_twitch_api_channel_device_auth,
            commands::twitch::finish_twitch_api_auth,
            commands::twitch::cancel_twitch_api_auth,
            commands::twitch::clear_twitch_api_auth,
            commands::twitch::select_twitch_api_channel,
            commands::twitch::forget_twitch_api_channel,
            // VTube Studio commands
            commands::vtube_studio::get_vtube_studio_settings,
            commands::vtube_studio::save_vtube_studio_settings,
            commands::vtube_studio::save_vtube_studio_typing_action,
            commands::vtube_studio::get_vtube_studio_scene_items,
            commands::vtube_studio::get_vtube_studio_item_status,
            commands::vtube_studio::refresh_vtube_studio_item,
            commands::vtube_studio::get_vtube_studio_current_model_hotkeys,
            commands::vtube_studio::test_vtube_studio_connection,
            commands::vtube_studio::test_vtube_studio_typing,
            commands::vtube_studio::set_vtube_studio_typing,
            commands::vtube_studio::connect_vtube_studio,
            commands::vtube_studio::disconnect_vtube_studio,
            commands::vtube_studio::restart_vtube_studio,
            commands::vtube_studio::get_vtube_studio_status,
            commands::vtube_studio::get_vtube_studio_authenticated,
            commands::vtube_studio::get_vtube_studio_desired_running,
            // VRChat commands
            commands::vrchat::get_vrchat_settings,
            commands::vrchat::save_vrchat_settings,
            commands::vrchat::set_vrchat_typing,
            commands::vrchat::send_vrchat_text,
            // Input server commands
            commands::input_server::get_input_server_settings,
            commands::input_server::save_input_server_settings,
            commands::input_server::get_incoming_settings,
            commands::input_server::save_incoming_settings,
            commands::input_server::get_input_server_status,
            commands::input_server::start_input_server,
            commands::input_server::stop_input_server,
            commands::input_server::get_input_server_token,
            commands::input_server::regenerate_input_server_token,
            commands::input_server::get_input_server_connection_url,
            commands::input_server::submit_input_server_test,
            commands::input_server::list_incoming_texts,
            commands::input_server::approve_incoming_text,
            commands::input_server::take_incoming_text_for_edit,
            commands::input_server::discard_incoming_text,
            // OCR commands
            commands::ocr::get_ocr_settings,
            commands::ocr::save_ocr_settings,
            commands::ocr::get_ocr_status,
            commands::ocr::list_ocr_packs,
            commands::ocr::list_ocr_monitors,
            commands::ocr::refresh_ocr_packs,
            commands::ocr::open_ocr_packs_folder,
            // OCR overlay lifecycle: preview (load the frozen frame), ready
            // (reveal it), failed (abandon it), then cancel/submit the session.
            commands::ocr::ocr_selection_preview,
            commands::ocr::ocr_selection_ready,
            commands::ocr::ocr_selection_failed,
            commands::ocr::ocr_selection_cancel,
            commands::ocr::ocr_selection_submit,
            // Logging commands
            get_logging_settings,
            save_logging_settings,
            // Localization commands
            commands::localization::get_localization,
            commands::localization::set_ui_language,
            // Proxy commands
            commands::proxy::test_proxy,
            commands::proxy::get_proxy_settings,
            commands::proxy::set_proxy_url,
            commands::proxy::set_openai_use_proxy,
            commands::proxy::set_telegram_proxy_mode,
            commands::proxy::get_telegram_proxy_status,
            commands::proxy::reconnect_telegram,
            // MTProxy commands
            commands::proxy::get_mtproxy_settings,
            commands::proxy::set_mtproxy_settings,
            commands::proxy::test_mtproxy,
            // AI commands
            set_ai_provider,
            set_ai_prompt,
            set_ai_openai_api_key,
            set_ai_openai_use_proxy,
            set_ai_zai_url,
            set_ai_zai_api_key,
            correct_text,
            set_editor_ai,
            get_editor_ai,
            set_editor_ai_completion,
            get_editor_ai_completion,
            get_ai_completion,
            set_ai_openai_model,
            get_ai_openai_model,
            set_ai_zai_model,
            get_ai_zai_model,
            set_ai_deepseek_api_key,
            set_ai_deepseek_model,
            set_ai_deepseek_use_proxy,
            get_ai_deepseek_model,
            set_ai_custom_url,
            set_ai_custom_api_key,
            set_ai_custom_model,
            set_ai_custom_use_proxy,
            get_ai_custom_model,
            ai_check_grammar,
            set_visible_tts_provider_ids,
            // Fish Audio commands
            commands::get_fish_audio_api_key,
            commands::set_fish_audio_api_key,
            commands::save_fish_audio_connection_settings,
            commands::get_fish_audio_reference_id,
            commands::set_fish_audio_reference_id,
            commands::get_fish_audio_voices,
            commands::add_fish_audio_voice,
            commands::remove_fish_audio_voice,
            commands::fetch_fish_audio_models,
            commands::fetch_fish_audio_image,
            commands::set_fish_audio_format,
            commands::set_fish_audio_temperature,
            commands::set_fish_audio_sample_rate,
            commands::set_fish_audio_use_proxy,
            commands::apply_fish_audio_proxy_settings,
            // ElevenLabs commands
            commands::get_elevenlabs_api_key,
            commands::save_elevenlabs_api_key,
            commands::save_elevenlabs_generation_settings,
            commands::get_elevenlabs_voice_id,
            commands::set_elevenlabs_voice_id,
            commands::get_elevenlabs_voices,
            commands::get_elevenlabs_models,
            commands::refresh_elevenlabs_voices,
            commands::refresh_elevenlabs_models,
            commands::set_elevenlabs_use_proxy,
            commands::apply_elevenlabs_proxy_settings,
            commands::get_elevenlabs_settings,
            // Unified settings commands
            commands::get_all_app_settings,
            commands::is_backend_ready,
            commands::confirm_backend_ready,
            // Window commands
            resize_main_window,
            set_main_bounds,
            remove_main_bounds,
            // Hotkey commands
            get_hotkey_settings,
            set_hotkey,
            set_hotkey_binding_enabled,
            reset_hotkey_to_default,
            unregister_hotkeys,
            reregister_hotkeys_cmd,
            set_hotkey_recording,
            // Editor hotkey commands
            set_editor_hotkey,
            set_editor_hotkey_enabled,
            reset_editor_hotkey,
            get_editor_hotkeys,
            toggle_playback_control_window,
            close_playback_control_window,
            toggle_soundpanel_window,
            get_visibility_snapshot,
            set_show_playback_on_start,
            get_show_playback_on_start,
            set_start_compact,
            set_hide_on_minimize,
            set_hide_extra_window_buttons,
            // Window appearance commands
            get_main_appearance,
            set_main_custom_background,
            set_main_opacity,
            set_main_bg_color,
            set_main_custom_opacity,
            set_main_opacity_compact_only,
            set_main_compact_dims,
            get_main_compact_dims,
            set_main_compact_view,
            get_main_compact_view,
            return_to_previous_window,
            get_soundpanel_appearance_source,
            set_soundpanel_appearance_source,
            get_playback_appearance_source,
            set_playback_appearance_source,
            // Playback window appearance commands
            pc_get_appearance,
            pc_set_opacity,
            pc_set_bg_color,
            // Playback commands
            commands::playback::playback_pause,
            commands::playback::playback_resume,
            commands::playback::playback_stop,
            commands::playback::playback_repeat,
            commands::playback::replay_phrase,
            commands::playback::cancel_queued_replay,
            commands::playback::get_playback_state,
            commands::playback::get_playback_activity,
            // Spellcheck command
            commands::spellcheck::spellcheck,
            // Speech queue commands
            commands::speech_queue::submit_speech,
            commands::speech_queue::get_speech_queue_state,
            commands::speech_queue::retry_speech_job,
            commands::speech_queue::cancel_speech_job,
            commands::speech_queue::skip_speech_job,
            commands::speech_queue::restore_cancelled_speech_job,
        ])
        .setup({
            let settings_clone = settings.clone();
            move |app| setup::init_app(app, settings_clone.clone())
        })
        .on_window_event(|window, event| {
            #[cfg(windows)]
            if window.label() == "main" {
                match event {
                    tauri::WindowEvent::Focused(focused) => {
                        if *focused {
                            // Initialize hotkeys when window gains focus (ensuring it's fully created)
                            static HOTKEY_INIT: std::sync::Once = std::sync::Once::new();

                            HOTKEY_INIT.call_once(|| {
                                let app_handle = window.app_handle().clone();
                                let app_state = app_handle.state::<AppState>();
                                let app_state_inner = app_state.inner().clone();

                                info!("Window focused - initializing hotkeys...");

                                // Initialize hotkeys using tauri-plugin-global-shortcut
                                match hotkeys::initialize_hotkeys(
                                    0,
                                    app_state_inner,
                                    app_handle.clone(),
                                ) {
                                    Ok(_) => {
                                        info!("Hotkeys initialized successfully");
                                    }
                                    Err(e) => {
                                        error!(error = %e, "Failed to initialize hotkeys");
                                    }
                                }
                            });
                        } else {
                            // Window lost focus - remove always-on-top
                            info!("Main window lost focus - removing always-on-top");
                            let _ = window.set_always_on_top(false);
                        }
                    }
                    tauri::WindowEvent::Resized(_) => {
                        // tao 0.34.6 forwards the WM_SIZE lparam, but tauri-runtime-wry
                        // 2.11.4 WindowEventWrapper::parse recomputes the size via
                        // inner_size(), so a 0x0 width/height is not a reliable
                        // minimized signal at this layer. Query the window state
                        // directly instead of assuming any particular dimensions.
                        match window.is_minimized() {
                            Ok(true) => {
                                let hide_on_minimize = window
                                    .app_handle()
                                    .state::<SettingsManager>()
                                    .get_hide_on_minimize();
                                if hide_on_minimize {
                                    info!("Main window minimized - hiding to tray (hide_on_minimize enabled)");
                                    let _ = window.hide();
                                }
                                // When hide_on_minimize is disabled, leave the
                                // normal taskbar minimization untouched.
                            }
                            Ok(false) => {
                                // Not minimized: ordinary resize, no action.
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to query main window minimized state");
                            }
                        }
                    }
                    tauri::WindowEvent::Destroyed => {
                        // Сохраняем позицию главного окна
                        if let Some(windows_manager) =
                            window.app_handle().try_state::<WindowsManager>()
                        {
                            if let Ok(pos) = window.outer_position() {
                                let x: i32 = pos.x;
                                let y: i32 = pos.y;
                                info!(x, y, "Main window destroyed - saving position");
                                let _ = windows_manager.set_main_position(Some(x), Some(y));
                            }
                        }

                        // Вспомогательные окна живут скрытыми всю сессию, поэтому закрытие
                        // главного окна само по себе не завершает процесс — координируем
                        // завершение явно (идемпотентно через begin_shutdown).
                        let app_state = window.app_handle().state::<AppState>();
                        if !app_state
                            .shutdown_started
                            .load(std::sync::atomic::Ordering::SeqCst)
                        {
                            info!("Main window destroyed - starting coordinated shutdown");
                            tauri::async_runtime::spawn(crate::commands::coordinate_shutdown(
                                window.app_handle().clone(),
                            ));
                        }
                    }
                    _ => {}
                }
            }

            #[cfg(windows)]
            if window.label() == "soundpanel" {
                if let tauri::WindowEvent::Focused(focused) = event {
                    let soundpanel_state =
                        window.app_handle().state::<soundpanel::SoundPanelState>();
                    soundpanel_state.set_window_focused(*focused);
                    if *focused {
                        // Возврат фокуса отменяет отложенное автокрытие.
                        let app_state = window.app_handle().state::<AppState>();
                        soundpanel_window::cancel_soundpanel_blur_hide(&app_state);
                        info!("Soundpanel gained focus - cancelled pending blur-hide");
                    } else {
                        let app_handle = window.app_handle();
                        let win_mgr = app_handle.state::<WindowsManager>();
                        let hide_on_blur = win_mgr.get_soundpanel_hide_on_blur();
                        let stay_visible = soundpanel_state.get_stay_visible();
                        let config_mode = soundpanel_state.get_config_mode();
                        if should_hide_soundpanel_on_blur(hide_on_blur, stay_visible, config_mode) {
                            info!(
                                "Soundpanel lost focus - scheduling deferred hide via hide_on_blur"
                            );
                            let app_state = app_handle.state::<AppState>();
                            soundpanel_window::schedule_soundpanel_blur_hide(
                                app_handle, &app_state,
                            );
                        } else if stay_visible {
                            info!("Soundpanel lost focus - stay_visible bypass");
                        } else {
                            info!("Soundpanel lost focus - hide_on_blur disabled");
                        }
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                crate::playback_window::save_playback_position(app_handle);
                crate::soundpanel_window::save_soundpanel_position(app_handle);
                if app_handle
                    .state::<AppState>()
                    .shutdown_started
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    return;
                }
                api.prevent_exit();
                tauri::async_runtime::spawn(crate::commands::coordinate_shutdown(
                    app_handle.clone(),
                ));
            }
        });
}

#[cfg(test)]
mod should_hide_tests {
    use super::*;

    #[test]
    fn hide_on_blur_ordinary() {
        assert!(should_hide_soundpanel_on_blur(true, false, false));
    }

    #[test]
    fn no_hide_when_hide_on_blur_disabled() {
        assert!(!should_hide_soundpanel_on_blur(false, false, false));
    }

    #[test]
    fn stay_visible_bypasses_hide_on_blur() {
        assert!(!should_hide_soundpanel_on_blur(true, true, false));
    }

    #[test]
    fn stay_visible_without_hide_on_blur_still_no_hide() {
        assert!(!should_hide_soundpanel_on_blur(false, true, false));
    }

    #[test]
    fn config_mode_suppresses_hide_on_blur() {
        assert!(!should_hide_soundpanel_on_blur(true, false, true));
    }
}

#[cfg(test)]
mod migrate_once_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A completed marker skips the migration entirely: even a recreated legacy
    /// directory must not be probed.
    #[test]
    fn migrate_skips_when_marker_true() {
        let ran = Arc::new(AtomicBool::new(false));
        let ran_flag = Arc::clone(&ran);
        migrate_legacy_audio_cache_once(
            true,
            || {
                ran_flag.store(true, Ordering::SeqCst);
                true
            },
            || Ok(()),
        );
        assert!(!ran.load(Ordering::SeqCst));
    }

    /// The marker is persisted only after a completed migration.
    #[test]
    fn migrate_persists_marker_only_after_complete() {
        let persisted = Arc::new(AtomicBool::new(false));
        let persisted_flag = Arc::clone(&persisted);
        migrate_legacy_audio_cache_once(
            false,
            || true,
            || {
                persisted_flag.store(true, Ordering::SeqCst);
                Ok(())
            },
        );
        assert!(persisted.load(Ordering::SeqCst));
    }

    /// An incomplete migration must not persist the marker.
    #[test]
    fn migrate_incomplete_does_not_persist_marker() {
        let persisted = Arc::new(AtomicBool::new(false));
        let persisted_flag = Arc::clone(&persisted);
        migrate_legacy_audio_cache_once(
            false,
            || false,
            || {
                persisted_flag.store(true, Ordering::SeqCst);
                Ok(())
            },
        );
        assert!(!persisted.load(Ordering::SeqCst));
    }

    /// A completed models marker skips the migration entirely: even a recreated
    /// legacy directory must not be probed.
    #[test]
    fn migrate_legacy_models_skips_when_marker_true() {
        let ran = Arc::new(AtomicBool::new(false));
        let ran_flag = Arc::clone(&ran);
        migrate_legacy_models_once(
            true,
            || {
                ran_flag.store(true, Ordering::SeqCst);
                true
            },
            || Ok(()),
        );
        assert!(!ran.load(Ordering::SeqCst));
    }

    /// The models marker is persisted only after a completed migration.
    #[test]
    fn migrate_legacy_models_persists_marker_only_after_complete() {
        let persisted = Arc::new(AtomicBool::new(false));
        let persisted_flag = Arc::clone(&persisted);
        migrate_legacy_models_once(
            false,
            || true,
            || {
                persisted_flag.store(true, Ordering::SeqCst);
                Ok(())
            },
        );
        assert!(persisted.load(Ordering::SeqCst));
    }

    /// An incomplete models migration must not persist the marker.
    #[test]
    fn migrate_legacy_models_incomplete_does_not_persist_marker() {
        let persisted = Arc::new(AtomicBool::new(false));
        let persisted_flag = Arc::clone(&persisted);
        migrate_legacy_models_once(
            false,
            || false,
            || {
                persisted_flag.store(true, Ordering::SeqCst);
                Ok(())
            },
        );
        assert!(!persisted.load(Ordering::SeqCst));
    }

    /// A marker-persist failure is tolerated: migration runs, the error does not
    /// propagate, and the next startup retries the save.
    #[test]
    fn migrate_legacy_models_persist_failure_is_tolerated() {
        let migrated = Arc::new(AtomicBool::new(false));
        let migrated_flag = Arc::clone(&migrated);
        migrate_legacy_models_once(
            false,
            || {
                migrated_flag.store(true, Ordering::SeqCst);
                true
            },
            || anyhow::bail!("simulated save failure"),
        );
        assert!(migrated.load(Ordering::SeqCst));
    }
}
