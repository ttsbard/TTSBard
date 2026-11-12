use crate::config::{
    normalize_typing_idle_timeout_ms, AppSettingsDto, EditorRoute, QuickEditorMode,
    SettingsManager, SpellSource, TtsProviderInfoDto, WindowsManager,
};
use crate::ipc::CommandError;
use crate::state::AppState;
use crate::stress::packs::RuAccentPackDescriptor;
use crate::stress::runtime::RuAccentRuntimeSlot;
use crate::system_fonts::SystemFontCatalog;
use crate::tts::pocket::{
    with_voice_store, PocketError, PocketModelDto, PocketTts, PocketVoiceInput,
    PocketVoiceSelection,
};
use crate::tts::TtsProvider;
use tauri::{AppHandle, Emitter, EventTarget, Listener, Manager, State};
use tracing::{error, info, warn};

pub mod ai;
pub mod history;
pub mod input_server;
pub mod localization;
pub mod logging;
pub mod ocr;
pub mod playback;
pub mod playback_window;
pub mod preprocessor;
pub mod proxy;
pub mod settings_recovery;
pub mod speech_queue;
pub mod spellcheck;
pub mod storage;
pub mod tabs;
pub mod telegram;
pub mod tts_pipeline;
pub mod twitch;
pub mod vrchat;
pub mod vtube_studio;
pub mod webview;
pub mod window;

pub use self::ai::*;
pub use self::playback::*;
pub use self::window::*;

pub const SETTINGS_CHANGED_EVENT: &str = "settings-changed";

pub fn emit_settings_changed(app_handle: &AppHandle) {
    let _ = app_handle.emit(SETTINGS_CHANGED_EVENT, ());
}

/// Run a sync manager operation on a blocking thread pool.
///
/// The manager is cloned (cheap — `Arc` + `PathBuf`) so the closure
/// owns its own handle and does not borrow `State<'_>`.
pub async fn persist_blocking<M, F, R>(manager: &M, op: F) -> Result<R, String>
where
    M: Clone + Send + 'static,
    F: FnOnce(&M) -> anyhow::Result<R> + Send + 'static,
    R: Send + 'static,
{
    let mgr = manager.clone();
    tokio::task::spawn_blocking(move || op(&mgr))
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))?
        .map_err(|e| e.to_string())
}

/// Emitted to the main webview to request a final editor-tab flush before the
/// backend tears down its services on shutdown.
const TABS_FLUSH_REQUEST_EVENT: &str = "tabs-flush-request";
/// Emitted by the frontend to acknowledge the flush request. The payload echoes
/// the request id so stale or unrelated acknowledgements are ignored.
const TABS_FLUSH_ACK_EVENT: &str = "tabs-flush-ack";
/// Upper bound for waiting on the frontend acknowledgement. Shutdown never
/// blocks past this even when the WebView is missing or broken.
const TABS_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Payload for the flush request sent to the main webview.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TabsFlushRequestPayload {
    request_id: String,
}

/// Payload for the flush acknowledgement emitted by the frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TabsFlushAckPayload {
    request_id: String,
    ok: bool,
}

/// Match a raw acknowledgement payload against the expected request id.
///
/// Returns `Some(ok)` only for a well-formed acknowledgement that echoes the
/// expected id; malformed payloads and stale/unrelated acknowledgements yield
/// `None` so they are ignored.
fn match_tabs_flush_ack(payload: &str, request_id: &str) -> Option<bool> {
    let ack: TabsFlushAckPayload = serde_json::from_str(payload).ok()?;
    (ack.request_id == request_id).then_some(ack.ok)
}

/// The outcome of awaiting a frontend flush acknowledgement.
enum TabFlushAck {
    /// Frontend acknowledged a successful flush.
    Success,
    /// Frontend acknowledged, but reported a save failure.
    Failure,
    /// No acknowledgement within the bounded window.
    Timeout,
    /// The acknowledgement channel closed without a value.
    Closed,
}

/// Await an acknowledgement on the given channel, bounded by `timeout`.
async fn await_tabs_flush_ack(
    rx: tokio::sync::oneshot::Receiver<bool>,
    timeout: std::time::Duration,
) -> TabFlushAck {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(true)) => TabFlushAck::Success,
        Ok(Ok(false)) => TabFlushAck::Failure,
        Ok(Err(_)) => TabFlushAck::Closed,
        Err(_) => TabFlushAck::Timeout,
    }
}

/// Ask the main webview to flush its editor tabs and wait for its
/// acknowledgement before tearing services down.
///
/// Best-effort: without a main window the flush is skipped, and a missing or
/// broken frontend is bounded by [`TABS_FLUSH_TIMEOUT`]. The acknowledgement
/// listener is registered before the request is emitted and removed afterwards
/// so it never outlives this single shutdown request.
async fn request_tab_flush(app_handle: &AppHandle) {
    let Some(main_window) = app_handle.get_webview_window("main") else {
        info!("request_tab_flush: no main window, skipping flush");
        return;
    };

    let request_id = uuid::Uuid::new_v4().to_string();

    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    let expected = request_id.clone();
    // Scope the acknowledgement listener to the main webview window so unrelated
    // webviews can never satisfy this shutdown request.
    let listener_id = main_window.listen(TABS_FLUSH_ACK_EVENT, move |event| {
        if let Some(ok) = match_tabs_flush_ack(event.payload(), &expected) {
            let mut guard = match tx.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(tx) = guard.take() {
                let _ = tx.send(ok);
            }
        }
    });

    // `WebviewWindow::emit` broadcasts globally; target the main window explicitly
    // so only its frontend subscription receives the flush request.
    if let Err(error) = app_handle.emit_to(
        EventTarget::webview_window("main"),
        TABS_FLUSH_REQUEST_EVENT,
        TabsFlushRequestPayload { request_id },
    ) {
        warn!("request_tab_flush: failed to emit flush request: {error}");
        main_window.unlisten(listener_id);
        return;
    }

    let outcome = await_tabs_flush_ack(rx, TABS_FLUSH_TIMEOUT).await;
    main_window.unlisten(listener_id);

    match outcome {
        TabFlushAck::Success => info!("request_tab_flush: frontend acknowledged successful flush"),
        TabFlushAck::Failure => warn!("request_tab_flush: frontend reported a flush failure"),
        TabFlushAck::Timeout => {
            warn!("request_tab_flush: timeout waiting for flush acknowledgement")
        }
        TabFlushAck::Closed => {
            warn!("request_tab_flush: acknowledgement channel closed unexpectedly")
        }
    }
}

/// Quit the application
#[tauri::command]
pub async fn quit_app(app_handle: AppHandle) -> Result<(), String> {
    info!("Quit requested - initiating graceful shutdown");
    coordinate_shutdown(app_handle).await;
    Ok(())
}

/// Coordinate a single graceful shutdown across all quit entry points.
///
/// Persists the main window position, stops the keyboard hook, cancels the
/// shutdown token, notifies the WebView and emits `app-exit` before exiting.
/// Only the first call wins; subsequent calls are ignored via `begin_shutdown`.
pub async fn coordinate_shutdown(app_handle: AppHandle) {
    let Some(state) = app_handle.try_state::<AppState>() else {
        error!("coordinate_shutdown: AppState not available, exiting immediately");
        app_handle.exit(0);
        return;
    };

    if !state.begin_shutdown() {
        info!("Shutdown already in progress - ignoring duplicate request");
        return;
    }

    if let Some(windows_manager) = app_handle.try_state::<WindowsManager>() {
        if let Some(main_window) = app_handle.get_webview_window("main") {
            if let Ok(pos) = main_window.outer_position() {
                let x = pos.x;
                let y = pos.y;
                info!(x, y, "Saving main window position");
                let wm = windows_manager.inner();
                let _ =
                    persist_blocking(wm, move |mgr| mgr.set_main_position(Some(x), Some(y))).await;
            }
        }
    }

    {
        let mut hook_guard = state.soundpanel_hook.lock();
        if let Some(ref mut hook_manager) = *hook_guard {
            hook_manager.stop();
        }
        *hook_guard = None;
    }

    // Flush editor tabs in the main webview before cancelling services so the
    // latest tab state is persisted for the quit_app/ExitRequested path. The
    // window close path already awaits its own flush via onCloseRequested.
    request_tab_flush(&app_handle).await;

    state.shutdown.cancel();
    info!("Shutdown token cancelled — all servers notified");

    // Stop the OCR runtime without waiting for an in-flight recognition: the
    // shutdown stop defers the real stop to a background task when the
    // transition lock is held, so app exit is never delayed by OCR.
    crate::commands::ocr::stop_ocr_runtime_for_shutdown(&app_handle, state.inner()).await;

    state.webview.send_event(crate::events::AppEvent::Quit);

    let _ = app_handle.emit("app-exit", ());

    // Short grace period so background tasks (WebView/Twitch servers, playback,
    // history flushes) observe the cancellation and release resources before
    // the process exits. 300ms is enough for in-flight cancellation without
    // making shutdown feel sluggish (it was 600ms previously).
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    app_handle.exit(0);
}

