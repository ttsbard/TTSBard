//! Sound Panel Tauri Commands
//!
//! Tauri команды для взаимодействия между frontend и backend.

use crate::commands::window::resolve_main_appearance;
use crate::config::{is_valid_hex_color, SettingsManager, WindowsManager};
use crate::soundpanel::intercept::InterceptSettings;
use crate::soundpanel::state::{SoundBinding, SoundPanelState, SoundSet, SoundSets};
use crate::soundpanel::storage::{delete_sound_file, persist_sets, stage_sound_file};
use crate::soundpanel_window::{
    emit_soundpanel_bindings_changed, hide_soundpanel_window, restore_soundpanel_foreground,
    restore_soundpanel_foreground_retaining_target, update_soundpanel_appearance,
};
use crate::state::AppState;
use tauri::{AppHandle, Emitter, Manager, State};
use tracing::{debug, error, info, warn};

/// Получить все привязки звуковой панели (активный набор)
#[tauri::command]
pub fn sp_get_bindings(state: State<'_, SoundPanelState>) -> Result<Vec<SoundBinding>, String> {
    debug!("Get bindings command");
    Ok(state.get_all_bindings())
}

/// Добавить новую привязку в активный набор
///
/// # Аргументы
/// * `key` - Клавиша (A-Z)
/// * `description` - Описание звука
/// * `file_path` - Путь к исходному аудиофайлу
#[tauri::command]
pub fn sp_add_binding(
    key: String,
    description: String,
    file_path: String,
    app_handle: AppHandle,
    state: State<'_, SoundPanelState>,
) -> Result<SoundBinding, String> {
    info!(key, description, "Add binding");

    let key_char = key.to_uppercase().chars().next().ok_or("Key is empty")?;

    if !(key_char.is_ascii_uppercase() || key_char.is_ascii_digit()) {
        return Err("Key must be A–Z or 0–9".to_string());
    }

    // Сериализует всю мутацию набора: проверка занятости клавиши, staged-аудио,
    // persist кандидата и публикация в runtime.
    let _writer_guard = state.lock_sets_writer()?;

    if let Some(existing) = state.get_binding(key_char) {
        return Err(format!(
            "Key {} is already bound to '{}'",
            key_char, existing.description
        ));
    }

    let appdata_path = state
        .appdata_path
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .clone();
    let original_snapshot = state.get_sets();

    let staged = stage_sound_file(&file_path, &appdata_path)?;
    let filename = staged.final_filename();

    let binding = SoundBinding {
        key: key_char,
        description,
        filename,
        original_path: Some(file_path),
    };

    let candidate = state.candidate_add_binding(&binding);

    if let Err(error) = persist_sets(&appdata_path, &candidate) {
        staged.cleanup();
        warn!(%error, "Persist failed on add; cleaning staged audio");
        return Err("Failed to save binding".to_string());
    }

    if let Err(error) = staged.commit() {
        error!(%error, "Staged audio promotion failed; persisting original snapshot as compensation");
        if let Err(compensation_error) = persist_sets(&appdata_path, &original_snapshot) {
            error!(%compensation_error, "Compensation persist failed; bindings file left with candidate snapshot");
        }
        return Err("Failed to save sound file".to_string());
    }

    state.publish_sets(candidate);

    let _ = emit_soundpanel_bindings_changed(&app_handle);

    info!("Binding added successfully");
    Ok(binding)
}

