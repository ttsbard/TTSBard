use crate::commands::playback::PlaybackState;
use crate::config::{Hotkey, HotkeySettings};
use crate::soundpanel::SoundPanelState;
use crate::state::{ActiveWindow, AppState};
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tracing::{debug, error, info, warn};

/// Shortcuts registered by this module (not by `OcrService`).
///
/// `tauri-plugin-global-shortcut` keeps a process-wide registry with no
/// enumeration API, so the plugin's best-effort "unregister all" cannot know
/// which combinations this module owns. Tracking them here lets
/// [`unregister_all_hotkeys`] really release them — including disabling an
/// individual hotkey at runtime — without touching the OCR capture shortcut,
/// which is registered and owned separately by [`crate::ocr::OcrService`].
fn registered_shortcuts() -> &'static Mutex<Vec<Shortcut>> {
    static REGISTERED: OnceLock<Mutex<Vec<Shortcut>>> = OnceLock::new();
    REGISTERED.get_or_init(|| Mutex::new(Vec::new()))
}

fn track_shortcut(shortcut: Shortcut) {
    let mut shortcuts = registered_shortcuts()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !shortcuts.contains(&shortcut) {
        shortcuts.push(shortcut);
    }
}

fn untrack_shortcut(shortcut: Shortcut) {
    let mut shortcuts = registered_shortcuts()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    shortcuts.retain(|tracked| *tracked != shortcut);
}

/// Whether `shortcut` is currently tracked as owned by this module.
///
/// The plugin registry is process-wide and also contains shortcuts registered by
/// [`crate::ocr::OcrService`], so this tracking list is the only way to tell an
/// owned registration from a foreign one.
#[cfg(target_os = "windows")]
fn is_tracked_shortcut(shortcut: Shortcut) -> bool {
    registered_shortcuts()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(&shortcut)
}

/// Whether a stored global hotkey should be registered with the OS.
fn should_register(hotkey: &Hotkey) -> bool {
    hotkey.enabled && !hotkey.key.is_empty()
}

/// Global binding names that this module registers with the OS on Windows.
///
/// These are the only bindings [`register_from_settings`] restores, and the only
/// ones a per-binding toggle may register or release. `ocr_capture` is owned by
/// [`crate::ocr::OcrService`], while `return_previous_window` and
/// `toggle_minimal_mode` are main-window-local keymap bindings, so none of them
/// belongs here.
#[cfg(target_os = "windows")]
const OS_REGISTERED_GLOBAL_NAMES: [&str; 6] = [
    "sound_panel",
    "main_window",
    "playback_pause",
    "playback_stop",
    "playback_repeat",
    "playback_control_window",
];

/// Whether this module owns the OS registration for `name`.
#[cfg(target_os = "windows")]
fn os_registered_binding(name: &str) -> bool {
    OS_REGISTERED_GLOBAL_NAMES.contains(&name)
}

/// Who owns a shortcut that the plugin reports as registered.
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutOwnership {
    /// The plugin confirms the shortcut is not registered at all.
    Absent,
    /// This module registered it.
    Owned,
    /// Another in-process owner (for example `OcrService`) registered it.
    Foreign,
}

/// Classify a shortcut from the plugin registry and this module's tracking.
///
/// A tracking entry without a live registration is stale and resolves to
/// [`ShortcutOwnership::Absent`], so callers clear it instead of acting on it.
#[cfg(target_os = "windows")]
fn shortcut_ownership(plugin_registered: bool, module_tracked: bool) -> ShortcutOwnership {
    match (plugin_registered, module_tracked) {
        (false, _) => ShortcutOwnership::Absent,
        (true, true) => ShortcutOwnership::Owned,
        (true, false) => ShortcutOwnership::Foreign,
    }
}