/// Launch the OS file manager for the given directory path.
fn open_in_file_manager(path: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args([path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args([path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .args([path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Open the application data/settings folder (%APPDATA%/ttsbard) in the OS file manager.
///
/// The path is fixed to the same directory `SettingsManager::new` uses
/// (`dirs::config_dir()/ttsbard`). It never falls back to the current working
/// directory, and no caller-controlled path is accepted.
#[tauri::command]
pub async fn open_app_folder() -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let app_dir = crate::paths::config_root()
            .map_err(|_| "Не удалось определить каталог конфигурации приложения".to_string())?;

        std::fs::create_dir_all(&app_dir)
            .map_err(|e| format!("Не удалось создать папку приложения: {}", e))?;

        let app_dir = app_dir
            .canonicalize()
            .map_err(|e| format!("Некорректный путь к папке приложения: {}", e))?;

        let path = app_dir
            .to_str()
            .ok_or_else(|| "Некорректный путь к папке приложения".to_string())?;

        open_in_file_manager(path)
    })
    .await
    .map_err(|e| format!("Операция открытия папки была прервана: {}", e))?
}

/// Synthesize text and export raw audio bytes to a file (no effects, no playback)
#[tauri::command]
pub async fn speak_text_raw_export(
    state: State<'_, AppState>,
    text: String,
    path: String,
) -> Result<(), String> {
    tts_pipeline::synthesize_and_export(&state, &text, &path).await
}

/// Get all application settings in a single call
#[tauri::command]
pub async fn get_all_app_settings(
    app_state: State<'_, AppState>,
    windows_manager: State<'_, WindowsManager>,
    settings_manager: State<'_, SettingsManager>,
    soundpanel_state: State<'_, crate::soundpanel::SoundPanelState>,
    consume_startup_notifications: Option<bool>,
) -> Result<AppSettingsDto, String> {
    info!("get_all_app_settings: Loading all settings");

    let config = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?;

    let webview_settings = {
        let s = app_state.webview.settings.read().await;
        s.clone()
    };

    let twitch_settings = {
        let s = app_state.twitch.settings.read().await;
        s.clone()
    };

    let windows_settings = windows_manager
        .load()
        .map_err(|e| format!("Failed to load windows settings: {}", e))?;

    let preprocessor = app_state.editor.get_preprocessor();

    let soundpanel_bindings = soundpanel_state.get_all_bindings();
    info!(
        count = soundpanel_bindings.len(),
        "get_all_app_settings: Loaded soundpanel bindings"
    );

    let mut settings = AppSettingsDto::from_all_sources(crate::config::AllSourcesParams {
        config: &config,
        webview_settings: &webview_settings,
        twitch_settings: &twitch_settings,
        windows_settings: &windows_settings,
        preprocessor: preprocessor.as_ref(),
        soundpanel_bindings,
    });
    // Saved enabled records the last manual state. Runtime follows start_on_boot
    // after launch, so return the actual output state to settings consumers.
    settings.vrchat.enabled = app_state.vrchat.is_enabled();
    // Startup notifications are one-shot: only the owning settings context may
    // drain them. A non-consuming read (omitted argument or `false`) must leave
    // both queues intact and return the empty transient fields already set by
    // `from_all_sources`, so a background Telegram read cannot steal them.
    if consume_startup_notifications == Some(true) {
        settings.notifications = app_state.take_notifications();
        settings.startup_errors = app_state.take_startup_errors();
    }

    // The generic `ElevenLabsSettings -> ElevenLabsSettingsDto` conversion inside
    // `from_all_sources` intentionally leaves `voices`/`models` empty: the cached
    // catalogs live separately in `elevenlabs-catalog.json`. Take the canonical
    // combined view (persisted settings plus cache) instead, or every UI reload
    // would erase the refreshed catalogs.
    settings.tts.elevenlabs = settings_manager.get_elevenlabs_settings_dto();

    // Populate runtime TTS provider info from the registry
    {
        let registry = app_state.tts_registry.lock();
        let active_id = registry.active_id().map(|s| s.to_string());
        settings.tts.providers = registry
            .iter()
            .map(|entry| {
                let (kind, runtime_status) = match &entry.provider {
                    TtsProvider::OpenAi(_) => ("openai", None),
                    TtsProvider::Silero(_) => ("silero", None),
                    TtsProvider::Local(_) => ("local-http", None),
                    TtsProvider::Fish(_) => ("fish", None),
                    TtsProvider::ElevenLabs(_) => ("elevenlabs", None),
                    TtsProvider::Piper(tts) => (
                        "piper",
                        Some(if tts.is_loaded() {
                            "ready"
                        } else {
                            "discovered"
                        }),
                    ),
                    TtsProvider::Pocket(tts) => ("pocket", Some(tts.status_string())),
                };
                TtsProviderInfoDto {
                    id: entry.id.clone(),
                    display_name: entry.display_name.clone(),
                    kind: kind.to_string(),
                    active: Some(&entry.id) == active_id.as_ref(),
                    runtime_status: runtime_status.map(str::to_string),
                }
            })
            .collect();
    }

    info!(
        tts_provider = ?settings.tts.provider,
        webview_enabled = settings.webview.enabled,
        hotkey_enabled = settings.general.hotkey_enabled,
        soundpanel_bindings_count = settings.soundpanel_bindings.len(),
        "get_all_app_settings: Settings loaded successfully"
    );

    Ok(settings)
}

/// Check if backend is ready (settings loaded, initialization complete)
#[tauri::command]
pub fn is_backend_ready(app_state: State<'_, AppState>) -> bool {
    app_state
        .backend_ready
        .load(std::sync::atomic::Ordering::SeqCst)
}

/// Confirm backend is ready and emit event if already ready
#[tauri::command]
pub async fn confirm_backend_ready(
    app_state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let ready = app_state
        .backend_ready
        .load(std::sync::atomic::Ordering::SeqCst);

    if ready {
        info!("confirm_backend_ready: Backend already ready, emitting event");
        let _ = app_handle.emit("backend-ready", &());
    } else {
        info!("confirm_backend_ready: Backend not ready yet");
    }

    Ok(())
}

/// Set quick editor behavior mode
#[tauri::command]
pub async fn set_editor_quick(
    value: String,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<String, String> {
    let mode = QuickEditorMode::from_str(&value)
        .ok_or_else(|| format!("Invalid quick editor mode: {}", value))?;
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_quick(mode)
    })
    .await?;

    emit_settings_changed(&app_handle);

    Ok(value)
}

/// Get quick editor behavior mode
#[tauri::command]
pub fn get_editor_quick(settings_manager: State<'_, SettingsManager>) -> String {
    settings_manager.get_editor_quick().as_str().to_string()
}

/// Set spellcheck enabled
#[tauri::command]
pub async fn set_editor_spellcheck_enabled(
    value: bool,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<bool, String> {
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_spellcheck_enabled(value)
    })
    .await?;

    emit_settings_changed(&app_handle);

    Ok(value)
}

/// Get spellcheck enabled
#[tauri::command]
pub fn get_editor_spellcheck_enabled(settings_manager: State<'_, SettingsManager>) -> bool {
    settings_manager.get_editor_spellcheck_enabled()
}

/// Set spellcheck source
#[tauri::command]
pub async fn set_editor_spellcheck_source(
    value: SpellSource,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<SpellSource, String> {
    let v = value.clone();
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_spellcheck_source(v)
    })
    .await?;

    emit_settings_changed(&app_handle);

    Ok(value)
}

/// Get spellcheck source
#[tauri::command]
pub fn get_editor_spellcheck_source(settings_manager: State<'_, SettingsManager>) -> SpellSource {
    settings_manager.get_editor_spellcheck_source()
}

/// Set editor height
#[tauri::command]
pub async fn set_editor_height(
    height: u32,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<u32, String> {
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_height(height)
    })
    .await?;

    emit_settings_changed(&app_handle);

    Ok(height.clamp(200, 1200))
}

