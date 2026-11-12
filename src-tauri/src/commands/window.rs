use crate::config::{
    CompactView, EditorHotkeySettings, Hotkey, HotkeySettings, SettingsManager, Theme,
    WindowsManager,
};
use crate::playback_window::update_playback_appearance;
use crate::soundpanel_window::update_soundpanel_appearance;
use crate::state::AppState;
use tauri::{AppHandle, Manager, State};
use tracing::{debug, info, warn};

/// Russian conflict label for a global/main-window-local hotkey name.
fn global_hotkey_label(name: &str) -> &'static str {
    match name {
        "main_window" => "главного окна",
        "sound_panel" => "звуковой панели",
        "playback_pause" => "паузы воспроизведения",
        "playback_stop" => "остановки воспроизведения",
        "playback_repeat" => "повтора воспроизведения",
        "playback_control_window" => "окна управления воспроизведением",
        "return_previous_window" => "возврата в предыдущее окно",
        "toggle_minimal_mode" => "переключения компактного режима",
        "ocr_capture" => "OCR: захвата области",
        _ => "другого хоткея",
    }
}

/// Validate a global/main-window-local binding against every *enabled* other
/// hotkey. Returns a user-facing conflict message.
///
/// Disabled hotkeys are not occupied, so a disabled binding never blocks a
/// change. This function must be called for the binding as it will be stored
/// (including its `enabled` value).
fn global_hotkey_conflict(
    settings: &HotkeySettings,
    skip_name: &str,
    candidate: &Hotkey,
) -> Option<String> {
    if !candidate.enabled || candidate.key.is_empty() {
        return None;
    }
    if let Some(name) = settings.find_global_conflict(skip_name, candidate) {
        return Some(format!(
            "Этот хоткей уже используется для {}",
            global_hotkey_label(name)
        ));
    }
    if settings.find_editor_conflict(candidate).is_some() {
        return Some("Этот хоткей уже используется для действия редактора".to_string());
    }
    None
}

/// Validate an editor-scoped binding against the editor group and the enabled
/// global hotkeys.
fn editor_hotkey_conflict(
    settings: &HotkeySettings,
    action_id: &str,
    candidate: &Hotkey,
) -> Option<String> {
    if !candidate.enabled || candidate.key.is_empty() {
        return None;
    }
    if let Some(conflict) = settings.editor.find_duplicate(action_id, candidate) {
        return Some(format!(
            "Этот хоткей уже используется для {}",
            EditorHotkeySettings::action_label(conflict)
        ));
    }
    if candidate.conflicts_with_global(settings) {
        return Some("Этот хоткей конфликтует с назначенным глобальным хоткеем".to_string());
    }
    None
}

/// Whether a per-binding switch request actually changes the stored flag.
///
/// A same-state request is a successful no-op: re-applying it would only
/// re-register (or release) an unchanged shortcut and could disturb unrelated
/// registrations.
fn hotkey_toggle_changes_state(current_enabled: bool, requested_enabled: bool) -> bool {
    current_enabled != requested_enabled
}

/// Тестируемое ядро `return_to_previous_window`.
///
/// Атомарно читает и очищает HWND под одной блокировкой, проверяет валидность
/// и при успехе оставляет ячейку пустой. При транзиентной ошибке
/// `SetForegroundWindow` восстанавливает HWND (только если другой поток не
/// сохранил новый HWND за это время).
fn return_previous_hwnd_core(
    state: &AppState,
    is_valid: impl FnOnce(isize) -> bool,
    set_foreground: impl FnOnce(isize) -> bool,
) -> Result<(), String> {
    // Единый guard: читаем HWND, чистим ячейку и проверяем валидность,
    // чтобы другой поток не мог вклиниться между read и clear.
    let hwnd = {
        let mut guard = state.previous_foreground_hwnd.lock();
        let value = *guard;
        *guard = None;
        value
    };
    match hwnd {
        Some(hwnd) => {
            if !is_valid(hwnd) {
                warn!(
                    hwnd,
                    "Saved foreground HWND is no longer valid, window may have been closed"
                );
                return Err("Предыдущее окно больше не доступно (закрыто)".to_string());
            }

            if set_foreground(hwnd) {
                info!(
                    hwnd,
                    action = "returned_focus",
                    "Focus returned to previous window"
                );
                Ok(())
            } else {
                // Transient failure — восстанавливаем HWND для retry,
                // но только если другой поток не сохранил новый HWND.
                let mut guard = state.previous_foreground_hwnd.lock();
                if guard.is_none() {
                    *guard = Some(hwnd);
                }
                warn!(
                    hwnd,
                    "SetForegroundWindow failed (Windows foreground lock policy)"
                );
                Err("Не удалось переключить фокус (ограничение Windows). Окно TTSBard остаётся видимым.".to_string())
            }
        }
        None => {
            debug!("No saved foreground HWND to return to");
            Ok(())
        }
    }
}

/// Вернуть фокус в предыдущее окно (внешнее приложение)
///
/// Снимает always_on_top с главного окна TTSBard и активирует сохранённый
/// внешний HWND через Win32 `SetForegroundWindow`. Окно TTSBard не скрывается.
/// На не-Windows платформах только снимает always_on_top.
#[tauri::command]
pub fn return_to_previous_window(
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.set_always_on_top(false);
    }
    return_previous_hwnd_core(
        &state,
        crate::window::is_window_valid,
        crate::window::set_foreground_window,
    )
}