/// Handler for sound panel toggle
pub fn handle_sound_panel(app_state: AppState, app_handle: AppHandle) {
    debug!("Sound panel hotkey triggered");

    // Check if hotkey recording is in progress
    if app_state.is_hotkey_recording() {
        debug!(
            hotkey = "sound_panel",
            status = "recording",
            "Hotkey recording in progress - ignoring"
        );
        return;
    }

    // Проверяем, включены ли хоткеи в настройках
    if !app_state.is_hotkey_enabled() {
        debug!(
            hotkey = "sound_panel",
            status = "disabled",
            "Hotkey is disabled in settings"
        );
        return;
    }

    // Устанавливаем soundpanel как активное окно
    app_state.set_active_window(ActiveWindow::SoundPanel);

    // Показать звуковую панель
    debug!(
        window = "soundpanel",
        action = "showing",
        "Showing soundpanel"
    );

    // Emit event to show soundpanel window
    if let Some(sp_state) = app_handle.try_state::<SoundPanelState>() {
        sp_state.emit_event(crate::events::AppEvent::ShowSoundPanelWindow);
    } else {
        error!(
            window = "soundpanel",
            error = "state_not_found",
            "ERROR: SoundPanel state not found"
        );
    }
}

pub fn handle_playback_pause(app_handle: AppHandle) {
    if let Some(state) = app_handle.try_state::<PlaybackState>() {
        let pb = &state.inner().0;
        let st = pb.get_state();
        match st.status {
            crate::playback::PlaybackStatus::Playing => {
                let _ = pb.pause();
            }
            crate::playback::PlaybackStatus::Paused => {
                let _ = pb.resume();
            }
            _ => {}
        }
    }
}

pub fn handle_playback_stop(app_handle: AppHandle) {
    if let Some(state) = app_handle.try_state::<PlaybackState>() {
        let _ = state.inner().0.stop();
    }
}

pub fn handle_playback_repeat(app_handle: AppHandle) {
    if let Some(state) = app_handle.try_state::<PlaybackState>() {
        let _ = state.inner().0.repeat();
    }
}

pub fn handle_playback_control_window(app_handle: AppHandle) {
    if let Some(sp_state) = app_handle.try_state::<SoundPanelState>() {
        sp_state.emit_event(crate::events::AppEvent::ShowPlaybackControlWindow);
    }
}

/// Handler for main window focus
pub fn handle_main_window(app_handle: AppHandle) {
    debug!("Main window hotkey triggered");

    // Check if hotkey recording is in progress
    if let Some(app_state) = app_handle.try_state::<AppState>() {
        if app_state.is_hotkey_recording() {
            debug!(
                hotkey = "main_window",
                status = "recording",
                "Hotkey recording in progress - ignoring"
            );
            return;
        }
    }

    // Показать главное окно
    info!(
        hotkey = "main_window",
        window = "main",
        action = "showing",
        "Detected - showing main window"
    );

    if let Some(window) = app_handle.get_webview_window("main") {
        // Проверяем, если окно уже в фокусе - игнорируем
        if let Ok(true) = window.is_focused() {
            debug!(
                window = "main",
                status = "already_focused",
                "Main window already focused - ignoring"
            );
            return;
        }

        // Сохраняем HWND внешнего окна перед активацией
        if let Some(app_state) = app_handle.try_state::<AppState>() {
            let fg = crate::window::get_foreground_hwnd();
            if let Some(hwnd) = fg {
                if !crate::window::is_own_window(hwnd) {
                    *app_state.previous_foreground_hwnd.lock() = Some(hwnd);
                    debug!(
                        hwnd,
                        action = "saved_foreground",
                        "Saved external foreground HWND before activation"
                    );
                }
            }
        }

        // Показать окно если оно скрыто
        let _ = window.show();

        // Развернуть если минимизировано
        let _ = window.unminimize();

        // Установить always-on-top для фокуса
        let _ = window.set_always_on_top(true);

        // Сфокусировать окно
        let _ = window.set_focus();

        info!(
            window = "main",
            status = "shown_and_focused",
            note = "always_on_top_will_be_removed_on_focus_loss",
            "Main window shown and focused"
        );
    }
}

