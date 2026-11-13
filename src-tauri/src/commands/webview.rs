use crate::config::validate_port;
use crate::config::SettingsManager;
use crate::ipc::CommandError;
use crate::state::AppState;
use crate::webview::{UpnpToggleOutcome, WebViewSettings};
use std::fs;
use tauri::{Manager, State};

fn validate_upnp_token(enabled: bool, access_token: Option<&str>) -> Result<(), String> {
    if enabled && access_token.is_none_or(str::is_empty) {
        return Err("Сначала сгенерируйте токен доступа для внешнего WebView".to_string());
    }
    Ok(())
}

/// Get current webview settings from AppState
#[tauri::command]
pub async fn get_webview_settings(state: State<'_, AppState>) -> Result<WebViewSettings, String> {
    let settings = state.webview.settings.read().await;
    Ok(WebViewSettings {
        enabled: settings.enabled,
        start_on_boot: settings.start_on_boot,
        port: settings.port,
        access_token: settings.access_token.clone(),
        upnp_enabled: settings.upnp_enabled,
        send_original_text: settings.send_original_text,
    })
}

/// Runtime listener state, separate from persisted `WebViewSettings.enabled`.
#[tauri::command]
pub fn get_webview_server_status(
    state: State<'_, AppState>,
) -> crate::webview::WebViewServerStatus {
    state.webview.status()
}

/// Get individual webview setting fields to avoid full cloning
#[tauri::command]
pub async fn get_webview_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.webview.settings.read().await.enabled)
}

#[tauri::command]
pub async fn get_webview_start_on_boot(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.webview.settings.read().await.start_on_boot)
}

#[tauri::command]
pub async fn get_webview_port(state: State<'_, AppState>) -> Result<u16, String> {
    Ok(state.webview.settings.read().await.port)
}

/// Save webview settings to AppState and persist to files
#[tauri::command]
pub async fn save_webview_settings(
    settings: WebViewSettings,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    tracing::info!(
        enabled = settings.enabled,
        start_on_boot = settings.start_on_boot,
        port = settings.port,
        "Saving webview settings"
    );

    // UPnP валидируем по фактически сохранённому токену из runtime-state, а не по
    // клиентскому DTO: access_token этим сохранением не меняется, сервер работает
    // именно с state, и DTO не должен подменять/обходить проверку.
    let stored_token = state.webview.settings.read().await.access_token.clone();
    validate_upnp_token(settings.upnp_enabled, stored_token.as_deref())?;
    validate_port(settings.port).map_err(|e| e.to_string())?;

    // Check if enabled status or port changed (start_on_boot doesn't require restart)
    let old_settings = state.webview.settings.read().await;
    let enabled_changed = old_settings.enabled != settings.enabled;
    let port_changed = old_settings.port != settings.port;
    let upnp_changed = old_settings.upnp_enabled != settings.upnp_enabled;
    drop(old_settings);

    // Get SettingsManager once and persist to config atomically
    let settings_manager = app_handle
        .try_state::<SettingsManager>()
        .ok_or_else(|| "SettingsManager not available".to_string())?;
    let start_on_boot = settings.start_on_boot;
    let port = settings.port;
    let upnp_enabled = settings.upnp_enabled;
    let send_original_text = settings.send_original_text;
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_webview_section(start_on_boot, port, upnp_enabled, send_original_text)
    })
    .await?;

    // Only after successful file save, update AppState (runtime state)
    let mut s = state.webview.settings.write().await;
    s.enabled = settings.enabled;
    s.start_on_boot = settings.start_on_boot;
    s.port = settings.port;
    s.upnp_enabled = settings.upnp_enabled;
    s.send_original_text = settings.send_original_text;
    drop(s);

    super::emit_settings_changed(&app_handle);

    // Trigger UPnP toggle if it changed (without server restart). Настройка уже
    // сохранена, поэтому отказ router'а только логируется: у полного сохранения
    // секции свой контракт ответа.
    if upnp_changed {
        let outcome = state.webview.apply_upnp_toggle(settings.upnp_enabled).await;
        state.webview.record_upnp_outcome(&outcome);
        if let UpnpToggleOutcome::ForwardFailed { code } = outcome {
            tracing::warn!(code, "UPnP toggle from settings save was not confirmed");
        }
    }

    // Trigger server restart if server settings changed
    // Note: start_on_boot changes don't require restart (only affects next boot)
    // A save with `enabled = true` while the server is down (например, retry
    // после ошибки запуска) тоже обязан применить настройки: без этого кнопка
    // запуска при неизменных полях не даёт серверу повторного старта.
    let server_needs_apply = settings.enabled
        && !matches!(
            state.webview.status(),
            crate::webview::WebViewServerStatus::Running
                | crate::webview::WebViewServerStatus::Starting,
        );
    if enabled_changed || port_changed || server_needs_apply {
        tracing::info!("Sending RestartWebViewServer event to WebView server");
        // Send restart event directly to WebView server using the state parameter
        state
            .webview
            .send_event(crate::events::AppEvent::RestartWebViewServer);
        tracing::debug!("RestartWebViewServer event sent successfully");
        Ok("saved_restarting".to_string())
    } else {
        Ok("saved".to_string())
    }
}