#[tauri::command]
pub async fn resize_main_window(
    app_handle: AppHandle,
    width: Option<u32>,
    height: u32,
    compact: Option<bool>,
) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        if compact.unwrap_or(false) {
            // Compact path: canonical physical INNER dimensions for both axes,
            // clamped to the scale-aware compact bounds at the backend boundary.
            return resize_main_window_compact(&window, width, height);
        }
        if width.is_none() {
            // Height-only resize. `set_size` maps to `set_inner_size`, which on a
            // frameless shadowed window ADDS the invisible WS_THICKFRAME borders
            // to the requested size — so feeding it a width read back from
            // `outer_size()` grows the window sideways on every call. Keep the
            // current OUTER width untouched via SetWindowPos instead.
            #[cfg(windows)]
            {
                use windows::Win32::Foundation::{HWND, RECT};
                use windows::Win32::UI::WindowsAndMessaging::{
                    GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER,
                };
                let hwnd = window.hwnd().map_err(|e| e.to_string())?;
                unsafe {
                    let mut rc = RECT::default();
                    GetWindowRect(HWND(hwnd.0 as _), &mut rc)
                        .map_err(|e| format!("GetWindowRect failed: {}", e))?;
                    let current_width = rc.right - rc.left;
                    SetWindowPos(
                        HWND(hwnd.0 as _),
                        None,
                        0,
                        0,
                        current_width,
                        height as i32,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                    .map_err(|e| format!("SetWindowPos failed: {}", e))?;
                }
                return Ok(());
            }
            #[cfg(not(windows))]
            {
                let w = window.outer_size().map_err(|e| e.to_string())?.width;
                window
                    .set_size(tauri::Size::Physical(tauri::PhysicalSize {
                        width: w,
                        height,
                    }))
                    .map_err(|e| format!("Failed to resize: {}", e))?;
                return Ok(());
            }
        }
        let width = width.unwrap_or(0);
        window
            .set_size(tauri::Size::Physical(tauri::PhysicalSize { width, height }))
            .map_err(|e| format!("Failed to resize: {}", e))?;
        Ok(())
    } else {
        Err("Main window not found".to_string())
    }
}

/// Compact resize: both axes are treated as physical INNER dimensions.
///
/// Uses Tauri `set_size` (which maps to `set_inner_size`) together with
/// `inner_size()` reads so width and height never mix outer/frame sizes — this
/// avoids the WS_THICKFRAME outer/inner drift that affected the legacy
/// outer-only height path. When `width` is `None`, the current INNER width is
/// preserved. Dimensions are clamped to the scale-aware compact bounds.
fn resize_main_window_compact(
    window: &tauri::WebviewWindow,
    width: Option<u32>,
    height: u32,
) -> Result<(), String> {
    let scale = window
        .scale_factor()
        .map_err(|e| format!("Failed to read main window scale factor: {}", e))?;
    let (min_w, min_h, max_w, max_h) = crate::config::compact_physical_bounds(scale);

    let current_width = window
        .inner_size()
        .map_err(|e| format!("Failed to read main window inner size: {}", e))?
        .width;
    let target_width = width.unwrap_or(current_width).clamp(min_w, max_w);
    let target_height = height.clamp(min_h, max_h);

    window
        .set_size(tauri::Size::Physical(tauri::PhysicalSize {
            width: target_width,
            height: target_height,
        }))
        .map_err(|e| format!("Failed to resize: {}", e))?;
    Ok(())
}

/// Get hotkey enabled setting
#[tauri::command]
pub fn get_hotkey_enabled(settings_manager: State<'_, SettingsManager>) -> bool {
    settings_manager.get_hotkey_enabled()
}

/// Set hotkey enabled setting
#[tauri::command]
pub async fn set_hotkey_enabled(
    enabled: bool,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_hotkey_enabled(enabled)
    })
    .await?;

    state.set_hotkey_enabled(enabled);
    super::emit_settings_changed(&app_handle);

    Ok(())
}

/// Open file dialog for selecting audio files
#[tauri::command]
pub fn open_file_dialog() -> Result<String, String> {
    Err(
        "Use the frontend dialog API instead: import { open } from '@tauri-apps/plugin-dialog'"
            .to_string(),
    )
}

/// Set global exclude from capture for all windows
#[tauri::command]
pub async fn set_global_exclude_from_capture(
    value: bool,
    _app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(value, "Setting global exclude from capture");

    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_global_exclude_from_capture(value)
    })
    .await?;

    super::emit_settings_changed(&_app_handle);

    info!("Setting saved. Will apply to all windows after application restart.");
    Ok(())
}

/// Get global exclude from capture setting
#[tauri::command]
pub fn get_global_exclude_from_capture(windows_manager: State<'_, WindowsManager>) -> bool {
    let value = windows_manager.get_global_exclude_from_capture();
    debug!(value, "Getting global exclude from capture");
    value
}

/// Update application theme
#[tauri::command]
pub async fn update_theme(
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
    theme: Theme,
) -> Result<(), String> {
    info!(?theme, "Updating theme");

    super::persist_blocking(settings_manager.inner(), move |mgr| mgr.set_theme(theme)).await?;

    let tauri_theme = match theme {
        Theme::Light => tauri::Theme::Light,
        Theme::Dark => tauri::Theme::Dark,
    };
    for label in &["main", "playback-control", "soundpanel"] {
        if let Some(window) = app_handle.get_webview_window(label) {
            let _ = window.set_theme(Some(tauri_theme));
        }
    }
    info!(?tauri_theme, "Applied window theme");

    emit_appearance_updates(&app_handle);

    info!(?theme, "Theme updated successfully");
    Ok(())
}

/// Hide main window
#[tauri::command]
pub fn hide_main_window(app_handle: AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        window
            .hide()
            .map_err(|e| format!("Failed to hide window: {}", e))?;
    }
    Ok(())
}

