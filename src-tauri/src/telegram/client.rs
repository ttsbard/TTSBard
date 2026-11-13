use super::types::{AuthState, OperationResult, UserInfo};
#[cfg(feature = "mtproxy")]
use crate::config::MtProxySettings;
use crate::config::ProxyMode;
use crate::secret_log;
use grammers_client::client::{LoginToken, PasswordToken};
use grammers_client::{Client, SignInError};
#[cfg(feature = "mtproxy")]
use grammers_mtsender::MtProxyConfig;
use grammers_mtsender::{ConnectionParams, InvocationError, SenderPool};
use grammers_session::storages::SqliteSession;
use grammers_session::updates::UpdatesLike;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, error, info, warn};

// NOTE: api_id is now stored in settings.json (settings.tts.telegram.api_id)
// The old telegram_config.json file is no longer used

/// Current proxy status for Telegram connection
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ProxyStatus {
    /// Current proxy mode
    pub mode: ProxyMode,
    /// Proxy URL being used (if any)
    pub proxy_url: Option<String>,
}

impl fmt::Display for ProxyStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.mode {
            ProxyMode::None => write!(f, "No proxy"),
            ProxyMode::Socks5 => write!(
                f,
                "SOCKS5 proxy: {}",
                self.proxy_url.as_deref().unwrap_or("not configured")
            ),
            ProxyMode::MtProxy => {
                if let Some(url) = &self.proxy_url {
                    write!(f, "MTProxy: {}", url)
                } else {
                    write!(f, "MTProxy: not configured")
                }
            }
        }
    }
}

/// Unified proxy configuration for all proxy types
#[derive(Debug, Clone)]
pub enum ProxyConfig {
    /// No proxy
    None,
    /// SOCKS5 proxy with URL
    Socks5(String),
    /// MTProxy configuration (only available with mtproxy feature)
    #[cfg(feature = "mtproxy")]
    MtProxy(MtProxySettings),
}

impl ProxyConfig {
    /// Validate the proxy configuration
    pub fn validate(&self) -> Result<(), String> {
        match self {
            ProxyConfig::None => Ok(()),
            ProxyConfig::Socks5(url) => {
                TelegramClient::validate_proxy_url(url, &ProxyMode::Socks5)?;
                Ok(())
            }
            #[cfg(feature = "mtproxy")]
            ProxyConfig::MtProxy(settings) => {
                if settings.host.is_none() {
                    return Err("MTProxy host is required".to_string());
                }
                if settings.secret.is_none() {
                    return Err("MTProxy secret is required".to_string());
                }
                Ok(())
            }
        }
    }

    /// Get the proxy mode for this configuration
    pub fn mode(&self) -> ProxyMode {
        match self {
            ProxyConfig::None => ProxyMode::None,
            ProxyConfig::Socks5(_) => ProxyMode::Socks5,
            #[cfg(feature = "mtproxy")]
            ProxyConfig::MtProxy(_) => ProxyMode::MtProxy,
        }
    }

    /// Get the proxy URL string for status display
    pub fn proxy_url_string(&self) -> Option<String> {
        match self {
            ProxyConfig::None => None,
            ProxyConfig::Socks5(url) => Some(url.clone()),
            #[cfg(feature = "mtproxy")]
            ProxyConfig::MtProxy(settings) => settings
                .host
                .as_ref()
                .map(|h| format!("{}:{}", h, settings.port)),
        }
    }