/// Unified action runner for Intercept bindings.
/// Dispatches a string action to the corresponding handler.
pub fn run_action(app_handle: &AppHandle, action: &str) {
    match action {
        "show_main_window" => {
            handle_main_window(app_handle.clone());
        }
        "show_soundpanel_window" => {
            if let Some(app_state) = app_handle.try_state::<AppState>() {
                let inner = app_state.inner().clone();
                handle_sound_panel(inner, app_handle.clone());
            }
        }
        "show_playback_control_window" => {
            handle_playback_control_window(app_handle.clone());
        }
        "playback_pause" => {
            handle_playback_pause(app_handle.clone());
        }
        "playback_stop" => {
            handle_playback_stop(app_handle.clone());
        }
        "playback_repeat" => {
            handle_playback_repeat(app_handle.clone());
        }
        "ocr_capture" => {
            if let Some(app_state) = app_handle.try_state::<AppState>() {
                crate::commands::ocr::handle_ocr_capture(app_handle, app_state.inner());
            }
        }
        other => {
            tracing::warn!(action = other, "Unknown intercept action");
        }
    }
}

/// Register a single hotkey with its handler
fn register_hotkey_internal(
    app_handle: &AppHandle,
    _app_state: &AppState,
    name: &str,
    shortcut: Shortcut,
    handler: impl Fn(AppHandle) + Send + Sync + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    let global_shortcut = app_handle.global_shortcut();

    // Unregister if already registered
    if global_shortcut.is_registered(shortcut) {
        debug!(hotkey = name, shortcut = ?shortcut, action = "unregistering_existing", "Shortcut already registered, unregistering first");
        let _ = global_shortcut.unregister(shortcut);
        untrack_shortcut(shortcut);
    }

    // Register the handler
    let app_handle_clone = app_handle.clone();
    global_shortcut.on_shortcut(shortcut, move |_app, _shortcut, event| {
        if event.state != ShortcutState::Pressed {
            return;
        }
        handler(app_handle_clone.clone());
    })?;
    track_shortcut(shortcut);

    debug!(hotkey = name, shortcut = ?shortcut, "Hotkey registered successfully");
    Ok(())
}

/// Register one module-owned global binding with its existing handler.
///
/// This is the single place that maps a binding name to its handler, so startup
/// registration ([`register_from_settings`]) and the per-binding enable command
/// always install the same behavior.
#[cfg(target_os = "windows")]
fn register_binding_by_name(
    app_handle: &AppHandle,
    app_state: &AppState,
    name: &str,
    shortcut: Shortcut,
) -> Result<(), Box<dyn std::error::Error>> {
    match name {
        "sound_panel" => {
            let app_state_sp = app_state.clone();
            register_hotkey_internal(app_handle, app_state, name, shortcut, move |app_handle| {
                handle_sound_panel(app_state_sp.clone(), app_handle);
            })
        }
        "main_window" => {
            register_hotkey_internal(app_handle, app_state, name, shortcut, handle_main_window)
        }
        "playback_pause" => {
            register_hotkey_internal(app_handle, app_state, name, shortcut, handle_playback_pause)
        }
        "playback_stop" => {
            register_hotkey_internal(app_handle, app_state, name, shortcut, handle_playback_stop)
        }
        "playback_repeat" => register_hotkey_internal(
            app_handle,
            app_state,
            name,
            shortcut,
            handle_playback_repeat,
        ),
        "playback_control_window" => register_hotkey_internal(
            app_handle,
            app_state,
            name,
            shortcut,
            handle_playback_control_window,
        ),
        other => Err(format!("Hotkey '{}' is not registered globally", other).into()),
    }
}