/// Get editor height
#[tauri::command]
pub fn get_editor_height(settings_manager: State<'_, SettingsManager>) -> u32 {
    settings_manager.get_editor_height()
}

/// Set VTS typing idle timeout in milliseconds
#[tauri::command]
pub async fn set_editor_typing_idle_timeout_ms(
    ms: u32,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<u32, String> {
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_typing_idle_timeout_ms(ms)
    })
    .await?;

    emit_settings_changed(&app_handle);
    Ok(normalize_typing_idle_timeout_ms(ms))
}

/// Get VTS typing idle timeout in milliseconds
#[tauri::command]
pub fn get_editor_typing_idle_timeout_ms(settings_manager: State<'_, SettingsManager>) -> u32 {
    settings_manager.get_editor_typing_idle_timeout_ms()
}

/// Set editor typing enabled state
#[tauri::command]
pub async fn set_editor_typing_enabled(
    enabled: bool,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<bool, String> {
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_typing_enabled(enabled)
    })
    .await?;

    emit_settings_changed(&app_handle);
    Ok(enabled)
}

/// Set editor autocomplete suggestions enabled state
#[tauri::command]
pub async fn set_editor_autocomplete_enabled(
    enabled: bool,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<bool, String> {
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_autocomplete_enabled(enabled)
    })
    .await?;

    emit_settings_changed(&app_handle);
    Ok(enabled)
}

/// Set built-in number conversion enabled state
#[tauri::command]
pub async fn set_editor_number_conversion_enabled(
    enabled: bool,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<bool, String> {
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_number_conversion_enabled(enabled)
    })
    .await?;

    emit_settings_changed(&app_handle);
    Ok(enabled)
}

/// Set keep-text-after-send state
#[tauri::command]
pub async fn set_editor_keep_text(
    enabled: bool,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<bool, String> {
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_keep_text(enabled)
    })
    .await?;

    emit_settings_changed(&app_handle);
    Ok(enabled)
}

/// Set default editor route
#[tauri::command]
pub async fn set_editor_default_route(
    route: EditorRoute,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<EditorRoute, String> {
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_default_route(route)
    })
    .await?;

    emit_settings_changed(&app_handle);

    Ok(route)
}

/// Set editor font family. The frontend selects from the catalog collected at
/// startup; accepting the name here preserves existing settings after a font
/// is later removed from Windows.
#[tauri::command]
pub async fn set_editor_font_family(
    family: String,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<String, String> {
    let selected_family = family.clone();
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_font_family(selected_family)
    })
    .await?;

    emit_settings_changed(&app_handle);

    Ok(family)
}

/// Get editor font family
#[tauri::command]
pub fn get_editor_font_family(settings_manager: State<'_, SettingsManager>) -> String {
    settings_manager.get_editor_font_family()
}

/// Return the Windows font families loaded once before the backend becomes ready.
#[tauri::command]
pub fn get_system_font_families(catalog: State<'_, SystemFontCatalog>) -> Vec<String> {
    catalog.families().to_vec()
}

/// Set editor font size in pixels (strict: invalid size is rejected without writes).
#[tauri::command]
pub async fn set_editor_font_size(
    size_px: u32,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<u32, String> {
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_font_size_px(size_px)
    })
    .await?;

    emit_settings_changed(&app_handle);

    Ok(size_px)
}

/// Get editor font size in pixels
#[tauri::command]
pub fn get_editor_font_size_px(settings_manager: State<'_, SettingsManager>) -> u32 {
    settings_manager.get_editor_font_size_px()
}

/// Set the interface font family. `"default"` keeps the built-in CSS stack;
/// any non-empty name is preserved so an uninstalled font cannot corrupt
/// settings.
#[tauri::command]
pub async fn set_ui_font_family(
    family: String,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<String, String> {
    let selected_family = family.clone();
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_ui_font_family(selected_family)
    })
    .await?;

    emit_settings_changed(&app_handle);

    Ok(family)
}

/// Set the interface font size in pixels (strict: an invalid size is rejected
/// without writes). The size becomes the `html` rem base of the main window.
#[tauri::command]
pub async fn set_ui_font_size(
    size_px: u32,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<u32, String> {
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_ui_font_size_px(size_px)
    })
    .await?;

    emit_settings_changed(&app_handle);

    Ok(size_px)
}

/// Get the interface font family
#[tauri::command]
pub fn get_ui_font_family(settings_manager: State<'_, SettingsManager>) -> String {
    settings_manager.get_ui_font_family()
}

/// Get the interface font size in pixels
#[tauri::command]
pub fn get_ui_font_size_px(settings_manager: State<'_, SettingsManager>) -> u32 {
    settings_manager.get_ui_font_size_px()
}

/// ROADMAP-119: block Piper model loading when eSpeak NG phonemization is
/// unusable. Built-in providers pass through unchanged. The readiness probe
/// (file I/O plus the one-time eSpeak initialization) runs on the blocking
/// pool; a not-ready verdict yields a structured not-ready error and never
/// starts a background model load. Repeated attempts reuse the cached probe
/// verdict, so no duplicate work is queued.
async fn gate_piper_espeak_readiness(
    app_handle: &AppHandle,
    state: &AppState,
    id: &str,
) -> Result<(), CommandError> {
    use crate::ipc::tts_provider::error_code;

    let provider = {
        let registry = state.tts_registry.lock();
        registry.get(id).map(|entry| entry.provider.clone())
    };
    let piper = match provider {
        Some(TtsProvider::Piper(tts)) => tts,
        _ => return Ok(()),
    };

    let error = match tokio::task::spawn_blocking(move || piper.check_phonemization()).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error,
        Err(e) => {
            warn!(id, error = %e, "Piper readiness check task failed");
            return Err(CommandError::new(
                error_code::READINESS_CHECK_FAILED,
                format!("Piper readiness check failed: {}", e),
                false,
            ));
        }
    };
    warn!(id, error = %error, "Piper provider request blocked before model load");
    let messages = app_handle
        .try_state::<crate::commands::localization::LocalizationState>()
        .map(|localization| localization.snapshot().messages)
        .unwrap_or_default();
    let code = match &error {
        crate::tts::piper::runtime::PiperRuntimeError::NotReady(
            crate::tts::piper::runtime::PiperReadiness::DataNotFound,
        ) => error_code::ESPEAK_DATA_NOT_FOUND,
        crate::tts::piper::runtime::PiperRuntimeError::NotReady(_) => {
            error_code::ESPEAK_DATA_DAMAGED
        }
        crate::tts::piper::runtime::PiperRuntimeError::VoiceUnavailable(_) => {
            error_code::ESPEAK_VOICE_UNAVAILABLE
        }
        _ => error_code::PREPARE_TASK_FAILED,
    };
    let message = crate::tts::piper::runtime::phonemization_error_key(&error)
        .and_then(|key| messages.get(key))
        .cloned()
        .unwrap_or_else(|| "Piper model preparation failed".to_string());
    Err(CommandError::new(code, message, false))
}

/// Prepare (warm up) a registered TTS provider by ID.
/// For Piper this loads the model into memory; for network providers it is a no-op.
#[tauri::command]
pub async fn prepare_tts_provider_by_id(
    id: String,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    use crate::ipc::tts_provider::error_code;

    info!(id, "Preparing TTS provider by ID");

    gate_piper_espeak_readiness(&app_handle, state.inner(), &id).await?;

    let provider = {
        let registry = state.tts_registry.lock();
        registry
            .get(&id)
            .map(|entry| entry.provider.clone())
            .ok_or(CommandError::new(
                error_code::UNKNOWN_ID,
                format!("Unknown provider ID: {}", id),
                false,
            ))?
    };

    match tokio::task::spawn_blocking(move || provider.prepare()).await {
        Ok(result) => {
            result.map_err(|e| CommandError::new(error_code::PREPARE_TASK_FAILED, e, false))
        }
        Err(e) => Err(CommandError::new(
            error_code::PREPARE_TASK_FAILED,
            format!("Provider preparation task failed: {}", e),
            false,
        )),
    }
}