/// Обновить привязку: описание и (опционально) заменить файл.
///
/// * `key` - Клавиша A–Z или 0–9
/// * `description` - Новое описание звука
/// * `file_path` - Путь к новому аудиофайлу; `None` — оставить текущий файл
#[tauri::command]
pub fn sp_update_binding(
    key: String,
    description: String,
    file_path: Option<String>,
    app_handle: AppHandle,
    state: State<'_, SoundPanelState>,
) -> Result<SoundBinding, String> {
    info!(key, description, "Update binding");

    let key_char = key.to_uppercase().chars().next().ok_or("Key is empty")?;

    let _writer_guard = state.lock_sets_writer()?;

    let binding = state
        .get_binding(key_char)
        .ok_or_else(|| format!("Key {} is not bound", key_char))?;

    let appdata_path = state
        .appdata_path
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .clone();
    let original_snapshot = state.get_sets();

    let mut updated = binding.clone();
    updated.description = description;

    let staged = match file_path.filter(|p| !p.is_empty()) {
        Some(file_path) => {
            let staged = stage_sound_file(&file_path, &appdata_path)?;
            updated.filename = staged.final_filename();
            updated.original_path = Some(file_path);
            Some(staged)
        }
        None => None,
    };

    let candidate = state.candidate_add_binding(&updated);

    if let Err(error) = persist_sets(&appdata_path, &candidate) {
        if let Some(staged) = staged {
            staged.cleanup();
        }
        warn!(%error, "Persist failed on update; cleaning staged audio");
        return Err("Failed to save binding".to_string());
    }

    if let Some(staged) = staged {
        if let Err(error) = staged.commit() {
            error!(%error, "Staged audio promotion failed; persisting original snapshot as compensation");
            if let Err(compensation_error) = persist_sets(&appdata_path, &original_snapshot) {
                error!(%compensation_error, "Compensation persist failed; bindings file left with candidate snapshot");
            }
            return Err("Failed to save sound file".to_string());
        }
    }

    let should_delete_old = !candidate.references_filename(&binding.filename);

    state.publish_sets(candidate);

    if should_delete_old {
        if let Err(error) = delete_sound_file(&binding.filename, &appdata_path) {
            warn!(%error, "Failed to clean up old sound file");
        }
    }

    let _ = emit_soundpanel_bindings_changed(&app_handle);

    info!("Binding updated successfully");
    Ok(updated)
}

/// Удалить привязку по клавише из активного набора
#[tauri::command]
pub fn sp_remove_binding(
    key: String,
    app_handle: AppHandle,
    state: State<'_, SoundPanelState>,
) -> Result<(), String> {
    info!(key, "Remove binding");

    let key_char = key.chars().next().ok_or("Key is empty")?;

    let _writer_guard = state.lock_sets_writer()?;

    let binding = state.get_binding(key_char);

    let appdata_path = state
        .appdata_path
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .clone();

    let candidate = state.candidate_remove_binding(key_char);

    if let Err(error) = persist_sets(&appdata_path, &candidate) {
        warn!(%error, "Persist failed on remove");
        return Err("Failed to save binding".to_string());
    }

    let should_delete_old = binding
        .as_ref()
        .map(|b| !candidate.references_filename(&b.filename))
        .unwrap_or(false);

    state.publish_sets(candidate);

    if should_delete_old {
        if let Some(binding) = binding {
            if let Err(error) = delete_sound_file(&binding.filename, &appdata_path) {
                warn!(%error, "Failed to clean up removed sound file");
            }
        }
    }

    let _ = emit_soundpanel_bindings_changed(&app_handle);

    info!("Binding removed successfully");
    Ok(())
}

/// Тестировать воспроизведение звука
///
/// Ставит указанный файл и снимок настроек в очередь SoundPanel без
/// блокирующего воспроизведения в команде.
#[tauri::command]
pub fn sp_test_sound(
    file_path: String,
    settings_manager: State<'_, SettingsManager>,
    state: State<'_, SoundPanelState>,
) -> Result<(), String> {
    info!(file_path, "Test sound");

    if !std::path::Path::new(&file_path).exists() {
        return Err("File not found".to_string());
    }

    let audio_settings = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?
        .audio;

    state.enqueue_sound(file_path, audio_settings)
}

/// Проверить, поддерживается ли формат файла
#[tauri::command]
pub fn sp_is_supported_format(filename: String) -> Result<bool, String> {
    Ok(crate::soundpanel::audio::is_supported_audio_format(
        &filename,
    ))
}