/// Register hotkeys from settings
#[cfg(target_os = "windows")]
pub fn register_from_settings(
    hotkey_settings: &HotkeySettings,
    app_state: &AppState,
    app_handle: &AppHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Registering hotkeys from settings");

    for &name in &OS_REGISTERED_GLOBAL_NAMES {
        let hotkey = hotkey_settings
            .get_global_by_name(name)
            .expect("listed OS-registered binding must exist");
        if should_register(hotkey) {
            let shortcut = hotkey.to_shortcut()?;
            register_binding_by_name(app_handle, app_state, name, shortcut)?;
        } else {
            debug!(
                hotkey = name,
                status = "disabled",
                "Skipping disabled hotkey"
            );
        }
    }

    info!(
        main_window = %hotkey_settings.main_window.format_display(),
        main_window_enabled = hotkey_settings.main_window.enabled,
        sound_panel = %hotkey_settings.sound_panel.format_display(),
        sound_panel_enabled = hotkey_settings.sound_panel.enabled,
        "Hotkeys registered successfully"
    );

    Ok(())
}

/// Unregister every shortcut tracked as owned by this module.
///
/// Only shortcuts actually registered by [`register_hotkey_internal`] are
/// released: the OCR capture shortcut is owned by `OcrService` and therefore is
/// never in this list, so recording a new global hotkey cannot break OCR.
pub fn unregister_all_hotkeys(app_handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    info!("Unregistering all hotkeys");

    let global_shortcut = app_handle.global_shortcut();
    let shortcuts: Vec<Shortcut> = {
        let mut tracked = registered_shortcuts()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        tracked.drain(..).collect()
    };

    for shortcut in shortcuts {
        if global_shortcut.is_registered(shortcut) {
            if let Err(error) = global_shortcut.unregister(shortcut) {
                warn!(shortcut = ?shortcut, error = %error, "Failed to unregister hotkey");
            }
        } else {
            debug!(shortcut = ?shortcut, "Hotkey was already unregistered");
        }
    }

    info!("All tracked hotkeys unregistered");
    Ok(())
}

/// Apply the OS registration state of one global binding.
///
/// Returns `Ok(true)` when the runtime registration changed and `Ok(false)`
/// when the switch needs no OS work (an empty combination, a main-window-local
/// name, or a non-Windows build, where this module registers nothing).
///
/// Only the requested binding is touched: unrelated registrations, including
/// the OCR-owned capture shortcut, are never released by a single toggle.
/// Ownership is decided by this module's tracking rather than by the plugin's
/// process-wide registry: enabling a combination that another in-app owner
/// already registered is an in-app conflict, and disabling it preserves that
/// foreign registration. Registration and unregistration failures are surfaced
/// before the caller persists anything, so the stored switch keeps describing
/// the previous state.
#[cfg(target_os = "windows")]
pub fn set_global_binding_runtime_state(
    app_handle: &AppHandle,
    app_state: &AppState,
    name: &str,
    hotkey: &Hotkey,
    enabled: bool,
) -> Result<bool, String> {
    if !os_registered_binding(name) || hotkey.key.is_empty() {
        return Ok(false);
    }

    let shortcut = hotkey.to_shortcut()?;
    let global_shortcut = app_handle.global_shortcut();
    let ownership = shortcut_ownership(
        global_shortcut.is_registered(shortcut),
        is_tracked_shortcut(shortcut),
    );

    if enabled {
        match ownership {
            ShortcutOwnership::Owned => {
                // Owned and already registered: keep the existing registration
                // (and handler) so re-enabling stays idempotent.
                debug!(hotkey = name, shortcut = ?shortcut, "Hotkey was already registered");
                Ok(false)
            }
            ShortcutOwnership::Foreign => {
                // Another in-process owner holds this combination. Adopting it
                // would later let a disable release a registration this module
                // does not own, so reject the enable and never track it.
                debug!(
                    hotkey = name,
                    shortcut = ?shortcut,
                    "Shortcut is registered by another in-app owner"
                );
                Err("Этот хоткей уже занят другим компонентом приложения".to_string())
            }
            ShortcutOwnership::Absent => {
                // The plugin confirms the shortcut is absent, so any tracking
                // entry is stale and must not survive the fresh registration.
                untrack_shortcut(shortcut);
                register_binding_by_name(app_handle, app_state, name, shortcut)
                    .map_err(|error| format!("Failed to register shortcut: {}", error))?;
                debug!(hotkey = name, shortcut = ?shortcut, "Hotkey enabled and registered");
                Ok(true)
            }
        }
    } else {
        match ownership {
            ShortcutOwnership::Owned => {
                global_shortcut
                    .unregister(shortcut)
                    .map_err(|error| format!("Failed to unregister shortcut: {}", error))?;
                untrack_shortcut(shortcut);
                debug!(hotkey = name, shortcut = ?shortcut, "Hotkey disabled and unregistered");
                Ok(true)
            }
            ShortcutOwnership::Foreign => {
                // Preserve the foreign registration: this module must not
                // release a shortcut it does not own. The caller can still
                // persist the disabled switch for its own binding.
                debug!(
                    hotkey = name,
                    shortcut = ?shortcut,
                    "Shortcut belongs to another in-app owner; preserving it"
                );
                Ok(false)
            }
            ShortcutOwnership::Absent => {
                // Clear stale ownership now that the plugin confirms absence.
                untrack_shortcut(shortcut);
                debug!(hotkey = name, shortcut = ?shortcut, "Hotkey was already unregistered");
                Ok(false)
            }
        }
    }
}