/// Select an already registered TTS provider by its stable concrete ID.
/// This is the single owner path for provider selection — prepare is executed
/// before persistence, so partial states are never observed.
#[tauri::command]
pub async fn select_tts_provider_by_id(
    id: String,
    app_handle: AppHandle,
    state: State<'_, AppState>,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), CommandError> {
    use crate::ipc::tts_provider::error_code;

    info!(id, "Selecting TTS provider by ID");
    gate_piper_espeak_readiness(&app_handle, state.inner(), &id).await?;

    let manager = settings_manager.inner().clone();
    state
        .select_tts_provider(id.clone(), move |provider_id, legacy_type| {
            let mut settings = manager.load().map_err(|e| e.to_string())?;
            settings.tts.provider_id = Some(provider_id);
            if let Some(provider_type) = legacy_type {
                settings.tts.provider = provider_type;
            }
            manager.save(&settings).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| CommandError::new(error_code::SELECTION_FAILED, e, false))?;
    emit_settings_changed(&app_handle);
    Ok(())
}

fn pocket_provider(state: &AppState, id: &str) -> Result<PocketTts, CommandError> {
    let provider = {
        let registry = state.tts_registry.lock();
        registry.get(id).map(|entry| entry.provider.clone())
    };
    match provider {
        Some(TtsProvider::Pocket(provider)) => Ok(provider),
        _ => Err(CommandError::new(
            crate::ipc::pocket::error_code::UNKNOWN_MODEL,
            format!("Unknown Pocket TTS model ID: {id}"),
            false,
        )),
    }
}

fn pocket_error(error: PocketError) -> CommandError {
    CommandError::new(error.code().as_str(), error.message(), false)
}

fn pocket_models_snapshot(state: &AppState) -> Vec<PocketModelDto> {
    state
        .tts_registry
        .lock()
        .iter()
        .filter_map(|entry| match &entry.provider {
            TtsProvider::Pocket(provider) => Some(provider.dto()),
            _ => None,
        })
        .collect()
}

/// DTO returned by the WAV reference check. `status` distinguishes a missing
/// file from an invalid one without throwing for the normal "user picked a bad
/// file" path.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PocketReferenceCheckDto {
    pub status: String,
    pub error_code: Option<String>,
    pub file_name: Option<String>,
    pub duration_seconds: Option<f32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<usize>,
}

/// List every valid Pocket TTS package with its selected voice and readiness.
#[tauri::command]
pub fn list_pocket_models(state: State<'_, AppState>) -> Vec<PocketModelDto> {
    pocket_models_snapshot(state.inner())
}

/// Re-scan `models/pocket` in the shared Piper root order. New packages are
/// registered, changed packages replaced, and removed packages dropped. The
/// returned snapshot is ready for the card to render.
#[tauri::command]
pub async fn refresh_pocket_models(
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<PocketModelDto>, String> {
    state.register_pocket_providers();
    emit_settings_changed(&app_handle);
    Ok(pocket_models_snapshot(state.inner()))
}

/// Validate a user-selected WAV reference. Readiness distinguishes missing and
/// invalid files; the command itself is total for those user-facing outcomes.
#[tauri::command]
pub fn validate_pocket_reference(path: String) -> PocketReferenceCheckDto {
    use crate::tts::pocket::audio::validate_reference_wav;

    let path = std::path::PathBuf::from(path);
    match validate_reference_wav(&path) {
        Ok(info) => PocketReferenceCheckDto {
            status: crate::ipc::pocket::readiness::READY.to_string(),
            error_code: None,
            file_name: Some(info.file_name),
            duration_seconds: Some(info.duration_seconds),
            sample_rate: Some(info.sample_rate),
            channels: Some(info.channels),
        },
        Err(error) => {
            let status = match &error {
                PocketError::ReferenceMissing => crate::ipc::pocket::readiness::REFERENCE_MISSING,
                _ => crate::ipc::pocket::readiness::REFERENCE_INVALID,
            };
            PocketReferenceCheckDto {
                status: status.to_string(),
                error_code: Some(match &error {
                    PocketError::ReferenceTooLong { .. } => "pocket.reference_too_long".to_string(),
                    _ => error.code().as_str().to_string(),
                }),
                file_name: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string),
                duration_seconds: None,
                sample_rate: None,
                channels: None,
            }
        }
    }
}

/// Set (or clear) the voice of a Pocket TTS model and persist it by package ID.
///
/// The selection is validated before persistence and the registry entry is
/// replaced only after the store write succeeds. Queue snapshots already taken
/// keep the previous selection.
#[tauri::command]
pub async fn set_pocket_voice(
    model_id: String,
    voice: Option<PocketVoiceInput>,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<PocketModelDto, CommandError> {
    use crate::tts::pocket::validate_selection;

    let dto: PocketModelDto = with_voice_store(|store| -> Result<_, CommandError> {
        let provider = pocket_provider(state.inner(), &model_id)?;
        let selection = match voice {
            Some(input) => {
                Some(validate_selection(provider.descriptor(), &input).map_err(pocket_error)?)
            }
            None => None,
        };
        let package_id = provider.descriptor().package_id.clone();
        let persistence_result = match &selection {
            Some(selection) => store.set(&package_id, selection),
            None => store.clear(&package_id),
        };
        persistence_result.map_err(|error| {
            CommandError::new(
                crate::ipc::pocket::error_code::PERSISTENCE_FAILED,
                error.message(),
                false,
            )
        })?;

        let updated = provider.with_voice(selection);
        {
            let mut registry = state.tts_registry.lock();
            let display_name = provider.descriptor().display_name.clone();
            registry.add_or_replace(crate::tts::registry::TtsProviderEntry {
                id: model_id,
                display_name,
                provider: TtsProvider::Pocket(updated.clone()),
            });
        }
        Ok(updated.dto())
    })?;
    emit_settings_changed(&app_handle);
    Ok(dto)
}

/// Normalize the selected WAV reference into the application config directory.
///
/// The user file is never modified: a mono 24 kHz copy with the active speech
/// at `RMS ~0.048` is written to `pocket-references/<package_id>.wav` and the
/// model selection is switched to that copy, so every future synthesis uses the
/// pitch-safe reference level. The blocking decode/encode runs off the async
/// runtime; persistence and registry update then follow the `set_pocket_voice`
/// transaction order (voice store lock, then registry).
#[tauri::command]
pub async fn normalize_pocket_reference(
    model_id: String,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<PocketModelDto, CommandError> {
    use crate::tts::pocket::audio::normalize_reference_wav_to_file;

    let (source_path, package_id) = {
        let provider = pocket_provider(state.inner(), &model_id)?;
        match provider.selection() {
            Some(PocketVoiceSelection::ReferenceWav(path)) => {
                (path.clone(), provider.descriptor().package_id.clone())
            }
            _ => {
                return Err(CommandError::new(
                    crate::ipc::pocket::error_code::REFERENCE_REQUIRED,
                    PocketError::ReferenceRequired.message(),
                    false,
                ))
            }
        }
    };

    let destination = crate::paths::config_root()
        .map_err(|_| {
            CommandError::new(
                crate::ipc::pocket::error_code::PERSISTENCE_FAILED,
                "Не удалось определить каталог конфигурации приложения".to_string(),
                false,
            )
        })?
        .join("pocket-references")
        .join(format!("{}.wav", sanitize_package_stem(&package_id)));

    let write_target = destination.clone();
    tokio::task::spawn_blocking(move || {
        normalize_reference_wav_to_file(&source_path, &write_target)
    })
    .await
    .map_err(|error| {
        CommandError::new(
            crate::ipc::pocket::error_code::RUNTIME_FAILED,
            format!("Не удалось подготовить референс Pocket TTS: {error}"),
            false,
        )
    })?
    .map_err(pocket_error)?;

    let selection = PocketVoiceSelection::ReferenceWav(destination);
    let dto = with_voice_store(|store| -> Result<PocketModelDto, CommandError> {
        let provider = pocket_provider(state.inner(), &model_id)?;
        store.set(&package_id, &selection).map_err(|error| {
            CommandError::new(
                crate::ipc::pocket::error_code::PERSISTENCE_FAILED,
                error.message(),
                false,
            )
        })?;
        let updated = provider.with_voice(Some(selection.clone()));
        {
            let mut registry = state.tts_registry.lock();
            let display_name = provider.descriptor().display_name.clone();
            registry.add_or_replace(crate::tts::registry::TtsProviderEntry {
                id: model_id.clone(),
                display_name,
                provider: TtsProvider::Pocket(updated.clone()),
            });
        }
        Ok(updated.dto())
    })?;
    emit_settings_changed(&app_handle);
    Ok(dto)
}

/// File-system-safe stem for a normalized reference file name.
fn sanitize_package_stem(package_id: &str) -> String {
    let sanitized: String = package_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "reference".to_string()
    } else {
        trimmed.to_string()
    }
}

/// DTO for a discovered local RUAccent pack without filesystem paths.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HomographAccentorPackDto {
    pub id: String,
    pub display_name: String,
    pub runtime_version: String,
    /// Diagnostic runtime status: `"not_loaded"`, `"loading"`, `"ready"`,
    /// `"failed"`. Never carries the failure message.
    pub runtime_status: String,
}