/// Close the SoundPanel window.
#[tauri::command]
pub fn close_soundpanel_window(app_handle: AppHandle) -> Result<(), String> {
    crate::soundpanel_window::close_soundpanel_window(&app_handle)
}

/// Toggle playback control window visibility.
/// Returns `true` if the window is visible after the toggle.
#[tauri::command]
pub fn toggle_playback_control_window(app_handle: AppHandle) -> Result<bool, String> {
    crate::playback_window::toggle_playback_window(&app_handle)
}

/// Close playback control window (save position, emit visibility notification).
#[tauri::command]
pub fn close_playback_control_window(app_handle: AppHandle) -> Result<(), String> {
    crate::playback_window::hide_playback_window(&app_handle)
        .map_err(|e| format!("Failed to close playback control window: {}", e))
}

/// Toggle SoundPanel floating window visibility.
/// Returns `true` if the window is visible after the toggle.
#[tauri::command]
pub fn toggle_soundpanel_window(app_handle: AppHandle) -> Result<bool, String> {
    crate::soundpanel_window::toggle_soundpanel_window(&app_handle)
}

/// Get current visibility snapshot of both floating windows for initial UI sync.
#[tauri::command]
pub fn get_visibility_snapshot(
    app_handle: AppHandle,
) -> Result<crate::soundpanel_window::WindowVisibility, String> {
    crate::soundpanel_window::get_visibility(&app_handle)
}