/// Run one per-binding switch as an ordered transaction.
///
/// The runtime change is applied first; the switch is persisted only after that
/// succeeds, so an OS rejection leaves the stored flag untouched. If
/// persistence then fails, the runtime change is rolled back. `apply` reports
/// whether the runtime changed at all, so a no-op switch never triggers a
/// rollback that would invent a registration.
pub(crate) async fn apply_binding_toggle<A, P, R>(
    apply: A,
    persist: P,
    rollback: R,
) -> Result<(), String>
where
    A: FnOnce() -> Result<bool, String>,
    P: Future<Output = Result<(), String>>,
    R: FnOnce() -> Result<(), String>,
{
    let runtime_changed = apply()?;
    match persist.await {
        Ok(()) => Ok(()),
        Err(persist_error) if !runtime_changed => {
            Err(format!("Failed to persist hotkey state: {}", persist_error))
        }
        Err(persist_error) => match rollback() {
            Ok(()) => Err(format!(
                "Failed to persist hotkey state: {}; the shortcut was restored to its previous state",
                persist_error
            )),
            Err(rollback_error) => Err(format!(
                "Failed to persist hotkey state: {}; restoring the previous shortcut state failed: {}",
                persist_error, rollback_error
            )),
        },
    }
}

/// Initialize hotkeys from settings
///
/// This function reads hotkey settings from the config and registers them.
pub fn initialize_hotkeys(
    _hwnd: isize,
    app_state: AppState,
    app_handle: AppHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        hotkey = "initializing",
        "Initializing global shortcuts from settings"
    );

    // Load hotkey settings
    let settings_manager = crate::config::SettingsManager::new()
        .map_err(|e| format!("Failed to create settings manager: {}", e))?;

    let settings = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?;

    // Unregister any existing hotkeys
    unregister_all_hotkeys(&app_handle)?;

    // Register hotkeys from settings
    register_from_settings(&settings.hotkeys, &app_state, &app_handle)?;

    info!(
        hotkey = "registration_complete",
        "Global shortcuts registered successfully"
    );
    info!(hotkey = %settings.hotkeys.main_window.format_display(), description = "show_main_window_with_always_on_top");
    info!(hotkey = %settings.hotkeys.sound_panel.format_display(), description = "show_soundpanel");

    Ok(())
}

/// Re-register hotkeys with new settings
///
/// This function is called when hotkey settings are changed.
/// It unregisters all hotkeys and re-registers them with the new settings.
pub fn reregister_hotkeys(app_handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    info!("Re-registering hotkeys with new settings");

    // Get app state
    let app_state = app_handle.state::<AppState>();
    let app_state_inner = app_state.inner().clone();

    // Load new hotkey settings
    let settings_manager = crate::config::SettingsManager::new()
        .map_err(|e| format!("Failed to create settings manager: {}", e))?;

    let settings = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?;

    // Unregister all hotkeys
    unregister_all_hotkeys(app_handle)?;

    // Register hotkeys from new settings
    register_from_settings(&settings.hotkeys, &app_state_inner, app_handle)?;

    info!("Hotkeys re-registered successfully");
    Ok(())
}