/// Map a runtime slot status to the safe wire string; `Failed` loses its message.
fn ruaccent_runtime_status_string(slot: &crate::stress::runtime::RuAccentRuntimeSlot) -> String {
    slot.status().as_safe_str().to_string()
}

/// Map a discovered RUAccent pack to its safe wire DTO.
fn homograph_accentor_pack_dto(
    state: &AppState,
    d: &RuAccentPackDescriptor,
) -> HomographAccentorPackDto {
    HomographAccentorPackDto {
        runtime_status: state
            .get_ruaccent_runtime_slot(&d.id)
            .as_ref()
            .map(ruaccent_runtime_status_string)
            .unwrap_or_else(|| "not_loaded".to_string()),
        id: d.id.clone(),
        display_name: d.display_name.clone(),
        runtime_version: d.runtime_version.clone(),
    }
}

/// Snapshot the currently discovered local RUAccent packs.
#[tauri::command]
pub fn list_homograph_accentor_packs(state: State<'_, AppState>) -> Vec<HomographAccentorPackDto> {
    state
        .get_ruaccent_packs()
        .iter()
        .map(|d| homograph_accentor_pack_dto(state.inner(), d))
        .collect()
}

/// Reconcile decision for a RUAccent pack refresh.
///
/// Pure and free of any runtime/ONNX dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuaccentRefreshReconcile {
    /// Nothing to change: the selection is still valid, or the runtime holds the
    /// removed model live.
    None,
    /// The selected model is confirmed gone everywhere: disable the layer,
    /// auto-selecting the single remaining model when exactly one is left and
    /// retaining the missing id when none remain.
    Disable { pack_id: Option<String> },
    /// There was no (or a dangling) selection and exactly one pack exists:
    /// select it without enabling.
    AutoSelect(String),
}

/// Decide the reconcile action for a RUAccent pack refresh. Symmetric with
/// [`crate::ocr::service::decide_ocr_refresh_reconcile`], with one extra rule:
/// a model held live stays selected (and untouched) even when the layer is
/// disabled, so it takes priority over any disabled-state auto-selection.
pub fn decide_ruaccent_refresh_reconcile(
    enabled: bool,
    saved_id: Option<&str>,
    pack_ids: &[String],
    holds_live: bool,
) -> RuaccentRefreshReconcile {
    if enabled
        && saved_id.is_some()
        && !pack_ids.iter().any(|id| Some(id.as_str()) == saved_id)
        && !holds_live
    {
        return RuaccentRefreshReconcile::Disable {
            pack_id: match pack_ids.len() {
                0 => saved_id.map(str::to_string),
                1 => Some(pack_ids[0].clone()),
                _ => None,
            },
        };
    }

    if holds_live {
        return RuaccentRefreshReconcile::None;
    }

    if !enabled && pack_ids.len() == 1 && saved_id != Some(pack_ids[0].as_str()) {
        return RuaccentRefreshReconcile::AutoSelect(pack_ids[0].clone());
    }

    RuaccentRefreshReconcile::None
}

/// Resolve the RUAccent pack search roots from the shared model root source
/// (ROADMAP-117): the process cwd first, then the effective data root
/// («Данные программы» — `storage.data_dir` override or the default). Order
/// matters: the first root wins on duplicate IDs. There is no separate
/// exe/resource root for user models: models next to the exe are found
/// whenever the process cwd equals the exe directory.
pub(crate) fn ruaccent_search_roots(_app_handle: &AppHandle) -> Vec<std::path::PathBuf> {
    crate::paths::model_search_root_paths()
}

/// Re-scan the RUAccent pack directories and reconcile the persisted selection.
///
/// The reconcile follows the shared checkbox rule: when the selected model is
/// gone and not held live the layer is disabled with a save (also clearing
/// `load_on_start`), auto-selecting the single remaining model; a dangling
/// selection with exactly one pack auto-selects it without enabling; a model
/// still held live is left untouched. Returns the fresh pack list.
#[tauri::command]
pub async fn refresh_homograph_accentor_packs(
    state: State<'_, AppState>,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<Vec<HomographAccentorPackDto>, String> {
    let roots = ruaccent_search_roots(&app_handle);
    state.refresh_ruaccent_packs(&roots);

    let accentor = state
        .settings_cache
        .read()
        .editor
        .homograph_accentor
        .clone();
    let pack_ids: Vec<String> = state
        .get_ruaccent_packs()
        .iter()
        .map(|d| d.id.clone())
        .collect();

    let holds_live = accentor
        .accentor_pack_id
        .as_deref()
        .and_then(|id| state.get_ruaccent_runtime_slot(id))
        .map(|slot| slot.is_live())
        .unwrap_or(false);

    let decision = decide_ruaccent_refresh_reconcile(
        accentor.enabled,
        accentor.accentor_pack_id.as_deref(),
        &pack_ids,
        holds_live,
    );

    match decision {
        RuaccentRefreshReconcile::None => {}
        RuaccentRefreshReconcile::Disable { pack_id } => {
            let new_id = pack_id.clone();
            persist_blocking(settings_manager.inner(), move |mgr| {
                mgr.set_editor_homograph_accentor(false, new_id)?;
                mgr.set_editor_homograph_accentor_load_on_start(false)
            })
            .await?;
            emit_settings_changed(&app_handle);
        }
        RuaccentRefreshReconcile::AutoSelect(id) => {
            let id_for_persist = id.clone();
            persist_blocking(settings_manager.inner(), move |mgr| {
                mgr.set_editor_homograph_accentor(false, Some(id_for_persist))
            })
            .await?;
            emit_settings_changed(&app_handle);
        }
    }

    Ok(state
        .get_ruaccent_packs()
        .iter()
        .map(|d| homograph_accentor_pack_dto(state.inner(), d))
        .collect())
}

/// Validate a homograph/accentor selection against the currently discovered
/// packs, returning the pack id to persist (or an error).
///
/// Rules:
/// - `enabled` requires a non-empty pack id that is present in `discovered_ids`;
/// - when disabled the pack id is kept as-is for a later enable, even when it is
///   no longer discovered (a dangling id is allowed per spec).
pub fn validate_homograph_accentor_selection(
    enabled: bool,
    accentor_pack_id: Option<String>,
    discovered_ids: &[String],
) -> Result<Option<String>, String> {
    if enabled {
        let id = accentor_pack_id
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "Требуется выбрать модель RUAccent для включения".to_string())?;
        if !discovered_ids.contains(&id) {
            return Err(format!("Неизвестная модель RUAccent: {}", id));
        }
        return Ok(Some(id));
    }

    Ok(accentor_pack_id)
}

/// Resolve the runtime slot to keep resident after a homograph/accentor
/// selection save.
///
/// Disabling keeps nothing: every runtime, including the selected one, is
/// unloaded. Enabling keeps only the selected slot.
fn ruaccent_keep_id(enabled: bool, pack_id: Option<&str>) -> Option<String> {
    if enabled {
        pack_id.map(str::to_string)
    } else {
        None
    }
}

