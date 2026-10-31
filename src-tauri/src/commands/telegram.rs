use crate::config::{ProxyMode, SettingsManager};
use crate::ipc::CommandError;
use crate::telegram::types::AuthState;
use crate::telegram::{
    bot::set_speaker, get_current_voice, get_limits, types::VoiceCode, CurrentVoice, Limits,
    ProxyStatus, SileroRuntimeSettings, SileroTtsBot, TelegramClient, TtsResult, UserInfo,
};
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::{Mutex, RwLock};
use tracing::info;

const TELEGRAM_NOT_INITIALIZED: &str = "telegram.not_initialized";
const TELEGRAM_STATUS_FAILED: &str = "telegram.status_failed";

/// Глобальное состояние Telegram клиента.
///
/// # Контракт разделения `client` (см. DECISION-018)
///
/// `client` намеренно остаётся `Arc<Mutex<Option<TelegramClient>>>` (разделяемым,
/// не private), потому что этот же Arc по контракту хранят долгоживущие движки —
/// `SileroTts` (`tts/silero.rs`) и AI flow — которым клиент нужен в их hot path
/// и которые не имеют и не должны иметь ссылки на `TelegramState`. Сделать mutex
/// строго private значило бы перерезать контракт hot path TTS (риск регрессии
/// латентности и ROADMAP-041 serializer-инвариантов) без технической выгоды: гонки
/// данных нет (Arc<Mutex> даёт synchronisation), паник нет (везде `Option` + `?`),
/// а `take()`/`disconnect()` под reconnect деградируют в graceful error.
///
/// Единственное правило доступа к `client` (обязательно для ВСЕХ, включая движки):
///   `lock → clone Option → drop guard → await` —
/// guard никогда не удерживается через network request / filesystem I/O / другой
/// длительный `await` (ROADMAP-059 инвариант #3).
///
/// Команды IPC не должны писать `state.client.lock()` напрямую — они делегируют
/// owner-методам ниже (`current_client` / `set_client` / `clear_client` /
/// `swap_client` / `with_client`). Прямой locker оправдан только для долгоживущего
/// движка без доступа к owner-API.
pub struct TelegramState {
    pub(crate) client: Arc<Mutex<Option<TelegramClient>>>,
    /// Cached proxy status (updated on connection changes)
    proxy_status: Arc<RwLock<ProxyStatus>>,
}

impl TelegramState {
    pub fn new() -> Self {
        Self {
            client: Arc::new(Mutex::new(None)),
            proxy_status: Arc::new(RwLock::new(ProxyStatus::default())),
        }
    }

    /// Get cached proxy status (fast, no lock on client)
    pub async fn get_proxy_status_cached(&self) -> ProxyStatus {
        self.proxy_status.read().await.clone()
    }

    /// Update cached proxy status
    pub async fn update_proxy_status(&self, status: ProxyStatus) {
        *self.proxy_status.write().await = status;
    }
}

impl Default for TelegramState {
    fn default() -> Self {
        Self::new()
    }
}

impl TelegramState {
    /// Текущий клиент (clone Option под коротким локом, guard не пересекает await).
    pub(crate) async fn current_client(&self) -> Option<TelegramClient> {
        self.client.lock().await.clone()
    }

    /// Сохранить/заменить клиент (для init / auto_restore).
    pub(crate) async fn set_client(&self, client: TelegramClient) {
        *self.client.lock().await = Some(client);
    }

    /// Вынуть и вернуть клиент (для disconnect / sign_out).
    /// Возвращает прежний клиент, оставляя None; disconnect вызывающий делает
    /// САМ после drop guard (не внутри этого метода).
    pub(crate) async fn clear_client(&self) -> Option<TelegramClient> {
        self.client.lock().await.take()
    }

