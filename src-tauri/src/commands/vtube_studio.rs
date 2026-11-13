use crate::config::{
    validate_vtube_host, SettingsManager, VTubeStudioSettings, VTubeStudioSettingsDto,
    VTubeStudioTypingAction, VTubeStudioTypingMode, VtsHotkeyInfoDto,
};
use crate::events::VTubeStudioConnectionStatus;
use crate::state::AppState;
use crate::vtube_studio::{ConnectOrigin, SceneItemRecord, VTubeStudioItemStatus};
use tauri::{AppHandle, Emitter, Manager, State};
use tracing::{debug, info, warn};

pub const VTS_STATUS_CHANGED_EVENT: &str = "vtube-studio-status-changed";
pub const VTS_ITEM_STATUS_CHANGED_EVENT: &str = "vtube-studio-item-status-changed";

fn emit_vts_status(app_handle: &AppHandle, status: &VTubeStudioConnectionStatus) {
    let _ = app_handle.emit(VTS_STATUS_CHANGED_EVENT, status);
}

fn emit_vts_item_status(app_handle: &AppHandle, status: &VTubeStudioItemStatus) {
    let _ = app_handle.emit(VTS_ITEM_STATUS_CHANGED_EVENT, status);
}

fn should_ensure_event_parameter(
    new_output_mode: &VTubeStudioTypingMode,
    old_output_mode: &VTubeStudioTypingMode,
    old_name: &str,
    new_name: &str,
) -> bool {
    *new_output_mode == VTubeStudioTypingMode::Event
        && (*old_output_mode != VTubeStudioTypingMode::Event || old_name != new_name)
}

#[cfg(test)]
fn should_publish_event_parameter(is_event_mode: bool, old_name: &str, new_name: &str) -> bool {
    is_event_mode && old_name != new_name
}

/// Whether the previous Event INPUT must be deleted after switching to a new
/// action. Only a real rename `Event(old) -> Event(new)` with distinct names
/// deletes; mode switches `Event <-> Hotkeys/Item` and unchanged names do not
/// (ROADMAP-061 contracts 8-10).
fn should_delete_old_parameter(
    new_is_event: bool,
    old_output_mode: &VTubeStudioTypingMode,
    old_name: &str,
    new_name: &str,
) -> bool {
    new_is_event
        && *old_output_mode == VTubeStudioTypingMode::Event
        && !old_name.is_empty()
        && old_name != new_name
}

#[allow(dead_code)]
fn format_event_save_message(created_or_error: Result<bool, String>, param_name: &str) -> String {
    match created_or_error {
        Ok(true) => format!(
            "Параметр '{}' создан в VTube Studio. Действие при наборе сохранено.",
            param_name
        ),
        Ok(false) => "Действие при наборе сохранено".to_string(),
        Err(e) => format!(
            "Действие при наборе сохранено, но не удалось создать параметр '{}' в VTube Studio: {}",
            param_name, e
        ),
    }
}

fn emit_vts_runtime_status(app_handle: &AppHandle, state: &AppState) {
    emit_vts_status(app_handle, &state.vtube_studio.get_connection_status());
    emit_vts_item_status(app_handle, &state.vtube_studio.get_item_status());
}

async fn persist_and_apply_vts_token(
    manager: &SettingsManager,
    runtime_settings: &tokio::sync::RwLock<VTubeStudioSettings>,
    token: String,
) -> Result<(), String> {
    let mgr = manager.clone();
    let persisted_token = token.clone();
    crate::commands::persist_blocking(&mgr, move |m| {
        m.set_vtube_studio_token(Some(persisted_token))
    })
    .await?;

    runtime_settings.write().await.token = Some(token);
    Ok(())
}

#[tauri::command]
pub async fn get_vtube_studio_settings(
    state: State<'_, AppState>,
) -> Result<VTubeStudioSettingsDto, String> {
    let settings = state.vtube_studio.settings.read().await;
    Ok(VTubeStudioSettingsDto::from(settings.clone()))
}