/// Enable/disable the homograph/accentor (RUAccent) layer and select a pack.
///
/// Enabling requires a pack id present among the currently discovered packs.
/// Only saves the setting — the model is not loaded until future pipeline use.
#[tauri::command]
pub async fn set_editor_homograph_accentor(
    enabled: bool,
    accentor_pack_id: Option<String>,
    state: State<'_, AppState>,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let discovered_ids: Vec<String> = state
        .get_ruaccent_packs()
        .into_iter()
        .map(|d| d.id)
        .collect();

    let pack_id =
        validate_homograph_accentor_selection(enabled, accentor_pack_id, &discovered_ids)?;

    let keep_id = ruaccent_keep_id(enabled, pack_id.as_deref());

    let pack_id_for_persist = pack_id.clone();
    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_homograph_accentor(enabled, pack_id_for_persist)
    })
    .await?;

    // Enabling keeps only the selected runtime resident; disabling unloads every
    // runtime including the selected one. Never load the newly selected model
    // automatically.
    unload_ruaccent_runtimes_except(state.inner(), keep_id.as_deref()).await?;

    emit_settings_changed(&app_handle);

    Ok(())
}

/// Validate a load-on-start enable request: a known selected model must exist.
fn validate_homograph_accentor_load_on_start(
    load_on_start: bool,
    accentor_pack_id: Option<String>,
    discovered_ids: &[String],
) -> Result<(), String> {
    if !load_on_start {
        return Ok(());
    }
    let id = accentor_pack_id
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Требуется выбрать модель RUAccent для автозагрузки".to_string())?;
    if !discovered_ids.contains(&id) {
        return Err(format!("Неизвестная модель RUAccent: {id}"));
    }
    Ok(())
}

/// Persist the `load_on_start` flag for the selected RUAccent model.
///
/// Enabling requires a known selected model among the currently discovered
/// packs. Only saves the setting — the model is loaded at next startup, not
/// here.
#[tauri::command]
pub async fn set_editor_homograph_accentor_load_on_start(
    load_on_start: bool,
    state: State<'_, AppState>,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let (accentor_pack_id, discovered_ids) = {
        let settings = state.settings_cache.read();
        (
            settings.editor.homograph_accentor.accentor_pack_id.clone(),
            state
                .get_ruaccent_packs()
                .into_iter()
                .map(|d| d.id)
                .collect::<Vec<String>>(),
        )
    };

    validate_homograph_accentor_load_on_start(load_on_start, accentor_pack_id, &discovered_ids)?;

    persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_homograph_accentor_load_on_start(load_on_start)
    })
    .await?;

    emit_settings_changed(&app_handle);

    Ok(())
}

/// Status-changed event name emitted when a RUAccent runtime starts loading or
/// reaches a terminal state.
pub const RUACCENT_RUNTIME_STATUS_CHANGED_EVENT: &str = "ruaccent-runtime-status-changed";
/// Error event name emitted when a RUAccent runtime load fails.
pub const RUACCENT_RUNTIME_ERROR_EVENT: &str = "ruaccent-runtime-error";

/// Safe payload for `ruaccent-runtime-status-changed`: selected model ID and a
/// safe status string only.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuAccentRuntimeStatusPayload {
    pub model_id: String,
    pub status: String,
}

/// Safe payload for `ruaccent-runtime-error`: selected model ID and a short
/// Russian user-facing message (no raw ONNX errors or paths).
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuAccentRuntimeErrorPayload {
    pub model_id: String,
    pub message: String,
}

/// Build the safe status-changed payload for a model ID and status string.
fn ruaccent_status_payload(model_id: &str, status: &str) -> RuAccentRuntimeStatusPayload {
    RuAccentRuntimeStatusPayload {
        model_id: model_id.to_string(),
        status: status.to_string(),
    }
}

/// Short Russian user-facing message for a failed RUAccent load. Deliberately
/// excludes raw ONNX errors and filesystem paths.
fn ruaccent_load_error_message() -> String {
    "Не удалось загрузить модель RUAccent. Проверьте целостность файлов модели.".to_string()
}

/// Short Russian user-facing message for an aborted RUAccent load task.
fn ruaccent_load_aborted_message() -> String {
    "Загрузка модели RUAccent была прервана.".to_string()
}

/// Release non-selected runtimes away from the async command thread. Dropping
/// an ONNX session may wait for an in-flight inference that owns the slot lock.
async fn unload_ruaccent_runtimes_except(
    state: &AppState,
    keep_id: Option<&str>,
) -> Result<(), String> {
    let slots = state.ruaccent_runtime_slots_except(keep_id);
    tokio::task::spawn_blocking(move || {
        for slot in slots {
            slot.unload();
        }
    })
    .await
    .map_err(|_| "Не удалось выгрузить предыдущую модель RUAccent".to_string())
}

/// Run the explicit load/retry lifecycle for a RUAccent model, emitting the
/// same status/error events as the startup loader.
///
/// Unloads other resident runtimes, runs the synchronous slot load inside
/// `spawn_blocking`, and returns a safe error on failure. Used by both the
/// manual load command and the startup loader.
pub(crate) async fn load_ruaccent_runtime(
    app_handle: &AppHandle,
    state: &AppState,
    model_id: String,
) -> Result<(), String> {
    let slot = state
        .get_ruaccent_runtime_slot(&model_id)
        .ok_or_else(|| format!("Неизвестная модель RUAccent: {model_id}"))?;

    unload_ruaccent_runtimes_except(state, Some(&model_id)).await?;

    if slot.is_ready() {
        return Ok(());
    }

    let _ = app_handle.emit(
        RUACCENT_RUNTIME_STATUS_CHANGED_EVENT,
        ruaccent_status_payload(&model_id, "loading"),
    );

    let slot_for_load = slot.clone();
    let load_result = match tokio::task::spawn_blocking(move || slot_for_load.load_or_retry()).await
    {
        Ok(result) => result,
        Err(_) => {
            let message = ruaccent_load_aborted_message();
            slot.mark_failed(message.clone());
            Err(message)
        }
    };

    match load_result {
        Ok(()) => {
            let _ = app_handle.emit(
                RUACCENT_RUNTIME_STATUS_CHANGED_EVENT,
                ruaccent_status_payload(&model_id, "ready"),
            );
            Ok(())
        }
        Err(_) => {
            let _ = app_handle.emit(
                RUACCENT_RUNTIME_STATUS_CHANGED_EVENT,
                ruaccent_status_payload(&model_id, "failed"),
            );
            let _ = app_handle.emit(
                RUACCENT_RUNTIME_ERROR_EVENT,
                RuAccentRuntimeErrorPayload {
                    model_id: model_id.clone(),
                    message: ruaccent_load_error_message(),
                },
            );
            Err(ruaccent_load_error_message())
        }
    }
}

/// Persist disabling the RUAccent layer after a failed load attempt.
///
/// A failed load clears `enabled` (and, for the startup path, `load_on_start`)
/// so the checkbox stays truthful. The selected pack id is kept as-is (a
/// dangling id is allowed). A persist error is only warned about; the caller
/// still returns its own load error.
async fn disable_ruaccent_after_failed_load(
    app_handle: &AppHandle,
    state: &AppState,
    also_load_on_start: bool,
) {
    let Some(settings_manager) = app_handle.try_state::<SettingsManager>() else {
        warn!("SettingsManager unavailable while disabling RUAccent after failed load");
        return;
    };

    let pack_id = state
        .settings_cache
        .read()
        .editor
        .homograph_accentor
        .accentor_pack_id
        .clone();

    let pack_id_for_persist = pack_id.clone();
    let result = persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_homograph_accentor(false, pack_id_for_persist)?;
        if also_load_on_start {
            mgr.set_editor_homograph_accentor_load_on_start(false)?;
        }
        Ok(())
    })
    .await;

    match result {
        Ok(()) => emit_settings_changed(app_handle),
        Err(error) => {
            warn!(error = %error, "Failed to persist RUAccent disable after failed load");
        }
    }
}