/// Перезапустить сервер WebView одним циклом без записи настроек.
///
/// Команда не меняет ни persisted-конфиг, ни runtime `enabled`: работающий
/// сервер получает ровно один цикл stop→start, а отключённый остаётся
/// отключённым (supervisor перечитает `enabled` и продолжит ждать).
#[tauri::command]
pub async fn restart_webview_server(state: State<'_, AppState>) -> Result<(), String> {
    tracing::info!("Sending RestartWebViewServer event (explicit restart command)");
    state
        .webview
        .send_event(crate::events::AppEvent::RestartWebViewServer);
    Ok(())
}

/// Get local IP address using UDP socket trick
#[tauri::command]
pub fn get_local_ip() -> Result<String, String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| format!("Failed to bind socket: {}", e))?;
    socket
        .connect("8.8.8.8:80")
        .map_err(|e| format!("Failed to connect: {}", e))?;
    let local_ip = socket
        .local_addr()
        .map_err(|e| format!("Failed to get local address: {}", e))?
        .ip()
        .to_string();
    Ok(local_ip)
}

/// Open template folder in file explorer
#[tauri::command]
pub async fn open_template_folder() -> Result<(), String> {
    let config_dir = crate::paths::config_root()
        .map_err(|_| "Failed to get config dir".to_string())?
        .join("webview");

    // Create directory first, before canonicalize
    fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;

    let config_dir = config_dir
        .canonicalize()
        .map_err(|e| format!("Invalid config dir: {}", e))?;

    let path = config_dir.to_str().ok_or("Invalid path")?;

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

/// Send test message to SSE (without TTS)
#[tauri::command]
pub async fn send_test_message(text: String, state: State<'_, AppState>) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Text cannot be empty".to_string());
    }

    // Send ONLY to WebView channel, not to TTS
    // This allows testing WebView display without triggering voice synthesis
    state
        .webview
        .send_event(crate::events::AppEvent::TextSentToTts(
            crate::events::RoutedText::broadcast(text),
        ));
    Ok(())
}

/// Reload templates from disk (hot reload without server restart)
#[tauri::command]
pub async fn reload_templates(state: State<'_, AppState>) -> Result<String, String> {
    // Send event to reload templates without restarting the server
    state
        .webview
        .send_event(crate::events::AppEvent::ReloadWebViewTemplates);
    Ok("reloaded".to_string())
}

// ==================== Security Commands ====================

/// Generate a new access token for external WebView access
#[tauri::command]
pub async fn generate_webview_token(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let token = uuid::Uuid::new_v4().to_string();

    let settings_manager = app_handle
        .try_state::<SettingsManager>()
        .ok_or_else(|| "SettingsManager not available".to_string())?;
    let persisted_token = token.clone();
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_webview_access_token(Some(persisted_token))
    })
    .await?;

    state.webview.settings.write().await.access_token = Some(token.clone());
    super::emit_settings_changed(&app_handle);

    Ok(token)
}

/// Get the masked access token (first 8 chars only)
#[tauri::command]
pub async fn get_webview_token(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let settings = state.webview.settings.read().await;
    Ok(settings.access_token.as_ref().map(|t| {
        if t.len() > 8 {
            format!("{}***", &t[..8])
        } else {
            t.clone()
        }
    }))
}

/// Copy the access token to clipboard
#[tauri::command]
pub async fn copy_webview_token(state: State<'_, AppState>) -> Result<String, String> {
    let settings = state.webview.settings.read().await;
    let token = settings
        .access_token
        .clone()
        .ok_or("Токен не сгенерирован")?;

    drop(settings);
    Ok(token)
}

/// Regenerate the access token
#[tauri::command]
pub async fn regenerate_webview_token(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let token = uuid::Uuid::new_v4().to_string();

    let settings_manager = app_handle
        .try_state::<SettingsManager>()
        .ok_or_else(|| "SettingsManager not available".to_string())?;
    let persisted_token = token.clone();
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_webview_access_token(Some(persisted_token))
    })
    .await?;

    state.webview.settings.write().await.access_token = Some(token);
    super::emit_settings_changed(&app_handle);

    // Restart server to apply new token
    state
        .webview
        .send_event(crate::events::AppEvent::RestartWebViewServer);

    Ok("Токен перегенерирован. Старый токен больше не действителен.".to_string())
}