/// Получить настройки внешнего вида floating окна звуковой панели
///
/// Если `appearance_source == "main"`, цвет наследуется от главного окна
/// (с учётом активной темы, когда собственный цвет главного окна выключен).
/// Прозрачность всегда берётся из собственной настройки SoundPanel.
#[tauri::command]
pub fn sp_get_floating_appearance(
    state: State<'_, SoundPanelState>,
    windows_manager: State<'_, WindowsManager>,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(u8, String), String> {
    let opacity = state.get_floating_opacity();
    if windows_manager.get_soundpanel_appearance_source() == "main" {
        let (_, color) = resolve_main_appearance(&windows_manager, &settings_manager);
        return Ok((opacity, color));
    }
    let color = state.get_floating_bg_color();
    Ok((opacity, color))
}

/// Установить прозрачность floating окна звуковой панели
///
/// Сначала сохраняет настройку в `WindowsManager`, и только после успешной
/// записи обновляет runtime-состояние (которое рассылает
/// `SoundPanelAppearanceChanged`).
#[tauri::command]
pub fn sp_set_floating_opacity(
    value: u8,
    state: State<'_, SoundPanelState>,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(value, "Setting opacity");
    windows_manager
        .set_soundpanel_opacity(value)
        .map_err(|e| format!("Failed to save settings: {}", e))?;
    state.set_floating_opacity(value);
    Ok(())
}

/// Установить цвет фона floating окна звуковой панели
///
/// Сначала сохраняет настройку в `WindowsManager`, и только после успешной
/// записи обновляет runtime-состояние (которое рассылает
/// `SoundPanelAppearanceChanged`).
#[tauri::command]
pub fn sp_set_floating_bg_color(
    color: String,
    state: State<'_, SoundPanelState>,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    if !is_valid_hex_color(&color) {
        return Err("Invalid color format. Use #RRGGBB".to_string());
    }
    info!(color, "Setting bg color");
    windows_manager
        .set_soundpanel_bg_color(color.clone())
        .map_err(|e| format!("Failed to save settings: {}", e))?;
    state.set_floating_bg_color(color);
    Ok(())
}

/// Установить clickthrough для floating окна звуковой панели
///
/// Сначала сохраняет настройку в `WindowsManager`, и только после успешной
/// записи обновляет runtime-состояние (которое рассылает
/// `SoundPanelAppearanceChanged`).
#[tauri::command]
pub fn sp_set_floating_clickthrough(
    enabled: bool,
    state: State<'_, SoundPanelState>,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(enabled, "Setting clickthrough");
    windows_manager
        .set_soundpanel_clickthrough(enabled)
        .map_err(|e| format!("Failed to save settings: {}", e))?;
    state.set_floating_clickthrough(enabled);
    Ok(())
}

/// Проверить, включен ли clickthrough для floating окна звуковой панели
#[tauri::command]
pub fn sp_is_floating_clickthrough_enabled(
    state: State<'_, SoundPanelState>,
) -> Result<bool, String> {
    Ok(state.is_floating_clickthrough_enabled())
}

/// Проверить, включён ли stay_visible
#[tauri::command]
pub fn sp_get_stay_visible(state: State<'_, SoundPanelState>) -> Result<bool, String> {
    Ok(state.get_stay_visible())
}

/// Установить stay_visible
///
/// Сначала сохраняет настройку на диск, затем обновляет runtime-состояние
/// и отправляет событие `soundpanel-appearance-update` для обновления
/// открытой панели.
#[tauri::command]
pub fn sp_set_stay_visible(
    enabled: bool,
    app_handle: AppHandle,
    state: State<'_, SoundPanelState>,
    windows_manager: State<'_, WindowsManager>,
) -> Result<(), String> {
    info!(enabled, "Setting stay_visible");
    windows_manager
        .set_soundpanel_stay_visible(enabled)
        .map_err(|e| format!("Failed to save settings: {}", e))?;
    state.set_stay_visible(enabled);
    let _ = update_soundpanel_appearance(&app_handle);
    Ok(())
}