/// Load a selected RUAccent model by ID (explicit command).
///
/// Validates the ID against discovery, unloads other resident runtimes, runs
/// the load inside `spawn_blocking`, and returns a safe error on failure.
#[tauri::command]
pub async fn load_homograph_accentor_model(
    model_id: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let discovered_ids: Vec<String> = state
        .get_ruaccent_packs()
        .into_iter()
        .map(|d| d.id)
        .collect();
    if !discovered_ids.contains(&model_id) {
        return Err(format!("Неизвестная модель RUAccent: {model_id}"));
    }

    match load_ruaccent_runtime(&app_handle, &state, model_id).await {
        Ok(()) => Ok(()),
        Err(error) => {
            // A failed manual load still clears the checkbox, but keeps
            // `load_on_start` (this was not an auto-load attempt).
            if state
                .settings_cache
                .read()
                .editor
                .homograph_accentor
                .enabled
            {
                disable_ruaccent_after_failed_load(&app_handle, &state, false).await;
            }
            Err(error)
        }
    }
}

/// Start the persisted RUAccent startup load after the frontend has registered
/// its global status/error listeners. Calling this command is idempotent.
#[tauri::command]
pub async fn start_homograph_accentor_startup_load(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let accentor = state
        .settings_cache
        .read()
        .editor
        .homograph_accentor
        .clone();
    if !accentor.load_on_start {
        return Ok(());
    }

    let model_id = match accentor.accentor_pack_id.filter(|id| !id.is_empty()) {
        Some(model_id) => model_id,
        None => {
            let message = "Не выбрана модель RUAccent для автозагрузки".to_string();
            let _ = app_handle.emit(
                RUACCENT_RUNTIME_ERROR_EVENT,
                RuAccentRuntimeErrorPayload {
                    model_id: String::new(),
                    message: message.clone(),
                },
            );
            disable_ruaccent_after_failed_load(&app_handle, &state, true).await;
            return Err(message);
        }
    };
    if state.get_ruaccent_runtime_slot(&model_id).is_none() {
        let message = "Выбранная модель RUAccent не найдена".to_string();
        let _ = app_handle.emit(
            RUACCENT_RUNTIME_ERROR_EVENT,
            RuAccentRuntimeErrorPayload {
                model_id,
                message: message.clone(),
            },
        );
        disable_ruaccent_after_failed_load(&app_handle, &state, true).await;
        return Err(message);
    }

    if let Err(error) = load_ruaccent_runtime(&app_handle, &state, model_id).await {
        disable_ruaccent_after_failed_load(&app_handle, &state, true).await;
        return Err(error);
    }
    Ok(())
}

/// Reject empty or whitespace-only preview input with a short Russian error.
fn validate_preview_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Текст пуст — нечего расставлять".to_string());
    }
    Ok(())
}

/// Render the native RUAccent preview for `text` in Silero `+` notation.
///
/// The runtime must already be ready. Inference and output-validation errors
/// are propagated (the preview never fails open to unchanged text).
fn native_preview_marked(runtime: &RuAccentRuntimeSlot, text: &str) -> Result<String, String> {
    let stress = crate::commands::tts_pipeline::native_stress_strict(runtime, text)?;
    Ok(crate::stress::adapters::ProviderStressAdapter::Silero.adapt(&stress))
}

/// Clear Russian error when no RUAccent model is selected.
fn preview_no_model_error() -> String {
    "Нет выбранной модели RUAccent. Выберите модель в настройках.".to_string()
}

/// Clear Russian error when the selected RUAccent model is not loaded.
fn preview_not_loaded_error() -> String {
    "Модель RUAccent не загружена. Загрузите модель в настройках.".to_string()
}

/// Resolve the ready preview runtime, distinguishing missing and not-loaded
/// models with clear Russian errors.
fn resolve_preview_runtime(
    accentor_pack_id: Option<&str>,
    slot: Option<RuAccentRuntimeSlot>,
) -> Result<RuAccentRuntimeSlot, String> {
    let pack_id = accentor_pack_id
        .filter(|id| !id.is_empty())
        .ok_or_else(preview_no_model_error)?;
    let runtime = slot.ok_or_else(|| format!("Неизвестная модель RUAccent: {pack_id}"))?;
    if !runtime.is_ready() {
        return Err(preview_not_loaded_error());
    }
    Ok(runtime)
}