    /// Clear the client only when it is still the same logical instance that
    /// started an asynchronous lifecycle operation.
    pub(crate) async fn clear_client_if_current(&self, expected: &TelegramClient) -> bool {
        let mut guard = self.client.lock().await;
        let is_current = guard
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(&current.client, &expected.client));
        if is_current {
            guard.take();
        }
        is_current
    }

    /// Replace the client with `new` and return the previous one (if any).
    /// Short lock only; the caller disconnects the old client AFTER the guard drops.
    pub(crate) async fn swap_client(&self, new: TelegramClient) -> Option<TelegramClient> {
        self.client.lock().await.replace(new)
    }

    /// Run an async operation on a reference to the current client without holding
    /// the state lock. Locks briefly to clone the Option<TelegramClient>, drops
    /// the guard, then awaits `f(&client)`. Returns an error when no client is set.
    ///
    /// Uses a boxed future so the borrow of `client` is captured inside the
    /// returned future without leaking a lifetime into the caller's signature.
    pub(crate) async fn with_client<R>(
        &self,
        f: impl for<'a> FnOnce(&'a TelegramClient) -> futures::future::BoxFuture<'a, Result<R, String>>,
    ) -> Result<R, String> {
        let client = self
            .current_client()
            .await
            .ok_or_else(|| "Telegram client not initialized".to_string())?;
        f(&client).await
    }
}

fn validate_telegram_init_api_id(api_id: Option<u32>) -> Result<u32, String> {
    match api_id {
        Some(api_id) if api_id > 0 => Ok(api_id),
        _ => Err(
            "Укажите Telegram API ID в настройках Telegram перед подключением Silero Bot."
                .to_string(),
        ),
    }
}

/// Инициализация Telegram клиента
#[tauri::command]
pub async fn telegram_init(
    state: State<'_, TelegramState>,
    settings_manager: State<'_, SettingsManager>,
    api_id: Option<u32>,
    api_hash: String,
    phone: String,
) -> Result<(), String> {
    // Валидация входных данных
    let api_id = validate_telegram_init_api_id(api_id)?;
    if api_hash.trim().is_empty() {
        return Err("API Hash не может быть пустым".to_string());
    }
    if phone.trim().is_empty() {
        return Err("Номер телефона не может быть пустым".to_string());
    }

    // Load proxy settings
    let settings = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?;

    info!(proxy_mode = ?settings.tts.telegram.proxy_mode, "Initializing Telegram with proxy settings");

    // Создаем новый клиент
    let client = TelegramClient::new();

    // Инициализируем с поддержкой выбранного прокси
    match &settings.tts.telegram.proxy_mode {
        ProxyMode::None => {
            info!("Initializing without proxy");
            client
                .init_with_proxy(api_id, api_hash, phone, None)
                .await
                .map_err(|e| format!("Ошибка инициализации клиента: {}", e))?;
        }
        ProxyMode::Socks5 => {
            let proxy_url = settings.tts.network.proxy.proxy_url.clone();
            info!(
                has_proxy = proxy_url.is_some(),
                "Initializing with SOCKS5 proxy"
            );
            client
                .init_with_proxy(api_id, api_hash, phone, proxy_url)
                .await
                .map_err(|e| format!("Ошибка инициализации клиента: {}", e))?;
        }
        ProxyMode::MtProxy => {
            #[cfg(feature = "mtproxy")]
            {
                info!("Initializing with MTProxy");
                client
                    .init_with_mtproxy(api_id, api_hash, phone, &settings.tts.network.mtproxy)
                    .await
                    .map_err(|e| format!("Ошибка инициализации клиента: {}", e))?;
            }
            #[cfg(not(feature = "mtproxy"))]
            {
                return Err("MTProxy feature is not enabled".to_string());
            }
        }
    }

    // Сохраняем клиент в состоянии
    state.set_client(client.clone()).await;

    // Update cached proxy status
    let proxy_status = client.get_proxy_status().await;
    state.update_proxy_status(proxy_status).await;

    Ok(())
}

/// Запрос кода подтверждения
#[tauri::command]
pub async fn telegram_request_code(state: State<'_, TelegramState>) -> Result<(), String> {
    let client = state
        .current_client()
        .await
        .ok_or_else(|| "Клиент не инициализирован. Сначала вызовите telegram_init.".to_string())?;

    client
        .request_code()
        .await
        .map_err(|e| format!("Ошибка запроса кода: {}", e))?;
    Ok(())
}