/// Установить транзитный флаг config-режима (не сохраняется на диск).
///
/// Пока активен config-режим, blur-скрытие панели подавляется (например,
/// открытие native file picker не должно скрывать панель).
#[tauri::command]
pub fn sp_set_config_mode(enabled: bool, state: State<'_, SoundPanelState>) -> Result<(), String> {
    info!(enabled, "Setting config_mode");
    state.set_config_mode(enabled);
    Ok(())
}

/// Обработка Escape: следует той же политике видимости, что и активация
/// звука, но без воспроизведения.
///
/// - `stay_visible == false`: `hide → restore`
/// - `stay_visible == true`:  `restore` (панель остаётся видимой)
#[tauri::command]
pub fn sp_escape_soundpanel(
    app_handle: AppHandle,
    state: State<'_, SoundPanelState>,
) -> Result<(), String> {
    let stay_visible = state.get_stay_visible();
    let app_state = app_handle.state::<AppState>();
    handoff_soundpanel_ordered(
        stay_visible,
        || {
            hide_soundpanel_window(&app_handle, &app_state)
                .map_err(|e| format!("Failed to hide window: {}", e))
        },
        || {
            if stay_visible {
                restore_soundpanel_foreground_retaining_target(&app_state)
            } else {
                restore_soundpanel_foreground(&app_state)
            }
        },
    )
}

/// Воспроизвести звук по клавише (A-Z).
///
/// При выключенном `stay_visible` сначала синхронно скрывается окно, затем
/// предпринимается попытка вернуть фокус сохранённому внешнему окну, и только
/// после этого звук ставится в очередь SoundPanel. Ошибка скрытия останавливает
/// выполнение; ошибка восстановления фокуса возвращается, но не подавляет
/// постановку в очередь; ошибка постановки в очередь (переполнение/worker)
/// возвращается наружу.
///
/// При включённом `stay_visible` панель остаётся видимой, фокус возвращается,
/// и звук ставится в очередь.
#[tauri::command]
pub fn sp_play_binding(key: String, app_handle: AppHandle) -> Result<(), String> {
    let key_char = key.chars().next().ok_or("Key is empty")?;
    if !(key_char.is_ascii_uppercase() || key_char.is_ascii_digit()) {
        return Err("Key must be A–Z or 0–9".to_string());
    }
    let state = app_handle.state::<SoundPanelState>();
    let binding = state
        .get_binding(key_char)
        .ok_or_else(|| format!("No binding for key {}", key_char))?;
    let stay_visible = state.get_stay_visible();
    info!(key = %key_char, description = binding.description, stay_visible, "Playing binding");

    let settings_manager = app_handle.state::<SettingsManager>();
    let audio_settings = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?
        .audio;

    let app_state = app_handle.state::<AppState>();
    play_binding_ordered(
        stay_visible,
        || {
            hide_soundpanel_window(&app_handle, &app_state)
                .map_err(|e| format!("Failed to hide window: {}", e))
        },
        || {
            if stay_visible {
                restore_soundpanel_foreground_retaining_target(&app_state)
            } else {
                restore_soundpanel_foreground(&app_state)
            }
        },
        || state.play_sound(&binding, audio_settings),
    )
}

/// Единственный владелец последовательности «скрыть → восстановить фокус → поставить в очередь».
///
/// При `stay_visible == true` скрытие пропускается. Восстановление фокуса
/// всегда происходит до постановки в очередь. Ошибка скрытия останавливает
/// выполнение только в unchecked-режиме. Ошибка восстановления фокуса
/// возвращается, но не подавляет постановку в очередь в обоих режимах. Ошибка
/// постановки в очередь имеет приоритет над ошибкой восстановления фокуса.
fn play_binding_ordered<H, R, F>(
    stay_visible: bool,
    hide: H,
    restore: R,
    play: F,
) -> Result<(), String>
where
    H: FnOnce() -> Result<(), String>,
    R: FnOnce() -> Result<(), String>,
    F: FnOnce() -> Result<(), String>,
{
    if !stay_visible {
        hide()?;
    }
    let restore_result = restore();
    let play_result = play();
    match play_result {
        Err(error) => Err(error),
        Ok(()) => restore_result,
    }
}