/// Preview the native RUAccent result in Silero `+` notation.
///
/// Explicit editor preview for testing: no speech is enqueued, nothing is
/// persisted, and a selected, loaded native pack is required. A missing or
/// not-loaded model and any inference/validation failure return a clear error;
/// the preview never silently returns unchanged text as success.
#[tauri::command]
pub async fn preview_contextual_ruaccent(
    text: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    validate_preview_text(&text)?;

    let accentor_pack_id = state
        .settings_cache
        .read()
        .editor
        .homograph_accentor
        .accentor_pack_id
        .clone();
    let slot = accentor_pack_id
        .as_deref()
        .and_then(|id| state.get_ruaccent_runtime_slot(id));
    let runtime = resolve_preview_runtime(accentor_pack_id.as_deref(), slot)?;

    tokio::task::spawn_blocking(move || native_preview_marked(&runtime, &text))
        .await
        .map_err(|e| format!("Предпросмотр RUAccent был прерван: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<String> {
        vec!["com.example.a".to_string(), "com.example.b".to_string()]
    }

    #[test]
    fn enable_requires_known_pack_id() {
        assert!(validate_homograph_accentor_selection(true, None, &ids()).is_err());
        assert!(validate_homograph_accentor_selection(true, Some(String::new()), &ids()).is_err());
        assert!(validate_homograph_accentor_selection(
            true,
            Some("com.example.unknown".to_string()),
            &ids()
        )
        .is_err());
    }

    #[test]
    fn enable_accepts_known_pack_id() {
        assert_eq!(
            validate_homograph_accentor_selection(true, Some("com.example.a".to_string()), &ids()),
            Ok(Some("com.example.a".to_string()))
        );
    }

    #[test]
    fn disable_keeps_selected_id_even_when_unknown() {
        assert_eq!(
            validate_homograph_accentor_selection(false, None, &ids()),
            Ok(None)
        );
        assert_eq!(
            validate_homograph_accentor_selection(false, Some("com.example.b".to_string()), &ids()),
            Ok(Some("com.example.b".to_string()))
        );
        // A dangling id is allowed on disable: it is kept for a later enable.
        assert_eq!(
            validate_homograph_accentor_selection(
                false,
                Some("com.example.unknown".to_string()),
                &ids()
            ),
            Ok(Some("com.example.unknown".to_string()))
        );
    }

    // ── RUAccent refresh reconcile decision ──

    #[test]
    fn ruaccent_refresh_enabled_missing_model_not_live_disables_with_none() {
        let packs = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            decide_ruaccent_refresh_reconcile(true, Some("gone"), &packs, false),
            RuaccentRefreshReconcile::Disable { pack_id: None }
        );
    }

    #[test]
    fn ruaccent_refresh_enabled_missing_model_single_remaining_autoselects_on_disable() {
        let packs = vec!["a".to_string()];
        assert_eq!(
            decide_ruaccent_refresh_reconcile(true, Some("gone"), &packs, false),
            RuaccentRefreshReconcile::Disable {
                pack_id: Some("a".to_string())
            }
        );
    }

    #[test]
    fn ruaccent_refresh_enabled_missing_model_held_live_is_none() {
        let packs = vec!["a".to_string()];
        assert_eq!(
            decide_ruaccent_refresh_reconcile(true, Some("gone"), &packs, true),
            RuaccentRefreshReconcile::None
        );
    }

    #[test]
    fn ruaccent_refresh_enabled_model_still_present_is_none() {
        let packs = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            decide_ruaccent_refresh_reconcile(true, Some("a"), &packs, false),
            RuaccentRefreshReconcile::None
        );
    }

    #[test]
    fn ruaccent_refresh_disabled_single_pack_autoselects_when_different() {
        let packs = vec!["a".to_string()];
        assert_eq!(
            decide_ruaccent_refresh_reconcile(false, None, &packs, false),
            RuaccentRefreshReconcile::AutoSelect("a".to_string())
        );
        assert_eq!(
            decide_ruaccent_refresh_reconcile(false, Some("gone"), &packs, false),
            RuaccentRefreshReconcile::AutoSelect("a".to_string())
        );
    }

    #[test]
    fn ruaccent_refresh_disabled_single_pack_already_selected_is_none() {
        let packs = vec!["a".to_string()];
        assert_eq!(
            decide_ruaccent_refresh_reconcile(false, Some("a"), &packs, false),
            RuaccentRefreshReconcile::None
        );
    }

    #[test]
    fn ruaccent_refresh_disabled_multiple_packs_is_none() {
        let packs = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            decide_ruaccent_refresh_reconcile(false, None, &packs, false),
            RuaccentRefreshReconcile::None
        );
    }

    #[test]
    fn ruaccent_refresh_empty_packs_enabled_disables_retaining_missing_id() {
        let packs: Vec<String> = vec![];
        assert_eq!(
            decide_ruaccent_refresh_reconcile(true, Some("gone"), &packs, false),
            RuaccentRefreshReconcile::Disable {
                pack_id: Some("gone".to_string())
            }
        );
    }

    #[test]
    fn ruaccent_refresh_disabled_single_pack_held_live_is_none() {
        let packs = vec!["a".to_string()];
        assert_eq!(
            decide_ruaccent_refresh_reconcile(false, Some("gone"), &packs, true),
            RuaccentRefreshReconcile::None
        );
    }

    // ── ruaccent_keep_id ──

    #[test]
    fn ruaccent_keep_id_unloads_everything_on_disable() {
        assert_eq!(ruaccent_keep_id(false, None), None);
        assert_eq!(ruaccent_keep_id(false, Some("com.example.a")), None);
    }

    #[test]
    fn ruaccent_keep_id_retains_only_selected_on_enable() {
        assert_eq!(
            ruaccent_keep_id(true, Some("com.example.a")),
            Some("com.example.a".to_string())
        );
        assert_eq!(ruaccent_keep_id(true, None), None);
    }

    // ── preview_contextual_ruaccent input validation ──

    #[test]
    fn preview_rejects_empty_and_whitespace_text() {
        assert!(validate_preview_text("").is_err());
        assert!(validate_preview_text("   \n\t ").is_err());
    }

    #[test]
    fn preview_accepts_non_empty_text() {
        assert!(validate_preview_text("замок был на холме под замком").is_ok());
    }

    // ── native_preview_marked requires a loaded, ready pack ──

    #[test]
    fn preview_propagates_not_ready_error_instead_of_unchanged_text() {
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-preview-native-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let slot = non_loadable_native_slot(dir.clone());

        let err =
            native_preview_marked(&slot, "замок").expect_err("must not return unchanged text");
        assert!(
            !err.contains('/') && !err.contains('\\'),
            "leaked path: {err}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── preview runtime resolution ──

    #[test]
    fn preview_error_helpers_are_safe_and_actionable() {
        assert!(preview_no_model_error().contains("Выберите модель в настройках"));
        assert!(!preview_no_model_error().contains("включите"));
        assert!(preview_not_loaded_error().contains("не загружена"));
        assert!(preview_not_loaded_error().contains("Загрузите модель в настройках"));
    }

    #[test]
    fn resolve_preview_runtime_distinguishes_missing_and_not_loaded() {
        assert!(resolve_preview_runtime(None, None)
            .unwrap_err()
            .contains("Выберите модель"));
        assert!(resolve_preview_runtime(Some(""), None)
            .unwrap_err()
            .contains("Выберите модель"));
        assert!(resolve_preview_runtime(Some("com.example.unknown"), None)
            .unwrap_err()
            .contains("Неизвестная модель"));

        let dir = std::env::temp_dir().join(format!(
            "ttsbard-preview-resolve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let slot = non_loadable_native_slot(dir.clone());

        assert!(
            resolve_preview_runtime(Some("com.example.preview"), Some(slot.clone()))
                .unwrap_err()
                .contains("не загружена")
        );

        slot.mark_ready();
        let ready = resolve_preview_runtime(Some("com.example.preview"), Some(slot)).unwrap();
        assert!(ready.is_ready());

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── load_on_start validation ──

    #[test]
    fn load_on_start_requires_known_selected_model() {
        assert!(validate_homograph_accentor_load_on_start(true, None, &ids()).is_err());
        assert!(
            validate_homograph_accentor_load_on_start(true, Some(String::new()), &ids()).is_err()
        );
        assert!(validate_homograph_accentor_load_on_start(
            true,
            Some("com.example.unknown".to_string()),
            &ids()
        )
        .is_err());
        assert!(validate_homograph_accentor_load_on_start(
            true,
            Some("com.example.a".to_string()),
            &ids()
        )
        .is_ok());
        assert!(validate_homograph_accentor_load_on_start(false, None, &ids()).is_ok());
    }

    // ── safe event payload helpers ──

    #[test]
    fn ruaccent_status_payload_is_safe() {
        let payload = ruaccent_status_payload("com.example.a", "loading");
        assert_eq!(payload.model_id, "com.example.a");
        assert_eq!(payload.status, "loading");
    }

    #[test]
    fn ruaccent_load_error_message_is_short_and_safe() {
        let message = ruaccent_load_error_message();
        assert!(!message.contains('/') && !message.contains('\\'));
        assert!(message.contains("RUAccent"));
        assert!(message.chars().count() < 120);
    }

    fn non_loadable_native_slot(pack_root: std::path::PathBuf) -> RuAccentRuntimeSlot {
        use crate::stress::packs::{RuAccentPackDescriptor, RuntimeCapability};
        let descriptor = RuAccentPackDescriptor {
            id: "com.example.preview".to_string(),
            display_name: "Preview".to_string(),
            runtime_version: "upstream".to_string(),
            pack_root,
            runtime_capability: RuntimeCapability::NativeTiny,
            omograph_model_id: "preview-model".to_string(),
        };
        RuAccentRuntimeSlot::new(descriptor)
    }

    // ── tabs flush acknowledgement matching ──

    #[test]
    fn tabs_flush_ack_accepts_matching_id() {
        assert_eq!(
            match_tabs_flush_ack(r#"{"request_id":"abc","ok":true}"#, "abc"),
            Some(true)
        );
        assert_eq!(
            match_tabs_flush_ack(r#"{"request_id":"abc","ok":false}"#, "abc"),
            Some(false)
        );
    }

    #[test]
    fn tabs_flush_ack_ignores_mismatched_id() {
        assert_eq!(
            match_tabs_flush_ack(r#"{"request_id":"other","ok":true}"#, "abc"),
            None
        );
    }

    #[test]
    fn tabs_flush_ack_ignores_malformed_payload() {
        assert_eq!(match_tabs_flush_ack("not json", "abc"), None);
        assert_eq!(match_tabs_flush_ack(r#"{"ok":true}"#, "abc"), None);
        assert_eq!(
            match_tabs_flush_ack(r#"{"request_id":123,"ok":true}"#, "abc"),
            None
        );
        assert_eq!(match_tabs_flush_ack(r#"{"request_id":"abc"}"#, "abc"), None);
    }

    // ── tabs flush bounded wait ──

    #[tokio::test]
    async fn tabs_flush_await_resolves_success_and_failure() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        tx.send(true).unwrap();
        assert!(matches!(
            await_tabs_flush_ack(rx, std::time::Duration::from_secs(1)).await,
            TabFlushAck::Success
        ));

        let (tx, rx) = tokio::sync::oneshot::channel();
        tx.send(false).unwrap();
        assert!(matches!(
            await_tabs_flush_ack(rx, std::time::Duration::from_secs(1)).await,
            TabFlushAck::Failure
        ));
    }

    #[tokio::test]
    async fn tabs_flush_await_times_out_when_sender_stays_silent() {
        // Keep the sender alive so the channel is not closed; the bounded wait
        // must elapse rather than resolving early.
        let (_tx, rx) = tokio::sync::oneshot::channel();
        assert!(matches!(
            await_tabs_flush_ack(rx, std::time::Duration::from_millis(10)).await,
            TabFlushAck::Timeout
        ));
    }

    #[tokio::test]
    async fn tabs_flush_await_detects_closed_channel() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        drop(tx);
        assert!(matches!(
            await_tabs_flush_ack(rx, std::time::Duration::from_secs(1)).await,
            TabFlushAck::Closed
        ));
    }
}