/// Ввод кода подтверждения
#[tauri::command]
pub async fn telegram_sign_in(
    state: State<'_, TelegramState>,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
    code: String,
) -> Result<AuthState, String> {
    if code.trim().is_empty() {
        return Err("Код не может быть пустым".to_string());
    }

    let client = state
        .current_client()
        .await
        .ok_or_else(|| "Клиент не инициализирован. Сначала вызовите telegram_init.".to_string())?;

    let auth_state = client
        .sign_in(&code)
        .await
        .map_err(|e| format!("Ошибка входа: {}", e))?;

    if let AuthState::Connected = &auth_state {
        let api_id_to_save = {
            let api_id_lock = client.api_id.lock().await;
            match &*api_id_lock {
                Some(id) => *id,
                None => return Err("api_id not set".to_string()),
            }
        };

        super::persist_blocking(settings_manager.inner(), move |mgr| {
            mgr.set_telegram_api_id(Some(api_id_to_save as i64))
        })
        .await?;

        super::emit_settings_changed(&app_handle);
    }

    Ok(auth_state)
}

/// Проверка пароля двухфакторной аутентификации
#[tauri::command]
pub async fn telegram_check_password(
    state: State<'_, TelegramState>,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
    password: String,
) -> Result<AuthState, String> {
    if password.trim().is_empty() {
        return Err("Пароль не может быть пустым".to_string());
    }

    let client = state
        .current_client()
        .await
        .ok_or_else(|| "Клиент не инициализирован".to_string())?;

    let auth_state = client
        .check_password(&password)
        .await
        .map_err(|e| format!("Ошибка 2FA: {}", e))?;

    if let AuthState::Connected = &auth_state {
        let api_id_to_save = {
            let api_id_lock = client.api_id.lock().await;
            match &*api_id_lock {
                Some(id) => *id,
                None => return Err("api_id not set".to_string()),
            }
        };

        super::persist_blocking(settings_manager.inner(), move |mgr| {
            mgr.set_telegram_api_id(Some(api_id_to_save as i64))
        })
        .await?;

        super::emit_settings_changed(&app_handle);
    }

    Ok(auth_state)
}

/// Отмена незавершённого auth-flow без удаления сессии
#[tauri::command]
pub async fn telegram_disconnect(state: State<'_, TelegramState>) -> Result<(), String> {
    let client_opt = state.clear_client().await;

    if let Some(client) = client_opt {
        tokio::spawn(async move {
            let _ = client.disconnect().await;
        });
    }

    Ok(())
}

/// Выход из аккаунта
#[tauri::command]
pub async fn telegram_sign_out(
    state: State<'_, TelegramState>,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let client = state
        .current_client()
        .await
        .ok_or_else(|| "Клиент не инициализирован".to_string())?;

    client
        .sign_out()
        .await
        .map_err(|e| format!("Ошибка выхода: {}", e))?;

    // A newer init/reconnect may have replaced the client while sign_out was
    // awaiting network I/O. Never let this stale completion clear that client
    // or its persisted session metadata.
    if !state.clear_client_if_current(&client).await {
        info!("Ignoring stale Telegram sign-out completion after client replacement");
        return Ok(());
    }

    // Удаляем сохранённый api_id из settings.json
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_telegram_api_id(None)
    })
    .await?;

    super::emit_settings_changed(&app_handle);

    Ok(())
}

/// Проверка статуса авторизации
#[tauri::command]
pub async fn telegram_get_status(state: State<'_, TelegramState>) -> Result<bool, CommandError> {
    let client = match state.current_client().await {
        Some(c) => c,
        None => return Ok(false), // Неинициализирован = не авторизован
    };

    if !client.is_initialized().await {
        return Err(CommandError::new(
            TELEGRAM_NOT_INITIALIZED,
            "Telegram client is not initialized",
            false,
        ));
    }

    client
        .is_authorized()
        .await
        .map_err(|error| CommandError::new(TELEGRAM_STATUS_FAILED, error, false))
}

/// Получение информации о пользователе
#[tauri::command]
pub async fn telegram_get_user(
    state: State<'_, TelegramState>,
) -> Result<Option<UserInfo>, String> {
    let client = match state.current_client().await {
        Some(c) => c,
        None => return Ok(None), // Неинициализирован = нет пользователя
    };

    // Проверяем авторизацию
    let is_authorized = client.is_authorized().await?;
    if !is_authorized {
        return Ok(None);
    }

    // Получаем информацию о пользователе
    let user_info = client.get_user_info().await?;
    Ok(Some(user_info))
}