#[tauri::command]
pub async fn save_vtube_studio_settings(
    enabled: bool,
    host: String,
    port: u16,
    start_on_boot: bool,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    if port < 1024 {
        return Err(format!("Invalid port: {}. Must be 1024-65535.", port));
    }
    let host = host.trim().to_string();
    validate_vtube_host(&host)?;

    info!(enabled, host = %host, port, start_on_boot, "Saving VTube Studio settings");

    let (old_host, old_port) = {
        let current = state.vtube_studio.settings.read().await;
        (current.host.clone(), current.port)
    };

    let endpoint_changed = old_host != host || old_port != port;

    let settings_manager = app_handle
        .try_state::<SettingsManager>()
        .ok_or_else(|| "SettingsManager not available".to_string())?;

    let (token, typing_action) = {
        let s = state.vtube_studio.settings.read().await;
        (s.token.clone(), s.typing_action.clone())
    };

    let persist_settings = VTubeStudioSettings {
        enabled,
        host: host.clone(),
        port,
        token: token.clone(),
        start_on_boot,
        typing_action,
    };

    let mgr = settings_manager.inner().clone();
    crate::commands::persist_blocking(&mgr, move |mgr| {
        mgr.set_vtube_studio_settings(&persist_settings)
    })
    .await?;

    {
        let mut s = state.vtube_studio.settings.write().await;
        s.enabled = enabled;
        s.host = host;
        s.port = port;
        s.start_on_boot = start_on_boot;
    }

    crate::commands::emit_settings_changed(&app_handle);

    if endpoint_changed {
        state.vtube_studio.disconnect().await;
        let status = state.vtube_studio.get_connection_status();
        emit_vts_status(&app_handle, &status);
        let item_status = state.vtube_studio.get_item_status();
        emit_vts_item_status(&app_handle, &item_status);
        info!("VTube Studio connection cleared due to endpoint change");
    }

    Ok("Настройки подключения VTube Studio сохранены".to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri IPC exposes these fields as named arguments.
pub async fn save_vtube_studio_typing_action(
    output_mode: String,
    parameter_name: String,
    start_hotkey_id: String,
    stop_hotkey_id: String,
    start_hotkey_name: String,
    stop_hotkey_name: String,
    item_file_name: Option<String>,
    item_type: Option<String>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let mode = match output_mode.as_str() {
        "Event" => VTubeStudioTypingMode::Event,
        "Hotkeys" => VTubeStudioTypingMode::Hotkeys,
        "Item" => VTubeStudioTypingMode::Item,
        other => {
            return Err(format!(
                "Invalid output mode: '{}'. Must be 'Event', 'Hotkeys', or 'Item'.",
                other
            ));
        }
    };

    let trimmed_parameter_name = parameter_name.trim().to_string();
    let trimmed_start = start_hotkey_id.trim().to_string();
    let trimmed_stop = stop_hotkey_id.trim().to_string();

    let (item_file_name_raw, item_type_raw) = match mode {
        VTubeStudioTypingMode::Event => {
            if trimmed_parameter_name.is_empty() {
                return Err("Parameter name must be non-empty.".to_string());
            }
            (None, None)
        }
        VTubeStudioTypingMode::Hotkeys => {
            if trimmed_start.is_empty() {
                return Err("Start hotkey ID must be non-empty for Hotkeys mode.".to_string());
            }
            if trimmed_stop.is_empty() {
                return Err("Stop hotkey ID must be non-empty for Hotkeys mode.".to_string());
            }
            (None, None)
        }
        VTubeStudioTypingMode::Item => {
            let file_name = match &item_file_name {
                Some(f) if !f.is_empty() => f.clone(),
                _ => {
                    return Err(
                        "itemFileName is required and must be non-empty for Item mode.".to_string(),
                    );
                }
            };
            let item_type_val = match &item_type {
                Some(t) if !t.is_empty() => t.clone(),
                _ => {
                    return Err(
                        "itemType is required and must be non-empty for Item mode.".to_string()
                    );
                }
            };
            (Some(file_name), Some(item_type_val))
        }
    };

    if !state.vtube_studio.is_live_authenticated_connection() {
        return Err("Подключитесь к VTube Studio, чтобы настроить действие.".to_string());
    }

    if mode == VTubeStudioTypingMode::Item {
        let file_name = item_file_name_raw.as_deref().unwrap_or("");
        let item_type_val = item_type_raw.as_deref().unwrap_or("");
        let records = state.vtube_studio.list_scene_items().await.map_err(|e| {
            let status = state.vtube_studio.get_connection_status();
            emit_vts_status(&app_handle, &status);
            format!(
                "Cannot validate item selection: {}. Make sure VTube Studio is connected.",
                e
            )
        })?;

        validate_item_selection(&records, file_name, item_type_val)?;
    }

    let (
        existing_file_name,
        existing_item_type,
        enabled,
        host,
        port,
        token,
        start_on_boot,
        old_parameter_name,
        old_output_mode,
    ) = {
        let s = state.vtube_studio.settings.read().await;
        (
            s.typing_action.item_file_name.clone(),
            s.typing_action.item_type.clone(),
            s.enabled,
            s.host.clone(),
            s.port,
            s.token.clone(),
            s.start_on_boot,
            s.typing_action.parameter_name.clone(),
            s.typing_action.output_mode.clone(),
        )
    };

    let (resolved_file_name, resolved_item_type) = resolve_item_metadata(
        &mode,
        &existing_file_name,
        &existing_item_type,
        item_file_name,
        item_type,
    );

    let typing_action = VTubeStudioTypingAction {
        output_mode: mode,
        parameter_name: trimmed_parameter_name,
        start_hotkey_id: trimmed_start,
        stop_hotkey_id: trimmed_stop,
        start_hotkey_name: start_hotkey_name.trim().to_string(),
        stop_hotkey_name: stop_hotkey_name.trim().to_string(),
        item_file_name: resolved_file_name,
        item_type: resolved_item_type,
    };

    info!(?typing_action, "Saving VTube Studio typing action");

    let settings_manager = app_handle
        .try_state::<SettingsManager>()
        .ok_or_else(|| "SettingsManager not available".to_string())?;

    let operations = LiveTypingActionSaveOperations {
        service: &state.vtube_studio,
        settings_manager: settings_manager.inner(),
        app_handle: &app_handle,
        enabled,
        host,
        port,
        token,
        start_on_boot,
    };
    let save_result = orchestrate_typing_action_save(
        &operations,
        &old_output_mode,
        &old_parameter_name,
        &typing_action,
    )
    .await?;

    let is_item_mode = typing_action.output_mode == VTubeStudioTypingMode::Item;
    let item_status = state.vtube_studio.refresh_item_action().await;
    emit_vts_item_status(&app_handle, &item_status);

    if is_item_mode {
        if matches!(item_status, VTubeStudioItemStatus::Ready { .. }) {
            return Ok("Действие при наборе сохранено".to_string());
        } else {
            return Err(format!(
                "Item action saved but item is not ready: {:?}. Check that the item exists in the scene.",
                item_status
            ));
        }
    }

    if let Some(warning) = save_result.cleanup_warning {
        return Ok(warning);
    }

    if save_result.ensured_parameter {
        Ok(format!(
            "Параметр '{}' создан в VTube Studio. Действие при наборе сохранено.",
            typing_action.parameter_name
        ))
    } else {
        Ok("Действие при наборе сохранено".to_string())
    }
}

fn skip_reason_text(reason: crate::vtube_studio::SkipReason) -> &'static str {
    use crate::vtube_studio::SkipReason;
    match reason {
        SkipReason::NotDesiredRunning => "VTube Studio не запущен",
        SkipReason::NotConnected => "нет соединения с VTube Studio",
        SkipReason::NotAuthenticated => "нет аутентификации",
        SkipReason::NoSocket => "соединение потеряно",
    }
}

#[derive(Debug, PartialEq, Eq)]
struct TypingActionSaveResult {
    ensured_parameter: bool,
    cleanup_warning: Option<String>,
}

#[async_trait::async_trait]
trait TypingActionSaveOperations: Sync {
    async fn require_live_connection(&self) -> Result<(), String>;
    async fn ensure_parameter(
        &self,
        parameter_name: &str,
    ) -> Result<crate::vtube_studio::EnsureOutcome, String>;
    async fn persist_action(&self, action: &VTubeStudioTypingAction) -> Result<(), String>;
    async fn apply_runtime(&self, action: &VTubeStudioTypingAction);
    fn notify_settings_changed(&self);
    async fn stop_old_event(&self, parameter_name: &str);
    async fn delete_parameter(
        &self,
        parameter_name: &str,
    ) -> Result<crate::vtube_studio::DeleteOutcome, String>;
}

struct LiveTypingActionSaveOperations<'a> {
    service: &'a crate::vtube_studio::VTubeStudioService,
    settings_manager: &'a SettingsManager,
    app_handle: &'a AppHandle,
    enabled: bool,
    host: String,
    port: u16,
    token: Option<String>,
    start_on_boot: bool,
}