    /// Convert to ConnectionParams for SenderPool
    pub fn to_connection_params(&self) -> ConnectionParams {
        match self {
            ProxyConfig::None => ConnectionParams::default(),
            ProxyConfig::Socks5(url) => ConnectionParams {
                proxy_url: Some(url.clone()),
                ..Default::default()
            },
            #[cfg(feature = "mtproxy")]
            ProxyConfig::MtProxy(settings) => {
                let host_ref = settings.host.as_ref();
                let secret_ref = settings.secret.as_ref();

                // Note: validation should be called before this method
                let host = host_ref.expect("MTProxy host should be validated");
                let secret = secret_ref.expect("MTProxy secret should be validated");

                ConnectionParams {
                    mtproxy: Some(MtProxyConfig {
                        host: host.clone(),
                        port: settings.port,
                        secret: secret.clone(),
                        dc_id: settings.dc_id,
                    }),
                    ..Default::default()
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct TelegramClient {
    pub(crate) pool_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub(crate) client: Arc<Mutex<Option<Client>>>,
    dispatcher_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    broadcast_tx: Arc<Mutex<Option<tokio::sync::broadcast::Sender<Arc<UpdatesLike>>>>>,
    login_token: Arc<Mutex<Option<LoginToken>>>,
    password_token: Arc<Mutex<Option<PasswordToken>>>,
    pub(crate) api_id: Arc<Mutex<Option<u32>>>,
    api_hash: Arc<Mutex<Option<String>>>,
    phone_number: Arc<Mutex<Option<String>>>,
    session_path: Arc<Mutex<Option<PathBuf>>>,
    /// Current proxy status
    proxy_status: Arc<Mutex<ProxyStatus>>,
    /// Serializes /limits request-response pairs so only one active at a time
    pub(crate) limits_mutex: Arc<tokio::sync::Mutex<()>>,
    /// Confirmed speaker voice id shared across clones.
    /// When the requested speaker matches this cache the /speaker round
    /// trip is skipped. Updated only after a positively parsed success
    /// response or get_current_voice sync; invalidated on (re)connect.
    pub(crate) confirmed_speaker: Arc<tokio::sync::Mutex<Option<String>>>,
    /// Serializes check / possible /speaker request / cache-update so
    /// concurrent set_speaker and get_current_voice calls are atomic.
    pub(crate) speaker_serializer: Arc<tokio::sync::Mutex<()>>,
}

impl fmt::Debug for TelegramClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let session_str = self
            .session_path
            .try_lock()
            .ok()
            .and_then(|g| g.clone())
            .map(|p| secret_log::safe_path_for_log(&p))
            .unwrap_or_else(|| "[locked]".to_string());
        f.debug_struct("TelegramClient")
            .field("session_path", &session_str)
            .finish()
    }
}

impl TelegramClient {
    pub fn new() -> Self {
        Self {
            pool_task: Arc::new(Mutex::new(None)),
            client: Arc::new(Mutex::new(None)),
            dispatcher_handle: Arc::new(Mutex::new(None)),
            broadcast_tx: Arc::new(Mutex::new(None)),
            login_token: Arc::new(Mutex::new(None)),
            password_token: Arc::new(Mutex::new(None)),
            api_id: Arc::new(Mutex::new(None)),
            api_hash: Arc::new(Mutex::new(None)),
            phone_number: Arc::new(Mutex::new(None)),
            session_path: Arc::new(Mutex::new(None)),
            proxy_status: Arc::new(Mutex::new(ProxyStatus::default())),
            limits_mutex: Arc::new(tokio::sync::Mutex::new(())),
            confirmed_speaker: Arc::new(tokio::sync::Mutex::new(None)),
            speaker_serializer: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Get session path in %APPDATA%\ttsbard\telegram.session
    fn get_session_path() -> Result<PathBuf, String> {
        let app_dir = crate::paths::config_root()
            .map_err(|e| format!("Failed to get app directory: {}", e))?;

        // Create directory if it doesn't exist
        std::fs::create_dir_all(&app_dir)
            .map_err(|e| format!("Failed to create app directory: {}", e))?;

        Ok(app_dir.join("telegram.session"))
    }

    /// Validate and normalize proxy URL
    ///
    /// Supports:
    /// - SOCKS5: socks5://host:port or socks5://user:pass@host:port
    fn validate_proxy_url(url: &str, mode: &ProxyMode) -> Result<String, String> {
        match mode {
            ProxyMode::None => Err("Proxy mode is None but URL was provided".to_string()),
            ProxyMode::Socks5 => {
                // Validate SOCKS5 URL format: socks5://[user:pass@]host:port
                if !url.starts_with("socks5://") {
                    return Err("Invalid SOCKS5 URL: must start with socks5://".to_string());
                }

                let url_without_scheme = &url[9..]; // Remove "socks5://"

                // Split into auth part and address part
                let (_auth, address) = match url_without_scheme.find('@') {
                    Some(at_idx) => {
                        let (user_pass, addr) = url_without_scheme.split_at(at_idx);
                        (Some(user_pass), &addr[1..]) // Skip '@'
                    }
                    None => (None, url_without_scheme),
                };

                // Split address into host and port
                let (host, port_str) = address
                    .rsplit_once(':')
                    .ok_or_else(|| "Invalid SOCKS5 URL: missing port".to_string())?;

                let _port: u16 = port_str
                    .parse()
                    .map_err(|_| "Invalid SOCKS5 URL: invalid port".to_string())?;

                info!(
                    host,
                    _port,
                    has_auth = _auth.is_some(),
                    "Validated SOCKS5 proxy"
                );

                Ok(url.to_string())
            }
            ProxyMode::MtProxy => {
                // MTProxy doesn't use URL format, it uses separate host/port/secret settings
                Err("MTProxy mode should not use validate_proxy_url".to_string())
            }
        }
    }

    /// Инициализация клиента с поддержкой прокси
    pub async fn init_with_proxy(
        &self,
        api_id: u32,
        api_hash: String,
        phone: String,
        proxy_url: Option<String>,
    ) -> Result<OperationResult, String> {
        // Determine proxy mode from URL
        let (proxy_mode, proxy_url_str) = if let Some(ref url) = &proxy_url {
            let mode = ProxyMode::from_url(url);
            if mode == ProxyMode::None && !url.is_empty() {
                warn!(safe_url = %secret_log::safe_url_for_log(url), "[AUTH] Unknown proxy URL format, using direct connection");
                (ProxyMode::None, None)
            } else {
                (mode, proxy_url.clone())
            }
        } else {
            (ProxyMode::None, None)
        };

        // Validate and normalize proxy URL
        let normalized_proxy_url = if let Some(url) = &proxy_url_str {
            match Self::validate_proxy_url(url, &proxy_mode) {
                Ok(url) => Some(url),
                Err(e) => {
                    error!(error = %e, "[AUTH] Failed to validate proxy URL");
                    return Err(e);
                }
            }
        } else {
            None
        };

        // Convert to ProxyConfig
        let proxy_config = if let Some(url) = normalized_proxy_url {
            ProxyConfig::Socks5(url)
        } else {
            ProxyConfig::None
        };

        self.init_with_config_internal(
            api_id,
            Some(api_hash),
            Some(phone),
            proxy_config,
            "Initializing",
        )
        .await
    }

    /// Internal: Unified initialization with ProxyConfig
    ///
    /// This is the core initialization logic shared by all init methods.
    /// It handles session creation, SenderPool setup, and authorization check.
    async fn init_with_config_internal(
        &self,
        api_id: u32,
        api_hash: Option<String>,
        phone: Option<String>,
        proxy_config: ProxyConfig,
        log_context: &str, // "Initializing" or "Restoring"
    ) -> Result<OperationResult, String> {
        // Validate proxy configuration
        proxy_config.validate()?;

        // Update proxy status
        *self.proxy_status.lock().await = ProxyStatus {
            mode: proxy_config.mode(),
            proxy_url: proxy_config.proxy_url_string(),
        };

        // Log based on proxy type
        match &proxy_config {
            ProxyConfig::None => {
                info!("[AUTH] {} without proxy", log_context);
            }
            ProxyConfig::Socks5(url) => {
                info!(safe_url = %secret_log::safe_url_for_log(url), "[AUTH] {} with SOCKS5 proxy", log_context);
            }
            #[cfg(feature = "mtproxy")]
            ProxyConfig::MtProxy(settings) => {
                info!(
                    host = %settings.host.as_deref().unwrap_or("none"),
                    port = %settings.port,
                    dc_id = ?settings.dc_id,
                    has_phone = phone.is_some(),
                    masked_secret = %crate::secret_log::mask_secret(settings.secret.as_deref().unwrap_or("")),
                    "[AUTH] {} with MTProxy", log_context
                );
            }
        }

        let session_path = Self::get_session_path()?;

        // Сохраняем для последующего использования
        *self.api_id.lock().await = Some(api_id);
        if let Some(ref hash) = api_hash {
            *self.api_hash.lock().await = Some(hash.clone());
        }
        if let Some(ref pn) = phone {
            *self.phone_number.lock().await = Some(pn.clone());
        }
        *self.session_path.lock().await = Some(session_path.clone());

        debug!(session_path = %secret_log::safe_path_for_log(&session_path), "[AUTH] Session path");
        debug!(exists = session_path.exists(), "[AUTH] Session exists");

        // Создаём сессию (откроем существующую или создадим новую)
        let session = SqliteSession::open(&session_path)
            .await
            .map_err(|e| format!("Не удалось создать сессию: {}", e))?;
        let session = Arc::new(session);

        // Abort old dispatcher and pool task first so in-flight waiters on
        // the old update source wake instead of blocking for the full timeout.
        {
            let mut old_disp = self.dispatcher_handle.lock().await;
            if let Some(handle) = old_disp.take() {
                handle.abort();
            }
        }
        {
            let mut old_pool = self.pool_task.lock().await;
            if let Some(handle) = old_pool.take() {
                handle.abort();
            }
        }
        // Drop old broadcast sender so pending subscribers get Closed
        {
            *self.broadcast_tx.lock().await = None;
        }

        // Acquire serializer and invalidate cache under it; hold the guard
        // through transport replacement so no stale speaker write can
        // repopulate the cache after invalidation.
        let _serializer_guard = self.speaker_serializer.lock().await;
        {
            let mut cache = self.confirmed_speaker.lock().await;
            *cache = None;
        }

        // Создаём SenderPool with appropriate configuration
        let conn_params = proxy_config.to_connection_params();
        let SenderPool {
            runner,
            handle,
            updates,
        } = match &proxy_config {
            ProxyConfig::None => {
                info!("[AUTH] Creating SenderPool without proxy");
                SenderPool::new(session, api_id as i32)
            }
            ProxyConfig::Socks5(_) => {
                info!("[AUTH] Creating SenderPool with SOCKS5 configuration");
                SenderPool::with_configuration(session, api_id as i32, conn_params)
            }
            #[cfg(feature = "mtproxy")]
            ProxyConfig::MtProxy(_) => {
                info!("[AUTH] Creating SenderPool with MTProxy configuration");
                SenderPool::with_configuration(session, api_id as i32, conn_params)
            }
        };

        // Create broadcast channel for update fan-out
        let (broadcast_tx, _) = broadcast::channel(256);

        // Создаём клиент
        let client = Client::new(handle);

        // Spawn dispatcher: reads from UnboundedReceiver, fans out via broadcast
        let dispatcher_tx = broadcast_tx.clone();
        let dispatcher = tokio::spawn(async move {
            let mut rx = updates;
            loop {
                match rx.recv().await {
                    Some(update) => {
                        let _ = dispatcher_tx.send(Arc::new(update));
                    }
                    None => {
                        debug!("Update dispatcher: raw receiver closed, exiting");
                        break;
                    }
                }
            }
        });

        // Запускаем runner
        let pool_task = tokio::spawn(async move {
            runner.run().await;
        });

        *self.pool_task.lock().await = Some(pool_task);
        *self.client.lock().await = Some(client);
        *self.broadcast_tx.lock().await = Some(broadcast_tx);
        *self.dispatcher_handle.lock().await = Some(dispatcher);

        // Проверяем, авторизован ли уже пользователь (только если есть phone).
        // Клонируем клиента под коротким локом и отпускаем guard до сетевого
        // `is_authorized().await`, чтобы не держать `self.client` на всё время
        // запроса к серверу.
        if phone.is_some() {
            let current = { self.client.lock().await.clone() };
            let is_authorized = match current {
                Some(c) => match c.is_authorized().await {
                    Ok(authorized) => {
                        debug!(authorized, "[AUTH] Session restored");
                        authorized
                    }
                    Err(e) => {
                        error!(error = %e, "[AUTH] Error checking authorization");
                        false
                    }
                },
                None => false,
            };

            if is_authorized {
                // Сессия валидна и пользователь авторизован
                info!("[AUTH] User already authorized, session restored");
                return Ok(OperationResult::success("Сессия восстановлена"));
            }

            // Нужно авторизоваться заново
            info!("[AUTH] Session not authorized, need to sign in");
            Ok(OperationResult::success(
                "Клиент инициализирован, требуется авторизация",
            ))
        } else {
            Ok(OperationResult::success(
                "Клиент инициализирован с существующей сессией",
            ))
        }
    }

    /// Инициализация клиента с поддержкой MTProxy
    #[cfg(feature = "mtproxy")]
    pub async fn init_with_mtproxy(
        &self,
        api_id: u32,
        api_hash: String,
        phone: String,
        mtproxy_settings: &MtProxySettings,
    ) -> Result<OperationResult, String> {
        // Clone settings since we need to own them for ProxyConfig
        let settings = mtproxy_settings.clone();

        self.init_with_config_internal(
            api_id,
            Some(api_hash),
            Some(phone),
            ProxyConfig::MtProxy(settings),
            "Initializing",
        )
        .await
    }

    /// Проверка авторизации
    pub async fn is_authorized(&self) -> Result<bool, String> {
        let client = {
            let guard = self.client.lock().await;
            guard
                .clone()
                .ok_or_else(|| "Клиент не инициализирован".to_string())?
        };

        tokio::time::timeout(tokio::time::Duration::from_secs(10), client.is_authorized())
            .await
            .map_err(|_| "Превышено время ожидания проверки авторизации".to_string())?
            .map_err(|e| format!("Ошибка проверки авторизации: {}", e))
    }

    /// Проверяет наличие внутреннего клиента без разбора текста ошибки.
    pub async fn is_initialized(&self) -> bool {
        self.client.lock().await.is_some()
    }

    /// Запрос кода подтверждения
    pub async fn request_code(&self) -> Result<AuthState, String> {
        let client = {
            let guard = self.client.lock().await;
            guard
                .clone()
                .ok_or_else(|| "Клиент не инициализирован".to_string())?
        };

        let api_hash = {
            let guard = self.api_hash.lock().await;
            guard
                .clone()
                .ok_or_else(|| "API hash не задан".to_string())?
        };

        let phone_number = {
            let guard = self.phone_number.lock().await;
            guard
                .clone()
                .ok_or_else(|| "Номер телефона не задан".to_string())?
        };

        let token = {
            let mut retry_delay_ms = 250;
            let mut retry_count = 0;

            loop {
                let result = tokio::time::timeout(
                    tokio::time::Duration::from_secs(15),
                    client.request_login_code(&phone_number, &api_hash),
                )
                .await;

                match result {
                    Ok(Ok(token)) => break token,
                    Ok(Err(InvocationError::Rpc(error)))
                        if error.code == 500 && error.name == "AUTH_RESTART" && retry_count < 2 =>
                    {
                        retry_count += 1;
                        warn!(
                            retry_count,
                            "Telegram requested authorization restart; retrying code request"
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay_ms))
                            .await;
                        retry_delay_ms *= 2;
                    }
                    Ok(Err(error)) => {
                        return Err(format!("Ошибка при запросе кода: {}", error));
                    }
                    Err(_) => {
                        return Err(
                            "Превышено время ожидания запроса кода. Проверьте подключение."
                                .to_string(),
                        );
                    }
                }
            }
        };

        *self.login_token.lock().await = Some(token);

        Ok(AuthState::CodeRequired)
    }

    /// Ввод кода подтверждения
    /// Uses timing normalization and jitter to prevent timing attacks
    pub async fn sign_in(&self, code: &str) -> Result<AuthState, String> {
        // Security: Don't log the actual code - prevents timing analysis via logs
        info!("[AUTH] Starting sign_in (code not logged for security)");

        let client = {
            let guard = self.client.lock().await;
            guard
                .clone()
                .ok_or_else(|| "Клиент не инициализирован".to_string())?
        };
        info!("[AUTH] Client acquired and cloned");

        // Забираем токен из Option (take() освобождает мьютекс, возвращая значение)
        let token = self
            .login_token
            .lock()
            .await
            .take()
            .ok_or_else(|| "Токен авторизации не найден. Сначала запросите код.".to_string())?;
        info!("[AUTH] Token acquired and removed from mutex");

        let start_time = std::time::Instant::now();

        // Добавляем таймаут на авторизацию (30 секунд)
        let sign_in_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(30),
            client.sign_in(&token, code),
        )
        .await;

        let elapsed = start_time.elapsed();

        let signed_in = match sign_in_result {
            Ok(result) => result,
            Err(_) => {
                error!("[AUTH] Sign in timed out after 30 seconds");
                let jitter = rand::random::<u64>() % 500;
                tokio::time::sleep(tokio::time::Duration::from_millis(500 + jitter)).await;
                let auth_check = tokio::time::timeout(
                    tokio::time::Duration::from_secs(5),
                    client.is_authorized(),
                )
                .await;
                if matches!(auth_check, Ok(Ok(true))) {
                    info!("[AUTH] Sign in recovered after timeout: already authorized");
                    return Ok(AuthState::Connected);
                }
                warn!("[AUTH] Sign in timed out and client not authorized — restart required");
                return Ok(AuthState::RestartRequired);
            }
        };

        info!("[AUTH] Sign in result: {:?}", signed_in.is_ok());
        info!("[AUTH] About to match on signed_in...");

        match signed_in {
            Ok(_) => {
                info!("[AUTH] Sign in successful (token already cleared)");
                // Normalize timing to ~1 second to prevent timing attacks
                let target_duration = std::time::Duration::from_millis(1000);
                let remaining = target_duration.saturating_sub(elapsed);
                tokio::time::sleep(remaining).await;
                Ok(AuthState::Connected)
            }
            Err(SignInError::PasswordRequired(password_token)) => {
                let jitter = rand::random::<u64>() % 500;
                tokio::time::sleep(tokio::time::Duration::from_millis(500 + jitter)).await;
                *self.password_token.lock().await = Some(password_token);
                Ok(AuthState::PasswordRequired)
            }
            Err(_e) => {
                error!("[AUTH] Sign in error (code invalid or expired)");
                let jitter = rand::random::<u64>() % 500;
                tokio::time::sleep(tokio::time::Duration::from_millis(1000 + jitter)).await;
                let auth_check = tokio::time::timeout(
                    tokio::time::Duration::from_secs(5),
                    client.is_authorized(),
                )
                .await;
                if matches!(auth_check, Ok(Ok(true))) {
                    info!("[AUTH] Sign in recovered: already authorized");
                    Ok(AuthState::Connected)
                } else {
                    warn!("[AUTH] Sign in failed and client not authorized — restart required");
                    Ok(AuthState::RestartRequired)
                }
            }
        }
    }

    /// Проверка пароля двухфакторной аутентификации
    pub async fn check_password(&self, password: &str) -> Result<AuthState, String> {
        info!("[AUTH] Starting 2FA password check");

        let client = {
            let guard = self.client.lock().await;
            guard
                .clone()
                .ok_or_else(|| "Клиент не инициализирован".to_string())?
        };

        let password_token = self
            .password_token
            .lock()
            .await
            .take()
            .ok_or_else(|| "Токен 2FA не найден".to_string())?;

        let start_time = std::time::Instant::now();

        let result = tokio::time::timeout(
            tokio::time::Duration::from_secs(30),
            client.check_password(password_token, password),
        )
        .await;

        let elapsed = start_time.elapsed();

        match result {
            Ok(Ok(_user)) => {
                info!("[AUTH] 2FA password check successful");
                let target_duration = std::time::Duration::from_millis(1000);
                let remaining = target_duration.saturating_sub(elapsed);
                tokio::time::sleep(remaining).await;
                Ok(AuthState::Connected)
            }
            Ok(Err(SignInError::InvalidPassword(new_token))) => {
                warn!("[AUTH] Invalid 2FA password");
                let jitter = rand::random::<u64>() % 500;
                tokio::time::sleep(tokio::time::Duration::from_millis(1000 + jitter)).await;
                *self.password_token.lock().await = Some(new_token);
                Err("Неверный пароль".to_string())
            }
            Ok(Err(SignInError::PasswordRequired(token))) => {
                error!("[AUTH] Unexpected PasswordRequired during check_password");
                let jitter = rand::random::<u64>() % 500;
                tokio::time::sleep(tokio::time::Duration::from_millis(500 + jitter)).await;
                *self.password_token.lock().await = Some(token);
                Err("Требуется пароль 2FA".to_string())
            }
            Ok(Err(_other)) => {
                error!("[AUTH] check_password error (network or API)");
                let jitter = rand::random::<u64>() % 500;
                tokio::time::sleep(tokio::time::Duration::from_millis(1000 + jitter)).await;
                let auth_check = tokio::time::timeout(
                    tokio::time::Duration::from_secs(5),
                    client.is_authorized(),
                )
                .await;
                if matches!(auth_check, Ok(Ok(true))) {
                    info!("[AUTH] check_password recovered: already authorized");
                    Ok(AuthState::Connected)
                } else {
                    warn!(
                        "[AUTH] check_password failed and client not authorized — restart required"
                    );
                    Ok(AuthState::RestartRequired)
                }
            }
            Err(_) => {
                error!("[AUTH] check_password timed out");
                let jitter = rand::random::<u64>() % 500;
                tokio::time::sleep(tokio::time::Duration::from_millis(500 + jitter)).await;
                let auth_check = tokio::time::timeout(
                    tokio::time::Duration::from_secs(5),
                    client.is_authorized(),
                )
                .await;
                if matches!(auth_check, Ok(Ok(true))) {
                    info!("[AUTH] check_password recovered after timeout: already authorized");
                    Ok(AuthState::Connected)
                } else {
                    warn!("[AUTH] check_password timed out and client not authorized — restart required");
                    Ok(AuthState::RestartRequired)
                }
            }
        }
    }

    /// Получение информации о пользователе
    pub async fn get_user_info(&self) -> Result<UserInfo, String> {
        debug!("[AUTH] Getting user info");

        let client = {
            let guard = self.client.lock().await;
            guard
                .clone()
                .ok_or_else(|| "Клиент не инициализирован".to_string())?
        };

        // Добавляем таймаут на получение информации (10 секунд)
        let me = tokio::time::timeout(tokio::time::Duration::from_secs(10), client.get_me())
            .await
            .map_err(|_| {
                "Превышено время ожидания при получении информации о пользователе".to_string()
            })?
            .map_err(|e| format!("Ошибка при получении информации: {}", e))?;

        debug!(username = ?me.username(), "[AUTH] User info received");

        let phone_number = {
            let guard = self.phone_number.lock().await;
            guard.as_ref().unwrap_or(&"unknown".to_string()).clone()
        };

        Ok(UserInfo {
            id: me.raw.id(),
            first_name: me.first_name().map(|s| s.to_string()),
            last_name: me.last_name().map(|s| s.to_string()),
            username: me.username().map(|s| s.to_string()),
            phone: phone_number,
        })
    }

    /// Отключение клиента
    pub async fn disconnect(&self) -> Result<OperationResult, String> {
        // Abort dispatcher first so in-flight waiters on the old update
        // source wake instead of blocking for the full timeout.
        {
            let mut old_disp = self.dispatcher_handle.lock().await;
            if let Some(handle) = old_disp.take() {
                handle.abort();
            }
        }
        // Drop broadcast sender so pending subscribers get Closed
        {
            *self.broadcast_tx.lock().await = None;
        }
        // Завершаем задачу пула
        {
            let mut pool_task = self.pool_task.lock().await;
            if let Some(task) = pool_task.take() {
                task.abort();
            }
        }

        // Acquire serializer and invalidate cache under it; hold the guard
        // through state clearing so no stale speaker write can repopulate
        // the cache after invalidation.
        let _serializer_guard = self.speaker_serializer.lock().await;
        {
            let mut cache = self.confirmed_speaker.lock().await;
            *cache = None;
        }

        // Сбрасываем состояние
        *self.client.lock().await = None;
        *self.login_token.lock().await = None;
        *self.password_token.lock().await = None;
        *self.api_id.lock().await = None;
        *self.api_hash.lock().await = None;
        *self.phone_number.lock().await = None;

        Ok(OperationResult::success("Клиент отключен"))
    }

    /// Удаление сессии (sign out)
    pub async fn sign_out(&self) -> Result<OperationResult, String> {
        // Сначала отключаем клиент
        self.disconnect().await?;

        // Удаляем файл сессии
        if let Some(session_path) = self.session_path.lock().await.as_ref() {
            if session_path.exists() {
                std::fs::remove_file(session_path)
                    .map_err(|e| format!("Не удалось удалить файл сессии: {}", e))?;
            }
        }

        Ok(OperationResult::success("Выход выполнен успешно"))
    }

    /// Инициализация с существующей сессией (без phone/api_hash)
    pub async fn init_empty(&self, api_id: u32) -> Result<OperationResult, String> {
        self.init_empty_with_proxy(api_id, None).await
    }

    /// Инициализация с существующей сессией с поддержкой прокси
    pub async fn init_empty_with_proxy(
        &self,
        api_id: u32,
        proxy_url: Option<String>,
    ) -> Result<OperationResult, String> {
        // Determine proxy mode from URL
        let (proxy_mode, proxy_url_str) = if let Some(ref url) = &proxy_url {
            let mode = ProxyMode::from_url(url);
            if mode == ProxyMode::None && !url.is_empty() {
                warn!(safe_url = %secret_log::safe_url_for_log(url), "[AUTH] Unknown proxy URL format, using direct connection");
                (ProxyMode::None, None)
            } else {
                (mode, proxy_url.clone())
            }
        } else {
            (ProxyMode::None, None)
        };

        // Validate and normalize proxy URL
        let normalized_proxy_url = if let Some(url) = &proxy_url_str {
            match Self::validate_proxy_url(url, &proxy_mode) {
                Ok(url) => Some(url),
                Err(e) => {
                    error!(error = %e, "[AUTH] Failed to validate proxy URL");
                    return Err(e);
                }
            }
        } else {
            None
        };

        // Convert to ProxyConfig
        let proxy_config = if let Some(url) = normalized_proxy_url {
            ProxyConfig::Socks5(url)
        } else {
            ProxyConfig::None
        };

        self.init_with_config_internal(api_id, None, None, proxy_config, "Restoring")
            .await
    }

    /// Инициализация с существующей сессией с поддержкой MTProxy
    #[cfg(feature = "mtproxy")]
    pub async fn init_empty_with_mtproxy(
        &self,
        api_id: u32,
        mtproxy_settings: &MtProxySettings,
    ) -> Result<OperationResult, String> {
        // Clone settings since we need to own them for ProxyConfig
        let settings = mtproxy_settings.clone();

        self.init_with_config_internal(
            api_id,
            None,
            None,
            ProxyConfig::MtProxy(settings),
            "Restoring",
        )
        .await
    }

    /// Get current proxy status
    pub async fn get_proxy_status(&self) -> ProxyStatus {
        self.proxy_status.lock().await.clone()
    }

    /// Subscribe to Telegram updates via the broadcast dispatcher.
    /// Returns an error if the update stream is not initialized.
    pub async fn subscribe_updates(
        &self,
    ) -> Result<tokio::sync::broadcast::Receiver<Arc<UpdatesLike>>, String> {
        let guard = self.broadcast_tx.lock().await;
        guard
            .as_ref()
            .map(|tx| tx.subscribe())
            .ok_or_else(|| "Update stream not initialized".to_string())
    }
}

impl Default for TelegramClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_config_validate_none() {
        assert!(ProxyConfig::None.validate().is_ok());
    }

    #[test]
    fn proxy_config_validate_socks5_valid() {
        let config = ProxyConfig::Socks5("socks5://127.0.0.1:1080".to_string());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn proxy_config_validate_socks5_no_port() {
        let config = ProxyConfig::Socks5("socks5://127.0.0.1".to_string());
        assert!(config.validate().is_err());
    }

    #[test]
    fn proxy_config_validate_socks5_bad_prefix() {
        let config = ProxyConfig::Socks5("http://127.0.0.1:1080".to_string());
        assert!(config.validate().is_err());
    }

    #[test]
    fn proxy_config_mode_none() {
        assert_eq!(ProxyConfig::None.mode(), ProxyMode::None);
    }

    #[test]
    fn proxy_config_mode_socks5() {
        let config = ProxyConfig::Socks5("socks5://127.0.0.1:1080".to_string());
        assert_eq!(config.mode(), ProxyMode::Socks5);
    }

    #[test]
    fn proxy_config_proxy_url_string_none() {
        assert_eq!(ProxyConfig::None.proxy_url_string(), None);
    }

    #[test]
    fn proxy_config_proxy_url_string_socks5() {
        let config = ProxyConfig::Socks5("socks5://127.0.0.1:1080".to_string());
        assert_eq!(
            config.proxy_url_string(),
            Some("socks5://127.0.0.1:1080".to_string())
        );
    }

    #[test]
    fn proxy_config_to_connection_params_none() {
        let params = ProxyConfig::None.to_connection_params();
        assert!(params.proxy_url.is_none());
    }

    #[test]
    fn proxy_config_to_connection_params_socks5() {
        let config = ProxyConfig::Socks5("socks5://127.0.0.1:1080".to_string());
        let params = config.to_connection_params();
        assert_eq!(
            params.proxy_url,
            Some("socks5://127.0.0.1:1080".to_string())
        );
    }

    #[test]
    fn validate_proxy_url_valid_socks5() {
        assert!(
            TelegramClient::validate_proxy_url("socks5://127.0.0.1:1080", &ProxyMode::Socks5,)
                .is_ok()
        );
    }

    #[test]
    fn validate_proxy_url_socks5_with_auth() {
        assert!(TelegramClient::validate_proxy_url(
            "socks5://user:pass@127.0.0.1:1080",
            &ProxyMode::Socks5,
        )
        .is_ok());
    }

    #[test]
    fn validate_proxy_url_socks5_missing_prefix() {
        assert!(TelegramClient::validate_proxy_url("127.0.0.1:1080", &ProxyMode::Socks5,).is_err());
    }

    #[test]
    fn validate_proxy_url_socks5_missing_port() {
        assert!(
            TelegramClient::validate_proxy_url("socks5://127.0.0.1", &ProxyMode::Socks5,).is_err()
        );
    }

    #[test]
    fn validate_proxy_url_none_mode() {
        assert!(TelegramClient::validate_proxy_url("anything", &ProxyMode::None,).is_err());
    }

    #[test]
    fn validate_proxy_url_mtproxy_mode() {
        assert!(TelegramClient::validate_proxy_url("anything", &ProxyMode::MtProxy,).is_err());
    }

    #[test]
    fn client_new_creates_with_default_state() {
        let client = TelegramClient::new();
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            assert!(client.pool_task.lock().await.is_none());
            assert!(client.client.lock().await.is_none());
            assert!(client.dispatcher_handle.lock().await.is_none());
            assert!(client.broadcast_tx.lock().await.is_none());
            assert!(client.login_token.lock().await.is_none());
            assert!(client.password_token.lock().await.is_none());
            assert!(client.api_id.lock().await.is_none());
            assert!(client.api_hash.lock().await.is_none());
            assert!(client.phone_number.lock().await.is_none());
            assert!(client.session_path.lock().await.is_none());
        });
    }

    #[test]
    fn client_default_proxy_status() {
        let client = TelegramClient::new();
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            let status = client.get_proxy_status().await;
            assert_eq!(status.mode, ProxyMode::None);
            assert!(status.proxy_url.is_none());
        });
    }

    /// AuthState-flow methods (request_code, sign_in, check_password) are NOT
    /// unit-testable in isolation because they depend on a live
    /// `grammers_client::Client` and an active `SqliteSession`.
    /// They can only be verified via integration/end-to-end tests with a real
    /// Telegram account.
    #[test]
    fn auth_state_flow_mockability_limitation() {
        // This test exists to document the limitation explicitly.
        // No compile-pass stub — the limitation is architectural.
    }

    // ── confirmed_speaker cache ────────────────────────────────────────

    #[test]
    fn confirmed_speaker_defaults_to_none() {
        let client = TelegramClient::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let cache = client.confirmed_speaker.lock().await;
            assert!(cache.is_none());
        });
    }

    #[test]
    fn confirmed_speaker_shared_across_clones() {
        let client = TelegramClient::new();
        let clone = client.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            let mut c = client.confirmed_speaker.lock().await;
            *c = Some("baya".to_string());
        });

        rt.block_on(async {
            let cached = clone.confirmed_speaker.lock().await;
            assert_eq!(cached.as_deref(), Some("baya"));
        });
    }

    #[test]
    fn confirmed_speaker_updates_visible_across_clones() {
        let client = TelegramClient::new();
        let clone = client.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            let mut c = client.confirmed_speaker.lock().await;
            *c = Some("first".to_string());
        });

        rt.block_on(async {
            let mut c = clone.confirmed_speaker.lock().await;
            assert_eq!(c.as_deref(), Some("first"));
            *c = Some("second".to_string());
        });

        rt.block_on(async {
            let cached = client.confirmed_speaker.lock().await;
            assert_eq!(cached.as_deref(), Some("second"));
        });
    }

    #[test]
    fn disconnect_invalidates_confirmed_speaker() {
        let client = TelegramClient::new();
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            {
                let mut c = client.confirmed_speaker.lock().await;
                *c = Some("stale-speaker".to_string());
            }
            let _ = client.disconnect().await;
            let cache = client.confirmed_speaker.lock().await;
            assert!(cache.is_none());
        });
    }

    #[test]
    fn disconnect_invalidation_visible_on_clone() {
        let client = TelegramClient::new();
        let clone = client.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            {
                let mut c = client.confirmed_speaker.lock().await;
                *c = Some("stale".to_string());
            }
            let _ = client.disconnect().await;
            let cache = clone.confirmed_speaker.lock().await;
            assert!(cache.is_none());
        });
    }

    // ── speaker_serializer ──────────────────────────────────────────────

    #[test]
    fn speaker_serializer_defaults_to_available() {
        let client = TelegramClient::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let guard = client.speaker_serializer.try_lock();
            assert!(guard.is_ok(), "serializer must be available by default");
            drop(guard);
        });
    }

    #[test]
    fn speaker_serializer_shared_across_clones() {
        let client = TelegramClient::new();
        let clone = client.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _g1 = client.speaker_serializer.lock().await;
            let try_g2 = clone.speaker_serializer.try_lock();
            assert!(
                try_g2.is_err(),
                "clone serializer must be held when original is locked"
            );
        });
    }
}