/// Синтез речи через Silero TTS (Telegram bot)
#[tauri::command]
pub async fn speak_text_silero(
    state: State<'_, TelegramState>,
    settings_manager: State<'_, SettingsManager>,
    text: String,
) -> Result<TtsResult, String> {
    tracing::debug!(text_len = text.len(), "Starting TTS via Silero");

    // Валидация
    let text = text.trim();
    if text.is_empty() {
        return Ok(TtsResult::error("Text cannot be empty".to_string()));
    }

    // Получаем клиент
    let client = state.current_client().await.ok_or_else(|| {
        "Telegram client not initialized. Please connect to Telegram first.".to_string()
    })?;

    // Проверяем авторизацию
    let is_authorized = client.is_authorized().await?;
    if !is_authorized {
        return Ok(TtsResult::error(
            "Not authorized in Telegram. Please sign in first.".to_string(),
        ));
    }

    // Load settings, construct runtime snapshot, release lock before synthesis
    let runtime_settings = {
        let settings = settings_manager
            .load()
            .map_err(|e| format!("Failed to load settings: {}", e))?;
        SileroRuntimeSettings::new(
            settings.tts.telegram.synthesis_response_timeout_ms,
            settings.tts.telegram.download_retry_delay_ms,
        )
    };

    // Выполняем синтез
    let result = SileroTtsBot::synthesize(&client, text, &runtime_settings).await?;

    Ok(result)
}

/// Получить текущий голос Silero TTS
#[tauri::command]
pub async fn telegram_get_current_voice(
    state: State<'_, TelegramState>,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<Option<CurrentVoice>, String> {
    tracing::debug!("Getting current voice");

    // Получаем клиент
    let client = state.current_client().await.ok_or_else(|| {
        "Telegram client not initialized. Please connect to Telegram first.".to_string()
    })?;

    // Проверяем авторизацию
    let is_authorized = client.is_authorized().await?;
    if !is_authorized {
        return Ok(None);
    }

    // Получаем текущий голос (может вернуть None при таймауте)
    let voice = get_current_voice(&client).await?;

    if let Some(ref current) = voice {
        let voice_id = &current.id;
        if !voice_id.is_empty() {
            let needs_sync = {
                let settings = settings_manager
                    .load()
                    .map_err(|e| format!("Failed to load settings: {}", e))?;
                settings.tts.telegram.current_voice_id != *voice_id
            };
            if needs_sync {
                let vid = voice_id.clone();
                super::persist_blocking(settings_manager.inner(), move |mgr| {
                    let mut settings = mgr
                        .load()
                        .map_err(|e| anyhow::anyhow!("Failed to load settings: {}", e))?;
                    settings.tts.telegram.current_voice_id = vid;
                    mgr.save(&settings)
                })
                .await?;
                super::emit_settings_changed(&app_handle);
            }
        }
    }

    Ok(voice)
}

/// Получить лимиты Silero TTS
#[tauri::command]
pub async fn telegram_get_limits(
    state: State<'_, TelegramState>,
) -> Result<Option<Limits>, String> {
    tracing::debug!("Getting limits");

    // Получаем клиент
    let client = state.current_client().await.ok_or_else(|| {
        "Telegram client not initialized. Please connect to Telegram first.".to_string()
    })?;

    // Проверяем авторизацию
    let is_authorized = client.is_authorized().await?;
    if !is_authorized {
        return Ok(None);
    }

    // Получаем лимиты (может вернуть None при таймауте)
    let limits = get_limits(&client).await?;

    Ok(limits)
}