/// Выполнить оконную часть Escape без воспроизведения.
fn handoff_soundpanel_ordered<H, R>(stay_visible: bool, hide: H, restore: R) -> Result<(), String>
where
    H: FnOnce() -> Result<(), String>,
    R: FnOnce() -> Result<(), String>,
{
    if !stay_visible {
        hide()?;
    }
    restore()
}

/// Получить настройки перехвата
#[tauri::command]
pub fn get_intercept_settings(state: State<'_, SoundPanelState>) -> InterceptSettings {
    state.get_intercept()
}

/// Включить/выключить перехват
#[tauri::command]
pub fn set_intercept_enabled(
    enabled: bool,
    state: State<'_, SoundPanelState>,
) -> Result<(), String> {
    state.set_intercept_enabled(enabled)
}

/// Установить биндинг перехвата
#[tauri::command]
pub fn set_intercept_binding(
    key: String,
    action: String,
    state: State<'_, SoundPanelState>,
) -> Result<(), String> {
    state.set_intercept_binding(key, action)
}

/// Очистить биндинг перехвата
#[tauri::command]
pub fn clear_intercept_binding(
    key: String,
    state: State<'_, SoundPanelState>,
) -> Result<(), String> {
    state.clear_intercept_binding(key)
}

/// Включить/выключить unrestricted-перехват (любая клавиша)
#[tauri::command]
pub fn set_intercept_allow_any_key(
    allow_any_key: bool,
    state: State<'_, SoundPanelState>,
) -> Result<(), String> {
    state.set_intercept_allow_any_key(allow_any_key)
}

// ---- Set management commands ----

/// Получить все наборы звуков
#[tauri::command]
pub fn sp_get_sets(state: State<'_, SoundPanelState>) -> Result<SoundSets, String> {
    Ok(state.get_sets())
}

/// Получить активный набор
#[tauri::command]
pub fn sp_get_active_set(state: State<'_, SoundPanelState>) -> Result<SoundSet, String> {
    Ok(state.get_active_set())
}

/// Сменить активный набор
///
/// Persist-first: state-метод сохраняет кандидатный снимок на диск и только
/// затем публикует его в runtime.
#[tauri::command]
pub fn sp_set_active_set(
    id: String,
    app_handle: AppHandle,
    state: State<'_, SoundPanelState>,
) -> Result<(), String> {
    info!(set_id = %id, "Setting active set");
    state.set_active_set(&id)?;

    let _ = emit_soundpanel_bindings_changed(&app_handle);
    let _ = app_handle.emit("soundpanel-active-set-changed", &id);

    Ok(())
}

/// Создать новый набор
///
/// Persist-first: state-метод сохраняет кандидатный снимок на диск и только
/// затем публикует его в runtime.
#[tauri::command]
pub fn sp_add_set(
    name: String,
    app_handle: AppHandle,
    state: State<'_, SoundPanelState>,
) -> Result<SoundSet, String> {
    info!(name, "Adding set");
    let set = state.add_set(&name)?;

    let _ = emit_soundpanel_bindings_changed(&app_handle);
    let _ = app_handle.emit("soundpanel-active-set-changed", &set.id);

    Ok(set)
}

/// Переименовать набор
///
/// Persist-first: state-метод сохраняет кандидатный снимок на диск и только
/// затем публикует его в runtime.
#[tauri::command]
pub fn sp_rename_set(
    id: String,
    name: String,
    app_handle: AppHandle,
    state: State<'_, SoundPanelState>,
) -> Result<(), String> {
    info!(set_id = %id, name, "Renaming set");
    state.rename_set(&id, &name)?;

    let _ = emit_soundpanel_bindings_changed(&app_handle);

    Ok(())
}