/// Stubs для не-Windows платформ (плагин работает везде, но для совместимости оставим)
#[cfg(not(target_os = "windows"))]
pub fn initialize_hotkeys(
    _hwnd: isize,
    _app_state: AppState,
    _app_handle: AppHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        platform = "non-windows",
        status = "supported",
        "Global shortcuts are supported on all platforms"
    );
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn unregister_all_hotkeys(_app_handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn reregister_hotkeys(_app_handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn register_from_settings(
    _hotkey_settings: &HotkeySettings,
    _app_state: &AppState,
    _app_handle: &AppHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

/// No-op outside Windows, matching the [`register_from_settings`] stub above:
/// this module never registers OS shortcuts there, so a per-binding toggle only
/// persists the stored switch.
#[cfg(not(target_os = "windows"))]
pub fn set_global_binding_runtime_state(
    _app_handle: &AppHandle,
    _app_state: &AppState,
    _name: &str,
    _hotkey: &Hotkey,
    _enabled: bool,
) -> Result<bool, String> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HotkeyModifier;
    use std::cell::RefCell;

    fn hotkey(key: &str, enabled: bool) -> Hotkey {
        Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: key.to_string(),
            enabled,
        }
    }

    #[test]
    fn should_register_only_enabled_non_empty_global_bindings() {
        assert!(should_register(&hotkey("F3", true)));
        assert!(!should_register(&hotkey("F3", false)));
        assert!(!should_register(&hotkey("", true)));
        assert!(!should_register(&hotkey("", false)));
    }

    #[test]
    fn tracked_shortcuts_are_removed_on_untrack() {
        let shortcut = Shortcut::new(None, tauri_plugin_global_shortcut::Code::F24);
        track_shortcut(shortcut);
        track_shortcut(shortcut);
        assert_eq!(
            registered_shortcuts()
                .lock()
                .unwrap()
                .iter()
                .filter(|tracked| **tracked == shortcut)
                .count(),
            1,
            "tracking the same shortcut twice must not duplicate it"
        );

        untrack_shortcut(shortcut);
        assert!(!registered_shortcuts().lock().unwrap().contains(&shortcut));
    }

    /// Module tracking, not the plugin's process-wide registry, distinguishes
    /// an owned shortcut from one registered by another in-app owner.
    #[cfg(target_os = "windows")]
    #[test]
    fn shortcut_ownership_distinguishes_owned_from_foreign() {
        assert_eq!(shortcut_ownership(false, false), ShortcutOwnership::Absent);
        assert_eq!(
            shortcut_ownership(false, true),
            ShortcutOwnership::Absent,
            "stale tracking must resolve to absent"
        );
        assert_eq!(shortcut_ownership(true, true), ShortcutOwnership::Owned);
        assert_eq!(
            shortcut_ownership(true, false),
            ShortcutOwnership::Foreign,
            "an untracked registration belongs to another owner"
        );

        let shortcut = Shortcut::new(None, tauri_plugin_global_shortcut::Code::F23);
        assert!(!is_tracked_shortcut(shortcut));
        track_shortcut(shortcut);
        assert!(is_tracked_shortcut(shortcut));
        untrack_shortcut(shortcut);
        assert!(!is_tracked_shortcut(shortcut));
    }

    /// The transaction must not persist when applying the runtime change fails:
    /// the stored switch keeps its previous value and no rollback runs.
    #[tokio::test]
    async fn binding_toggle_keeps_previous_state_when_apply_fails() {
        let calls = RefCell::new(Vec::new());

        let result = apply_binding_toggle(
            || {
                calls.borrow_mut().push("apply");
                Err("register rejected".to_string())
            },
            async {
                calls.borrow_mut().push("persist");
                Ok(())
            },
            || {
                calls.borrow_mut().push("rollback");
                Ok(())
            },
        )
        .await;

        assert!(result.unwrap_err().contains("register rejected"));
        assert_eq!(*calls.borrow(), vec!["apply"]);
    }

    /// A successful runtime change is persisted only afterwards, and a
    /// persistence failure rolls the runtime back instead of leaving the
    /// switch and the registration out of sync.
    #[tokio::test]
    async fn binding_toggle_rolls_back_when_persist_fails() {
        let calls = RefCell::new(Vec::new());

        let result = apply_binding_toggle(
            || {
                calls.borrow_mut().push("apply");
                Ok(true)
            },
            async {
                calls.borrow_mut().push("persist");
                Err("disk full".to_string())
            },
            || {
                calls.borrow_mut().push("rollback");
                Ok(())
            },
        )
        .await;

        let error = result.unwrap_err();
        assert!(error.contains("disk full"), "unexpected: {error}");
        assert!(error.contains("restored"), "unexpected: {error}");
        assert_eq!(*calls.borrow(), vec!["apply", "persist", "rollback"]);
    }

    /// A failed rollback is surfaced together with the persistence error, so a
    /// possibly still-active shortcut is never reported as a clean success.
    #[tokio::test]
    async fn binding_toggle_reports_rollback_failure() {
        let calls = RefCell::new(Vec::new());

        let result = apply_binding_toggle(
            || {
                calls.borrow_mut().push("apply");
                Ok(true)
            },
            async {
                calls.borrow_mut().push("persist");
                Err("disk full".to_string())
            },
            || {
                calls.borrow_mut().push("rollback");
                Err("unregister rejected".to_string())
            },
        )
        .await;

        let error = result.unwrap_err();
        assert!(error.contains("disk full"), "unexpected: {error}");
        assert!(error.contains("unregister rejected"), "unexpected: {error}");
        assert_eq!(*calls.borrow(), vec!["apply", "persist", "rollback"]);
    }

    /// A switch that needed no runtime change (empty or window-local binding)
    /// must not run a rollback: there is no registration to restore.
    #[tokio::test]
    async fn binding_toggle_skips_rollback_without_runtime_change() {
        let calls = RefCell::new(Vec::new());

        let result = apply_binding_toggle(
            || {
                calls.borrow_mut().push("apply");
                Ok(false)
            },
            async {
                calls.borrow_mut().push("persist");
                Err("disk full".to_string())
            },
            || {
                calls.borrow_mut().push("rollback");
                Ok(())
            },
        )
        .await;

        assert!(result.unwrap_err().contains("disk full"));
        assert_eq!(*calls.borrow(), vec!["apply", "persist"]);
    }

    /// The happy path persists after the runtime changed and never rolls back.
    #[tokio::test]
    async fn binding_toggle_persists_after_runtime_change() {
        let calls = RefCell::new(Vec::new());

        let result = apply_binding_toggle(
            || {
                calls.borrow_mut().push("apply");
                Ok(true)
            },
            async {
                calls.borrow_mut().push("persist");
                Ok(())
            },
            || {
                calls.borrow_mut().push("rollback");
                Ok(())
            },
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(*calls.borrow(), vec!["apply", "persist"]);
    }

    /// Only the six module-owned global shortcuts are registered with the OS;
    /// OCR and the main-window-local bindings must never be released by a
    /// per-binding toggle.
    #[cfg(target_os = "windows")]
    #[test]
    fn os_registered_binding_covers_only_module_shortcuts() {
        for name in OS_REGISTERED_GLOBAL_NAMES {
            assert!(os_registered_binding(name), "{} must be registered", name);
        }
        assert!(!os_registered_binding("ocr_capture"));
        assert!(!os_registered_binding("return_previous_window"));
        assert!(!os_registered_binding("toggle_minimal_mode"));
        assert!(!os_registered_binding("bogus"));
    }
}