/// Автоматически восстановить сессию при старте приложения
#[tauri::command]
pub async fn telegram_auto_restore(
    state: State<'_, TelegramState>,
    settings_manager: State<'_, SettingsManager>,
) -> Result<bool, String> {
    info!("Auto-restoring session...");

    // Загружаем сохранённый api_id из settings.json
    let api_id = match settings_manager.get_telegram_api_id() {
        Some(id) => id as u32, // Convert i64 to u32
        None => {
            info!("No saved api_id found");
            return Ok(false);
        }
    };

    info!(api_id, "Found saved api_id");

    // Load proxy settings
    let settings = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {}", e))?;

    info!(proxy_mode = ?settings.tts.telegram.proxy_mode, "Restoring session with proxy settings");

    // Создаём новый клиент
    let client = TelegramClient::new();

    // Инициализируем с пустыми данными (сессия уже есть) и поддержкой выбранного прокси
    match &settings.tts.telegram.proxy_mode {
        ProxyMode::None => {
            info!("Initializing without proxy");
            client
                .init_empty(api_id)
                .await
                .map_err(|e| format!("Ошибка инициализации сессии: {}", e))?;
        }
        ProxyMode::Socks5 => {
            let proxy_url = settings.tts.network.proxy.proxy_url.clone();
            info!(
                has_proxy = proxy_url.is_some(),
                "Initializing with SOCKS5 proxy"
            );
            client
                .init_empty_with_proxy(api_id, proxy_url)
                .await
                .map_err(|e| format!("Ошибка инициализации сессии: {}", e))?;
        }
        ProxyMode::MtProxy => {
            #[cfg(feature = "mtproxy")]
            {
                info!("Initializing with MTProxy");
                client
                    .init_empty_with_mtproxy(api_id, &settings.tts.network.mtproxy)
                    .await
                    .map_err(|e| format!("Ошибка инициализации сессии: {}", e))?;
            }
            #[cfg(not(feature = "mtproxy"))]
            {
                return Err("MTProxy feature is not enabled".to_string());
            }
        }
    }

    // Сохраняем клиент в состоянии
    state.set_client(client.clone()).await;

    // Update cached proxy status
    let proxy_status = client.get_proxy_status().await;
    state.update_proxy_status(proxy_status).await;

    // Проверяем авторизацию
    let is_authorized = client.is_authorized().await?;

    if is_authorized {
        info!("Session auto-restored successfully");
        Ok(true)
    } else {
        info!("Session exists but not authorized");
        Ok(false)
    }
}

/// Установить голос, отправив "/speaker {code}" боту
#[tauri::command]
pub async fn telegram_set_speaker(
    state: State<'_, TelegramState>,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
    voice_code: String,
) -> Result<bool, String> {
    // 1. Валидация voice_code (не пустой)
    let voice_code = voice_code.trim().to_string();
    if voice_code.is_empty() {
        return Err("Voice code cannot be empty".to_string());
    }

    // 2. Получить клиент из state
    let client = state
        .current_client()
        .await
        .ok_or_else(|| "Telegram client not initialized".to_string())?;

    // 3. Вызвать bot::set_speaker()
    let success = set_speaker(&client, &voice_code).await?;

    // 4. Если успешно - обновить current_voice_id в настройках
    if success {
        let vid = voice_code;
        super::persist_blocking(settings_manager.inner(), move |mgr| {
            let mut settings = mgr
                .load()
                .map_err(|e| anyhow::anyhow!("Failed to load settings: {}", e))?;

            settings.tts.telegram.current_voice_id = vid;

            mgr.save(&settings)
        })
        .await?;

        super::emit_settings_changed(&app_handle);
    }

    Ok(success)
}

/// Set Silero synthesis response timeout in milliseconds (ROADMAP-133)
#[tauri::command]
pub async fn set_silero_synthesis_timeout(
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
    timeout_ms: u32,
) -> Result<(), String> {
    info!(timeout_ms, "Setting Silero synthesis response timeout");

    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_silero_synthesis_response_timeout_ms(timeout_ms)
    })
    .await?;

    super::emit_settings_changed(&app_handle);

    info!("Silero synthesis response timeout updated successfully");
    Ok(())
}

/// Добавить голос в список сохраненных
#[tauri::command]
pub async fn telegram_add_voice_code(
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
    voice: VoiceCode,
) -> Result<(), String> {
    if voice.id.trim().is_empty() {
        return Err("Voice ID cannot be empty".to_string());
    }

    super::persist_blocking(settings_manager.inner(), move |mgr| {
        let mut settings = mgr
            .load()
            .map_err(|e| anyhow::anyhow!("Failed to load settings: {}", e))?;

        if settings
            .tts
            .telegram
            .voices
            .iter()
            .any(|v| v.id == voice.id)
        {
            return Err(anyhow::anyhow!("Voice with this ID already exists"));
        }

        settings.tts.telegram.voices.push(voice);
        mgr.save(&settings)
    })
    .await?;

    super::emit_settings_changed(&app_handle);

    Ok(())
}