#[async_trait::async_trait]
impl TypingActionSaveOperations for LiveTypingActionSaveOperations<'_> {
    async fn require_live_connection(&self) -> Result<(), String> {
        match self.service.live_connection_skip_reason().await {
            None => Ok(()),
            Some(reason) => Err(format!(
                "Подключитесь к VTube Studio, чтобы настроить действие: {}.",
                skip_reason_text(reason)
            )),
        }
    }

    async fn ensure_parameter(
        &self,
        parameter_name: &str,
    ) -> Result<crate::vtube_studio::EnsureOutcome, String> {
        self.service
            .ensure_event_parameter_if_connected(parameter_name)
            .await
    }

    async fn persist_action(&self, action: &VTubeStudioTypingAction) -> Result<(), String> {
        let persist_settings = VTubeStudioSettings {
            enabled: self.enabled,
            host: self.host.clone(),
            port: self.port,
            token: self.token.clone(),
            start_on_boot: self.start_on_boot,
            typing_action: action.clone(),
        };
        crate::commands::persist_blocking(self.settings_manager, move |manager| {
            manager.set_vtube_studio_settings(&persist_settings)
        })
        .await
    }

    async fn apply_runtime(&self, action: &VTubeStudioTypingAction) {
        self.service.settings.write().await.typing_action = action.clone();
    }

    fn notify_settings_changed(&self) {
        crate::commands::emit_settings_changed(self.app_handle);
    }

    async fn stop_old_event(&self, parameter_name: &str) {
        self.service
            .stop_event_typing_and_reset(parameter_name)
            .await;
    }

    async fn delete_parameter(
        &self,
        parameter_name: &str,
    ) -> Result<crate::vtube_studio::DeleteOutcome, String> {
        self.service
            .delete_event_parameter_if_connected(parameter_name)
            .await
    }
}

async fn orchestrate_typing_action_save<O: TypingActionSaveOperations>(
    operations: &O,
    old_output_mode: &VTubeStudioTypingMode,
    old_parameter_name: &str,
    new_action: &VTubeStudioTypingAction,
) -> Result<TypingActionSaveResult, String> {
    operations.require_live_connection().await?;

    let should_ensure = should_ensure_event_parameter(
        &new_action.output_mode,
        old_output_mode,
        old_parameter_name,
        &new_action.parameter_name,
    );

    if should_ensure {
        use crate::vtube_studio::EnsureOutcome;
        match operations
            .ensure_parameter(&new_action.parameter_name)
            .await?
        {
            EnsureOutcome::Ensured => {}
            EnsureOutcome::Skipped(reason) => {
                return Err(format!(
                    "Не удалось создать параметр '{}' в VTube Studio: {}",
                    new_action.parameter_name,
                    skip_reason_text(reason)
                ));
            }
        }
    }

    if let Err(persist_error) = operations.persist_action(new_action).await {
        if should_ensure {
            warn!(
                new_param = %new_action.parameter_name,
                error = %persist_error,
                "Persist failed after creating new Event INPUT; INPUT may remain in VTS (recoverable, not deleted)."
            );
        }
        return Err(persist_error);
    }

    operations.apply_runtime(new_action).await;
    operations.notify_settings_changed();

    let needs_delete = should_delete_old_parameter(
        new_action.output_mode == VTubeStudioTypingMode::Event,
        old_output_mode,
        old_parameter_name,
        &new_action.parameter_name,
    );
    let cleanup_warning = if needs_delete {
        operations.stop_old_event(old_parameter_name).await;
        match operations.delete_parameter(old_parameter_name).await {
            Ok(crate::vtube_studio::DeleteOutcome::Deleted)
            | Ok(crate::vtube_studio::DeleteOutcome::NotFound) => None,
            Ok(crate::vtube_studio::DeleteOutcome::Skipped(reason)) => Some(format!(
                "Действие при наборе сохранено, но старый параметр '{}' мог не удалиться: {}",
                old_parameter_name,
                skip_reason_text(reason)
            )),
            Err(error) => Some(format!(
                "Действие при наборе сохранено, но не удалось удалить старый параметр '{}': {}",
                old_parameter_name, error
            )),
        }
    } else {
        None
    };

    Ok(TypingActionSaveResult {
        ensured_parameter: should_ensure,
        cleanup_warning,
    })
}

#[tauri::command]
pub async fn get_vtube_studio_scene_items(
    state: State<'_, AppState>,
) -> Result<Vec<SceneItemRecord>, String> {
    state.vtube_studio.list_scene_items().await
}

#[tauri::command]
pub async fn get_vtube_studio_item_status(
    state: State<'_, AppState>,
) -> Result<VTubeStudioItemStatus, String> {
    Ok(state.vtube_studio.get_item_status())
}