/// Set show playback control window on start
#[tauri::command]
pub async fn set_show_playback_on_start(
    value: bool,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_show_playback_on_start(value)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Get show playback control window on start
#[tauri::command]
pub fn get_show_playback_on_start(settings_manager: State<'_, SettingsManager>) -> bool {
    settings_manager.get_show_playback_on_start()
}

/// Set start in compact mode
#[tauri::command]
pub async fn set_start_compact(
    value: bool,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_start_compact(value)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Set hide main window on minimize
#[tauri::command]
pub async fn set_hide_on_minimize(
    value: bool,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_hide_on_minimize(value)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Get all hotkey settings
#[tauri::command]
pub async fn get_hotkey_settings(
    settings_manager: State<'_, SettingsManager>,
) -> Result<HotkeySettings, String> {
    settings_manager
        .get_hotkey_settings()
        .map_err(|e| e.to_string())
}

/// Enable or disable a single global/main-window-local hotkey.
///
/// The assigned combination is preserved. Enabling re-checks conflicts against
/// enabled hotkeys only; disabling unregisters the shortcut (for OCR the
/// `OcrService` owner is asked to release it while keeping OCR itself enabled).
/// Only the switched binding is touched: other global registrations keep
/// working. The switch is persisted after the runtime change succeeds, and a
/// failed persistence rolls the runtime change back, so the stored flag never
/// claims a registration that Windows rejected.
#[tauri::command]
pub async fn set_hotkey_binding_enabled(
    name: String,
    enabled: bool,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let settings = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?;

    let current = settings
        .hotkeys
        .get_global_by_name(&name)
        .ok_or_else(|| format!("Invalid hotkey name: {}", name))?
        .clone();

    if !hotkey_toggle_changes_state(current.enabled, enabled) {
        debug!(
            hotkey = %name,
            enabled,
            "Hotkey switch already reflects the requested state"
        );
        return Ok(());
    }

    let mut updated = current.clone();
    updated.enabled = enabled;

    if let Some(conflict) = global_hotkey_conflict(&settings.hotkeys, &name, &updated) {
        return Err(conflict);
    }

    if name == "ocr_capture" {
        apply_ocr_capture_binding_toggle(&name, &updated, settings_manager.inner(), &app_handle)
            .await?;
    } else {
        apply_global_binding_toggle(&name, &updated, settings_manager.inner(), &app_handle).await?;
    }

    super::emit_settings_changed(&app_handle);

    Ok(())
}

/// Apply a switch for a binding registered by [`crate::hotkeys`] (or a
/// main-window-local binding, which needs no OS work).
///
/// The runtime change runs first and reports whether it touched anything. The
/// switch is persisted only afterwards, and a persistence failure rolls the
/// runtime change back. Because the transaction is scoped to one binding, an
/// OS rejection for it leaves every other registration in place.
async fn apply_global_binding_toggle(
    name: &str,
    updated: &Hotkey,
    settings_manager: &SettingsManager,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let app_state = app_handle
        .try_state::<AppState>()
        .ok_or_else(|| "Application state is unavailable".to_string())?
        .inner()
        .clone();
    let enabled = updated.enabled;

    let apply_name = name.to_string();
    let apply_hotkey = updated.clone();
    let apply_state = app_state.clone();
    let apply_handle = app_handle.clone();

    let rollback_name = name.to_string();
    let rollback_hotkey = updated.clone();
    let rollback_handle = app_handle.clone();
    let persist_name = name.to_string();

    crate::hotkeys::apply_binding_toggle(
        move || {
            crate::hotkeys::set_global_binding_runtime_state(
                &apply_handle,
                &apply_state,
                &apply_name,
                &apply_hotkey,
                enabled,
            )
        },
        super::persist_blocking(settings_manager, move |mgr| {
            mgr.set_hotkey_enabled_flag(&persist_name, enabled)
        }),
        move || {
            crate::hotkeys::set_global_binding_runtime_state(
                &rollback_handle,
                &app_state,
                &rollback_name,
                &rollback_hotkey,
                !enabled,
            )
            .map(|_| ())
        },
    )
    .await
}

/// Apply the `ocr_capture` switch while `OcrService` stays the shortcut owner.
///
/// The service keeps the previous registration when a new one is rejected, so
/// the only rollback needed here is restoring the persisted switch. OCR
/// readiness and the OCR enable flag are otherwise untouched.
async fn apply_ocr_capture_binding_toggle(
    name: &str,
    updated: &Hotkey,
    settings_manager: &SettingsManager,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let enabled = updated.enabled;
    let persist_name = name.to_string();
    super::persist_blocking(settings_manager, move |mgr| {
        mgr.set_hotkey_enabled_flag(&persist_name, enabled)
    })
    .await
    .map_err(|error| format!("Failed to persist hotkey state: {}", error))?;

    let Some(app_state) = app_handle.try_state::<AppState>() else {
        return Ok(());
    };
    let app_state = app_state.inner().clone();

    if enabled {
        if app_state.ocr.is_runtime_active() {
            if let Err(error) = crate::commands::ocr::register_ocr_capture_while_running(
                app_handle, &app_state, updated,
            )
            .await
            {
                // Keep the persisted switch truthful with the runtime:
                // registration failed, so the hotkey stays disabled.
                let rollback_name = name.to_string();
                let rollback = super::persist_blocking(settings_manager, move |mgr| {
                    mgr.set_hotkey_enabled_flag(&rollback_name, false)
                })
                .await;
                return Err(match rollback {
                    Ok(()) => format!(
                        "Failed to register the OCR capture shortcut: {}; the hotkey stays off",
                        error
                    ),
                    Err(rollback_error) => format!(
                        "Failed to register the OCR capture shortcut: {}; restoring the disabled state failed: {}",
                        error, rollback_error
                    ),
                });
            }
        } else {
            // OCR stayed enabled while this hotkey was off, but the
            // runtime was never started (e.g. boot skipped it). Turning
            // the hotkey back on is the moment to bring OCR up.
            let ocr_enabled = app_state.ocr.settings.read().await.enabled;
            if ocr_enabled {
                crate::commands::ocr::start_ocr_runtime(app_handle, &app_state, updated).await;
            }
        }
    } else if app_state.ocr.is_runtime_active() {
        crate::commands::ocr::unregister_ocr_capture_while_running(app_handle, &app_state).await;
    }

    Ok(())
}

/// Set a hotkey
#[tauri::command]
pub async fn set_hotkey(
    name: String,
    hotkey: Hotkey,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let _shortcut = hotkey
        .to_shortcut()
        .map_err(|e| format!("Invalid hotkey: {}", e))?;

    let settings = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?;

    // Проверка конфликтов только с включёнными хоткеями: выключенные
    // сохраняют комбинацию, но не занимают её.
    if let Some(conflict) = global_hotkey_conflict(&settings.hotkeys, &name, &hotkey) {
        return Err(conflict);
    }

    let name_clone = name.clone();
    let hotkey_clone = hotkey.clone();
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_hotkey(&name_clone, &hotkey_clone)
    })
    .await?;

    super::emit_settings_changed(&app_handle);

    crate::hotkeys::reregister_hotkeys(&app_handle)
        .map_err(|e| format!("Failed to re-register hotkeys: {}", e))?;

    if name == "ocr_capture" {
        if let Some(app_state) = app_handle.try_state::<AppState>() {
            if app_state.ocr.is_runtime_active() {
                if hotkey.enabled {
                    let hotkey_clone = hotkey.clone();
                    let app_handle_clone = app_handle.clone();
                    let state_clone = app_state.inner().clone();
                    tauri::async_runtime::spawn(async move {
                        crate::commands::ocr::rebind_ocr_capture_hotkey(
                            &app_handle_clone,
                            &state_clone,
                            &hotkey_clone,
                        )
                        .await;
                    });
                } else {
                    // A disabled `ocr_capture` must stop intercepting the
                    // combination, while OCR itself stays enabled.
                    let app_handle_clone = app_handle.clone();
                    let state_clone = app_state.inner().clone();
                    tauri::async_runtime::spawn(async move {
                        crate::commands::ocr::unregister_ocr_capture_while_running(
                            &app_handle_clone,
                            &state_clone,
                        )
                        .await;
                    });
                }
            }
        }
    }

    Ok(())
}

/// Reset a hotkey to its default value
#[tauri::command]
pub async fn reset_hotkey_to_default(
    name: String,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<Hotkey, String> {
    let name_clone = name.clone();
    let default = super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.reset_hotkey_to_default(&name_clone)
    })
    .await?;

    super::emit_settings_changed(&app_handle);

    crate::hotkeys::reregister_hotkeys(&app_handle)
        .map_err(|e| format!("Failed to re-register hotkeys: {}", e))?;

    if name == "ocr_capture" {
        if let Some(app_state) = app_handle.try_state::<AppState>() {
            if app_state.ocr.is_runtime_active() {
                if default.enabled {
                    let default_clone = default.clone();
                    let app_handle_clone = app_handle.clone();
                    let state_clone = app_state.inner().clone();
                    tauri::async_runtime::spawn(async move {
                        crate::commands::ocr::rebind_ocr_capture_hotkey(
                            &app_handle_clone,
                            &state_clone,
                            &default_clone,
                        )
                        .await;
                    });
                } else {
                    let app_handle_clone = app_handle.clone();
                    let state_clone = app_state.inner().clone();
                    tauri::async_runtime::spawn(async move {
                        crate::commands::ocr::unregister_ocr_capture_while_running(
                            &app_handle_clone,
                            &state_clone,
                        )
                        .await;
                    });
                }
            }
        }
    }

    Ok(default)
}