/// Удалить голос из списка сохраненных
#[tauri::command]
pub async fn telegram_remove_voice_code(
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
    voice_id: String,
) -> Result<(), String> {
    let vid = voice_id.clone();
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        let mut settings = mgr
            .load()
            .map_err(|e| anyhow::anyhow!("Failed to load settings: {}", e))?;

        settings.tts.telegram.voices.retain(|v| v.id != vid);

        if settings.tts.telegram.current_voice_id == vid {
            settings.tts.telegram.current_voice_id.clear();
        }

        mgr.save(&settings)
    })
    .await?;

    super::emit_settings_changed(&app_handle);

    Ok(())
}

/// Выбрать голос из списка
#[tauri::command]
pub async fn telegram_select_voice(
    state: State<'_, TelegramState>,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
    voice_id: String,
) -> Result<bool, String> {
    // 1. Отправить "/speaker {voice_id}" боту
    let speaker_voice_id = voice_id.clone();
    let success = state
        .with_client(|client| Box::pin(async move { set_speaker(client, &speaker_voice_id).await }))
        .await?;

    // 2. Если успешно - обновить current_voice_id
    if success {
        let vid = voice_id.clone();
        super::persist_blocking(settings_manager.inner(), move |mgr| {
            let mut settings = mgr
                .load()
                .map_err(|e| anyhow::anyhow!("Failed to load settings: {}", e))?;

            settings.tts.telegram.current_voice_id = vid;

            mgr.save(&settings)
        })
        .await?;

        super::emit_settings_changed(&app_handle);
    }

    // 3. Вернуть результат
    Ok(success)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram::TelegramClient;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Runtime::new().unwrap()
    }

    #[test]
    fn telegram_init_reports_missing_api_id_clearly() {
        let error = validate_telegram_init_api_id(None).expect_err("missing ID must fail");
        assert_eq!(
            error,
            "Укажите Telegram API ID в настройках Telegram перед подключением Silero Bot."
        );
    }

    #[test]
    fn telegram_init_rejects_zero_api_id() {
        assert!(validate_telegram_init_api_id(Some(0)).is_err());
        assert_eq!(validate_telegram_init_api_id(Some(12345)), Ok(12345));
    }

    #[test]
    fn swap_client_returns_none_when_no_client() {
        let state = TelegramState::new();
        let new_client = TelegramClient::new();
        rt().block_on(async {
            let old = state.swap_client(new_client).await;
            assert!(old.is_none());
        });
    }

    #[test]
    fn swap_client_returns_old_and_stores_new() {
        let state = TelegramState::new();
        let client_a = TelegramClient::new();
        let client_b = TelegramClient::new();
        rt().block_on(async {
            assert!(state.current_client().await.is_none());

            let old1 = state.swap_client(client_a).await;
            assert!(old1.is_none());
            assert!(state.current_client().await.is_some());

            let old2 = state.swap_client(client_b).await;
            assert!(old2.is_some());
            assert!(state.current_client().await.is_some());
        });
    }

    #[test]
    fn with_client_returns_error_when_no_client() {
        let state = TelegramState::new();
        rt().block_on(async {
            let result: Result<(), String> = state
                .with_client(|_c| Box::pin(async move { Ok(()) }))
                .await;
            assert!(result.is_err());
        });
    }

    #[test]
    fn clear_client_if_current_rejects_stale_instance() {
        let state = TelegramState::new();
        let stale = TelegramClient::new();
        let current = TelegramClient::new();
        rt().block_on(async {
            state.set_client(current.clone()).await;

            assert!(!state.clear_client_if_current(&stale).await);
            let stored = state.current_client().await.expect("current client");
            assert!(Arc::ptr_eq(&stored.client, &current.client));
        });
    }

    #[test]
    fn clear_client_if_current_removes_matching_instance() {
        let state = TelegramState::new();
        let current = TelegramClient::new();
        rt().block_on(async {
            state.set_client(current.clone()).await;

            assert!(state.clear_client_if_current(&current).await);
            assert!(state.current_client().await.is_none());
        });
    }

    #[test]
    fn swap_client_does_not_hold_lock_across_await() {
        // Structural assertion: swap_client returns the old client via Option,
        // so any disconnect must happen OUTSIDE the method. The signature is
        // the guarantee. This test exercises the happy path with no disconnect.
        let state = TelegramState::new();
        let client = TelegramClient::new();
        rt().block_on(async {
            let old = state.swap_client(client).await;
            assert!(old.is_none());
        });
    }
}