/// Set UPnP enabled status
///
/// Настройка — пожелание, mapping — факт: команда сохраняет настройку и затем
/// применяет переключение к живому владельцу, возвращая то, что действительно
/// произошло (`applied` / `preference_only` / `forward_failed` с кодом причины).
/// Поэтому UI не подтверждает открытый порт, которого нет.
#[tauri::command]
pub async fn set_webview_upnp_enabled(
    enabled: bool,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<UpnpToggleOutcome, CommandError> {
    tracing::info!(enabled = enabled, "Setting UPnP enabled");

    let access_token = state.webview.settings.read().await.access_token.clone();
    validate_upnp_token(enabled, access_token.as_deref()).map_err(|e| {
        tracing::warn!(enabled = enabled, error = %e, "UPnP validation failed");
        CommandError::new("webview.upnp.token_required", e, false)
    })?;

    let settings_manager = app_handle
        .try_state::<SettingsManager>()
        .ok_or_else(|| "SettingsManager not available".to_string())
        .map_err(|e| {
            tracing::warn!(enabled = enabled, error = %e, "UPnP settings manager unavailable");
            CommandError::new("webview.settings_save_failed", e, false)
        })?;
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_webview_upnp_enabled(enabled)
    })
    .await
    .map_err(|e| {
        tracing::warn!(enabled = enabled, error = %e, "Failed to persist UPnP setting");
        CommandError::new("webview.settings_save_failed", e, false)
    })?;

    state.webview.settings.write().await.upnp_enabled = enabled;
    super::emit_settings_changed(&app_handle);

    // Настройка уже сохранена: отказ router'а возвращается как результат, а не как
    // ошибка команды, иначе UI откатил бы тумблер, который backend принял.
    let outcome = state.webview.apply_upnp_toggle(enabled).await;
    state.webview.record_upnp_outcome(&outcome);
    Ok(outcome)
}

/// Get UPnP enabled status
#[tauri::command]
pub async fn get_webview_upnp_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.webview.settings.read().await.upnp_enabled)
}

/// Фактический runtime-статус UPnP-проброса: mapping — факт, а не пожелание.
#[tauri::command]
pub fn get_webview_upnp_status(
    state: State<'_, AppState>,
) -> crate::webview::service::UpnpForwardStatus {
    state.webview.upnp_forward_status()
}

#[cfg(test)]
mod tests {
    use super::{parse_ipv4_response, validate_upnp_token};

    #[test]
    fn upnp_requires_configured_token() {
        assert!(validate_upnp_token(true, None).is_err());
        assert!(validate_upnp_token(true, Some("")).is_err());
        assert!(validate_upnp_token(true, Some("token")).is_ok());
    }

    #[test]
    fn disabling_upnp_never_requires_token() {
        assert!(validate_upnp_token(false, None).is_ok());
    }

    #[test]
    fn parses_trimmed_ipv4_response() {
        assert_eq!(
            parse_ipv4_response("  5.166.39.240\n").map(|ip| ip.to_string()),
            Some("5.166.39.240".to_string())
        );
    }

    #[test]
    fn rejects_invalid_empty_and_ipv6_response() {
        assert!(parse_ipv4_response("").is_none());
        assert!(parse_ipv4_response("   ").is_none());
        assert!(parse_ipv4_response("<html>5.166.39.240</html>").is_none());
        assert!(parse_ipv4_response("2001:db8::1").is_none());
        assert!(parse_ipv4_response("999.1.1.1").is_none());
    }
}

/// Forward typing state to WebView SSE (consumer adapter for the editor typing burst)
#[tauri::command]
pub async fn set_webview_typing(typing: bool, state: State<'_, AppState>) -> Result<(), String> {
    state
        .webview
        .send_event(crate::events::AppEvent::WebViewTypingChanged(typing));
    Ok(())
}

/// Parse a plain-text response body as a trimmed IPv4 address.
fn parse_ipv4_response(body: &str) -> Option<std::net::Ipv4Addr> {
    body.trim().parse().ok()
}

/// Get external/public IP address with fallback
#[tauri::command]
pub async fn get_external_ip() -> Result<String, String> {
    let sources = [
        "https://api.ipify.org?format=text",
        "https://icanhazip.com",
        "https://ifconfig.me",
    ];

    // Bypass any configured proxy so the returned address is the direct public
    // IPv4 used by the router/WebView port mapping.
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| {
            "Не удалось получить внешний IP. Проверьте подключение к интернету.".to_string()
        })?;

    for url in sources {
        let resp = match client.get(url).send().await {
            Ok(resp) => resp,
            Err(_) => continue,
        };
        let resp = match resp.error_for_status() {
            Ok(resp) => resp,
            Err(_) => continue,
        };
        if let Ok(body) = resp.text().await {
            if let Some(ip) = parse_ipv4_response(&body) {
                return Ok(ip.to_string());
            }
        }
    }

    Err("Не удалось получить внешний IP. Проверьте подключение к интернету.".to_string())
}