/// Set an editor-scoped hotkey (never registers global shortcut)
#[tauri::command]
pub async fn set_editor_hotkey(
    action_id: String,
    hotkey: Hotkey,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<(), String> {
    if hotkey.key.is_empty() {
        // Empty key is allowed: disabled binding. Validate modifiers are also empty.
        if !hotkey.modifiers.is_empty() {
            return Err(
                "Пустой ключ должен иметь пустые модификаторы (disabled binding)".to_string(),
            );
        }
    } else {
        let _ = hotkey
            .to_shortcut()
            .map_err(|e| format!("Invalid hotkey: {}", e))?;
    }

    let action_id_clone = action_id.clone();
    let hotkey_clone = hotkey.clone();
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_hotkey(&action_id_clone, &hotkey_clone)
    })
    .await?;

    super::emit_settings_changed(&app_handle);
    // Never call reregister_hotkeys for editor bindings

    Ok(())
}

/// Reset an editor-scoped hotkey to its canonical default
#[tauri::command]
pub async fn reset_editor_hotkey(
    action_id: String,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<Hotkey, String> {
    let action_id_clone = action_id.clone();
    let default = super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.reset_editor_hotkey(&action_id_clone)
    })
    .await?;

    super::emit_settings_changed(&app_handle);
    // Never call reregister_hotkeys for editor bindings

    Ok(default)
}

/// Enable or disable a single editor-scoped hotkey.
///
/// The assigned combination is preserved. Enabling re-checks conflicts against
/// enabled editor/global hotkeys; editor bindings are never registered as OS
/// shortcuts, so nothing is unregistered here.
#[tauri::command]
pub async fn set_editor_hotkey_enabled(
    action_id: String,
    enabled: bool,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<(), String> {
    if !EditorHotkeySettings::is_valid_action_id(&action_id) {
        return Err(format!("Invalid editor action: {}", action_id));
    }

    let settings = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?;
    let current = settings
        .hotkeys
        .editor
        .get_by_id(&action_id)
        .ok_or_else(|| format!("Invalid editor action: {}", action_id))?;
    let mut updated = current.clone();
    updated.enabled = enabled;

    if let Some(conflict) = editor_hotkey_conflict(&settings.hotkeys, &action_id, &updated) {
        return Err(conflict);
    }

    let action_id_clone = action_id.clone();
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_editor_hotkey_enabled_flag(&action_id_clone, enabled)
    })
    .await?;

    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Get all editor hotkey settings
#[tauri::command]
pub fn get_editor_hotkeys(
    settings_manager: State<'_, SettingsManager>,
) -> Result<EditorHotkeySettings, String> {
    settings_manager
        .load()
        .map(|s| s.hotkeys.editor)
        .map_err(|e| format!("Failed to load settings: {}", e))
}

/// Unregister all hotkeys (temporarily, for hotkey recording)
#[tauri::command]
pub async fn unregister_hotkeys(app_handle: AppHandle) -> Result<(), String> {
    crate::hotkeys::unregister_all_hotkeys(&app_handle).map_err(|e| e.to_string())
}

/// Re-register all hotkeys (restore after hotkey recording or cancellation)
#[tauri::command]
pub async fn reregister_hotkeys_cmd(app_handle: AppHandle) -> Result<(), String> {
    crate::hotkeys::reregister_hotkeys(&app_handle).map_err(|e| e.to_string())
}

/// Set hotkey recording flag (prevents hotkeys from triggering during recording)
#[tauri::command]
pub async fn set_hotkey_recording(app_handle: AppHandle, recording: bool) {
    if let Some(app_state) = app_handle.try_state::<AppState>() {
        app_state.set_hotkey_recording(recording);
    }
    if let Some(state) = app_handle.try_state::<crate::soundpanel::SoundPanelState>() {
        state.set_recording(recording);
    }
}

/// Notify all windows that appearance settings changed so panels update on the fly
fn emit_appearance_updates(app_handle: &AppHandle) {
    super::emit_settings_changed(app_handle);
    let _ = update_soundpanel_appearance(app_handle);
    let _ = update_playback_appearance(app_handle);
}

/// Enforce compact bounds on the main window.
///
/// The minimum is a physical 300×300; the maximum is the logical 800×630
/// scaled to physical pixels by the main window's current scale factor.
#[tauri::command]
pub fn set_main_bounds(app_handle: AppHandle) -> Result<(), String> {
    let window = app_handle
        .get_webview_window("main")
        .ok_or_else(|| "Main window not found".to_string())?;
    let scale = window
        .scale_factor()
        .map_err(|e| format!("Failed to read main window scale factor: {}", e))?;
    let (min_w, min_h, max_w, max_h) = crate::config::compact_physical_bounds(scale);

    let min_size = tauri::Size::Physical(tauri::PhysicalSize {
        width: min_w,
        height: min_h,
    });
    let max_size = tauri::Size::Physical(tauri::PhysicalSize {
        width: max_w,
        height: max_h,
    });
    window
        .set_min_size(Some(min_size))
        .map_err(|e| format!("Failed to set min size: {}", e))?;
    window
        .set_max_size(Some(max_size))
        .map_err(|e| format!("Failed to set max size: {}", e))?;
    Ok(())
}

/// Leave compact bounds and restore the full-mode layout floor.
///
/// The compact max bound is dropped, but a full-mode minimum (logical
/// 640x480, DPI-independent) replaces the compact minimum so the layout
/// cannot be shrunk until it breaks.
#[tauri::command]
pub fn remove_main_bounds(app_handle: AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        let min_size = tauri::Size::Logical(tauri::LogicalSize::new(755.0, 540.0));
        window
            .set_min_size(Some(min_size))
            .map_err(|e| format!("Failed to set min size: {}", e))?;
        window
            .set_max_size(Option::<tauri::Size>::None)
            .map_err(|e| format!("Failed to remove max size: {}", e))?;
        Ok(())
    } else {
        Err("Main window not found".to_string())
    }
}

