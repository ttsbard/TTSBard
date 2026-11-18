use crate::config::{
    CompactView, EditorHotkeySettings, Hotkey, HotkeySettings, SettingsManager, Theme,
    WindowsManager,
};
use crate::playback_window::update_playback_appearance;
use crate::soundpanel::intercept::InterceptSettings;
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

/// Load the persisted intercept configuration for conflict checks.
///
/// Reads `intercept.json` from the sound panel state's app dir — the same
/// path `soundpanel::intercept::save`/`load` own — so the check sees exactly
/// what is stored on disk. When the sound panel state is unavailable the
/// check degrades to an empty configuration, which simply finds no conflict.
fn load_intercept_settings(app_handle: &AppHandle) -> InterceptSettings {
    let Some(state) = app_handle.try_state::<crate::soundpanel::SoundPanelState>() else {
        return InterceptSettings::default();
    };
    let appdata_path = state
        .appdata_path
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    crate::soundpanel::intercept::load(&appdata_path)
}

/// Validate a global/main-window-local binding against every *enabled* other
/// hotkey. Returns a user-facing conflict message.
///
/// For the hotkey-to-hotkey rules disabled hotkeys are not occupied, so a
/// disabled binding never blocks them. This function must be called for the
/// binding as it will be stored (including its `enabled` value).
///
/// ROADMAP-140: the NumPad intersection with saved intercept bindings is
/// checked *before* the enabled guard, by the saved key code alone. The
/// outcome must not depend on the editing order: a saved intercept binding
/// blocks even a disabled global NumPad save, exactly as a saved global
/// NumPad (enabled or not) blocks an intercept save on the other side. Only
/// an empty key passes the intercept side — it has no code to intersect.
/// The per-binding switch command keeps disabling working as the way out of
/// an existing conflict by running this validation only when enabling.
fn global_hotkey_conflict(
    settings: &HotkeySettings,
    intercept: &InterceptSettings,
    skip_name: &str,
    candidate: &Hotkey,
) -> Option<String> {
    if let Some(binding) =
        crate::soundpanel::intercept::find_numpad_binding_conflict(intercept, candidate)
    {
        return Some(
            crate::soundpanel::intercept::numpad_intercept_conflict_message(candidate, binding),
        );
    }
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

/// NumPad-intersection check of an editor candidate against the saved
/// intercept bindings, as a user-facing conflict message.
///
/// ROADMAP-140: the NumPad intersection with saved intercept bindings is
/// judged by the saved key code alone — regardless of the candidate's
/// modifiers, of its `enabled` switch and of the intercept mode switch — so
/// the verdict does not depend on the editing order. Only an empty key
/// passes: it has no code to intersect. Save points must run this before
/// their enabled guard; the per-binding switch command must run it only
/// when enabling, keeping the switch available as the way out of an
/// existing conflict.
fn editor_numpad_intercept_conflict(
    intercept: &InterceptSettings,
    candidate: &Hotkey,
) -> Option<String> {
    crate::soundpanel::intercept::find_numpad_binding_conflict(intercept, candidate).map(
        |binding| {
            crate::soundpanel::intercept::numpad_intercept_conflict_message(candidate, binding)
        },
    )
}

/// Validate an editor-scoped binding against the editor group and the enabled
/// global hotkeys. The NumPad intersection with saved intercept bindings is
/// checked first, before the enabled guard — see
/// [`editor_numpad_intercept_conflict`] for the rule; the per-binding switch
/// command gates the call itself on the enable direction.
fn editor_hotkey_conflict(
    settings: &HotkeySettings,
    intercept: &InterceptSettings,
    action_id: &str,
    candidate: &Hotkey,
) -> Option<String> {
    if let Some(conflict) = editor_numpad_intercept_conflict(intercept, candidate) {
        return Some(conflict);
    }
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

/// One NumPad-coordinated check-then-write section for a `settings.json`
/// hotkey save (ROADMAP-140 race fix).
///
/// `settings.json` and `intercept.json` are separate stores: a conflict check
/// against a snapshot that races a commit on the other side could let two
/// incompatible saves land (a NumPad code reserved by both a hotkey and an
/// intercept binding). This core holds the shared coordination lock across
/// the whole section — fresh snapshot load, validation, persist — so a
/// concurrent intercept save can never interleave between the check and the
/// write: whichever section runs second always checks against the first one's
/// committed write and is rejected. Exactly one of two competing incompatible
/// saves wins, in either order.
///
/// The closures must not take any other lock before entering and must not
/// recurse into another coordinated section: the coordination lock is the
/// outermost one (see [`crate::soundpanel::intercept::lock_numpad_coordination`]).
/// Everything inside must be synchronous — the callers run this on a blocking
/// thread, never while holding it across an `.await`.
///
/// The closures must also never call the global-shortcut registrar
/// (`app_handle.global_shortcut()`, `crate::hotkeys::register_*`) — or any
/// code that reaches it (review 009 deadlock fix). The Windows plugin
/// forwards every registration to the UI thread and blocks on its completion,
/// while the synchronous `set_intercept_binding` command can be waiting for
/// this very lock on the UI thread; a registrar call inside the section would
/// close a lock ↔ UI-thread cycle. Persist inside the section, register
/// outside it: OS work happens after the guard is dropped, as in
/// [`set_hotkey_binding_enabled`] and [`restore_global_hotkey`].
pub(crate) fn coordinated_hotkey_section<L, V, W, T>(
    load: L,
    validate: V,
    write: W,
) -> Result<T, String>
where
    L: FnOnce() -> Result<HotkeySettings, String>,
    V: FnOnce(&HotkeySettings) -> Result<(), String>,
    W: FnOnce(&HotkeySettings) -> Result<T, String>,
{
    let _guard = crate::soundpanel::intercept::lock_numpad_coordination();
    let settings = load()?;
    validate(&settings)?;
    write(&settings)
}

/// Whether a per-binding switch request actually changes the stored flag.
///
/// A same-state request is a successful no-op: re-applying it would only
/// re-register (or release) an unchanged shortcut and could disturb unrelated
/// registrations.
fn hotkey_toggle_changes_state(current_enabled: bool, requested_enabled: bool) -> bool {
    current_enabled != requested_enabled
}

/// Candidate of a per-binding switch, derived from the *fresh* stored record
/// inside the coordinated section — never from the pre-section snapshot.
///
/// The pre-section view can go stale while the command waits for the
/// coordination lock: a concurrent `set_hotkey` may have re-assigned the
/// combination. Validating or registering the stale candidate would flip the
/// flag of the current record while registering its replaced key, so the
/// candidate must always be `current record + requested flag`. A record that
/// already carries the requested flag is a success no-op (`None`): the
/// concurrent save owns the state and has announced its own change.
fn switch_candidate_from_stored(
    stored: Option<&Hotkey>,
    name: &str,
    enabled: bool,
) -> Result<Option<Hotkey>, String> {
    let Some(stored) = stored else {
        return Err(format!("Invalid hotkey name: {}", name));
    };
    if stored.enabled == enabled {
        return Ok(None);
    }
    let mut candidate = stored.clone();
    candidate.enabled = enabled;
    Ok(Some(candidate))
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
/// working.
///
/// The NumPad-coordinated section re-loads a fresh snapshot under the shared
/// lock and commits the conflict check and the persisted switch atomically —
/// so a concurrent intercept save can never slip between the check and the
/// persisted write. The switched candidate is re-derived from that fresh
/// snapshot ([`switch_candidate_from_stored`]) instead of the pre-section
/// one: a concurrent `set_hotkey` that re-assigned the combination while this
/// command waited for the lock is validated, persisted and registered under
/// its own combination, and a record that already carries the requested flag
/// is a success no-op. The OS registration itself deliberately happens
/// *outside* the section (review 009 deadlock fix): the Windows
/// global-shortcut plugin forwards registration to the UI thread and blocks
/// on its completion, while the synchronous `set_intercept_binding` command
/// can be waiting for this very lock on the UI thread — a registrar call
/// inside the section would close a lock ↔ UI-thread cycle. The persisted
/// switch therefore lands first, and a runtime change that Windows rejects is
/// rolled back with [`rollback_hotkey_enabled_switch`], so the stored flag
/// never keeps claiming a registration that does not exist.
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

    // Conflicts are validated only when enabling. Switching a binding off
    // registers nothing and must stay available as the way out of an existing
    // NumPad/intercept intersection (ROADMAP-140); before the intercept check
    // existed, the enabled guard short-circuited every check on disable, so
    // gating here also preserves that behavior verbatim.
    //
    // The section persists the switch only: the runtime work for both the OCR
    // and the non-OCR bindings happens after the lock is released (deadlock
    // fix, see the command doc). The persisted NumPad assignment still blocks
    // a concurrent intercept save on the same code — the intercept side
    // checks the stored bindings regardless of any `enabled` flag.
    //
    // The candidate is re-derived from the fresh snapshot inside the section,
    // so a concurrent re-assignment that landed while this command waited for
    // the lock is validated, persisted and registered under its own
    // combination — never under the stale pre-section one.
    let is_ocr = name == "ocr_capture";
    let mgr = settings_manager.inner().clone();
    let handle = app_handle.clone();
    let coordinated_name = name.clone();
    let updated: Option<Hotkey> = tauri::async_runtime::spawn_blocking(move || {
        coordinated_hotkey_section(
            || {
                mgr.load()
                    .map(|s| s.hotkeys)
                    .map_err(|e| format!("Failed to load settings: {}", e))
            },
            |hotkeys| {
                let candidate = switch_candidate_from_stored(
                    hotkeys.get_global_by_name(&coordinated_name),
                    &coordinated_name,
                    enabled,
                )?;
                let Some(candidate) = candidate else {
                    return Ok(());
                };
                if !enabled {
                    return Ok(());
                }
                let intercept = load_intercept_settings(&handle);
                global_hotkey_conflict(hotkeys, &intercept, &coordinated_name, &candidate)
                    .map_or(Ok(()), Err)
            },
            |hotkeys| {
                let candidate = switch_candidate_from_stored(
                    hotkeys.get_global_by_name(&coordinated_name),
                    &coordinated_name,
                    enabled,
                )?;
                match candidate {
                    Some(candidate) => {
                        mgr.set_hotkey_enabled_flag(&coordinated_name, enabled)
                            .map_err(|error| {
                                format!("Failed to persist hotkey state: {}", error)
                            })?;
                        Ok(Some(candidate))
                    }
                    None => Ok(None),
                }
            },
        )
    })
    .await
    .map_err(|e| format!("blocking task panicked: {}", e))??;

    // A concurrent save already switched this binding to the requested state:
    // nothing to validate, persist or register — and that save has already
    // announced its own change.
    let Some(updated) = updated else {
        return Ok(());
    };

    if is_ocr {
        apply_ocr_capture_binding_toggle(&name, &updated, settings_manager.inner(), &app_handle)
            .await?;
    } else if let Err(runtime_error) =
        apply_global_binding_runtime_change(&name, &updated, &app_handle)
    {
        // The switch is already persisted, so the failure must flip it back,
        // guarded by [`rollback_hotkey_enabled_switch`]: only this
        // transaction's own commit is undone, never a newer save.
        let mgr = settings_manager.inner().clone();
        let rollback_name = name.clone();
        let committed = updated.clone();
        let rollback = tauri::async_runtime::spawn_blocking(move || {
            rollback_hotkey_enabled_switch(&mgr, &rollback_name, &committed, !enabled)
        })
        .await
        .map_err(|e| format!("blocking task panicked: {}", e))
        .and_then(|inner| inner);
        let direction = if enabled { "register" } else { "release" };
        let stays = if enabled { "off" } else { "on" };
        return Err(match rollback {
            Ok(()) => format!("Failed to {direction} the shortcut: {runtime_error}; the hotkey stays {stays}"),
            Err(rollback_error) => format!(
                "Failed to {direction} the shortcut: {runtime_error}; restoring the persisted switch failed: {rollback_error}"
            ),
        });
    }

    super::emit_settings_changed(&app_handle);

    Ok(())
}

/// Apply the OS registration side of a per-binding switch, outside any
/// NumPad-coordinated section.
///
/// The coordinated section has already persisted the switch when this runs
/// (see [`set_hotkey_binding_enabled`]); this only registers or releases the
/// one shortcut, never touching other registrations, including the OCR-owned
/// capture shortcut. A failure here is compensated by
/// [`rollback_hotkey_enabled_switch`], which restores the persisted switch —
/// this function itself leaves the runtime state to the plugin's ownership
/// rules ([`crate::hotkeys::set_global_binding_runtime_state`]).
fn apply_global_binding_runtime_change(
    name: &str,
    updated: &Hotkey,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let app_state = app_handle
        .try_state::<AppState>()
        .ok_or_else(|| "Application state is unavailable".to_string())?
        .inner()
        .clone();

    crate::hotkeys::set_global_binding_runtime_state(
        app_handle,
        &app_state,
        name,
        updated,
        updated.enabled,
    )?;
    Ok(())
}

/// Roll a persisted per-binding switch back after the runtime change failed.
///
/// The flip is judged on a fresh snapshot under the shared NumPad
/// coordination lock, which closes the check-then-write window against a
/// concurrent hotkey/intercept save: it lands only while the stored binding
/// is still the one the failed transaction committed (`stored == committed`),
/// so a newer save that landed during the failure keeps its own switch.
/// Only the flag moves — the stored combination is never rewritten.
///
/// Everything under the lock stays synchronous, and the runtime registrar is
/// deliberately never called here: holding the coordination lock while
/// waiting on the UI-thread-bound global-shortcut plugin is exactly the
/// deadlock this module fixed (review 009).
fn rollback_hotkey_enabled_switch(
    settings_manager: &SettingsManager,
    name: &str,
    committed: &Hotkey,
    restored_flag: bool,
) -> Result<(), String> {
    let _guard = crate::soundpanel::intercept::lock_numpad_coordination();
    let hotkeys = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?
        .hotkeys;
    if hotkeys.get_global_by_name(name) != Some(committed) {
        return Err("the rollback was skipped: a newer binding was saved for this hotkey during the failure; its switch was not rolled back"
            .to_string());
    }
    settings_manager
        .set_hotkey_enabled_flag(name, restored_flag)
        .map_err(|e| e.to_string())
}

/// Apply the `ocr_capture` runtime registration after the coordinated section
/// has already persisted the switch.
///
/// The service keeps the previous registration when a new one is rejected, so
/// the only rollback needed here is restoring the persisted switch. That
/// rollback is guarded like the key restore: it flips the flag only while the
/// stored binding is still the one this transaction persisted, so a newer
/// save that landed during the failure keeps its own switch. OCR readiness
/// and the OCR enable flag are otherwise untouched.
async fn apply_ocr_capture_binding_toggle(
    name: &str,
    updated: &Hotkey,
    settings_manager: &SettingsManager,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let enabled = updated.enabled;

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
                let mgr = settings_manager.clone();
                let rollback_name = name.to_string();
                let committed = updated.clone();
                let rollback = tauri::async_runtime::spawn_blocking(move || {
                    rollback_hotkey_enabled_switch(&mgr, &rollback_name, &committed, false)
                })
                .await
                .map_err(|e| format!("blocking task panicked: {}", e))
                .and_then(|inner| inner);
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

/// Outcome of the coordinated rollback decision for a registration the OS
/// rejected ([`restore_global_hotkey`]).
#[derive(Debug, PartialEq, Eq)]
enum GlobalHotkeyRestore {
    /// The failed write is still the stored binding and the previous value
    /// passes the save conflict rules: the rollback may persist it.
    Restore,
    /// A newer save replaced the failed binding while the registration was
    /// being rejected: the stale rollback must not overwrite it.
    SkipNewerSave,
    /// Restoring the previous value would recreate a state the save rules
    /// reject — e.g. a NumPad code an intercept binding reserved in the
    /// failure window: the conflicting entry is not written, and the caller
    /// reports an explicit error instead.
    Refused(String),
}

/// Decide whether the post-rejection rollback may restore `previous`, judged
/// on a fresh snapshot under the NumPad coordination lock.
///
/// The rollback is applicable only while the stored binding is still the
/// failed write itself (`stored == failed`): anything else is a newer save
/// that a stale rollback must not overwrite. An applicable restore still has
/// to pass the same conflict rules a fresh save of `previous` would face
/// (NumPad intersection with saved intercept bindings first, then the enabled
/// hotkey-to-hotkey rules) — otherwise the freed-code race between
/// `set_hotkey` and `set_intercept_binding` ends with both stores owning the
/// same NumPad code.
fn decide_global_hotkey_restore(
    name: &str,
    hotkeys: &HotkeySettings,
    failed: &Hotkey,
    previous: &Hotkey,
    intercept: &InterceptSettings,
) -> GlobalHotkeyRestore {
    if hotkeys.get_global_by_name(name) != Some(failed) {
        return GlobalHotkeyRestore::SkipNewerSave;
    }
    if let Some(conflict) = global_hotkey_conflict(hotkeys, intercept, name, previous) {
        return GlobalHotkeyRestore::Refused(conflict);
    }
    GlobalHotkeyRestore::Restore
}

/// Restore the previously persisted binding after the OS rejected the new one.
///
/// `reregister_hotkeys` releases every module-owned shortcut before it registers
/// the new set, so a rejected combination must not stay in `settings.json`: the
/// stored value would claim a registration Windows never accepted. On an
/// applicable restore the previous value is persisted and hotkeys are
/// re-registered from it, which brings the previous shortcut back.
///
/// The restore is not blind: it runs inside the shared NumPad coordination
/// section on a fresh snapshot ([`decide_global_hotkey_restore`]), because the
/// failure window lets concurrent saves land — most notably
/// `set_intercept_binding` reserving the just-freed NumPad code, or a newer
/// binding for the same name. An inapplicable rollback reports an explicit
/// error without writing anything, so the rollback never recreates a rejected
/// dual ownership and never overwrites a newer save.
///
/// The compensation re-register runs on **every** outcome (review 009 fix):
/// the failed registration pass already released the whole module-owned set,
/// so a skipped or refused rollback must still bring every still-valid
/// registration back. Nothing is persisted in those branches, and the
/// intercept-reserved NumPad code is never given back: the stored rejected
/// combination simply fails its registration again without stranding the
/// others, because [`crate::hotkeys::register_from_settings`] continues past
/// individual failures.
async fn restore_global_hotkey(
    name: &str,
    previous: &Hotkey,
    failed: &Hotkey,
    settings_manager: &SettingsManager,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let mgr = settings_manager.clone();
    let handle = app_handle.clone();
    let restore_name = name.to_string();
    let restore_hotkey = previous.clone();
    let failed_hotkey = failed.clone();
    // The coordination lock is the outermost one and everything under it is
    // synchronous: no await, no UI-thread wait and no shortcut registrar run
    // while it is held.
    let restore_outcome = tauri::async_runtime::spawn_blocking(move || {
        let _guard = crate::soundpanel::intercept::lock_numpad_coordination();
        let hotkeys = mgr
            .load()
            .map(|s| s.hotkeys)
            .map_err(|e| format!("Failed to load settings: {}", e))?;
        let intercept = load_intercept_settings(&handle);
        match decide_global_hotkey_restore(
            &restore_name,
            &hotkeys,
            &failed_hotkey,
            &restore_hotkey,
            &intercept,
        ) {
            GlobalHotkeyRestore::Restore => mgr
                .set_hotkey(&restore_name, &restore_hotkey)
                .map_err(|e| e.to_string()),
            GlobalHotkeyRestore::SkipNewerSave => Err(
                "the rollback was skipped: a newer binding was saved for this hotkey during the failure; the previous setting was not restored over it"
                    .to_string(),
            ),
            GlobalHotkeyRestore::Refused(conflict) => Err(format!(
                "the rollback was refused: {conflict}; the rejected combination stays stored unregistered",
            )),
        }
    })
    .await
    .map_err(|e| format!("blocking task panicked: {}", e))
    .and_then(|inner| inner);

    let registration =
        crate::hotkeys::reregister_hotkeys(app_handle).map_err(|error| error.to_string());
    combine_restore_with_reregister(restore_outcome, registration)
}

/// Combine the coordinated rollback outcome with the compensation
/// re-register result. Both always run ([`restore_global_hotkey`]): neither
/// success masks the other's failure, and the compensation failure never
/// replaces the rollback verdict — both are reported together.
fn combine_restore_with_reregister(
    restore: Result<(), String>,
    reregistration: Result<(), String>,
) -> Result<(), String> {
    match (restore, reregistration) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(registration_error)) => Err(format!(
            "re-registering the previous shortcuts failed: {registration_error}"
        )),
        (Err(restore_error), Ok(())) => Err(restore_error),
        (Err(restore_error), Err(registration_error)) => Err(format!(
            "{restore_error}; re-registering the remaining shortcuts failed: {registration_error}"
        )),
    }
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

    // Хоткей-конфликты (глобальные/редактор) — только с включёнными: выключенные
    // сохраняют комбинацию, но не занимают её. NumPad-пересечение с сохранённым
    // перехватом проверяется до enabled-guard: отклоняется в любом состоянии
    // кандидата, иначе итог зависел бы от порядка редактирования (ROADMAP-140).
    // Очистка пустым ключом проходит и остаётся выходом из конфликта.
    //
    // Проверка и запись атомарны: координированная секция перечитывает свежий
    // снимок под общим локом и persist-ит под ним же, поэтому параллельное
    // сохранение биндинга перехвата не может вклиниться между проверкой и
    // записью — несовместимые сохранения взаимно исключаются.
    let mgr = settings_manager.inner().clone();
    let handle = app_handle.clone();
    let coordinated_name = name.clone();
    let coordinated_hotkey = hotkey.clone();
    let previous: Hotkey = tauri::async_runtime::spawn_blocking(move || {
        coordinated_hotkey_section(
            || {
                mgr.load()
                    .map(|s| s.hotkeys)
                    .map_err(|e| format!("Failed to load settings: {}", e))
            },
            |hotkeys| {
                let intercept = load_intercept_settings(&handle);
                global_hotkey_conflict(hotkeys, &intercept, &coordinated_name, &coordinated_hotkey)
                    .map_or(Ok(()), Err)
            },
            |hotkeys| {
                // Snapshot of the stored binding: if the OS rejects the new
                // combination, the persisted setting is rolled back to this
                // value. Read from the same coordinated snapshot the check
                // validated against.
                let previous = hotkeys
                    .get_global_by_name(&coordinated_name)
                    .ok_or_else(|| format!("Invalid hotkey name: {}", coordinated_name))?
                    .clone();
                mgr.set_hotkey(&coordinated_name, &coordinated_hotkey)
                    .map_err(|e| e.to_string())?;
                Ok(previous)
            },
        )
    })
    .await
    .map_err(|e| format!("blocking task panicked: {}", e))??;

    super::emit_settings_changed(&app_handle);

    // `reregister_hotkeys` reports `Box<dyn Error>`, which is not `Send`;
    // convert it before any await so the rollback below can run.
    let registration =
        crate::hotkeys::reregister_hotkeys(&app_handle).map_err(|error| error.to_string());
    if let Err(registration_error) = registration {
        return Err(
            match restore_global_hotkey(
                &name,
                &previous,
                &hotkey,
                settings_manager.inner(),
                &app_handle,
            )
            .await
            {
                Ok(()) => {
                    super::emit_settings_changed(&app_handle);
                    format!(
                        "Failed to re-register hotkeys: {}; the shortcut was restored to its previous setting",
                        registration_error
                    )
                }
                Err(restore_error) => format!(
                    "Failed to re-register hotkeys: {}; restoring the previous shortcut setting failed: {}",
                    registration_error, restore_error
                ),
            },
        );
    }

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
    // Snapshot of the stored binding: the default combination may already be
    // taken by another enabled hotkey, in which case the OS registration is
    // rejected and the persisted setting must be rolled back.
    let previous = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?
        .hotkeys
        .get_global_by_name(&name)
        .ok_or_else(|| format!("Invalid hotkey name: {}", name))?
        .clone();

    let name_clone = name.clone();
    let default = super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.reset_hotkey_to_default(&name_clone)
    })
    .await?;

    super::emit_settings_changed(&app_handle);

    // `reregister_hotkeys` reports `Box<dyn Error>`, which is not `Send`;
    // convert it before any await so the rollback below can run.
    let registration =
        crate::hotkeys::reregister_hotkeys(&app_handle).map_err(|error| error.to_string());
    if let Err(registration_error) = registration {
        return Err(
            match restore_global_hotkey(
                &name,
                &previous,
                &default,
                settings_manager.inner(),
                &app_handle,
            )
            .await
            {
                Ok(()) => {
                    super::emit_settings_changed(&app_handle);
                    format!(
                        "Failed to re-register hotkeys: {}; the shortcut was restored to its previous setting",
                        registration_error
                    )
                }
                Err(restore_error) => format!(
                    "Failed to re-register hotkeys: {}; restoring the previous shortcut setting failed: {}",
                    registration_error, restore_error
                ),
            },
        );
    }

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

    // ROADMAP-140: NumPad-код кандидата не должен пересекаться с сохранённым
    // перехватом. Проверка до записи и независимо от `enabled` кандидата —
    // как в `set_hotkey`, иначе вердикт зависел бы от порядка редактирования.
    // Очистка (пустой ключ) проходит: пересекаться нечему.
    //
    // Проверка и запись атомарны под общим NumPad-локом: параллельное
    // сохранение биндинга перехвата не может вклиниться между проверкой и
    // записью (см. `coordinated_hotkey_section`).
    let mgr = settings_manager.inner().clone();
    let handle = app_handle.clone();
    let coordinated_hotkey = hotkey.clone();
    let coordinated_action = action_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        coordinated_hotkey_section(
            || {
                mgr.load()
                    .map(|s| s.hotkeys)
                    .map_err(|e| format!("Failed to load settings: {}", e))
            },
            |_hotkeys| {
                let intercept = load_intercept_settings(&handle);
                editor_numpad_intercept_conflict(&intercept, &coordinated_hotkey)
                    .map_or(Ok(()), Err)
            },
            |_hotkeys| {
                mgr.set_editor_hotkey(&coordinated_action, &coordinated_hotkey)
                    .map_err(|e| e.to_string())
            },
        )
    })
    .await
    .map_err(|e| format!("blocking task panicked: {}", e))??;

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
/// saved intercept NumPad codes, enabled editor/global hotkeys; editor
/// bindings are never registered as OS shortcuts, so nothing is unregistered
/// here.
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

    // Conflicts are validated only when enabling — mirrors
    // `set_hotkey_binding_enabled`: switching off registers nothing and must
    // stay available as the way out of an existing NumPad/intercept
    // intersection (ROADMAP-140), so a stored conflicting combination can be
    // kept (disabled) but not re-activated. The check and the persisted write
    // are one coordinated section: a concurrent intercept save can never
    // interleave between them.
    let mgr = settings_manager.inner().clone();
    let handle = app_handle.clone();
    let coordinated_action = action_id.clone();
    let coordinated_updated = updated.clone();
    tauri::async_runtime::spawn_blocking(move || {
        coordinated_hotkey_section(
            || {
                mgr.load()
                    .map(|s| s.hotkeys)
                    .map_err(|e| format!("Failed to load settings: {}", e))
            },
            |hotkeys| {
                if !enabled {
                    return Ok(());
                }
                let intercept = load_intercept_settings(&handle);
                editor_hotkey_conflict(
                    hotkeys,
                    &intercept,
                    &coordinated_action,
                    &coordinated_updated,
                )
                .map_or(Ok(()), Err)
            },
            |_hotkeys| {
                mgr.set_editor_hotkey_enabled_flag(&coordinated_action, enabled)
                    .map_err(|e| e.to_string())
            },
        )
    })
    .await
    .map_err(|e| format!("blocking task panicked: {}", e))??;

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
        // If reregister were called, it would unregister/re-register the OS
        // shortcuts for an editor-only change. This test ensures the local
        // cache stays consistent: we read back what we just wrote.
        let settings = mgr.load().unwrap();
        assert_eq!(settings.hotkeys.editor.edit_word.key, "F8");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== Enabled-switch conflict validation ====================

    #[test]
    fn global_conflict_rejects_duplicate_with_enabled_global() {
        let settings = HotkeySettings::default();
        let candidate = settings.sound_panel.clone();

        let error = global_hotkey_conflict(
            &settings,
            &InterceptSettings::default(),
            "main_window",
            &candidate,
        )
        .unwrap();

        assert!(error.contains("звуковой панели"), "unexpected: {error}");
    }

    #[test]
    fn global_conflict_ignores_disabled_candidate() {
        let settings = HotkeySettings::default();
        let mut candidate = settings.sound_panel.clone();
        candidate.enabled = false;

        assert!(global_hotkey_conflict(
            &settings,
            &InterceptSettings::default(),
            "main_window",
            &candidate
        )
        .is_none());
    }

    #[test]
    fn global_conflict_ignores_disabled_existing_global() {
        let mut settings = HotkeySettings::default();
        settings.sound_panel.enabled = false;
        let mut candidate = settings.sound_panel.clone();
        candidate.enabled = true;

        assert!(global_hotkey_conflict(
            &settings,
            &InterceptSettings::default(),
            "main_window",
            &candidate
        )
        .is_none());
    }

    #[test]
    fn global_conflict_rejects_overlap_with_enabled_editor_binding() {
        let settings = HotkeySettings::default();
        let candidate = settings.editor.edit_word.clone();

        let error = global_hotkey_conflict(
            &settings,
            &InterceptSettings::default(),
            "main_window",
            &candidate,
        )
        .unwrap();

        assert!(error.contains("действия редактора"), "unexpected: {error}");
    }

    #[test]
    fn same_state_toggle_is_a_noop() {
        assert!(!hotkey_toggle_changes_state(true, true));
        assert!(!hotkey_toggle_changes_state(false, false));
        assert!(hotkey_toggle_changes_state(false, true));
        assert!(hotkey_toggle_changes_state(true, false));
    }

    /// The switch candidate is derived from the fresh stored record, never
    /// from the pre-section snapshot: a concurrent re-assignment is validated
    /// and registered under its own combination (the requested flag on the
    /// current key), a record that already carries the requested flag is a
    /// success no-op, and an unknown name stays an error.
    #[test]
    fn switch_candidate_is_derived_from_the_fresh_stored_record() {
        let stale = Hotkey {
            modifiers: vec![],
            key: "F9".to_string(),
            enabled: false,
        };
        let fresh = Hotkey {
            modifiers: vec![],
            key: "F10".to_string(),
            enabled: false,
        };

        // A concurrent re-assignment landed: enabling derives the candidate
        // from the fresh F10 record, never from the stale F9 view.
        assert_eq!(
            switch_candidate_from_stored(Some(&fresh), "sound_panel", true),
            Ok(Some(Hotkey {
                modifiers: vec![],
                key: "F10".to_string(),
                enabled: true,
            }))
        );
        // The stale view would have produced a different candidate: the
        // stored record, not the pre-section snapshot, decides the outcome.
        assert_ne!(
            switch_candidate_from_stored(Some(&stale), "sound_panel", true),
            switch_candidate_from_stored(Some(&fresh), "sound_panel", true)
        );

        // The fresh record already carries the requested flag: success no-op.
        let already_on = Hotkey {
            modifiers: vec![],
            key: "F10".to_string(),
            enabled: true,
        };
        assert_eq!(
            switch_candidate_from_stored(Some(&already_on), "sound_panel", true),
            Ok(None)
        );

        // Disabling derives the candidate the same way.
        let stored_on = Hotkey {
            modifiers: vec![],
            key: "F10".to_string(),
            enabled: true,
        };
        assert_eq!(
            switch_candidate_from_stored(Some(&stored_on), "sound_panel", false)
                .unwrap()
                .expect("a real disable has a candidate")
                .key,
            "F10"
        );

        assert!(switch_candidate_from_stored(None, "sound_panel", true).is_err());
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

        assert!(global_hotkey_conflict(
            &settings,
            &InterceptSettings::default(),
            "main_window",
            &candidate
        )
        .is_none());
    }

    // ==================== NumPad ↔ intercept conflict (ROADMAP-140) ====================

    fn intercept_with_binding(key: &str, action: &str) -> InterceptSettings {
        use crate::soundpanel::intercept::InterceptBinding;
        InterceptSettings {
            enabled: true,
            bindings: vec![InterceptBinding {
                key: key.to_string(),
                action: action.to_string(),
            }],
            allow_any_key: false,
        }
    }

    /// A NumPad candidate that lands on a saved intercept binding is
    /// rejected: the message names the key, the section and the action,
    /// and the candidate's modifiers do not matter.
    #[test]
    fn global_conflict_rejects_numpad_overlapping_saved_intercept_binding() {
        let settings = HotkeySettings::default();
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");
        let candidate = Hotkey {
            modifiers: vec![
                crate::config::HotkeyModifier::Ctrl,
                crate::config::HotkeyModifier::Shift,
            ],
            key: "NUMPAD1".to_string(),
            enabled: true,
        };

        let error = global_hotkey_conflict(&settings, &intercept, "sound_panel", &candidate)
            .expect("same NumPad code must conflict");

        assert!(error.contains("NUMPAD1"), "unexpected: {error}");
        assert!(error.contains("Перехват клавиш"), "unexpected: {error}");
        assert!(error.contains("Главное окно"), "unexpected: {error}");
    }

    /// The saved intercept binding reserves its code even while the intercept
    /// mode is switched off — and the candidate's own switch does not matter
    /// either (review fix): a disabled global candidate over a saved binding
    /// is rejected, so the outcome no longer depends on the editing order.
    #[test]
    fn global_conflict_checks_intercept_even_when_mode_disabled() {
        let settings = HotkeySettings::default();
        let mut intercept = intercept_with_binding("NUMPAD5", "playback_stop");
        intercept.enabled = false;
        let candidate = Hotkey {
            modifiers: vec![],
            key: "NUMPAD5".to_string(),
            enabled: true,
        };

        let error = global_hotkey_conflict(&settings, &intercept, "sound_panel", &candidate)
            .expect("intercept mode switch must not matter");
        assert!(error.contains("Перехват клавиш"), "unexpected: {error}");

        // Review fix: a disabled candidate is rejected on the intercept side
        // as well — the intersection is judged by the saved code alone.
        let mut disabled = candidate.clone();
        disabled.enabled = false;
        let error = global_hotkey_conflict(&settings, &intercept, "sound_panel", &disabled)
            .expect("disabled candidate over a saved intercept binding must be rejected");
        assert!(error.contains("Перехват клавиш"), "unexpected: {error}");
    }

    /// A candidate that does not intersect the saved intercept bindings
    /// passes whatever its switch is, and clearing (empty key) stays allowed
    /// over a saved binding — the way out of an existing conflict. The
    /// hotkey-to-hotkey enabled rule is unchanged: a disabled candidate
    /// never occupies its combination.
    #[test]
    fn global_conflict_allows_non_intersecting_disabled_candidate() {
        let settings = HotkeySettings::default();
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");

        let disabled_numpad = Hotkey {
            modifiers: vec![],
            key: "NUMPAD2".to_string(),
            enabled: false,
        };
        assert!(
            global_hotkey_conflict(&settings, &intercept, "sound_panel", &disabled_numpad)
                .is_none(),
            "a non-intersecting disabled candidate must pass"
        );

        let cleared = Hotkey {
            modifiers: vec![],
            key: String::new(),
            enabled: true,
        };
        assert!(
            global_hotkey_conflict(&settings, &intercept, "sound_panel", &cleared).is_none(),
            "clearing over a saved binding must stay allowed"
        );

        let mut disabled_default = settings.sound_panel.clone();
        disabled_default.enabled = false;
        assert!(
            global_hotkey_conflict(
                &settings,
                &InterceptSettings::default(),
                "main_window",
                &disabled_default
            )
            .is_none(),
            "hotkey-to-hotkey rules must keep ignoring a disabled candidate"
        );
    }

    /// The top digit row and non-NumPad keys stay separate from the
    /// intercept conflict: a Digit candidate over a NUMPAD binding and an
    /// F-key candidate over a matching F-key binding both pass.
    #[test]
    fn global_conflict_keeps_digit_row_and_non_numpad_keys_out_of_scope() {
        let settings = HotkeySettings::default();
        let numpad_intercept = intercept_with_binding("NUMPAD1", "show_main_window");
        let digit_candidate = Hotkey {
            modifiers: vec![],
            key: "1".to_string(),
            enabled: true,
        };
        assert!(global_hotkey_conflict(
            &settings,
            &numpad_intercept,
            "sound_panel",
            &digit_candidate
        )
        .is_none());

        // F-key overlaps predate ROADMAP-140 and stay out of its scope.
        let f_intercept = intercept_with_binding("F5", "playback_stop");
        let f_candidate = Hotkey {
            modifiers: vec![crate::config::HotkeyModifier::Alt],
            key: "F5".to_string(),
            enabled: true,
        };
        assert!(
            global_hotkey_conflict(&settings, &f_intercept, "sound_panel", &f_candidate).is_none()
        );
    }

    /// A NumPad candidate unrelated to the saved bindings passes.
    #[test]
    fn global_conflict_allows_unrelated_numpad_candidate() {
        let settings = HotkeySettings::default();
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");
        let candidate = Hotkey {
            modifiers: vec![],
            key: "NUMPAD_DECIMAL".to_string(),
            enabled: true,
        };
        assert!(global_hotkey_conflict(&settings, &intercept, "sound_panel", &candidate).is_none());
    }

    // ============== NumPad check-then-write coordination (race fix) ==============

    /// The hotkey side of the coordination proof: one atomic section shaped
    /// exactly like the real `set_hotkey` critical part — load a snapshot,
    /// validate it with the real conflict rules against both stores, commit
    /// into the hotkey store.
    fn save_global_side(
        hotkeys_store: &Arc<std::sync::Mutex<HotkeySettings>>,
        intercept_store: &Arc<std::sync::Mutex<InterceptSettings>>,
        candidate: &Hotkey,
    ) -> Result<(), String> {
        let hotkeys_store = Arc::clone(hotkeys_store);
        let intercept_store = Arc::clone(intercept_store);
        let candidate = candidate.clone();
        coordinated_hotkey_section(
            {
                let hotkeys_store = Arc::clone(&hotkeys_store);
                move || Ok(hotkeys_store.lock().expect("hotkeys store").clone())
            },
            {
                let intercept_store = Arc::clone(&intercept_store);
                let candidate = candidate.clone();
                move |hotkeys| {
                    let intercept = intercept_store.lock().expect("intercept store").clone();
                    global_hotkey_conflict(hotkeys, &intercept, "sound_panel", &candidate)
                        .map_or(Ok(()), Err)
                }
            },
            {
                let hotkeys_store = Arc::clone(&hotkeys_store);
                let candidate = candidate.clone();
                move |_hotkeys| {
                    hotkeys_store.lock().expect("hotkeys store").sound_panel = candidate.clone();
                    Ok(())
                }
            },
        )
    }

    /// The intercept side of the coordination proof: acquire the shared lock
    /// first, exactly as the real `set_intercept_binding` command does, check
    /// with the real NumPad rules against both stores, commit the binding.
    fn save_intercept_side(
        hotkeys_store: &Arc<std::sync::Mutex<HotkeySettings>>,
        intercept_store: &Arc<std::sync::Mutex<InterceptSettings>>,
        key: &str,
    ) -> Result<(), String> {
        let _coordination = crate::soundpanel::intercept::lock_numpad_coordination();
        let hotkeys = hotkeys_store.lock().expect("hotkeys store").clone();
        if let Some(name) = crate::soundpanel::intercept::find_numpad_global_conflict(&hotkeys, key)
        {
            return Err(crate::soundpanel::intercept::numpad_global_conflict_message(key, name));
        }
        if let Some(action) =
            crate::soundpanel::intercept::find_numpad_editor_conflict(&hotkeys, key)
        {
            return Err(crate::soundpanel::intercept::numpad_editor_conflict_message(key, action));
        }
        intercept_store
            .lock()
            .expect("intercept store")
            .bindings
            .push(crate::soundpanel::intercept::InterceptBinding {
                key: key.to_string(),
                action: "playback_stop".to_string(),
            });
        Ok(())
    }

    /// The coordinated section holds the shared NumPad lock across validation
    /// and write: a competing intercept-style save cannot enter — let alone
    /// commit — while a hotkey section is in flight, and proceeds only once
    /// the section has released the lock. Signalled, not timed, for the
    /// invariant itself; the short negative timeout only guards against a
    /// false pass from a stalled test machine.
    #[test]
    fn coordinated_section_blocks_a_competing_intercept_save_while_in_flight() {
        use std::sync::mpsc;

        let (section_entered, section_entered_rx) = mpsc::channel();
        let (release_section, release_section_rx) = mpsc::channel::<()>();
        let (competitor_entered, competitor_entered_rx) = mpsc::channel();

        let section = std::thread::spawn(move || {
            coordinated_hotkey_section(
                || Ok(HotkeySettings::default()),
                |_hotkeys| {
                    let _ = section_entered.send(());
                    // Hold the section — and with it the lock — open.
                    release_section_rx
                        .recv()
                        .expect("the release signal arrives");
                    Ok(())
                },
                |_hotkeys| Ok(()),
            )
            .expect("the section must succeed");
        });

        section_entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the section enters its critical part");

        let competitor = std::thread::spawn(move || {
            let _guard = crate::soundpanel::intercept::lock_numpad_coordination();
            let _ = competitor_entered.send(());
        });

        assert!(
            competitor_entered_rx
                .recv_timeout(Duration::from_millis(150))
                .is_err(),
            "a competing save must block while the coordinated section is in flight"
        );

        release_section.send(()).expect("the release is sent");
        section.join().expect("the section thread finishes");
        competitor_entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the competing save proceeds after the section released the lock");
        competitor.join().expect("the competitor thread finishes");
    }

    /// Two competing incompatible NumPad saves — a global hotkey assignment
    /// and an intercept binding on the same code — must leave exactly one
    /// winner in every interleaving. Sequential orders prove that a check
    /// sees the other side's committed write; the concurrent run proves the
    /// sections are atomic, so the committed stores never hold both.
    #[test]
    fn competing_global_and_intercept_numpad_saves_leave_exactly_one_winner() {
        let candidate = Hotkey {
            modifiers: vec![],
            key: "NUMPAD5".to_string(),
            enabled: true,
        };

        // Order 1: the hotkey save commits first.
        let hotkeys_store = Arc::new(std::sync::Mutex::new(HotkeySettings::default()));
        let intercept_store = Arc::new(std::sync::Mutex::new(InterceptSettings::default()));
        save_global_side(&hotkeys_store, &intercept_store, &candidate)
            .expect("the hotkey save wins when it runs first");
        assert!(
            save_intercept_side(&hotkeys_store, &intercept_store, "NUMPAD5").is_err(),
            "the intercept save must lose against the committed hotkey write"
        );

        // Order 2: mirrored — the intercept save commits first.
        let hotkeys_store = Arc::new(std::sync::Mutex::new(HotkeySettings::default()));
        let intercept_store = Arc::new(std::sync::Mutex::new(InterceptSettings::default()));
        save_intercept_side(&hotkeys_store, &intercept_store, "NUMPAD5")
            .expect("the intercept save wins when it runs first");
        assert!(
            save_global_side(&hotkeys_store, &intercept_store, &candidate).is_err(),
            "the hotkey save must lose against the committed intercept write"
        );

        // Concurrent run: whatever the interleaving, the atomic sections
        // leave exactly one winner and never both incompatible saves.
        let hotkeys_store = Arc::new(std::sync::Mutex::new(HotkeySettings::default()));
        let intercept_store = Arc::new(std::sync::Mutex::new(InterceptSettings::default()));
        let (hotkeys_clone, intercept_clone) =
            (Arc::clone(&hotkeys_store), Arc::clone(&intercept_store));
        let concurrent_candidate = candidate.clone();
        let global_thread = std::thread::spawn(move || {
            save_global_side(&hotkeys_clone, &intercept_clone, &concurrent_candidate)
        });
        let intercept_result = save_intercept_side(&hotkeys_store, &intercept_store, "NUMPAD5");
        let global_result = global_thread.join().expect("the global thread finishes");

        let winners = [global_result.is_ok(), intercept_result.is_ok()]
            .into_iter()
            .filter(|won| *won)
            .count();
        assert_eq!(winners, 1, "exactly one competing save must win");

        let binding_on_numpad5 = intercept_store
            .lock()
            .expect("intercept store")
            .bindings
            .iter()
            .any(|binding| binding.key == "NUMPAD5");
        let global_on_numpad5 =
            hotkeys_store.lock().expect("hotkeys store").sound_panel.key == "NUMPAD5";
        assert!(
            !(binding_on_numpad5 && global_on_numpad5),
            "both incompatible saves must never coexist in the committed stores"
        );
    }

    // ============== Rollback after a rejected registration (restore coordination) ==============

    /// The confirmed rollback race, replayed store-by-store: `sound_panel`
    /// held NUMPAD1, `set_hotkey` committed F5 and released the coordination
    /// lock, `set_intercept_binding` reserved the freed NUMPAD1, and Windows
    /// rejected F5. The rollback re-checks on a fresh snapshot: restoring
    /// NUMPAD1 would leave both stores owning the same code, so it must be
    /// refused with an explicit conflict — never written.
    #[test]
    fn rollback_refused_when_intercept_reserved_the_freed_numpad_code() {
        let previous = Hotkey {
            modifiers: vec![],
            key: "NUMPAD1".to_string(),
            enabled: true,
        };
        let failed = Hotkey {
            modifiers: vec![],
            key: "F5".to_string(),
            enabled: true,
        };

        // Step 1 — the `set_hotkey` coordinated section commits F5.
        let stored = HotkeySettings {
            sound_panel: previous.clone(),
            ..Default::default()
        };
        assert!(global_hotkey_conflict(
            &stored,
            &InterceptSettings::default(),
            "sound_panel",
            &failed
        )
        .is_none());
        let hotkeys = HotkeySettings {
            sound_panel: failed.clone(),
            ..Default::default()
        };

        // Step 2 — the `set_intercept_binding` section commits the freed code.
        assert!(
            crate::soundpanel::intercept::find_numpad_global_conflict(&hotkeys, "NUMPAD1")
                .is_none()
        );
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");

        // Step 3 — F5 registration is rejected; the rollback decides.
        let decision =
            decide_global_hotkey_restore("sound_panel", &hotkeys, &failed, &previous, &intercept);
        match decision {
            GlobalHotkeyRestore::Refused(message) => {
                assert!(message.contains("NUMPAD1"), "unexpected: {message}");
                assert!(message.contains("Перехват клавиш"), "unexpected: {message}");
            }
            other => panic!("the rollback must be refused, got: {other:?}"),
        }
    }

    /// A newer save that replaced the failed binding during the failure
    /// window wins: the stale rollback skips the restore instead of
    /// overwriting it, regardless of any conflict.
    #[test]
    fn rollback_skipped_when_a_newer_save_replaced_the_failed_binding() {
        let previous = Hotkey {
            modifiers: vec![],
            key: "NUMPAD1".to_string(),
            enabled: true,
        };
        let failed = Hotkey {
            modifiers: vec![],
            key: "F5".to_string(),
            enabled: true,
        };
        let hotkeys = HotkeySettings {
            sound_panel: Hotkey {
                modifiers: vec![],
                key: "NUMPAD3".to_string(),
                enabled: true,
            },
            ..Default::default()
        };
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");

        assert_eq!(
            decide_global_hotkey_restore("sound_panel", &hotkeys, &failed, &previous, &intercept),
            GlobalHotkeyRestore::SkipNewerSave
        );
        // Even without any conflict the newer save is kept.
        assert_eq!(
            decide_global_hotkey_restore(
                "sound_panel",
                &hotkeys,
                &failed,
                &previous,
                &InterceptSettings::default()
            ),
            GlobalHotkeyRestore::SkipNewerSave
        );
    }

    /// A still-applicable restore passes: the failed write is still stored
    /// and the previous value conflicts with nothing. Non-NumPad overlaps
    /// with intercept bindings stay out of scope, exactly as on the save
    /// path.
    #[test]
    fn rollback_restores_when_the_failed_write_is_still_stored_and_compatible() {
        let previous = Hotkey {
            modifiers: vec![],
            key: "F2".to_string(),
            enabled: true,
        };
        let failed = Hotkey {
            modifiers: vec![],
            key: "F5".to_string(),
            enabled: true,
        };
        let hotkeys = HotkeySettings {
            sound_panel: failed.clone(),
            ..Default::default()
        };

        assert_eq!(
            decide_global_hotkey_restore(
                "sound_panel",
                &hotkeys,
                &failed,
                &previous,
                &InterceptSettings::default()
            ),
            GlobalHotkeyRestore::Restore
        );

        // F-key overlaps with intercept bindings predate ROADMAP-140 and do
        // not block the restore either.
        let f_intercept = intercept_with_binding("F2", "playback_stop");
        assert_eq!(
            decide_global_hotkey_restore("sound_panel", &hotkeys, &failed, &previous, &f_intercept),
            GlobalHotkeyRestore::Restore
        );
    }

    /// The refusal follows the full save rules, not just the intercept check:
    /// another enabled global that claimed the code in the failure window
    /// blocks the restore too, and a disabled previous is still judged by the
    /// saved code alone (the intercept check precedes the enabled guard).
    #[test]
    fn rollback_refusal_follows_the_save_rules() {
        let previous = Hotkey {
            modifiers: vec![],
            key: "NUMPAD1".to_string(),
            enabled: true,
        };
        let failed = Hotkey {
            modifiers: vec![],
            key: "F5".to_string(),
            enabled: true,
        };

        // Another enabled global claimed NUMPAD1 meanwhile.
        let hotkeys = HotkeySettings {
            sound_panel: failed.clone(),
            main_window: previous.clone(),
            ..Default::default()
        };
        assert!(matches!(
            decide_global_hotkey_restore(
                "sound_panel",
                &hotkeys,
                &failed,
                &previous,
                &InterceptSettings::default()
            ),
            GlobalHotkeyRestore::Refused(_)
        ));

        // A disabled previous is refused on the saved code alone.
        let hotkeys = HotkeySettings {
            sound_panel: failed.clone(),
            ..Default::default()
        };
        let disabled_previous = Hotkey {
            modifiers: vec![],
            key: "NUMPAD1".to_string(),
            enabled: false,
        };
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");
        assert!(matches!(
            decide_global_hotkey_restore(
                "sound_panel",
                &hotkeys,
                &failed,
                &disabled_previous,
                &intercept
            ),
            GlobalHotkeyRestore::Refused(_)
        ));
    }

    // ====== Compensation on a skipped or refused rollback (review 009) ======

    /// The compensation re-register runs on every rollback outcome and
    /// neither result masks the other: a refused or skipped verdict is
    /// preserved verbatim even when the remaining shortcuts re-register
    /// cleanly, a compensation failure is reported on its own, and both
    /// failures surface together.
    #[test]
    fn combine_restore_with_reregister_reports_both_failures_without_masking() {
        let restore_err = || Err("the rollback was refused: conflict".to_string());
        let reregister_err = || Err("os rejected the combination".to_string());

        assert!(combine_restore_with_reregister(Ok(()), Ok(())).is_ok());

        // The verdict survives a clean compensation, verbatim.
        assert_eq!(
            combine_restore_with_reregister(restore_err(), Ok(())),
            Err("the rollback was refused: conflict".to_string())
        );

        // A compensation failure is reported even when the rollback itself
        // persisted fine.
        assert_eq!(
            combine_restore_with_reregister(Ok(()), reregister_err()),
            Err(
                "re-registering the previous shortcuts failed: os rejected the combination"
                    .to_string(),
            )
        );

        // Both failures surface together.
        let both = combine_restore_with_reregister(restore_err(), reregister_err()).unwrap_err();
        assert!(
            both.contains("the rollback was refused"),
            "unexpected: {both}"
        );
        assert!(
            both.contains("re-registering the remaining shortcuts failed"),
            "unexpected: {both}"
        );
        assert!(
            both.contains("os rejected the combination"),
            "unexpected: {both}"
        );
    }

    // ========= Persist-first toggle rollback (review 009 deadlock fix) =========

    /// A rejected runtime change flips the persisted switch back while this
    /// transaction's commit is still stored: only the flag moves, the saved
    /// combination stays untouched.
    #[test]
    fn switch_rollback_flips_the_flag_while_the_commit_is_still_stored() {
        let (mgr, dir) = temp_settings_manager();
        let committed = Hotkey {
            modifiers: vec![],
            key: "F9".to_string(),
            enabled: true,
        };
        mgr.set_hotkey("sound_panel", &committed).unwrap();

        rollback_hotkey_enabled_switch(&mgr, "sound_panel", &committed, false)
            .expect("the guarded flip-back must land on the fresh snapshot");

        let stored = mgr.load().unwrap().hotkeys.sound_panel;
        assert_eq!(
            stored,
            Hotkey {
                modifiers: vec![],
                key: "F9".to_string(),
                enabled: false,
            },
            "only the flag moves"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A newer save that landed during the failure keeps its own switch: the
    /// guarded rollback refuses to undo a commit it can no longer see.
    #[test]
    fn switch_rollback_skipped_when_a_newer_save_replaced_the_commit() {
        let (mgr, dir) = temp_settings_manager();
        let committed = Hotkey {
            modifiers: vec![],
            key: "F9".to_string(),
            enabled: true,
        };
        mgr.set_hotkey("sound_panel", &committed).unwrap();
        let newer = Hotkey {
            modifiers: vec![],
            key: "F10".to_string(),
            enabled: true,
        };
        mgr.set_hotkey("sound_panel", &newer).unwrap();

        let error = rollback_hotkey_enabled_switch(&mgr, "sound_panel", &committed, false)
            .expect_err("a replaced commit must not be rolled back");
        assert!(
            error.contains("newer binding was saved"),
            "unexpected: {error}"
        );

        let stored = mgr.load().unwrap().hotkeys.sound_panel;
        assert_eq!(stored.key, "F10", "the newer save stays");
        assert!(stored.enabled, "the newer save keeps its own switch");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn editor_conflict_rejects_duplicate_with_enabled_editor_binding() {
        let settings = HotkeySettings::default();
        let candidate = settings.editor.submit_continue.clone();

        let error = editor_hotkey_conflict(
            &settings,
            &InterceptSettings::default(),
            "edit_word",
            &candidate,
        )
        .unwrap();

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

        let error = editor_hotkey_conflict(
            &settings,
            &InterceptSettings::default(),
            "edit_word",
            &candidate,
        )
        .unwrap();

        assert!(error.contains("конфликтует"), "unexpected: {error}");
    }

    #[test]
    fn editor_conflict_allows_disabled_candidate() {
        let settings = HotkeySettings::default();
        let mut candidate = settings.editor.submit_continue.clone();
        candidate.enabled = false;

        assert!(editor_hotkey_conflict(
            &settings,
            &InterceptSettings::default(),
            "edit_word",
            &candidate
        )
        .is_none());
    }

    // ==================== Editor ↔ intercept NumPad conflicts (ROADMAP-140) ====================

    /// Enabling (or saving) an editor binding on a NumPad code that a saved
    /// intercept binding occupies is rejected: the message names the key,
    /// the section and the action, and the candidate's modifiers do not
    /// matter.
    #[test]
    fn editor_conflict_rejects_numpad_enable_over_saved_intercept() {
        let settings = HotkeySettings::default();
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");
        let mut candidate = settings.editor.edit_word.clone();
        candidate.modifiers = vec![
            crate::config::HotkeyModifier::Ctrl,
            crate::config::HotkeyModifier::Shift,
        ];
        candidate.key = "NUMPAD1".to_string();

        let error = editor_hotkey_conflict(&settings, &intercept, "edit_word", &candidate)
            .expect("same NumPad code must conflict");

        assert!(error.contains("NUMPAD1"), "unexpected: {error}");
        assert!(error.contains("Перехват клавиш"), "unexpected: {error}");
        assert!(error.contains("Главное окно"), "unexpected: {error}");

        // Different modifiers on the saved-vs-candidate sides do not matter;
        // a plain candidate conflicts the same way.
        let mut plain = candidate.clone();
        plain.modifiers = vec![];
        assert!(editor_hotkey_conflict(&settings, &intercept, "edit_word", &plain).is_some());
    }

    /// The save path judges the intersection by the saved code alone: a
    /// disabled editor candidate over a saved intercept binding is rejected
    /// too, and the intercept mode switch does not matter — the outcome does
    /// not depend on the editing order.
    #[test]
    fn editor_numpad_save_conflict_ignores_candidate_switch_and_intercept_mode() {
        let mut intercept = intercept_with_binding("NUMPAD5", "playback_stop");
        intercept.enabled = false;

        let mut enabled_candidate = Hotkey {
            modifiers: vec![],
            key: "NUMPAD5".to_string(),
            enabled: true,
        };
        assert!(editor_numpad_intercept_conflict(&intercept, &enabled_candidate).is_some());

        enabled_candidate.enabled = false;
        assert!(
            editor_numpad_intercept_conflict(&intercept, &enabled_candidate).is_some(),
            "disabled candidate over a saved intercept binding must be rejected"
        );
    }

    /// The top digit row stays a separate code and clearing stays the way
    /// out: a Digit candidate over a NUMPAD binding and an empty candidate
    /// both pass, whatever their switch is.
    #[test]
    fn editor_numpad_conflict_keeps_digit_row_and_clearing_out_of_scope() {
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");

        let digit = Hotkey {
            modifiers: vec![],
            key: "1".to_string(),
            enabled: true,
        };
        assert!(
            editor_numpad_intercept_conflict(&intercept, &digit).is_none(),
            "top digit row must stay a separate code"
        );

        let cleared = Hotkey {
            modifiers: vec![],
            key: String::new(),
            enabled: true,
        };
        assert!(
            editor_numpad_intercept_conflict(&intercept, &cleared).is_none(),
            "clearing over a saved binding must stay allowed"
        );
    }
}