#[tauri::command]
pub async fn refresh_vtube_studio_item(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<VTubeStudioItemStatus, String> {
    let status = state.vtube_studio.refresh_item_action().await;
    emit_vts_item_status(&app_handle, &status);
    info!(?status, "VTube Studio item action refreshed");
    Ok(status)
}

#[tauri::command]
pub async fn get_vtube_studio_current_model_hotkeys(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<Vec<VtsHotkeyInfoDto>, String> {
    info!("Requesting VTS current model hotkeys");

    let hotkeys = state
        .vtube_studio
        .get_current_model_hotkeys()
        .await
        .inspect_err(|_| {
            let status = state.vtube_studio.get_connection_status();
            emit_vts_status(&app_handle, &status);
        })?;

    let dtos: Vec<VtsHotkeyInfoDto> = hotkeys
        .into_iter()
        .map(|h| VtsHotkeyInfoDto {
            hotkey_id: h.hotkey_id,
            name: h.name,
            hotkey_type: h.hotkey_type,
            description: h.description,
        })
        .collect();

    Ok(dtos)
}

#[tauri::command]
pub async fn test_vtube_studio_connection(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let settings_manager = app_handle
        .try_state::<SettingsManager>()
        .ok_or_else(|| "SettingsManager not available".to_string())?;
    let (host, port, stored_token) = {
        let settings = state.vtube_studio.settings.read().await;
        (settings.host.clone(), settings.port, settings.token.clone())
    };

    info!(
        host = %host,
        port,
        has_token = stored_token.is_some(),
        "Testing VTube Studio connection"
    );

    let result = state
        .vtube_studio
        .test_connection(&host, port, stored_token.as_deref())
        .await;

    match result {
        Ok(new_token) => {
            if let Some(tok) = new_token {
                info!("Persisting new VTS authentication token");
                if let Err(error) = persist_and_apply_vts_token(
                    settings_manager.inner(),
                    &state.vtube_studio.settings,
                    tok,
                )
                .await
                {
                    state.vtube_studio.disconnect().await;
                    emit_vts_runtime_status(&app_handle, state.inner());
                    return Err(error);
                }
            }

            // Сокет устанавливается сервисом только при живом desired_running;
            // после параллельного disconnect() попытка отбрасывается. Поэтому
            // auth-флаг не должен «утекать» в UI при погашенном желании.
            if state.vtube_studio.is_desired_running() {
                state.vtube_studio.mark_authenticated(true);
            }

            let item_status = state.vtube_studio.get_item_status();
            emit_vts_item_status(&app_handle, &item_status);

            Ok("Successfully connected to VTube Studio.".to_string())
        }
        Err(e) => {
            state.vtube_studio.mark_authenticated(false);
            let item_status = state.vtube_studio.get_item_status();
            emit_vts_item_status(&app_handle, &item_status);
            Err(format!("VTube Studio connection failed: {}", e))
        }
    }
}

#[tauri::command]
pub async fn connect_vtube_studio(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let settings_manager = app_handle
        .try_state::<SettingsManager>()
        .ok_or_else(|| "SettingsManager not available".to_string())?;
    let (host, port, stored_token) = {
        let settings = state.vtube_studio.settings.read().await;
        (settings.host.clone(), settings.port, settings.token.clone())
    };

    info!(
        host = %host,
        port,
        has_token = stored_token.is_some(),
        "Connect VTube Studio"
    );

    let result = state
        .vtube_studio
        .connect(&host, port, stored_token.as_deref(), ConnectOrigin::Manual)
        .await;

    match result {
        Ok(new_token) => {
            if let Some(tok) = new_token {
                info!("Persisting new VTS authentication token");
                if let Err(error) = persist_and_apply_vts_token(
                    settings_manager.inner(),
                    &state.vtube_studio.settings,
                    tok,
                )
                .await
                {
                    state.vtube_studio.disconnect().await;
                    emit_vts_runtime_status(&app_handle, state.inner());
                    return Err(error);
                }
            }
            emit_vts_runtime_status(&app_handle, state.inner());
            Ok("Подключено к VTube Studio".to_string())
        }
        Err(e) => {
            emit_vts_runtime_status(&app_handle, state.inner());
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn disconnect_vtube_studio(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    info!("Disconnect VTube Studio");
    state.vtube_studio.disconnect().await;
    let status = state.vtube_studio.get_connection_status();
    emit_vts_status(&app_handle, &status);
    let item_status = state.vtube_studio.get_item_status();
    emit_vts_item_status(&app_handle, &item_status);
    Ok("Disconnected from VTube Studio".to_string())
}

#[tauri::command]
pub async fn restart_vtube_studio(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let settings_manager = app_handle
        .try_state::<SettingsManager>()
        .ok_or_else(|| "SettingsManager not available".to_string())?;
    info!("Restart VTube Studio");

    state.vtube_studio.disconnect().await;

    let (host, port, stored_token) = {
        let settings = state.vtube_studio.settings.read().await;
        (settings.host.clone(), settings.port, settings.token.clone())
    };

    let result = state
        .vtube_studio
        .connect(&host, port, stored_token.as_deref(), ConnectOrigin::Manual)
        .await;

    match result {
        Ok(new_token) => {
            if let Some(tok) = new_token {
                info!("Persisting new VTS authentication token");
                if let Err(error) = persist_and_apply_vts_token(
                    settings_manager.inner(),
                    &state.vtube_studio.settings,
                    tok,
                )
                .await
                {
                    state.vtube_studio.disconnect().await;
                    emit_vts_runtime_status(&app_handle, state.inner());
                    return Err(error);
                }
            }
            emit_vts_runtime_status(&app_handle, state.inner());
            Ok("Restarted VTube Studio".to_string())
        }
        Err(e) => {
            emit_vts_runtime_status(&app_handle, state.inner());
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn get_vtube_studio_status(
    state: State<'_, AppState>,
) -> Result<VTubeStudioConnectionStatus, String> {
    Ok(state.vtube_studio.get_connection_status())
}

#[tauri::command]
pub fn get_vtube_studio_authenticated(state: State<'_, AppState>) -> bool {
    state.vtube_studio.is_live_authenticated_connection()
}

#[tauri::command]
pub fn get_vtube_studio_desired_running(state: State<'_, AppState>) -> bool {
    state.vtube_studio.is_desired_running()
}

#[tauri::command]
pub async fn test_vtube_studio_typing(
    timeout_ms: u64,
    repeat_count: u64,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    if !(100..=5000).contains(&timeout_ms) {
        return Err("Таймаут должен быть от 100 до 5000 мс".to_string());
    }
    if !(1..=10).contains(&repeat_count) {
        return Err("Повторы должны быть от 1 до 10".to_string());
    }

    info!(timeout_ms, repeat_count, "Testing VTube Studio typing");

    let result = state
        .vtube_studio
        .test_typing_action(timeout_ms, repeat_count)
        .await;

    match result {
        Ok(msg) => Ok(msg),
        Err(e) => {
            let status = state.vtube_studio.get_connection_status();
            emit_vts_status(&app_handle, &status);
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn set_vtube_studio_typing(
    typing: bool,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let (host, port, token) = {
        let settings = state.vtube_studio.settings.read().await;
        (settings.host.clone(), settings.port, settings.token.clone())
    };

    let mode = state
        .vtube_studio
        .settings
        .read()
        .await
        .typing_action
        .output_mode
        .clone();

    if mode == VTubeStudioTypingMode::Item {
        if typing && !state.vtube_studio.is_desired_running() {
            debug!("VTS: Item set_typing(true) ignored — desired_running is false");
            return Ok(());
        }
        state.vtube_studio.record_item_desired(typing);
        let svc = state.vtube_studio.clone();
        let app_clone = app_handle.clone();
        tokio::spawn(async move {
            let status = svc.run_item_sync().await;
            emit_vts_item_status(&app_clone, &status);
        });
        return Ok(());
    }

    let stored_token = match token.as_deref() {
        None | Some("") => {
            debug!("VTS: set_typing({}) called but no token — no-op", typing);
            return Ok(());
        }
        Some(t) => t,
    };

    if typing && !state.vtube_studio.is_desired_running() {
        debug!("VTS: set_typing(true) ignored — desired_running is false");
        return Ok(());
    }

    let status_before = state.vtube_studio.get_connection_status();

    debug!(typing, "VTS: set_vtube_studio_typing");
    let result = state
        .vtube_studio
        .set_typing(typing, &host, port, stored_token)
        .await;

    let status_after = state.vtube_studio.get_connection_status();
    if status_before != status_after {
        emit_vts_status(&app_handle, &status_after);
    }

    result
}

pub fn validate_item_selection(
    records: &[SceneItemRecord],
    file_name: &str,
    item_type: &str,
) -> Result<(), String> {
    if file_name.is_empty() {
        return Err("Item file name must be non-empty.".to_string());
    }
    if item_type.is_empty() {
        return Err("Item type must be non-empty.".to_string());
    }

    let matching: Vec<&SceneItemRecord> = records
        .iter()
        .filter(|r| r.file_name == file_name)
        .collect();

    if matching.is_empty() {
        return Err(format!(
            "No scene item with file name '{}' found. Add the item to the scene and try again.",
            file_name
        ));
    }

    if matching.len() > 1 {
        return Err(format!(
            "Multiple scene item types found for file name '{}' ({} types). Remove duplicates from the scene to use a specific item.",
            file_name,
            matching.len()
        ));
    }

    let record = matching[0];

    if record.duplicate_count > 1 {
        return Err(format!(
            "Multiple instances ({}) of item '{}' (type '{}') in scene. Remove duplicate instances to use this item.",
            record.duplicate_count, file_name, record.item_type
        ));
    }

    if !record.supported {
        return Err(format!(
            "Item '{}' has unsupported type '{}'. Use a PNG, JPG, GIF, or AnimationFolder item instead.",
            file_name, record.item_type
        ));
    }

    if record.item_type != item_type {
        return Err(format!(
            "Type mismatch: item '{}' has type '{}' but '{}' was selected. Refresh the item list and try again.",
            file_name, record.item_type, item_type
        ));
    }

    Ok(())
}

fn resolve_item_metadata(
    mode: &VTubeStudioTypingMode,
    existing_file_name: &str,
    existing_item_type: &str,
    supplied_file_name: Option<String>,
    supplied_item_type: Option<String>,
) -> (String, String) {
    match mode {
        VTubeStudioTypingMode::Event | VTubeStudioTypingMode::Hotkeys => {
            let file_name = supplied_file_name.unwrap_or_else(|| existing_file_name.to_string());
            let item_type = supplied_item_type.unwrap_or_else(|| existing_item_type.to_string());
            (file_name, item_type)
        }
        VTubeStudioTypingMode::Item => (
            supplied_file_name.unwrap_or_default(),
            supplied_item_type.unwrap_or_default(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vts_token_persist_failure_preserves_runtime_token() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-vts-token-failure-{}-{}",
            std::process::id(),
            unique
        ));
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        let runtime_settings = tokio::sync::RwLock::new(VTubeStudioSettings {
            token: Some("old-token".to_string()),
            ..VTubeStudioSettings::default()
        });
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(persist_and_apply_vts_token(
            &manager,
            &runtime_settings,
            "new-token".to_string(),
        ));

        assert!(result.is_err());
        assert_eq!(
            runtime.block_on(async { runtime_settings.read().await.token.clone() }),
            Some("old-token".to_string())
        );
    }

    #[test]
    fn vts_token_commit_updates_persistence_before_runtime() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-vts-token-success-{}-{}",
            std::process::id(),
            unique
        ));
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let runtime_settings = tokio::sync::RwLock::new(VTubeStudioSettings::default());
        let runtime = tokio::runtime::Runtime::new().unwrap();

        runtime
            .block_on(persist_and_apply_vts_token(
                &manager,
                &runtime_settings,
                "new-token".to_string(),
            ))
            .unwrap();

        assert_eq!(
            manager.get_vtube_studio_settings().token,
            Some("new-token".to_string())
        );
        assert_eq!(
            runtime.block_on(async { runtime_settings.read().await.token.clone() }),
            Some("new-token".to_string())
        );
        let disk: crate::config::AppSettings =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(disk.vtube_studio.token, Some("new-token".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_record(
        file_name: &str,
        item_type: &str,
        supported: bool,
        duplicate_count: u32,
    ) -> SceneItemRecord {
        SceneItemRecord {
            file_name: file_name.to_string(),
            item_type: item_type.to_string(),
            supported,
            duplicate_count,
        }
    }

    #[test]
    fn exact_case_sensitive_one_supported_succeeds() {
        let records = vec![make_record("icon.png", "PNG", true, 1)];
        let result = validate_item_selection(&records, "icon.png", "PNG");
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn wrong_case_fails() {
        let records = vec![make_record("icon.png", "PNG", true, 1)];
        let result = validate_item_selection(&records, "ICON.PNG", "PNG");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ICON.PNG"));
    }

    #[test]
    fn missing_filename_fails() {
        let records = vec![make_record("icon.png", "PNG", true, 1)];
        let result = validate_item_selection(&records, "ghost.png", "PNG");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("ghost.png"),
            "expected filename in error: {}",
            err
        );
        assert!(
            err.contains("Add the item to the scene"),
            "expected actionable message: {}",
            err
        );
        assert!(
            !err.contains("instanceID"),
            "error must not contain instanceID: {}",
            err
        );
        assert!(
            !err.contains("token"),
            "error must not contain token: {}",
            err
        );
    }

    #[test]
    fn duplicate_count_gt_1_fails() {
        let records = vec![make_record("dup.png", "PNG", true, 3)];
        let result = validate_item_selection(&records, "dup.png", "PNG");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("3"),
            "expected duplicate count in error: {}",
            err
        );
        assert!(
            err.contains("Remove duplicate instances"),
            "expected actionable message: {}",
            err
        );
    }

    #[test]
    fn two_catalog_records_same_filename_different_type_fails_ambiguous() {
        let records = vec![
            make_record("logo.png", "PNG", true, 1),
            make_record("logo.png", "GIF", true, 1),
        ];
        let result = validate_item_selection(&records, "logo.png", "PNG");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Multiple scene item types"),
            "expected ambiguous message: {}",
            err
        );
        assert!(
            err.contains("logo.png"),
            "expected filename in error: {}",
            err
        );
    }

    #[test]
    fn unsupported_fails() {
        let records = vec![make_record("model.moc3", "Live2D", false, 1)];
        let result = validate_item_selection(&records, "model.moc3", "Live2D");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("unsupported type"),
            "expected unsupported message: {}",
            err
        );
        assert!(err.contains("Live2D"), "expected type in error: {}", err);
    }

    #[test]
    fn type_mismatch_fails() {
        let records = vec![make_record("banner.jpg", "JPG", true, 1)];
        let result = validate_item_selection(&records, "banner.jpg", "PNG");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Type mismatch"),
            "expected type mismatch: {}",
            err
        );
        assert!(
            err.contains("JPG"),
            "expected actual type in error: {}",
            err
        );
    }

    #[test]
    fn unicode_filename_is_preserved() {
        let records = vec![make_record("кот 😺.png", "PNG", true, 1)];
        let result = validate_item_selection(&records, "кот 😺.png", "PNG");
        assert!(
            result.is_ok(),
            "expected Ok for unicode filename, got {:?}",
            result
        );
    }

    #[test]
    fn exact_whitespace_filename_is_preserved() {
        let records = vec![make_record("  my item  .png", "PNG", true, 1)];
        let result = validate_item_selection(&records, "  my item  .png", "PNG");
        assert!(
            result.is_ok(),
            "expected Ok for whitespace filename, got {:?}",
            result
        );
    }

    #[test]
    fn error_text_contains_next_action_and_no_instance_id_or_token() {
        let records = vec![make_record("icon.png", "PNG", false, 1)];
        let result = validate_item_selection(&records, "icon.png", "PNG");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            !err.contains("instanceID"),
            "error must not contain instanceID: {}",
            err
        );
        assert!(
            !err.contains("token"),
            "error must not contain token: {}",
            err
        );
        assert!(
            err.contains("unsupported type") || err.contains("Use a"),
            "error should suggest a next action: {}",
            err
        );
    }

    #[test]
    fn empty_file_name_fails() {
        let records = vec![make_record("pic.png", "PNG", true, 1)];
        let result = validate_item_selection(&records, "", "PNG");
        assert!(result.is_err());
    }

    #[test]
    fn empty_item_type_fails() {
        let records = vec![make_record("pic.png", "PNG", true, 1)];
        let result = validate_item_selection(&records, "pic.png", "");
        assert!(result.is_err());
    }

    #[test]
    fn event_missing_args_preserves_mixed_case_unicode() {
        let existing = ("МикСケース 😺.png".to_string(), "PNG".to_string());
        let result = resolve_item_metadata(
            &VTubeStudioTypingMode::Event,
            &existing.0,
            &existing.1,
            None,
            None,
        );
        assert_eq!(result, existing);
    }

    #[test]
    fn hotkeys_missing_args_preserves_metadata() {
        let existing = ("   spaced  .png".to_string(), "GIF".to_string());
        let result = resolve_item_metadata(
            &VTubeStudioTypingMode::Hotkeys,
            &existing.0,
            &existing.1,
            None,
            None,
        );
        assert_eq!(result, existing);
    }

    #[test]
    fn event_explicit_optional_replaces_exactly_including_whitespace() {
        let result = resolve_item_metadata(
            &VTubeStudioTypingMode::Event,
            "old.png",
            "PNG",
            Some("  new name  .jpg".to_string()),
            Some("JPG".to_string()),
        );
        assert_eq!(result.0, "  new name  .jpg");
        assert_eq!(result.1, "JPG");
    }

    #[test]
    fn hotkeys_explicit_optional_replaces_exactly() {
        let result = resolve_item_metadata(
            &VTubeStudioTypingMode::Hotkeys,
            "old.png",
            "PNG",
            Some("exact.gif".to_string()),
            Some("GIF".to_string()),
        );
        assert_eq!(result.0, "exact.gif");
        assert_eq!(result.1, "GIF");
    }

    #[test]
    fn item_supplied_validated_values_unchanged() {
        let result = resolve_item_metadata(
            &VTubeStudioTypingMode::Item,
            "old.png",
            "PNG",
            Some("アイテム.png".to_string()),
            Some("PNG".to_string()),
        );
        assert_eq!(result.0, "アイテム.png");
        assert_eq!(result.1, "PNG");
    }

    #[test]
    fn item_missing_args_falls_back_to_default() {
        let result =
            resolve_item_metadata(&VTubeStudioTypingMode::Item, "old.png", "PNG", None, None);
        assert_eq!(result.0, "");
        assert_eq!(result.1, "");
    }

    #[test]
    fn event_only_type_supplied_preserves_existing_filename() {
        let result = resolve_item_metadata(
            &VTubeStudioTypingMode::Event,
            "existing.png",
            "PNG",
            None,
            Some("GIF".to_string()),
        );
        assert_eq!(result.0, "existing.png");
        assert_eq!(result.1, "GIF");
    }

    #[test]
    fn hotkeys_only_filename_supplied_preserves_existing_type() {
        let result = resolve_item_metadata(
            &VTubeStudioTypingMode::Hotkeys,
            "old.gif",
            "GIF",
            Some("new.png".to_string()),
            None,
        );
        assert_eq!(result.0, "new.png");
        assert_eq!(result.1, "GIF");
    }

    #[test]
    fn format_event_save_true_includes_param_name() {
        let msg = format_event_save_message(Ok(true), "TTSBardTyping");
        assert!(msg.contains("создан в VTube Studio"), "got: {}", msg);
        assert!(msg.contains("TTSBardTyping"), "got: {}", msg);
        assert!(
            msg.contains("Действие при наборе сохранено"),
            "got: {}",
            msg
        );
    }

    #[test]
    fn format_event_save_false_returns_generic_saved_message() {
        let msg = format_event_save_message(Ok(false), "MyCustomParam");
        assert_eq!(msg, "Действие при наборе сохранено");
    }

    #[test]
    fn format_event_save_error_reports_saved_but_creation_failed() {
        let msg = format_event_save_message(
            Err("VTS error 356: invalid parameter name".to_string()),
            "BadParam",
        );
        assert!(
            msg.contains("Действие при наборе сохранено"),
            "got: {}",
            msg
        );
        assert!(
            msg.contains("не удалось создать параметр 'BadParam'"),
            "got: {}",
            msg
        );
        assert!(msg.contains("VTS error 356"), "got: {}", msg);
        assert!(!msg.contains("откат"), "got: {}", msg);
    }

    #[test]
    fn should_publish_event_parameter_event_changed() {
        assert!(should_publish_event_parameter(true, "old", "new"));
    }

    #[test]
    fn should_publish_event_parameter_event_unchanged() {
        assert!(!should_publish_event_parameter(true, "same", "same"));
    }

    #[test]
    fn should_publish_event_parameter_non_event_changed() {
        assert!(!should_publish_event_parameter(false, "old", "new"));
    }

    #[test]
    fn guard_condition_fresh_service_rejected() {
        let svc = crate::vtube_studio::VTubeStudioService::new();
        assert!(!svc.is_live_authenticated_connection());
    }

    #[test]
    fn guard_condition_connected_not_authenticated_fails() {
        let svc = crate::vtube_studio::VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        assert!(!svc.is_live_authenticated_connection());
    }

    #[test]
    fn guard_condition_connected_and_authenticated_passes() {
        let svc = crate::vtube_studio::VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.mark_authenticated(true);
        assert!(svc.is_live_authenticated_connection());
    }

    #[test]
    fn guard_condition_not_desired_running_fails() {
        let svc = crate::vtube_studio::VTubeStudioService::new();
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.mark_authenticated(true);
        assert!(!svc.is_live_authenticated_connection());
    }

    #[test]
    fn guard_condition_authenticated_but_disconnected_fails() {
        let svc = crate::vtube_studio::VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.mark_authenticated(true);
        assert!(!svc.is_live_authenticated_connection());
    }

    #[test]
    fn get_vtube_studio_authenticated_command_name_matches_registered_function() {
        assert_eq!(
            "get_vtube_studio_authenticated",
            stringify!(get_vtube_studio_authenticated)
        );
    }

    #[test]
    fn ensure_event_parameter_if_connected_skips_when_disconnected() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = crate::vtube_studio::VTubeStudioService::new();
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
        let result = rt.block_on(svc.ensure_event_parameter_if_connected("TestParam"));
        assert!(
            matches!(result, Ok(crate::vtube_studio::EnsureOutcome::Skipped(_))),
            "expected Skipped, got {:?}",
            result
        );
    }

    #[test]
    fn ensure_event_parameter_if_connected_skips_when_not_authenticated() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = crate::vtube_studio::VTubeStudioService::new();
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.set_desired_running(true);
        assert!(!svc.is_authenticated());
        let result = rt.block_on(svc.ensure_event_parameter_if_connected("TestParam"));
        assert!(
            matches!(result, Ok(crate::vtube_studio::EnsureOutcome::Skipped(_))),
            "expected Skipped, got {:?}",
            result
        );
    }

    #[test]
    fn create_before_persist_old_differs_from_new_triggers_publish() {
        assert!(should_publish_event_parameter(true, "OldInput", "NewInput"));
    }

    #[test]
    fn create_before_persist_old_equals_new_skips_publish() {
        assert!(!should_publish_event_parameter(
            true,
            "SameInput",
            "SameInput"
        ));
    }

    #[test]
    fn create_before_persist_hotkeys_mode_never_publishes() {
        assert!(!should_publish_event_parameter(
            false, "OldInput", "NewInput"
        ));
    }

    #[test]
    fn create_before_persist_item_mode_never_publishes() {
        assert!(!should_publish_event_parameter(false, "old", "new"));
    }

    #[test]
    fn guard_rejects_race_connected_to_connecting() {
        let svc = crate::vtube_studio::VTubeStudioService::new();
        svc.set_connection_status(VTubeStudioConnectionStatus::Connecting);
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connecting
        );
        assert!(!svc.is_authenticated());
    }

    #[test]
    fn guard_rejects_race_connected_to_error() {
        let svc = crate::vtube_studio::VTubeStudioService::new();
        svc.set_connection_status(VTubeStudioConnectionStatus::Error);
        assert!(!svc.is_authenticated());
    }

    // ---------------------------------------------------------------------------
    // delete_event_parameter_if_connected guard condition tests (pure)
    // ---------------------------------------------------------------------------

    #[test]
    fn delete_param_if_connected_skips_when_disconnected() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = crate::vtube_studio::VTubeStudioService::new();
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
        let result = rt.block_on(svc.delete_event_parameter_if_connected("OldParam"));
        assert!(
            matches!(result, Ok(crate::vtube_studio::DeleteOutcome::Skipped(_))),
            "expected Skipped, got {:?}",
            result
        );
    }

    #[test]
    fn delete_param_if_connected_skips_when_not_authenticated() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = crate::vtube_studio::VTubeStudioService::new();
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.set_desired_running(true);
        assert!(!svc.is_authenticated());
        let result = rt.block_on(svc.delete_event_parameter_if_connected("OldParam"));
        assert!(
            matches!(result, Ok(crate::vtube_studio::DeleteOutcome::Skipped(_))),
            "expected Skipped, got {:?}",
            result
        );
    }

    #[test]
    fn delete_param_if_connected_skips_when_not_desired_running() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = crate::vtube_studio::VTubeStudioService::new();
        let result = rt.block_on(svc.delete_event_parameter_if_connected("OldParam"));
        assert!(
            matches!(result, Ok(crate::vtube_studio::DeleteOutcome::Skipped(_))),
            "expected Skipped, got {:?}",
            result
        );
    }

    #[test]
    fn delete_param_if_connected_skips_when_no_socket() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = crate::vtube_studio::VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.mark_authenticated(true);
        let result = rt.block_on(svc.delete_event_parameter_if_connected("OldParam"));
        assert!(
            matches!(result, Ok(crate::vtube_studio::DeleteOutcome::Skipped(_))),
            "expected Skipped, got {:?}",
            result
        );
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected,
            "connection status unchanged when no socket"
        );
        assert!(svc.is_authenticated());
    }

    // ---------------------------------------------------------------------------
    // Transition tests: Event↔Hotkeys/Item → deletion НЕ выполняется
    // ---------------------------------------------------------------------------

    #[test]
    fn event_to_hotkeys_should_not_delete() {
        assert!(!should_publish_event_parameter(
            false, "OldInput", "NewInput"
        ));
    }

    #[test]
    fn event_to_item_should_not_delete() {
        assert!(!should_publish_event_parameter(
            false, "OldInput", "NewInput"
        ));
    }

    #[test]
    fn hotkeys_to_event_creates_new_param() {
        // Hotkeys -> Event with a fresh name: the new INPUT must be created.
        // (Deletion of an old INPUT is gated separately by needs_delete, which
        // requires old_output_mode == Event; that path is covered by the
        // command-level order, not by should_publish_event_parameter.)
        assert!(should_publish_event_parameter(true, "", "NewInput"));
    }

    #[test]
    fn event_to_event_old_equals_new_should_not_delete() {
        assert!(!should_publish_event_parameter(true, "Same", "Same"));
    }

    #[test]
    fn event_to_event_old_differs_new_should_delete() {
        assert!(should_publish_event_parameter(true, "OldInput", "NewInput"));
    }

    // ---------------------------------------------------------------------------
    // semantic vs non-semantic errors for deletion
    // ---------------------------------------------------------------------------

    #[test]
    fn delete_semantic_error_401_is_semantic() {
        use crate::vtube_studio::is_semantic_vts_error;
        assert!(is_semantic_vts_error(
            "Delete parameter failed: VTS error 401"
        ));
    }

    #[test]
    fn delete_transport_error_is_not_semantic() {
        use crate::vtube_studio::is_semantic_vts_error;
        assert!(!is_semantic_vts_error(
            "Delete parameter failed: Read error: connection reset"
        ));
    }

    struct RecordingSaveOperations {
        events: std::sync::Mutex<Vec<String>>,
        live_error: Option<String>,
        ensure_outcome: Result<crate::vtube_studio::EnsureOutcome, String>,
        persist_error: Option<String>,
        delete_outcome: Result<crate::vtube_studio::DeleteOutcome, String>,
        runtime_action: std::sync::Mutex<Option<VTubeStudioTypingAction>>,
    }

    impl Default for RecordingSaveOperations {
        fn default() -> Self {
            Self {
                events: std::sync::Mutex::new(Vec::new()),
                live_error: None,
                ensure_outcome: Ok(crate::vtube_studio::EnsureOutcome::Ensured),
                persist_error: None,
                delete_outcome: Ok(crate::vtube_studio::DeleteOutcome::Deleted),
                runtime_action: std::sync::Mutex::new(None),
            }
        }
    }

    impl RecordingSaveOperations {
        fn record(&self, event: impl Into<String>) {
            self.events.lock().unwrap().push(event.into());
        }

        fn recorded(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl TypingActionSaveOperations for RecordingSaveOperations {
        async fn require_live_connection(&self) -> Result<(), String> {
            self.record("live");
            match &self.live_error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        async fn ensure_parameter(
            &self,
            parameter_name: &str,
        ) -> Result<crate::vtube_studio::EnsureOutcome, String> {
            self.record(format!("ensure:{parameter_name}"));
            self.ensure_outcome.clone()
        }

        async fn persist_action(&self, action: &VTubeStudioTypingAction) -> Result<(), String> {
            self.record(format!("persist:{}", action.parameter_name));
            match &self.persist_error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        async fn apply_runtime(&self, action: &VTubeStudioTypingAction) {
            self.record(format!("runtime:{}", action.parameter_name));
            *self.runtime_action.lock().unwrap() = Some(action.clone());
        }

        fn notify_settings_changed(&self) {
            self.record("notify");
        }

        async fn stop_old_event(&self, parameter_name: &str) {
            self.record(format!("stop:{parameter_name}"));
        }

        async fn delete_parameter(
            &self,
            parameter_name: &str,
        ) -> Result<crate::vtube_studio::DeleteOutcome, String> {
            self.record(format!("delete:{parameter_name}"));
            self.delete_outcome.clone()
        }
    }

    fn regression_action(
        output_mode: VTubeStudioTypingMode,
        parameter_name: &str,
    ) -> VTubeStudioTypingAction {
        VTubeStudioTypingAction {
            output_mode,
            parameter_name: parameter_name.to_string(),
            start_hotkey_id: "start".to_string(),
            stop_hotkey_id: "stop".to_string(),
            start_hotkey_name: String::new(),
            stop_hotkey_name: String::new(),
            item_file_name: String::new(),
            item_type: String::new(),
        }
    }

    #[tokio::test]
    async fn regression_full_rename_order_is_executable() {
        let operations = RecordingSaveOperations::default();
        let action = regression_action(VTubeStudioTypingMode::Event, "NewInput");

        let result = orchestrate_typing_action_save(
            &operations,
            &VTubeStudioTypingMode::Event,
            "OldInput",
            &action,
        )
        .await
        .unwrap();

        assert_eq!(
            operations.recorded(),
            [
                "live",
                "ensure:NewInput",
                "persist:NewInput",
                "runtime:NewInput",
                "notify",
                "stop:OldInput",
                "delete:OldInput",
            ]
        );
        assert!(result.ensured_parameter);
        assert_eq!(result.cleanup_warning, None);
        assert_eq!(
            operations
                .runtime_action
                .lock()
                .unwrap()
                .as_ref()
                .map(|saved| saved.parameter_name.as_str()),
            Some("NewInput")
        );
    }

    #[tokio::test]
    async fn regression_skipped_ensure_never_persists_or_applies_runtime() {
        let operations = RecordingSaveOperations {
            ensure_outcome: Ok(crate::vtube_studio::EnsureOutcome::Skipped(
                crate::vtube_studio::SkipReason::NoSocket,
            )),
            ..Default::default()
        };
        let action = regression_action(VTubeStudioTypingMode::Event, "NewInput");

        let error = orchestrate_typing_action_save(
            &operations,
            &VTubeStudioTypingMode::Event,
            "OldInput",
            &action,
        )
        .await
        .unwrap_err();

        assert!(error.contains("соединение потеряно"));
        assert_eq!(operations.recorded(), ["live", "ensure:NewInput"]);
        assert!(operations.runtime_action.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn regression_mode_to_event_same_name_still_ensures() {
        for old_mode in [VTubeStudioTypingMode::Hotkeys, VTubeStudioTypingMode::Item] {
            let operations = RecordingSaveOperations::default();
            let action = regression_action(VTubeStudioTypingMode::Event, "SameInput");
            orchestrate_typing_action_save(&operations, &old_mode, "SameInput", &action)
                .await
                .unwrap();
            assert_eq!(
                operations.recorded(),
                [
                    "live",
                    "ensure:SameInput",
                    "persist:SameInput",
                    "runtime:SameInput",
                    "notify",
                ]
            );
        }
    }

    #[tokio::test]
    async fn regression_unchanged_event_does_not_ensure_or_delete() {
        let operations = RecordingSaveOperations::default();
        let action = regression_action(VTubeStudioTypingMode::Event, "SameInput");
        let result = orchestrate_typing_action_save(
            &operations,
            &VTubeStudioTypingMode::Event,
            "SameInput",
            &action,
        )
        .await
        .unwrap();

        assert_eq!(
            operations.recorded(),
            ["live", "persist:SameInput", "runtime:SameInput", "notify"]
        );
        assert!(!result.ensured_parameter);
    }

    #[tokio::test]
    async fn regression_persist_failure_does_not_compensate_or_apply_runtime() {
        let operations = RecordingSaveOperations {
            persist_error: Some("disk full".to_string()),
            ..Default::default()
        };
        let action = regression_action(VTubeStudioTypingMode::Event, "NewInput");

        let error = orchestrate_typing_action_save(
            &operations,
            &VTubeStudioTypingMode::Event,
            "OldInput",
            &action,
        )
        .await
        .unwrap_err();

        assert_eq!(error, "disk full");
        assert_eq!(
            operations.recorded(),
            ["live", "ensure:NewInput", "persist:NewInput"]
        );
        assert!(operations.runtime_action.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn regression_skipped_delete_returns_partial_success_warning() {
        let operations = RecordingSaveOperations {
            delete_outcome: Ok(crate::vtube_studio::DeleteOutcome::Skipped(
                crate::vtube_studio::SkipReason::NoSocket,
            )),
            ..Default::default()
        };
        let action = regression_action(VTubeStudioTypingMode::Event, "NewInput");

        let result = orchestrate_typing_action_save(
            &operations,
            &VTubeStudioTypingMode::Event,
            "OldInput",
            &action,
        )
        .await
        .unwrap();

        let warning = result.cleanup_warning.unwrap();
        assert!(warning.contains("OldInput"));
        assert!(warning.contains("соединение потеряно"));
    }

    #[tokio::test]
    async fn regression_delete_not_found_is_full_success() {
        let operations = RecordingSaveOperations {
            delete_outcome: Ok(crate::vtube_studio::DeleteOutcome::NotFound),
            ..Default::default()
        };
        let action = regression_action(VTubeStudioTypingMode::Event, "NewInput");

        let result = orchestrate_typing_action_save(
            &operations,
            &VTubeStudioTypingMode::Event,
            "OldInput",
            &action,
        )
        .await
        .unwrap();

        assert_eq!(result.cleanup_warning, None);
        assert_eq!(operations.recorded().last().unwrap(), "delete:OldInput");
    }
}