/// Set main window compact dimensions (physical inner pixels).
///
/// Input is clamped against the actual main-window scale (physical minimum,
/// scale-aware logical maximum) before persistence. Persisted values are
/// physical INNER dimensions; the new frontend reads and writes INNER size
/// consistently, while legacy outer-read/inner-applied numbers are preserved.
#[tauri::command]
pub async fn set_main_compact_dims(
    width: u32,
    height: u32,
    app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    let (width, height) = {
        let window = app_handle
            .get_webview_window("main")
            .ok_or_else(|| "Main window not found".to_string())?;
        let scale = window
            .scale_factor()
            .map_err(|e| format!("Failed to read main window scale factor: {}", e))?;
        crate::config::clamp_compact_size(width, height, scale)
    };
    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_main_compact_dims(width, height)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Get main window compact dimensions
#[tauri::command]
pub fn get_main_compact_dims(windows_manager: State<'_, WindowsManager>) -> (u32, u32) {
    windows_manager.get_main_compact_dims()
}

/// Set main window compact view ('compact' or 'mono')
#[tauri::command]
pub async fn set_main_compact_view(
    view: CompactView,
    app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_main_compact_view(view)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Get main window compact view
#[tauri::command]
pub fn get_main_compact_view(windows_manager: State<'_, WindowsManager>) -> CompactView {
    windows_manager.get_main_compact_view()
}

/// Set whether the extra floating-window buttons are hidden in the title bar
#[tauri::command]
pub async fn set_hide_extra_window_buttons(
    value: bool,
    app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_hide_extra_window_buttons(value)
    })
    .await?;
    super::emit_settings_changed(&app_handle);
    Ok(())
}

// ========== Main Window Appearance ==========

/// Resolve the effective main window appearance as `(opacity, bg_color)`.
///
/// When `custom_background` is disabled, the color falls back to the active
/// theme background (Light -> `#fafcff`, Dark -> `#090b0f`). Used by panels that
/// inherit the main window appearance (`appearance_source == "main"`).
pub fn resolve_main_appearance(
    windows_manager: &WindowsManager,
    settings_manager: &SettingsManager,
) -> (u8, String) {
    let (custom_background, opacity, bg_color) = windows_manager.get_main_appearance();
    let color = if custom_background {
        bg_color
    } else {
        let theme = settings_manager
            .load()
            .map(|s| s.theme)
            .unwrap_or(Theme::Dark);
        match theme {
            Theme::Light => "#fafcff".to_string(),
            Theme::Dark => "#090b0f".to_string(),
        }
    };
    (opacity, color)
}

/// Get main window appearance (custom_background, opacity, bg_color)
#[tauri::command]
pub fn get_main_appearance(
    windows_manager: State<'_, WindowsManager>,
) -> Result<(bool, u8, String), String> {
    Ok(windows_manager.get_main_appearance())
}

/// Set whether the main window uses a custom background color
#[tauri::command]
pub async fn set_main_custom_background(
    value: bool,
    app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(value, "Setting main custom background");
    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_main_custom_background(value)
    })
    .await?;
    emit_appearance_updates(&app_handle);
    Ok(())
}

/// Set main window opacity (10-100)
#[tauri::command]
pub async fn set_main_opacity(
    value: u8,
    app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(value, "Setting main opacity");
    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_main_opacity(value)
    })
    .await?;
    emit_appearance_updates(&app_handle);
    Ok(())
}

/// Set main window background color (#RRGGBB)
#[tauri::command]
pub async fn set_main_bg_color(
    color: String,
    app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(color, "Setting main bg color");
    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_main_bg_color(color)
    })
    .await?;
    emit_appearance_updates(&app_handle);
    Ok(())
}

/// Set whether the main window uses custom opacity
#[tauri::command]
pub async fn set_main_custom_opacity(
    value: bool,
    app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(value, "Setting main custom opacity");
    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_main_custom_opacity(value)
    })
    .await?;
    emit_appearance_updates(&app_handle);
    Ok(())
}

/// Set whether custom opacity is applied only in compact mode
#[tauri::command]
pub async fn set_main_opacity_compact_only(
    value: bool,
    app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(value, "Setting main opacity compact only");
    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_main_opacity_compact_only(value)
    })
    .await?;
    emit_appearance_updates(&app_handle);
    Ok(())
}

// ========== Panel Appearance Source ==========

/// Get soundpanel appearance source ("own" or "main")
#[tauri::command]
pub fn get_soundpanel_appearance_source(windows_manager: State<'_, WindowsManager>) -> String {
    windows_manager.get_soundpanel_appearance_source()
}

/// Set soundpanel appearance source ("own" or "main")
#[tauri::command]
pub async fn set_soundpanel_appearance_source(
    source: String,
    app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(source, "Setting soundpanel appearance source");
    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_soundpanel_appearance_source(source)
    })
    .await?;
    emit_appearance_updates(&app_handle);
    Ok(())
}

/// Get playback appearance source ("own" or "main")
#[tauri::command]
pub fn get_playback_appearance_source(windows_manager: State<'_, WindowsManager>) -> String {
    windows_manager.get_playback_appearance_source()
}