/// Удалить набор
///
/// Persist-first: state-метод сохраняет кандидатный снимок на диск и только
/// затем публикует его в runtime.
#[tauri::command]
pub fn sp_remove_set(
    id: String,
    app_handle: AppHandle,
    state: State<'_, SoundPanelState>,
) -> Result<(), String> {
    info!(set_id = %id, "Removing set");
    state.remove_set(&id)?;

    let _ = emit_soundpanel_bindings_changed(&app_handle);
    let _ = app_handle.emit("soundpanel-active-set-changed", "");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn play_with_recording(
        stay_visible: bool,
        hide_result: Result<(), String>,
        restore_result: Result<(), String>,
    ) -> (Result<(), String>, Vec<&'static str>) {
        let order = RefCell::new(Vec::new());
        let result = play_binding_ordered(
            stay_visible,
            || {
                order.borrow_mut().push("hide");
                hide_result.clone()
            },
            || {
                order.borrow_mut().push("restore");
                restore_result.clone()
            },
            || {
                order.borrow_mut().push("play");
                Ok(())
            },
        );
        (result, order.into_inner())
    }

    #[test]
    fn unchecked_hide_and_restore_complete_before_play() {
        let (result, order) = play_with_recording(false, Ok(()), Ok(()));
        assert!(result.is_ok());
        assert_eq!(order, vec!["hide", "restore", "play"]);
    }

    #[test]
    fn unchecked_hide_failure_prevents_restore_and_playback() {
        let (result, order) = play_with_recording(false, Err("hide failed".to_string()), Ok(()));
        assert!(result.is_err());
        assert_eq!(order, vec!["hide"]);
    }

    #[test]
    fn unchecked_restore_failure_still_permits_playback_and_reports_failure() {
        let (result, order) = play_with_recording(false, Ok(()), Err("restore failed".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("restore failed"));
        assert_eq!(order, vec!["hide", "restore", "play"]);
    }

    #[test]
    fn checked_skips_hide_and_orders_restore_then_play() {
        let (result, order) = play_with_recording(true, Ok(()), Ok(()));
        assert!(result.is_ok());
        assert_eq!(order, vec!["restore", "play"]);
    }

    #[test]
    fn checked_restore_failure_still_permits_playback_and_reports_failure() {
        let (result, order) = play_with_recording(true, Ok(()), Err("restore failed".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("restore failed"));
        assert_eq!(order, vec!["restore", "play"]);
    }

    #[test]
    fn unchecked_play_failure_is_reported_and_restore_still_runs() {
        let order = RefCell::new(Vec::new());
        let result = play_binding_ordered(
            false,
            || {
                order.borrow_mut().push("hide");
                Ok(())
            },
            || {
                order.borrow_mut().push("restore");
                Ok(())
            },
            || {
                order.borrow_mut().push("play");
                Err("queue full".to_string())
            },
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("queue full"));
        assert_eq!(order.into_inner(), vec!["hide", "restore", "play"]);
    }

    #[test]
    fn play_error_takes_precedence_over_restore_error() {
        let result = play_binding_ordered(
            false,
            || Ok(()),
            || Err("restore failed".to_string()),
            || Err("queue full".to_string()),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("queue full"));
    }

    fn record_escape(
        stay_visible: bool,
        hide_result: Result<(), String>,
        restore_result: Result<(), String>,
    ) -> (Result<(), String>, Vec<&'static str>) {
        let order = RefCell::new(Vec::new());
        let result = handoff_soundpanel_ordered(
            stay_visible,
            || {
                order.borrow_mut().push("hide");
                hide_result.clone()
            },
            || {
                order.borrow_mut().push("restore");
                restore_result.clone()
            },
        );
        (result, order.into_inner())
    }

    #[test]
    fn escape_unchecked_orders_hide_then_restore() {
        let (result, order) = record_escape(false, Ok(()), Ok(()));
        assert!(result.is_ok());
        assert_eq!(order, vec!["hide", "restore"]);
    }

    #[test]
    fn escape_checked_orders_only_restore() {
        let (result, order) = record_escape(true, Ok(()), Ok(()));
        assert!(result.is_ok());
        assert_eq!(order, vec!["restore"]);
    }

    #[test]
    fn escape_unchecked_hide_failure_prevents_restore() {
        let (result, order) = record_escape(false, Err("hide failed".to_string()), Ok(()));
        assert!(result.is_err());
        assert_eq!(order, vec!["hide"]);
    }
}