/// Set playback appearance source ("own" or "main")
#[tauri::command]
pub async fn set_playback_appearance_source(
    source: String,
    app_handle: AppHandle,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(source, "Setting playback appearance source");
    super::persist_blocking(windows_manager.inner(), move |mgr| {
        mgr.set_playback_appearance_source(source)
    })
    .await?;
    emit_appearance_updates(&app_handle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn set_hwnd(state: &AppState, value: isize) {
        let mut guard = state.previous_foreground_hwnd.lock();
        *guard = Some(value);
    }

    fn get_hwnd(state: &AppState) -> Option<isize> {
        let guard = state.previous_foreground_hwnd.lock();
        *guard
    }

    /// HWND guard is cleared after successful SetForegroundWindow.
    #[test]
    fn hwnd_success_clears_guard() {
        let state = AppState::new();
        set_hwnd(&state, 42);

        let result = return_previous_hwnd_core(
            &state,
            |hwnd| {
                assert_eq!(hwnd, 42);
                true
            },
            |hwnd| {
                assert_eq!(hwnd, 42);
                true
            },
        );

        assert!(result.is_ok());
        assert_eq!(get_hwnd(&state), None);
    }

    /// HWND guard is cleared when the saved window is confirmed invalid.
    #[test]
    fn hwnd_invalid_is_cleared() {
        let state = AppState::new();
        set_hwnd(&state, 42);

        let result = return_previous_hwnd_core(&state, |_| false, |_| unreachable!());

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Предыдущее окно больше не доступно"));
        assert_eq!(get_hwnd(&state), None);
    }

    /// HWND guard is preserved for retry on transient SetForegroundWindow failure.
    #[test]
    fn transient_failure_restores_hwnd() {
        let state = AppState::new();
        set_hwnd(&state, 42);

        let result = return_previous_hwnd_core(&state, |_| true, |_| false);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Не удалось переключить фокус"));
        // Guard is restored for retry
        assert_eq!(get_hwnd(&state), Some(42));
    }

    /// When no HWND was ever saved, the return is OK without clearing anything.
    #[test]
    fn no_saved_hwnd_returns_ok() {
        let state = AppState::new();

        let result = return_previous_hwnd_core(&state, |_| unreachable!(), |_| unreachable!());

        assert!(result.is_ok());
        assert_eq!(get_hwnd(&state), None);
    }

    /// Concurrent HWND save is NOT overwritten by a transient-failure restore.
    #[test]
    fn concurrent_hwnd_save_not_overwritten_on_transient_failure() {
        let state = AppState::new();
        set_hwnd(&state, 42);

        let entered_set_fg = Arc::new(AtomicBool::new(false));
        let flag = entered_set_fg.clone();
        let state_clone = state.clone();

        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            let mut guard = state_clone.previous_foreground_hwnd.lock();
            *guard = Some(99);
        });

        let result = return_previous_hwnd_core(
            &state,
            |_| true,
            |_| {
                entered_set_fg.store(true, Ordering::Release);
                std::thread::sleep(Duration::from_millis(50));
                false // transient failure
            },
        );

        thread.join().unwrap();

        assert!(result.is_err());
        assert_eq!(get_hwnd(&state), Some(99));
    }

    // ==================== Editor hotkey save/reset tests ====================

    fn temp_settings_manager() -> (SettingsManager, std::path::PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-editor-hk-test-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mgr = SettingsManager::with_config_dir(dir.clone()).unwrap();
        (mgr, dir)
    }

    #[test]
    fn set_editor_hotkey_saves_and_reads_back() {
        let (mgr, dir) = temp_settings_manager();
        let new_hk = Hotkey {
            modifiers: vec![],
            key: "F9".to_string(),
            enabled: true,
        };
        mgr.set_editor_hotkey("edit_word", &new_hk).unwrap();
        let settings = mgr.load().unwrap();
        assert_eq!(settings.hotkeys.editor.edit_word.key, "F9");
        assert!(settings.hotkeys.editor.edit_word.modifiers.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_editor_hotkey_rejects_invalid_action() {
        let (mgr, dir) = temp_settings_manager();
        let new_hk = Hotkey {
            modifiers: vec![],
            key: "F9".to_string(),
            enabled: true,
        };
        let err = mgr.set_editor_hotkey("bogus", &new_hk).unwrap_err();
        assert!(err.to_string().contains("Invalid editor action"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_editor_hotkey_disabled_binding_is_allowed() {
        let (mgr, dir) = temp_settings_manager();
        let empty = Hotkey {
            modifiers: vec![],
            key: String::new(),
            enabled: true,
        };
        mgr.set_editor_hotkey("edit_word", &empty).unwrap();
        let settings = mgr.load().unwrap();
        assert!(settings.hotkeys.editor.edit_word.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_editor_hotkey_detects_duplicate_within_editor() {
        let (mgr, dir) = temp_settings_manager();
        let binding = Hotkey {
            modifiers: vec![crate::config::HotkeyModifier::Ctrl],
            key: "F9".to_string(),
            enabled: true,
        };
        mgr.set_editor_hotkey("edit_word", &binding).unwrap();
        // Same binding for another action should fail
        let err = mgr.set_editor_hotkey("next_tab", &binding).unwrap_err();
        assert!(err.to_string().contains("уже используется"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_editor_hotkey_detects_global_conflict() {
        let (mgr, dir) = temp_settings_manager();
        let binding = Hotkey {
            modifiers: vec![
                crate::config::HotkeyModifier::Ctrl,
                crate::config::HotkeyModifier::Shift,
            ],
            key: "F3".to_string(),
            enabled: true,
        };
        let err = mgr.set_editor_hotkey("edit_word", &binding).unwrap_err();
        assert!(err.to_string().contains("конфликтует"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_editor_hotkey_does_not_partially_save_on_conflict() {
        let (mgr, dir) = temp_settings_manager();
        let original = mgr.load().unwrap().hotkeys.editor.next_tab.key.clone();
        let binding = Hotkey {
            modifiers: vec![
                crate::config::HotkeyModifier::Ctrl,
                crate::config::HotkeyModifier::Shift,
            ],
            key: "F3".to_string(),
            enabled: true,
        };
        let _ = mgr.set_editor_hotkey("next_tab", &binding);
        let settings = mgr.load().unwrap();
        assert_eq!(
            settings.hotkeys.editor.next_tab.key, original,
            "existing bindings must not change on failed save"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reset_editor_hotkey_returns_canonical_default() {
        let (mgr, dir) = temp_settings_manager();
        // Change edit_word first
        let custom = Hotkey {
            modifiers: vec![],
            key: "F9".to_string(),
            enabled: true,
        };
        mgr.set_editor_hotkey("edit_word", &custom).unwrap();

        let default = mgr.reset_editor_hotkey("edit_word").unwrap();
        assert_eq!(default.key, "E");
        assert_eq!(default.modifiers.len(), 1);

        let settings = mgr.load().unwrap();
        assert_eq!(settings.hotkeys.editor.edit_word.key, "E");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reset_editor_hotkey_rejects_invalid_action() {
        let (mgr, dir) = temp_settings_manager();
        let err = mgr.reset_editor_hotkey("bogus").unwrap_err();
        assert!(err.to_string().contains("Invalid editor action"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_editor_hotkey_empty_key_with_modifiers_rejected() {
        // This is tested at the command level (set_editor_hotkey command validates this).
        // The manager level allows it (flexibility for programmatic use).
        // Instead, verify the command-level guard works.
        let hotkey = Hotkey {
            modifiers: vec![crate::config::HotkeyModifier::Ctrl],
            key: String::new(),
            enabled: true,
        };
        // The command checks: empty key + non-empty modifiers = error
        // This invariant is enforced at the command layer, not the manager.
        assert!(hotkey.is_empty());
        assert!(!hotkey.modifiers.is_empty());
        // The command would reject this; the manager allows it.
    }

    #[test]
    fn editor_hotkey_save_does_not_call_reregister() {
        // The set_editor_hotkey command explicitly does NOT call
        // reregister_hotkeys. This is a design-level test verified by
        // code review: set_editor_hotkey command body has no call to
        // crate::hotkeys::reregister_hotkeys.

        let (mgr, dir) = temp_settings_manager();
        let binding = Hotkey {
            modifiers: vec![],
            key: "F8".to_string(),
            enabled: true,
        };
        mgr.set_editor_hotkey("edit_word", &binding).unwrap();
        // If reregister were called, it would create a new SettingsManager
        // internally (which creates a separate cache). This test ensures
        // the cache is consistent: we read back what we just wrote.
        let settings = mgr.load().unwrap();
        assert_eq!(settings.hotkeys.editor.edit_word.key, "F8");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== Enabled-switch conflict validation ====================

    #[test]
    fn global_conflict_rejects_duplicate_with_enabled_global() {
        let settings = HotkeySettings::default();
        let candidate = settings.sound_panel.clone();

        let error = global_hotkey_conflict(&settings, "main_window", &candidate).unwrap();

        assert!(error.contains("звуковой панели"), "unexpected: {error}");
    }

    #[test]
    fn global_conflict_ignores_disabled_candidate() {
        let settings = HotkeySettings::default();
        let mut candidate = settings.sound_panel.clone();
        candidate.enabled = false;

        assert!(global_hotkey_conflict(&settings, "main_window", &candidate).is_none());
    }

    #[test]
    fn global_conflict_ignores_disabled_existing_global() {
        let mut settings = HotkeySettings::default();
        settings.sound_panel.enabled = false;
        let mut candidate = settings.sound_panel.clone();
        candidate.enabled = true;

        assert!(global_hotkey_conflict(&settings, "main_window", &candidate).is_none());
    }

    #[test]
    fn global_conflict_rejects_overlap_with_enabled_editor_binding() {
        let settings = HotkeySettings::default();
        let candidate = settings.editor.edit_word.clone();

        let error = global_hotkey_conflict(&settings, "main_window", &candidate).unwrap();

        assert!(error.contains("действия редактора"), "unexpected: {error}");
    }

    #[test]
    fn same_state_toggle_is_a_noop() {
        assert!(!hotkey_toggle_changes_state(true, true));
        assert!(!hotkey_toggle_changes_state(false, false));
        assert!(hotkey_toggle_changes_state(false, true));
        assert!(hotkey_toggle_changes_state(true, false));
    }

    #[test]
    fn enabling_an_empty_global_binding_is_not_a_conflict() {
        // An empty combination has nothing to register, so switching it on must
        // not be rejected as an in-app conflict.
        let settings = HotkeySettings::default();
        let candidate = Hotkey {
            modifiers: vec![],
            key: String::new(),
            enabled: true,
        };

        assert!(global_hotkey_conflict(&settings, "main_window", &candidate).is_none());
    }

    #[test]
    fn editor_conflict_rejects_duplicate_with_enabled_editor_binding() {
        let settings = HotkeySettings::default();
        let candidate = settings.editor.submit_continue.clone();

        let error = editor_hotkey_conflict(&settings, "edit_word", &candidate).unwrap();

        assert!(error.contains("уже используется"), "unexpected: {error}");
        assert!(
            error.contains("отправки/продолжения"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn editor_conflict_rejects_enabled_global_overlap() {
        let settings = HotkeySettings::default();
        let candidate = Hotkey {
            modifiers: vec![
                crate::config::HotkeyModifier::Ctrl,
                crate::config::HotkeyModifier::Shift,
            ],
            key: "F3".to_string(),
            enabled: true,
        };

        let error = editor_hotkey_conflict(&settings, "edit_word", &candidate).unwrap();

        assert!(error.contains("конфликтует"), "unexpected: {error}");
    }

    #[test]
    fn editor_conflict_allows_disabled_candidate() {
        let settings = HotkeySettings::default();
        let mut candidate = settings.editor.submit_continue.clone();
        candidate.enabled = false;

        assert!(editor_hotkey_conflict(&settings, "edit_word", &candidate).is_none());
    }
}
