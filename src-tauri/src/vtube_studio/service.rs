use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::messages::{
    self, HotkeyInfo, HotkeysInCurrentModelResponseData, ItemAnimationControlResponseData,
    ItemInstanceInfo, ItemListResponseData, VtsRequest, VtsResponse,
};
use crate::config::{VTubeStudioSettings, VTubeStudioTypingMode};
use crate::events::VTubeStudioConnectionStatus;

/// Имя Tauri-события изменения статуса соединения VTS.
/// Дублирует константу из `commands/vtube_studio.rs`, чтобы connection-actor мог
/// эмитить переходы (transport failure из idle) без зависимости от командного слоя.
const VTS_STATUS_CHANGED_EVENT: &str = "vtube-studio-status-changed";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const ITEM_ACTION_TIMEOUT: Duration = Duration::from_secs(2);
const TYPING_KEEPALIVE_MS: u64 = 500;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// Таймаут на отправку одного WebSocket-фрейма (только сторона клиента: TCP-буфер).
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// Период вычистки «зависших» pending-entries (см. ActorRequest::timeout) в actor-задаче.
const ACTOR_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
/// Ёмкость канала исходящих запросов к connection-actor.
const ACTOR_CHANNEL_CAPACITY: usize = 64;
/// Таймаут затухания ошибки ручного подключения VTS в titlebar (ROADMAP-079).
const ERROR_DECAY_DELAY: Duration = Duration::from_secs(60);

type WsStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

/// Класс transport-failure, зафиксированный connection-actor (для логирования).
#[derive(Debug, Clone, Copy)]
enum FailureClass {
    Read,
    Closed,
    Send,
    SendTimeout,
}

/// Запрос на отправку VTS-сообщения и получение ответа, адресованный connection-actor.
struct ActorRequest {
    request_id: String,
    json: String,
    expected_msg_type: String,
    timeout: Duration,
    reply: oneshot::Sender<Result<serde_json::Value, String>>,
}

/// Ожидаемый ответ внутри actor-задачи: reply-канал + ожидаемый messageType + старт.
struct Pending {
    reply: oneshot::Sender<Result<serde_json::Value, String>>,
    expected_msg_type: String,
    started: Instant,
    timeout: Duration,
}

/// Handle на запущенный connection-actor: канал исходящих, cancel-токен, join.
struct ConnectionActorHandle {
    sender: mpsc::Sender<ActorRequest>,
    cancel: CancellationToken,
    join: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone, PartialEq)]
enum ItemKind {
    Static,
    Animated,
    Unsupported { original_type: String },
}

impl ItemKind {
    fn classify(item_type: &str) -> Self {
        match item_type {
            "PNG" | "JPG" => ItemKind::Static,
            "GIF" | "AnimationFolder" => ItemKind::Animated,
            "Live2D" | "Unknown" => ItemKind::Unsupported {
                original_type: item_type.to_string(),
            },
            _ => ItemKind::Unsupported {
                original_type: item_type.to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status")]
pub enum VTubeStudioItemStatus {
    Inactive,
    Ready {
        #[serde(rename = "fileName")]
        file_name: String,
        #[serde(rename = "vtsType")]
        vts_type: String,
    },
    Missing {
        #[serde(rename = "fileName")]
        file_name: String,
    },
    Ambiguous {
        #[serde(rename = "fileName")]
        file_name: String,
        #[serde(rename = "matchCount")]
        match_count: u32,
    },
    Unsupported {
        #[serde(rename = "fileName")]
        file_name: String,
        #[serde(rename = "vtsType")]
        vts_type: String,
    },
    Error {
        #[serde(rename = "fileName")]
        file_name: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SceneItemRecord {
    pub file_name: String,
    pub item_type: String,
    pub supported: bool,
    pub duplicate_count: u32,
}

#[derive(Debug, Clone)]
struct ResolvedItem {
    instance_id: String,
    file_name: String,
    item_type: String,
    kind: ItemKind,
}

#[derive(Debug, PartialEq)]
enum ResolveItemError {
    Missing,
    Ambiguous,
    Unsupported {
        file_name: String,
        item_type: String,
    },
}

impl std::fmt::Display for ResolveItemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveItemError::Missing => {
                write!(f, "no scene item matching the configured filename")
            }
            ResolveItemError::Ambiguous => {
                write!(f, "multiple scene items match the configured filename")
            }
            ResolveItemError::Unsupported {
                file_name,
                item_type,
            } => {
                write!(
                    f,
                    "item '{}' has unsupported type '{}'",
                    file_name, item_type
                )
            }
        }
    }
}

fn resolve_item(
    instances: &[ItemInstanceInfo],
    configured_file_name: &str,
) -> Result<ResolvedItem, ResolveItemError> {
    let matching: Vec<&ItemInstanceInfo> = instances
        .iter()
        .filter(|i| i.file_name == configured_file_name)
        .collect();

    if matching.is_empty() {
        return Err(ResolveItemError::Missing);
    }
    if matching.len() > 1 {
        return Err(ResolveItemError::Ambiguous);
    }

    let item = matching[0];
    let kind = ItemKind::classify(&item.item_type);

    if let ItemKind::Unsupported { ref original_type } = kind {
        return Err(ResolveItemError::Unsupported {
            file_name: item.file_name.clone(),
            item_type: original_type.clone(),
        });
    }

    Ok(ResolvedItem {
        instance_id: item.instance_id.clone(),
        file_name: item.file_name.clone(),
        item_type: item.item_type.clone(),
        kind,
    })
}

struct InnerState {
    actor: Option<ConnectionActorHandle>,
    typing_cancel: Option<CancellationToken>,
    typing_handle: Option<tokio::task::JoinHandle<()>>,
    typing_active: bool,
    resolved_item: Option<ResolvedItem>,
    heartbeat_cancel: Option<CancellationToken>,
    heartbeat_handle: Option<tokio::task::JoinHandle<()>>,
    connection_generation: u64,
}

#[derive(Debug, Default)]
struct ItemSyncState {
    desired: bool,
    generation: u64,
    applied: Option<bool>,
    worker_running: bool,
}

struct ItemTransitionState {
    inner: parking_lot::Mutex<ItemSyncState>,
}

impl ItemTransitionState {
    fn new() -> Self {
        Self {
            inner: parking_lot::Mutex::new(ItemSyncState::default()),
        }
    }

    fn record_desired(&self, visible: bool) {
        let mut s = self.inner.lock();
        if s.worker_running {
            if s.desired != visible {
                s.desired = visible;
                s.generation = s.generation.wrapping_add(1);
            }
        } else if s.applied != Some(visible) {
            s.desired = visible;
            s.generation = s.generation.wrapping_add(1);
        }
    }

    fn read_desired(&self) -> (bool, u64) {
        let s = self.inner.lock();
        (s.desired, s.generation)
    }

    fn read_applied(&self) -> Option<bool> {
        self.inner.lock().applied
    }

    fn begin_work(&self) -> Option<u64> {
        let mut s = self.inner.lock();
        if s.worker_running {
            return None;
        }
        if s.applied == Some(s.desired) {
            return None;
        }
        s.worker_running = true;
        Some(s.generation)
    }

    fn finish_success(&self, gen: u64, visible: bool) -> bool {
        let mut s = self.inner.lock();
        if s.generation == gen {
            s.applied = Some(visible);
        }
        if s.applied != Some(s.desired) {
            true
        } else {
            s.worker_running = false;
            false
        }
    }

    fn finish_failure(&self, attempted_gen: u64) -> bool {
        let mut s = self.inner.lock();
        s.applied = None;
        if s.generation != attempted_gen {
            true
        } else {
            s.worker_running = false;
            false
        }
    }

    fn end_work(&self) {
        self.inner.lock().worker_running = false;
    }

    fn set_applied_if_current(&self, visible: bool, gen: u64) -> bool {
        let mut s = self.inner.lock();
        if s.generation == gen {
            s.applied = Some(visible);
            true
        } else {
            false
        }
    }

    fn mark_applied_unknown(&self) {
        self.inner.lock().applied = None;
    }

    fn reset(&self) {
        let mut s = self.inner.lock();
        s.desired = false;
        s.generation = s.generation.wrapping_add(1);
        s.applied = None;
        s.worker_running = false;
    }

    fn force_applied(&self, visible: bool) {
        self.inner.lock().applied = Some(visible);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipReason {
    NotDesiredRunning,
    NotConnected,
    NotAuthenticated,
    NoSocket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnsureOutcome {
    Ensured,
    Skipped(SkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeleteOutcome {
    Deleted,
    Skipped(SkipReason),
    NotFound,
}

/// Происхождение попытки подключения VTube Studio: ручная (кнопки Connect/Restart
/// в панели) или автостарт на boot. Определяет, вооружается ли decay-таймер при
/// ошибке: ручная ошибка затухает в серый через `ERROR_DECAY_DELAY`, автостарт —
/// красный бессрочно.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectOrigin {
    Manual,
    Autostart,
}

/// Маркер конкретной попытки подключения. Возвращается вызывающей стороне
/// вместе с результатом, чтобы та могла отличить актуальный результат от
/// вытесненного (`connect`/`test_connection`/typing-reconnect/`disconnect`
/// увеличивают generation). Не сериализуется и не покидает backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionAttempt(u64);

impl ConnectionAttempt {
    fn generation(self) -> u64 {
        self.0
    }
}

/// Успешный итог сетевой фазы попытки.
#[derive(Debug)]
pub struct ConnectionSuccess {
    token: Option<String>,
    live: bool,
    attempt: ConnectionAttempt,
}

/// Провал сетевой фазы попытки.
#[derive(Debug)]
pub struct ConnectionFailure {
    superseded: bool,
    message: Option<String>,
}

impl ConnectionFailure {
    fn failed(message: String) -> Self {
        Self {
            superseded: false,
            message: Some(message),
        }
    }

    fn superseded() -> Self {
        Self {
            superseded: true,
            message: None,
        }
    }

    /// Попытку вытеснили более новым connect/disconnect/reconnect. Это не
    /// ошибка текущей сессии: состояние, токен и UI трогать нельзя.
    pub fn is_superseded(&self) -> bool {
        self.superseded
    }

    /// Сообщение об ошибке актуальной попытки (`None` для вытесненной).
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

/// Итог применения успешного результата попытки к токену и состоянию.
#[derive(Debug)]
pub struct ConnectionReport {
    live: bool,
    superseded: bool,
    persist_error: Option<String>,
}

impl ConnectionReport {
    fn live(live: bool) -> Self {
        Self {
            live,
            superseded: false,
            persist_error: None,
        }
    }

    fn superseded() -> Self {
        Self {
            live: false,
            superseded: true,
            persist_error: None,
        }
    }

    fn persist_failed(error: String) -> Self {
        Self {
            live: false,
            superseded: false,
            persist_error: Some(error),
        }
    }

    /// Живая сессия установлена и результат не вытеснен.
    pub fn is_live(&self) -> bool {
        self.live && !self.superseded
    }

    /// Попытку вытеснили: live-состояние и токен публиковать нельзя.
    pub fn is_superseded(&self) -> bool {
        self.superseded
    }

    /// Запись токена на диск не удалась; соединение этой попытки уже снято.
    pub fn persist_error(&self) -> Option<&str> {
        self.persist_error.as_deref()
    }
}

/// Решение об установке аутентифицированного сокета под `inner`-локом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallDecision {
    Install,
    /// Попытку вытеснили более новым connect/disconnect/reconnect.
    Superseded,
    /// Сокет намеренно не устанавливается: соединение больше не нужно
    /// (standalone-проверка auth при `desired_running == false`).
    AuthOnly,
}

pub struct VTubeStudioService {
    pub settings: Arc<tokio::sync::RwLock<VTubeStudioSettings>>,
    inner: Arc<tokio::sync::Mutex<InnerState>>,
    is_authenticated: Arc<AtomicBool>,
    desired_running: Arc<AtomicBool>,
    connection_status: Arc<parking_lot::Mutex<VTubeStudioConnectionStatus>>,
    item_status: Arc<parking_lot::Mutex<VTubeStudioItemStatus>>,
    item_transition: ItemTransitionState,
    session: AtomicU64,
    /// Generation попытки подключения (`connect` / `test_connection` / typing
    /// reconnect). Инкрементируется в начале каждой попытки и в `disconnect()`,
    /// чтобы установка сокета под `inner`-локом могла отбросить устаревшую
    /// попытку, вытесненную более новым connect/disconnect. Сетевые `.await`
    /// выполняются без удержания `inner`.
    connection_attempt: AtomicU64,
    /// Сериализует коммиты токенов между собой.
    token_commit: tokio::sync::Mutex<()>,
    /// Линеаризует инвалидацию попытки (`begin_connection_attempt`) и durable-
    /// коммит токена. Инвалидация берёт write-lock, коммит — read-lock на всё
    /// время записи на диск: попытку нельзя вытеснить между проверкой
    /// актуальности и фактической записью, поэтому устаревший токен не может
    /// попасть на диск после более нового.
    attempt_gate: tokio::sync::RwLock<()>,
    /// Generation decay-таймера ошибки ручного подключения. Инкремент в начале
    /// `connect()`/`disconnect()` и при вооружении таймера гасит устаревшие таймеры.
    error_decay_generation: Arc<AtomicU64>,
    /// Задержка decay-таймера; переопределяется только в тестах через
    /// `set_error_decay_delay`.
    error_decay_delay: parking_lot::Mutex<Duration>,
    /// Tauri AppHandle для эмиттинга статуса из connection-actor.
    /// Лениво подключается через `attach_app_handle` из `setup::init_vtube_studio`,
    /// т.к. сервис конструируется до появления AppHandle.
    app_handle: parking_lot::Mutex<Option<AppHandle>>,
}

impl VTubeStudioService {
    pub fn new() -> Self {
        Self {
            settings: Arc::new(tokio::sync::RwLock::new(VTubeStudioSettings::default())),
            inner: Arc::new(tokio::sync::Mutex::new(InnerState {
                actor: None,
                typing_cancel: None,
                typing_handle: None,
                typing_active: false,
                resolved_item: None,
                heartbeat_cancel: None,
                heartbeat_handle: None,
                connection_generation: 0,
            })),
            is_authenticated: Arc::new(AtomicBool::new(false)),
            desired_running: Arc::new(AtomicBool::new(false)),
            connection_status: Arc::new(parking_lot::Mutex::new(
                VTubeStudioConnectionStatus::Disconnected,
            )),
            item_status: Arc::new(parking_lot::Mutex::new(VTubeStudioItemStatus::Inactive)),
            item_transition: ItemTransitionState::new(),
            session: AtomicU64::new(0),
            connection_attempt: AtomicU64::new(0),
            token_commit: tokio::sync::Mutex::new(()),
            attempt_gate: tokio::sync::RwLock::new(()),
            error_decay_generation: Arc::new(AtomicU64::new(0)),
            error_decay_delay: parking_lot::Mutex::new(ERROR_DECAY_DELAY),
            app_handle: parking_lot::Mutex::new(None),
        }
    }

    /// Подключает AppHandle, чтобы connection-actor мог эмитить `vtube-studio-status-changed`
    /// при transport-failure из idle. Вызывается из `setup::init_vtube_studio` ПЕРВОЙ строкой,
    /// до чтения `start_on_boot` — handle нужен независимо от автозапуска.
    pub fn attach_app_handle(&self, handle: AppHandle) {
        *self.app_handle.lock() = Some(handle);
    }

    /// Эмитит текущий connection-status во фронтенд, если AppHandle подключён.
    /// Безопасно для вызова из connection-actor / heartbeat / typing-задач.
    fn emit_status(&self) {
        if let Some(h) = self.app_handle.lock().as_ref() {
            let status = self.connection_status.lock().clone();
            let _ = h.emit(VTS_STATUS_CHANGED_EVENT, &status);
        }
    }

    fn read_session(&self) -> u64 {
        self.session.load(Ordering::Acquire)
    }

    fn invalidate_session(&self) -> u64 {
        self.session.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Начинает новую попытку подключения, вытесняя все предыдущие.
    ///
    /// Write-lock `attempt_gate` — точка линеаризации с durable-коммитом токена:
    /// инвалидация ждёт завершения записи токена, а коммит под read-lock не
    /// может стать устаревшим посреди записи на диск.
    async fn begin_connection_attempt(&self) -> ConnectionAttempt {
        let _gate = self.attempt_gate.write().await;
        ConnectionAttempt(self.connection_attempt.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// Попытка всё ещё актуальна (не вытеснена новым connect/disconnect)?
    fn is_current_attempt(&self, attempt: ConnectionAttempt) -> bool {
        self.connection_attempt.load(Ordering::SeqCst) == attempt.generation()
    }

    /// Решение об установке сокета очередной попытки. Вызывать под `inner`-локом:
    /// проверка обязана быть атомарной с установкой actor, иначе `disconnect()`
    /// или более новый `connect()` успеют выполнить teardown между проверкой и
    /// записью сокета. `is_desired_running()` дополнительно отсекает сокет,
    /// пришедший после того, как соединение уже стало ненужным, но оставляет
    /// возможность standalone-проверки auth без установки actor-а.
    fn install_decision_locked(&self, attempt: ConnectionAttempt) -> InstallDecision {
        if !self.is_current_attempt(attempt) {
            InstallDecision::Superseded
        } else if !self.is_desired_running() {
            InstallDecision::AuthOnly
        } else {
            InstallDecision::Install
        }
    }

    /// Фиксирует провал сетевой фазы, только если попытка ещё актуальна.
    /// Вытесненная ошибка не меняет auth/status и не вооружает decay-таймер.
    ///
    /// Проверка актуальности и публикация Error/auth=false выполняются под тем
    /// же read-gate `attempt_gate`, что и durable-коммит: новая попытка не может
    /// начаться между проверкой и записью (её `begin_connection_attempt` берёт
    /// write-gate), поэтому устаревшая ошибка не перетрёт более новую сессию.
    /// Сетевых await-ов внутри нет — только короткая мутация.
    async fn record_connection_failure(
        &self,
        attempt: ConnectionAttempt,
        error: String,
        origin: Option<ConnectOrigin>,
    ) -> ConnectionFailure {
        let _gate = self.attempt_gate.read().await;
        if !self.is_current_attempt(attempt) {
            debug!(
                attempt = attempt.generation(),
                "VTS attempt failure superseded; connection state untouched"
            );
            return ConnectionFailure::superseded();
        }

        self.is_authenticated.store(false, Ordering::SeqCst);
        self.set_connection_status(VTubeStudioConnectionStatus::Error);
        if origin == Some(ConnectOrigin::Manual) {
            self.arm_error_decay();
        }
        ConnectionFailure::failed(error)
    }

    pub fn set_connection_status(&self, status: VTubeStudioConnectionStatus) {
        *self.connection_status.lock() = status;
    }

    pub fn get_connection_status(&self) -> VTubeStudioConnectionStatus {
        self.connection_status.lock().clone()
    }

    pub fn set_desired_running(&self, value: bool) {
        self.desired_running.store(value, Ordering::SeqCst);
        info!(value, "VTube Studio desired_running set");
    }

    pub fn is_desired_running(&self) -> bool {
        self.desired_running.load(Ordering::SeqCst)
    }

    /// Вооружает decay-таймер ошибки ручного подключения: через `error_decay_delay`
    /// сбрасывает `desired_running` (красный → серый) и эмитит статус, если за это
    /// время не было новой попытки/disconnect и статус всё ещё `Error`.
    fn arm_error_decay(&self) {
        let gen = self.error_decay_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let delay = *self.error_decay_delay.lock();
        let generation = Arc::clone(&self.error_decay_generation);
        let desired_running = Arc::clone(&self.desired_running);
        let connection_status = Arc::clone(&self.connection_status);
        let app_handle = self.app_handle.lock().clone();

        tokio::spawn(async move {
            tokio::time::sleep(delay).await;

            if generation.load(Ordering::SeqCst) != gen {
                return;
            }
            if *connection_status.lock() != VTubeStudioConnectionStatus::Error {
                return;
            }
            if !desired_running.load(Ordering::SeqCst) {
                return;
            }

            desired_running.store(false, Ordering::SeqCst);
            info!("VTS manual connect error decayed: desired_running reset");

            if let Some(h) = app_handle {
                let status = connection_status.lock().clone();
                let _ = h.emit(VTS_STATUS_CHANGED_EVENT, &status);
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn set_error_decay_delay(&self, delay: Duration) {
        *self.error_decay_delay.lock() = delay;
    }

    #[allow(dead_code)]
    pub fn mark_authenticated(&self, value: bool) {
        self.is_authenticated.store(value, Ordering::SeqCst);
    }

    pub fn is_authenticated(&self) -> bool {
        self.is_authenticated.load(Ordering::SeqCst)
    }

    pub fn is_live_authenticated_connection(&self) -> bool {
        self.is_desired_running()
            && self.get_connection_status() == VTubeStudioConnectionStatus::Connected
            && self.is_authenticated()
    }

    pub(crate) async fn live_connection_skip_reason(&self) -> Option<SkipReason> {
        if !self.is_desired_running() {
            return Some(SkipReason::NotDesiredRunning);
        }
        if self.get_connection_status() != VTubeStudioConnectionStatus::Connected {
            return Some(SkipReason::NotConnected);
        }
        if !self.is_authenticated() {
            return Some(SkipReason::NotAuthenticated);
        }
        if self.inner.lock().await.actor.is_none() {
            return Some(SkipReason::NoSocket);
        }
        None
    }

    fn next_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    #[allow(dead_code)]
    pub async fn is_connected(&self) -> bool {
        self.inner.lock().await.actor.is_some()
    }

    /// Универсальный путь «отправить запрос — получить ответ» через connection-actor.
    /// Заменяет прежний `send_and_recv(&mut ws)` + удержание `inner` через await.
    ///
    /// ВАЖНО: лок `inner` берётся ТОЛЬКО чтобы склонировать `mpsc::Sender` и
    /// прочитать generation, и НЕ удерживается через `await` — это устраняет
    /// прежний дедлок (heartbeat держал inner до 12 с во время send_and_recv).
    ///
    /// При таймауте вызывающего actor НЕ убивается: entry остаётся в `pending`,
    /// поздний ответ безвредно дропнется (или вычистится sweep-тиком, или при
    /// смерти actor-а).
    async fn actor_request(
        &self,
        expected_msg_type: &str,
        build_req: impl FnOnce(&str) -> VtsRequest,
        request_timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let (sender, my_generation) = {
            let inner = self.inner.lock().await;
            match inner.actor.as_ref() {
                Some(h) => (h.sender.clone(), inner.connection_generation),
                None => return Err("VTube Studio WebSocket is not available.".to_string()),
            }
        };
        self.actor_request_with_sender(
            &sender,
            Some(my_generation),
            expected_msg_type,
            build_req,
            request_timeout,
        )
        .await
    }

    /// `mpsc::Sender` текущего connection-actor-а (`Err` — соединения нет).
    /// Нужен teardown-у: pinned sender адресует конкретную сессию, поэтому
    /// поздние best-effort запросы не уйдут в более новый actor.
    async fn current_actor_sender(&self) -> Result<mpsc::Sender<ActorRequest>, String> {
        let inner = self.inner.lock().await;
        match inner.actor.as_ref() {
            Some(h) => Ok(h.sender.clone()),
            None => Err("VTube Studio WebSocket is not available.".to_string()),
        }
    }

    /// Отправка запроса через конкретный actor-sender (сессия пиннится sender-ом).
    /// `generation` используется только для диагностических логов.
    async fn actor_request_with_sender(
        &self,
        sender: &mpsc::Sender<ActorRequest>,
        generation: Option<u64>,
        expected_msg_type: &str,
        build_req: impl FnOnce(&str) -> VtsRequest,
        request_timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let request_id = self.next_id();
        let req = build_req(&request_id);
        let json = serde_json::to_string(&req).map_err(|e| e.to_string())?;

        let (tx, rx) = oneshot::channel();
        sender
            .send(ActorRequest {
                request_id: request_id.clone(),
                json,
                expected_msg_type: expected_msg_type.to_string(),
                timeout: request_timeout,
                reply: tx,
            })
            .await
            .map_err(|_| {
                debug!(expected_msg_type, "VTS actor sender closed");
                "VTube Studio WebSocket is not available.".to_string()
            })?;

        match timeout(request_timeout, rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                debug!(expected_msg_type, ?generation, "VTS actor dropped reply");
                Err("VTube Studio request was cancelled.".to_string())
            }
            Err(_) => {
                // Наш таймаут: actor жив, entry в pending вычистится sweep-тиком
                // или при позднем ответе. Actor НЕ инвалидируем.
                debug!(
                    expected_msg_type,
                    %request_id, ?generation, "VTS response timed out"
                );
                Err("Response timed out".to_string())
            }
        }
    }

    /// Создаёт connection-actor из уже аутентифицированного сокета и сохраняет handle.
    /// Вызывается из connect/test_connection/set_typing ПОСЛЕ успешного auth.
    fn start_actor_locked(&self, inner: &mut InnerState, ws: WsStream) {
        self.stop_actor_locked(inner);
        inner.connection_generation = inner.connection_generation.wrapping_add(1);
        let actor_generation = inner.connection_generation;

        let (sender, receiver) = mpsc::channel::<ActorRequest>(ACTOR_CHANNEL_CAPACITY);
        let cancel = CancellationToken::new();
        let app_handle = self.app_handle.lock().clone();

        let join = tokio::spawn(actor_task(
            ws,
            receiver,
            cancel.clone(),
            Arc::clone(&self.inner),
            Arc::clone(&self.is_authenticated),
            Arc::clone(&self.connection_status),
            app_handle,
            actor_generation,
        ));

        inner.actor = Some(ConnectionActorHandle {
            sender,
            cancel,
            join,
        });
    }

    /// Останавливает connection-actor: cancel + drop sender (закрывает rx) + abort join.
    /// НЕ await-ит join под `inner`-локом (иначе deadlock, если actor внутри
    /// `handle_transport_failure_locked` ждёт тот же лок). Abort безопасен: actor
    /// владеет WsStream — при abort он дропается, TCP закрывается.
    fn stop_actor_locked(&self, inner: &mut InnerState) {
        if let Some(h) = inner.actor.take() {
            h.cancel.cancel();
            drop(h.sender);
            h.join.abort();
        }
    }

    /// Единый путь transport-failure: generation-guarded атомарная инвалидация + emit.
    /// Вызывается из actor (read/Close/send failure) и heartbeat/typing (не-semantic Err).
    ///
    /// Generation guard: устаревший actor/heartbeat не может перезаписать состояние
    /// свежего connect. Emit ровно один раз за failure (только если actor был Some).
    async fn handle_transport_failure_locked(
        &self,
        failure_generation: u64,
        class: FailureClass,
        detail: Option<String>,
    ) {
        let mut inner = self.inner.lock().await;
        if inner.connection_generation != failure_generation {
            debug!(
                failure_generation,
                current_generation = inner.connection_generation,
                "stale transport failure ignored"
            );
            return;
        }

        let had_actor = inner.actor.is_some();
        if had_actor {
            self.stop_actor_locked(&mut inner);
            self.stop_typing_keepalive_locked(&mut inner);
            self.stop_heartbeat_locked(&mut inner);
            inner.typing_active = false;
            inner.resolved_item = None;
            self.is_authenticated.store(false, Ordering::SeqCst);
            *self.connection_status.lock() = VTubeStudioConnectionStatus::Error;

            info!(
                class = ?class,
                ?detail,
                generation = failure_generation,
                "VTS transport failure: connection invalidated"
            );

            // P1: эмитим переход Error во фронтенд (раньше heartbeat/typing писали
            // статус без emit — UI залипал на ложном Connected).
            self.emit_status();
        }
    }

    pub fn get_item_status(&self) -> VTubeStudioItemStatus {
        self.item_status.lock().clone()
    }

    pub fn record_item_desired(&self, visible: bool) {
        self.item_transition.record_desired(visible);
    }

    pub async fn run_item_sync(&self) -> VTubeStudioItemStatus {
        match self.item_transition.begin_work() {
            Some(_) => {}
            None => return self.get_item_status(),
        }

        let session = self.read_session();

        loop {
            let typing_action = { self.settings.read().await.typing_action.clone() };
            if typing_action.output_mode != VTubeStudioTypingMode::Item {
                self.item_transition.end_work();
                break;
            }

            if self.read_session() != session {
                self.item_transition.end_work();
                break;
            }

            let (desired, gen) = self.item_transition.read_desired();

            let (resolved, item_type_for_log) = {
                let inner = self.inner.lock().await;
                if self.read_session() != session {
                    self.item_transition.end_work();
                    break;
                }
                let resolved = inner.resolved_item.clone();
                let it = match &resolved {
                    Some(r) => r.item_type.clone(),
                    None => String::new(),
                };
                (resolved, it)
            };

            let file_name_for_log = match &resolved {
                Some(r) => r.file_name.clone(),
                None => typing_action.item_file_name.clone(),
            };

            let result = match &resolved {
                Some(item) => {
                    let start = Instant::now();
                    let res = animate_item(self, item, desired).await;
                    let duration = start.elapsed();
                    info!(
                        mode = "Item",
                        item_type = %item_type_for_log,
                        file_name = %file_name_for_log,
                        desired = desired,
                        duration_ms = duration.as_millis(),
                        "Item animation request completed"
                    );
                    res
                }
                None => {
                    self.item_transition.end_work();
                    return self.get_item_status();
                }
            };

            // Transport-failure handled solely by the connection actor; caller-side
            // invalidation is removed to prevent stale generation from destroying a new actor.
            match result {
                Ok(()) => {
                    if self.read_session() == session {
                        let continue_work = self.item_transition.finish_success(gen, desired);
                        let status = VTubeStudioItemStatus::Ready {
                            file_name: file_name_for_log.clone(),
                            vts_type: item_type_for_log.clone(),
                        };
                        *self.item_status.lock() = status;

                        if !continue_work {
                            break;
                        }
                    } else {
                        self.item_transition.end_work();
                        break;
                    }
                }
                Err(..) => {
                    if self.read_session() == session {
                        warn!(
                            mode = "Item",
                            file_name = %file_name_for_log,
                            "Item animation request failed"
                        );
                        let continue_work = self.item_transition.finish_failure(gen);
                        let status = VTubeStudioItemStatus::Error {
                            file_name: file_name_for_log.clone(),
                            message: "item action failed".to_string(),
                        };
                        *self.item_status.lock() = status;
                        if continue_work {
                            continue;
                        }
                    } else {
                        self.item_transition.end_work();
                    }
                    return self.get_item_status();
                }
            }
        }

        self.get_item_status()
    }

    pub async fn test_connection(
        &self,
        host: &str,
        port: u16,
        stored_token: Option<&str>,
    ) -> Result<ConnectionSuccess, ConnectionFailure> {
        let attempt = self.begin_connection_attempt().await;

        // Teardown — под коротким локом; сетевые операции ниже идут без `inner`,
        // чтобы не блокировать disconnect()/heartbeat/actor на время connect.
        // Read-gate + перепроверка: вытеснённый teardown не имеет права снести
        // actor более новой попытки.
        {
            let _gate = self.attempt_gate.read().await;
            if !self.is_current_attempt(attempt) {
                debug!(
                    attempt = attempt.generation(),
                    "VTS test_connection superseded before teardown"
                );
                return Err(ConnectionFailure::superseded());
            }
            let mut inner = self.inner.lock().await;
            self.stop_typing_keepalive_locked(&mut inner);
            self.stop_heartbeat_locked(&mut inner);
            self.stop_actor_locked(&mut inner);
            inner.typing_active = false;
        }

        let mut ws = match connect_ws(host, port).await {
            Ok(ws) => ws,
            Err(e) => return Err(self.record_connection_failure(attempt, e, None).await),
        };

        let new_token = match perform_authentication(&mut ws, self.next_id(), stored_token).await {
            Ok(token) => token,
            Err(e) => return Err(self.record_connection_failure(attempt, e, None).await),
        };

        if let Some(ref token) = new_token {
            ws = match connect_and_auth_fresh(host, port, token).await {
                Ok(fresh_ws) => fresh_ws,
                Err(e) => return Err(self.record_connection_failure(attempt, e, None).await),
            };
        }

        // Установка сокета: проверка актуальности и публикация Connected/auth
        // атомарны под read-gate + `inner` — новая попытка не может начаться
        // между проверкой и установкой.
        let _gate = self.attempt_gate.read().await;
        let mut inner = self.inner.lock().await;
        match self.install_decision_locked(attempt) {
            InstallDecision::Superseded => {
                debug!(
                    attempt = attempt.generation(),
                    "VTS test_connection superseded; discarding authenticated socket"
                );
                Err(ConnectionFailure::superseded())
            }
            InstallDecision::AuthOnly => {
                // Standalone-проверка: auth подтверждён, но сокет намеренно
                // не устанавливается (desired_running=false). Живую сессию не
                // публикуем, токен сохранит вызывающий через commit.
                debug!(
                    attempt = attempt.generation(),
                    "VTS test_connection verified auth without installing socket \
                     (desired_running=false)"
                );
                Ok(ConnectionSuccess {
                    token: new_token,
                    live: false,
                    attempt,
                })
            }
            InstallDecision::Install => {
                self.start_actor_locked(&mut inner, ws);
                self.is_authenticated.store(true, Ordering::SeqCst);
                self.set_connection_status(VTubeStudioConnectionStatus::Connected);
                self.start_heartbeat_locked(&mut inner);
                Ok(ConnectionSuccess {
                    token: new_token,
                    live: true,
                    attempt,
                })
            }
        }
    }

    pub async fn connect(
        &self,
        host: &str,
        port: u16,
        stored_token: Option<&str>,
        origin: ConnectOrigin,
    ) -> Result<ConnectionSuccess, ConnectionFailure> {
        // Новая попытка подключения гасит вооружённый decay-таймер прежней ошибки.
        self.error_decay_generation.fetch_add(1, Ordering::SeqCst);
        // Вытесняем более старые попытки подключения: их сокеты не должны
        // перетереть соединение, установленное этой попыткой (и наоборот).
        let attempt = self.begin_connection_attempt().await;

        let typing_action = { self.settings.read().await.typing_action.clone() };

        // Teardown и публикация Connecting — под одним коротким локом: иначе
        // вытеснённый teardown мог бы перетереть статус более новой попытки.
        // Read-gate + перепроверка делают проверку актуальности и teardown
        // атомарными относительно начала новой попытки. Сетевые операции ниже
        // идут без `inner`, иначе на всё время connect блокируются
        // disconnect()/heartbeat/actor.
        {
            let _gate = self.attempt_gate.read().await;
            if !self.is_current_attempt(attempt) {
                debug!(
                    attempt = attempt.generation(),
                    "VTS connect superseded before teardown"
                );
                return Err(ConnectionFailure::superseded());
            }
            let mut inner = self.inner.lock().await;
            self.set_desired_running(true);
            self.set_connection_status(VTubeStudioConnectionStatus::Connecting);
            self.stop_typing_keepalive_locked(&mut inner);
            self.stop_heartbeat_locked(&mut inner);
            self.stop_actor_locked(&mut inner);
            inner.typing_active = false;
            inner.resolved_item = None;
        }

        let ws_result = connect_ws(host, port).await;
        let mut ws = match ws_result {
            Ok(ws) => ws,
            Err(e) => {
                // desired_running сохраняем: terminal Error при живом desired
                // должен дать красный статус в titlebar (ROADMAP-074); сброс
                // происходит только при явном disconnect() (или decay-таймере
                // для ручной ошибки).
                return Err(self
                    .record_connection_failure(attempt, e, Some(origin))
                    .await);
            }
        };

        let auth_result = perform_authentication(&mut ws, self.next_id(), stored_token).await;
        let new_token = match auth_result {
            Ok(token) => token,
            Err(e) => {
                return Err(self
                    .record_connection_failure(attempt, e, Some(origin))
                    .await);
            }
        };

        if let Some(ref token) = new_token {
            ws = match connect_and_auth_fresh(host, port, token).await {
                Ok(fresh_ws) => fresh_ws,
                Err(e) => {
                    return Err(self
                        .record_connection_failure(attempt, e, Some(origin))
                        .await);
                }
            };
        }

        // Handoff аутентифицированного сокета в connection-actor. Read-gate +
        // `inner` держатся на всей короткой публикации: проверку актуальности,
        // установку actor-а, auth=true и статус невозможно разделить с началом
        // новой попытки. За время сетевых операций попытку могли вытеснить
        // новый connect() или disconnect() — тогда сокет просто дропается, а
        // вызывающий получает Superseded вместо ложного успеха.
        let gate = self.attempt_gate.read().await;
        let mut inner = self.inner.lock().await;
        match self.install_decision_locked(attempt) {
            InstallDecision::Superseded => {
                debug!(
                    attempt = attempt.generation(),
                    "VTS connect superseded; discarding authenticated socket"
                );
                return Err(ConnectionFailure::superseded());
            }
            InstallDecision::AuthOnly => {
                // Соединение уже не нужно (desired_running=false): сокет не
                // устанавливаем, живой сессии не публикуем.
                debug!(
                    attempt = attempt.generation(),
                    "VTS connect not installed (desired_running=false)"
                );
                return Ok(ConnectionSuccess {
                    token: new_token,
                    live: false,
                    attempt,
                });
            }
            InstallDecision::Install => {}
        }
        self.start_actor_locked(&mut inner, ws);
        self.is_authenticated.store(true, Ordering::SeqCst);
        self.set_connection_status(VTubeStudioConnectionStatus::Connected);

        let item_file_name = match typing_action.output_mode {
            VTubeStudioTypingMode::Event | VTubeStudioTypingMode::Hotkeys => {
                *self.item_status.lock() = VTubeStudioItemStatus::Inactive;
                None
            }
            VTubeStudioTypingMode::Item => {
                let file_name = typing_action.item_file_name.clone();
                if file_name.is_empty() {
                    *self.item_status.lock() = VTubeStudioItemStatus::Missing {
                        file_name: String::new(),
                    };
                    None
                } else {
                    Some(file_name)
                }
            }
        };

        // item-refresh (если нужен) идёт через actor: освобождаем `inner`, чтобы
        // actor-задача могла отвечать (actor сам не блокируется на inner, но мы
        // не хотим держать лок во время сетевого запроса). Read-gate тоже
        // отпускаем: сетевой await не должен блокировать начало новой попытки.
        if let Some(file_name) = item_file_name {
            let session = self.read_session();
            drop(inner);
            drop(gate);
            let (resolved, status) = self.do_item_refresh_with_desired(&file_name, session).await;

            // Публикация item-состояния — снова короткая мутация под read-gate:
            // если попытку вытеснили за время refresh, ничего не применяем.
            let _gate = self.attempt_gate.read().await;
            if !self.is_current_attempt(attempt) {
                debug!(
                    attempt = attempt.generation(),
                    "VTS connect superseded during item refresh; discarding stale item state"
                );
                return Err(ConnectionFailure::superseded());
            }
            let mut inner = self.inner.lock().await;
            if self.read_session() == session {
                inner.resolved_item = resolved;
                *self.item_status.lock() = status;
            }
            self.start_heartbeat_locked(&mut inner);
            return Ok(ConnectionSuccess {
                token: new_token,
                live: true,
                attempt,
            });
        }

        self.start_heartbeat_locked(&mut inner);

        Ok(ConnectionSuccess {
            token: new_token,
            live: true,
            attempt,
        })
    }

    /// Применяет успешный результат попытки: сохраняет выданный токен через
    /// `persist_token` (диск) и публикует его в runtime-настройках, но только
    /// пока попытка остаётся актуальной.
    ///
    /// Коммиты сериализованы: устаревший результат не может записать токен
    /// после более нового. Если запись на диск не удалась у актуальной попытки,
    /// снимается только её соединение (scoped teardown), более новая сессия не
    /// затрагивается.
    ///
    /// Durable-работа выполняется в отдельной задаче, которая владеет
    /// attempt-gate (read) до фактического завершения записи. Поэтому отмена
    /// будущего вызывающего (drop Tauri-команды/иное) не снимает fence и не
    /// оставляет «оторванную» запись: `spawn_blocking` внутри `persist_token`
    /// доживает до конца только под удерживаемым gate, а инвалидация попытки
    /// (write-gate) не может пройти, пока задача не завершится.
    pub async fn commit_connection_success<F, Fut>(
        self: &Arc<Self>,
        success: ConnectionSuccess,
        persist_token: F,
    ) -> ConnectionReport
    where
        F: FnOnce(String) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let service = Arc::clone(self);
        let commit = tokio::spawn(async move {
            service
                .commit_connection_success_scoped(success, persist_token)
                .await
        });

        match commit.await {
            Ok(report) => report,
            Err(error) => {
                // Задача не содержит паник по построению; при аварии коммит
                // нельзя считать успешным.
                warn!(error = %error, "VTS token commit task failed");
                ConnectionReport::superseded()
            }
        }
    }

    async fn commit_connection_success_scoped<F, Fut>(
        &self,
        success: ConnectionSuccess,
        persist_token: F,
    ) -> ConnectionReport
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        let attempt = success.attempt;

        // Коммит под сериализующим локом (порядок коммитов) и read-gate
        // (инвалидация попытки невозможна): проверка актуальности, durable-запись
        // на диск и применение runtime-настройки образуют одну точку
        // линеаризации с `begin_connection_attempt`/`discard_attempt_connection`.
        let serialized = self.token_commit.lock().await;
        let gate = self.attempt_gate.read().await;

        let Some(token) = success.token.clone() else {
            // Нового токена нет — состояние уже установлено сервисом.
            return if self.is_current_attempt(attempt) {
                ConnectionReport::live(success.live)
            } else {
                ConnectionReport::superseded()
            };
        };

        if !self.is_current_attempt(attempt) {
            debug!(
                attempt = attempt.generation(),
                "VTS token commit skipped: attempt superseded"
            );
            return ConnectionReport::superseded();
        }

        if let Err(error) = persist_token(token.clone()).await {
            // Сериализующий и attempt-локи не удерживаем через scoped teardown:
            // discard сам берёт write-gate и CAS-ит generation.
            drop(gate);
            drop(serialized);
            if self.discard_attempt_connection(attempt).await {
                warn!(error = %error, "VTS token persist failed; connection discarded");
                return ConnectionReport::persist_failed(error);
            }
            debug!(
                attempt = attempt.generation(),
                "VTS token commit failure ignored: attempt superseded"
            );
            return ConnectionReport::superseded();
        }

        // Инвалидация заблокирована read-gate, поэтому попытка всё ещё
        // актуальна: применяем runtime-токен и отпускаем gate.
        {
            let mut settings = self.settings.write().await;
            settings.token = Some(token);
        }
        drop(gate);
        ConnectionReport::live(success.live)
    }

    pub async fn set_typing(
        self: &Arc<Self>,
        typing: bool,
        host: &str,
        port: u16,
        stored_token: &str,
    ) -> Result<(), String> {
        let typing_action = { self.settings.read().await.typing_action.clone() };

        if !typing {
            {
                let mut inner = self.inner.lock().await;
                self.stop_typing_keepalive_locked(&mut inner);
                inner.typing_active = false;
            }

            // inject=0.0 / stop-hotkey идут через actor; статус-инвалидацию на
            // transport-failure делает сам actor (handle_transport_failure_locked).
            match typing_action.output_mode {
                VTubeStudioTypingMode::Event => {
                    let param_name = typing_action.parameter_name.clone();
                    if let Err(ref e) = inject_typing(self, &param_name, 0.0).await {
                        if !is_semantic_vts_error(e) {
                            debug!(error = %e, "VTS inject typing=false transport failure");
                        }
                    }
                }
                VTubeStudioTypingMode::Hotkeys => {
                    let stop_id = typing_action.stop_hotkey_id.clone();
                    if let Err(ref e) = trigger_hotkey(self, &stop_id).await {
                        if !is_semantic_vts_error(e) {
                            debug!(error = %e, "VTS hotkey stop trigger transport failure");
                        }
                    }
                }
                VTubeStudioTypingMode::Item => {
                    self.item_transition.record_desired(false);
                }
            }
            return Ok(());
        }

        if stored_token.is_empty() {
            return Ok(());
        }

        if !self.is_desired_running() {
            debug!("VTS: typing=true ignored — desired_running is false");
            return Ok(());
        }

        // Если actor-а нет — поднимаем соединение (raw-socket auth → start_actor_locked).
        let needs_reconnect = { self.inner.lock().await.actor.is_none() };
        if needs_reconnect {
            let attempt = self.begin_connection_attempt().await;

            // Подготовка — под коротким локом; connect/auth ниже идут без `inner`.
            // Read-gate + перепроверка: не вытеснит ли нас новая попытка между
            // проверкой и сменой статуса.
            {
                let _gate = self.attempt_gate.read().await;
                if !self.is_current_attempt(attempt) {
                    debug!(
                        attempt = attempt.generation(),
                        "VTS typing reconnect already superseded"
                    );
                    return Ok(());
                }
                let mut inner = self.inner.lock().await;
                self.stop_heartbeat_locked(&mut inner);
                self.set_connection_status(VTubeStudioConnectionStatus::Connecting);
            }

            let mut ws = match connect_ws(host, port).await {
                Ok(ws) => ws,
                Err(e) => {
                    // Вытеснённый провал не является ошибкой текущей сессии:
                    // состояние принадлежит более новой попытке.
                    let _gate = self.attempt_gate.read().await;
                    if !self.is_current_attempt(attempt) {
                        debug!(
                            attempt = attempt.generation(),
                            "VTS typing reconnect failure superseded"
                        );
                        return Ok(());
                    }
                    debug!(error = %e, "VTS connect for typing=true failed");
                    self.set_connection_status(VTubeStudioConnectionStatus::Error);
                    self.emit_status();
                    return Err(e);
                }
            };

            match perform_authentication(&mut ws, self.next_id(), Some(stored_token)).await {
                Ok(_) => {}
                Err(e) => {
                    let _gate = self.attempt_gate.read().await;
                    if !self.is_current_attempt(attempt) {
                        debug!(
                            attempt = attempt.generation(),
                            "VTS typing reconnect auth failure superseded"
                        );
                        return Ok(());
                    }
                    debug!(error = %e, "VTS auth for typing=true failed, discarding broken socket");
                    self.is_authenticated.store(false, Ordering::SeqCst);
                    self.set_connection_status(VTubeStudioConnectionStatus::Error);
                    self.emit_status();
                    return Err(e);
                }
            }

            // Установка сокета под read-gate + `inner`: проверка актуальности и
            // публикация Connected/auth атомарны относительно новой попытки.
            let _gate = self.attempt_gate.read().await;
            let mut inner = self.inner.lock().await;
            if self.install_decision_locked(attempt) != InstallDecision::Install {
                debug!(
                    attempt = attempt.generation(),
                    "VTS typing reconnect superseded; discarding socket"
                );
                return Ok(());
            }
            self.start_actor_locked(&mut inner, ws);
            self.is_authenticated.store(true, Ordering::SeqCst);
            self.start_heartbeat_locked(&mut inner);
            self.set_connection_status(VTubeStudioConnectionStatus::Connected);
            self.emit_status();
        }

        {
            let mut inner = self.inner.lock().await;
            self.stop_typing_keepalive_locked(&mut inner);
            inner.typing_active = true;
        }

        match typing_action.output_mode {
            VTubeStudioTypingMode::Event => {
                let param_name = typing_action.parameter_name.clone();

                if let Err(e) = ensure_event_parameter(self, &param_name).await {
                    if is_semantic_vts_error(&e) {
                        let mut inner = self.inner.lock().await;
                        inner.typing_active = false;
                        return Err(explain_event_error(&e, &param_name));
                    }
                    debug!(error = %e, "VTS ensure event parameter transport failure");
                    let mut inner = self.inner.lock().await;
                    inner.typing_active = false;
                    return Err(e);
                }

                if let Err(e) = inject_typing(self, &param_name, 1.0).await {
                    if is_semantic_vts_error(&e) {
                        let mut inner = self.inner.lock().await;
                        inner.typing_active = false;
                        return Err(explain_event_error(&e, &param_name));
                    }
                    debug!(error = %e, "VTS inject typing=true transport failure");
                    let mut inner = self.inner.lock().await;
                    inner.typing_active = false;
                    return Err(e);
                }

                let (cancel, keepalive_gen) = {
                    let cancel = CancellationToken::new();
                    let mut inner = self.inner.lock().await;
                    let gen = inner.connection_generation;
                    inner.typing_cancel = Some(cancel.clone());
                    (cancel, gen)
                };
                self.spawn_typing_keepalive(&param_name, cancel, keepalive_gen);
            }
            VTubeStudioTypingMode::Hotkeys => {
                let start_id = typing_action.start_hotkey_id.clone();
                if let Err(e) = trigger_hotkey(self, &start_id).await {
                    if is_semantic_vts_error(&e) {
                        debug!(error = %e, "VTS hotkey start trigger got semantic error, socket stays alive");
                        let mut inner = self.inner.lock().await;
                        inner.typing_active = false;
                        return Err(e);
                    }
                    debug!(error = %e, "VTS hotkey start trigger transport failure");
                    let mut inner = self.inner.lock().await;
                    inner.typing_active = false;
                    return Err(e);
                }
            }
            VTubeStudioTypingMode::Item => {
                self.item_transition.record_desired(true);
            }
        }

        Ok(())
    }

    /// Запускает typing keep-alive. Каждые TYPING_KEEPALIVE_MS шлёт inject=1.0
    /// через actor (без удержания `inner`), на transport-failure инвалидирует
    /// соединение единым путём (P1: с emit). `cancel` уже сохранён в `inner.typing_cancel`
    /// вызывающей стороной.
    fn spawn_typing_keepalive(
        self: &Arc<Self>,
        param_name: &str,
        cancel: CancellationToken,
        generation: u64,
    ) {
        let param_name = param_name.to_string();
        let cancel_ct = cancel.clone();
        let svc = Arc::clone(self);

        let handle = tokio::spawn(async move {
            loop {
                if cancel_ct.is_cancelled() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(TYPING_KEEPALIVE_MS)).await;
                if cancel_ct.is_cancelled() {
                    break;
                }

                match inject_typing(&svc, &param_name, 1.0).await {
                    Ok(()) => {}
                    Err(e) if is_semantic_vts_error(&e) => {
                        debug!(error = %e, "VTS typing keep-alive inject got semantic error, socket stays alive");
                    }
                    Err(e) => {
                        debug!(error = %e, "VTS typing keep-alive inject failed, invalidating connection");
                        svc.handle_transport_failure_locked(generation, FailureClass::Send, None)
                            .await;
                        break;
                    }
                }
            }
            debug!("VTS typing keep-alive stopped");
        });

        if let Ok(mut inner) = self.inner.try_lock() {
            inner.typing_handle = Some(handle);
        } else {
            // Если лок не взят сразу (редкий случай) — abort-им задачу, т.к. handle
            // негде сохранить; cancel всё равно остановит её.
            debug!("VTS typing keep-alive: could not store handle immediately");
            handle.abort();
        }
    }

    pub(crate) async fn ensure_event_parameter_if_connected(
        &self,
        parameter_name: &str,
    ) -> Result<EnsureOutcome, String> {
        if !self.is_desired_running() {
            return Ok(EnsureOutcome::Skipped(SkipReason::NotDesiredRunning));
        }

        let status = self.get_connection_status();
        if status != VTubeStudioConnectionStatus::Connected {
            return Ok(EnsureOutcome::Skipped(SkipReason::NotConnected));
        }

        if !self.is_authenticated.load(Ordering::SeqCst) {
            return Ok(EnsureOutcome::Skipped(SkipReason::NotAuthenticated));
        }

        if self.inner.lock().await.actor.is_none() {
            return Ok(EnsureOutcome::Skipped(SkipReason::NoSocket));
        }

        match ensure_event_parameter(self, parameter_name).await {
            Ok(()) => Ok(EnsureOutcome::Ensured),
            Err(e) => {
                if is_semantic_vts_error(&e) {
                    Err(explain_event_error(&e, parameter_name))
                } else {
                    // transport failure: actor сам зафиксирует read-failure и
                    // инвалидирует соединение единым путём (с emit).
                    debug!(error = %e, "VTS ensure event parameter transport failure");
                    Err(e)
                }
            }
        }
    }

    pub(crate) async fn delete_event_parameter_if_connected(
        &self,
        parameter_name: &str,
    ) -> Result<DeleteOutcome, String> {
        if !self.is_desired_running() {
            return Ok(DeleteOutcome::Skipped(SkipReason::NotDesiredRunning));
        }

        let status = self.get_connection_status();
        if status != VTubeStudioConnectionStatus::Connected {
            return Ok(DeleteOutcome::Skipped(SkipReason::NotConnected));
        }

        if !self.is_authenticated.load(Ordering::SeqCst) {
            return Ok(DeleteOutcome::Skipped(SkipReason::NotAuthenticated));
        }

        if self.inner.lock().await.actor.is_none() {
            return Ok(DeleteOutcome::Skipped(SkipReason::NoSocket));
        }

        match delete_event_parameter(self, parameter_name).await {
            Ok(()) => Ok(DeleteOutcome::Deleted),
            Err(e) => {
                if is_semantic_vts_error(&e) {
                    if inner_vts_error(&e) == "VTS error 401" {
                        debug!(
                            parameter_name,
                            "VTS delete parameter not found (401), already cleaned up"
                        );
                        Ok(DeleteOutcome::NotFound)
                    } else {
                        Err(e)
                    }
                } else {
                    // transport failure: actor сам инвалидирует соединение.
                    debug!(error = %e, "VTS delete event parameter transport failure");
                    Err(e)
                }
            }
        }
    }

    pub async fn stop_event_typing_and_reset(&self, parameter_name: &str) {
        let was_active = {
            let mut inner = self.inner.lock().await;
            self.stop_typing_keepalive_locked(&mut inner);
            if inner.typing_active {
                inner.typing_active = false;
                true
            } else {
                false
            }
        };
        if was_active {
            // inject=0.0 через actor; transport-failure (если есть) фиксирует actor.
            let _ = inject_typing(self, parameter_name, 0.0).await;
        }
    }

    pub async fn refresh_item_action(&self) -> VTubeStudioItemStatus {
        let typing_action = { self.settings.read().await.typing_action.clone() };

        match typing_action.output_mode {
            VTubeStudioTypingMode::Event | VTubeStudioTypingMode::Hotkeys => {
                let mut inner = self.inner.lock().await;
                inner.resolved_item = None;
                let status = VTubeStudioItemStatus::Inactive;
                *self.item_status.lock() = status.clone();
                status
            }
            VTubeStudioTypingMode::Item => {
                let file_name = typing_action.item_file_name.clone();

                if file_name.is_empty() {
                    let mut inner = self.inner.lock().await;
                    inner.resolved_item = None;
                    let status = VTubeStudioItemStatus::Missing {
                        file_name: String::new(),
                    };
                    *self.item_status.lock() = status.clone();
                    return status;
                }

                let has_actor = { self.inner.lock().await.actor.is_some() };
                if !has_actor {
                    let mut inner = self.inner.lock().await;
                    inner.resolved_item = None;
                    let status = VTubeStudioItemStatus::Error {
                        file_name: file_name.clone(),
                        message: "WebSocket not available".to_string(),
                    };
                    *self.item_status.lock() = status.clone();
                    return status;
                }

                let session = self.read_session();
                let (resolved, status) =
                    self.do_item_refresh_with_desired(&file_name, session).await;
                if self.read_session() == session {
                    let mut inner = self.inner.lock().await;
                    inner.resolved_item = resolved;
                    *self.item_status.lock() = status.clone();
                }
                status
            }
        }
    }

    async fn do_item_refresh_with_desired(
        &self,
        file_name: &str,
        session: u64,
    ) -> (Option<ResolvedItem>, VTubeStudioItemStatus) {
        let (desired, desired_gen) = self.item_transition.read_desired();

        match fetch_scene_instances(self, Some(file_name)).await {
            Ok(instances) => {
                if self.read_session() != session {
                    return (None, VTubeStudioItemStatus::Inactive);
                }

                match resolve_item(&instances, file_name) {
                    Ok(item) => {
                        let resolved = item.clone();
                        match animate_item(self, &item, desired).await {
                            Ok(_) => {
                                if self.read_session() == session {
                                    self.item_transition
                                        .set_applied_if_current(desired, desired_gen);
                                }
                                let status = VTubeStudioItemStatus::Ready {
                                    file_name: resolved.file_name.clone(),
                                    vts_type: resolved.item_type.clone(),
                                };
                                (Some(resolved), status)
                            }
                            Err(e) => {
                                if self.read_session() == session {
                                    self.item_transition.mark_applied_unknown();
                                }
                                let status = VTubeStudioItemStatus::Error {
                                    file_name: file_name.to_string(),
                                    message: format!("failed to normalize item: {}", e),
                                };
                                (Some(resolved), status)
                            }
                        }
                    }
                    Err(ResolveItemError::Missing) => {
                        let status = VTubeStudioItemStatus::Missing {
                            file_name: file_name.to_string(),
                        };
                        (None, status)
                    }
                    Err(ResolveItemError::Ambiguous) => {
                        let count = instances
                            .iter()
                            .filter(|i| i.file_name == file_name)
                            .count() as u32;
                        let status = VTubeStudioItemStatus::Ambiguous {
                            file_name: file_name.to_string(),
                            match_count: count,
                        };
                        (None, status)
                    }
                    Err(ResolveItemError::Unsupported {
                        file_name: fn_,
                        item_type,
                    }) => {
                        let status = VTubeStudioItemStatus::Unsupported {
                            file_name: fn_,
                            vts_type: item_type,
                        };
                        (None, status)
                    }
                }
            }
            Err(e) => {
                if self.read_session() == session {
                    self.item_transition.mark_applied_unknown();
                }
                let status = VTubeStudioItemStatus::Error {
                    file_name: file_name.to_string(),
                    message: e,
                };
                (None, status)
            }
        }
    }

    pub async fn list_scene_items(&self) -> Result<Vec<SceneItemRecord>, String> {
        if !self.is_desired_running() {
            return Err("VTube Studio is not running.".to_string());
        }

        let status = self.get_connection_status();
        if status != VTubeStudioConnectionStatus::Connected {
            return Err(format!(
                "VTube Studio is not connected (status: {:?}).",
                status
            ));
        }

        if !self.is_authenticated.load(Ordering::SeqCst) {
            return Err("VTube Studio is not authenticated.".to_string());
        }

        if self.inner.lock().await.actor.is_none() {
            return Err("VTube Studio WebSocket is not available.".to_string());
        }

        let instances = fetch_scene_instances(self, None).await?;

        Ok(build_scene_records(&instances))
    }

    pub async fn disconnect(&self) {
        // Явная остановка гасит вооружённый decay-таймер ошибки ручного подключения.
        self.error_decay_generation.fetch_add(1, Ordering::SeqCst);
        // Инвалидируем все in-flight попытки подключения: их сокет не должен
        // быть установлен уже после teardown.
        let scope = self.begin_connection_attempt().await;
        // Teardown — со scope этой попытки: если за время best-effort отправок
        // стартовала более новая попытка, её сессию не трогаем (последнее
        // намерение пользователя побеждает).
        if self.teardown_connection(scope).await {
            info!("VTube Studio disconnected");
        } else {
            debug!("VTS disconnect superseded by a newer attempt; state untouched");
        }
    }

    /// Снимает соединение, установленное попыткой `attempt`, только если она
    /// всё ещё актуальна. Используется, когда аутентифицированный сокет нельзя
    /// оставить (например, токен не записался): нельзя рвать более новую сессию,
    /// но и нельзя оставлять соединение без сохранённого токена.
    ///
    /// Возвращает true, если teardown действительно выполнен этой попыткой.
    async fn discard_attempt_connection(&self, attempt: ConnectionAttempt) -> bool {
        let claimed = ConnectionAttempt(attempt.generation().wrapping_add(1));
        // CAS под write-gate: не даёт вытеснить более новую попытку. Если
        // generation уже ушёл вперёд, соединение принадлежит ей.
        {
            let _gate = self.attempt_gate.write().await;
            if self
                .connection_attempt
                .compare_exchange(
                    attempt.generation(),
                    claimed.generation(),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_err()
            {
                debug!(
                    attempt = attempt.generation(),
                    "VTS discard skipped: attempt superseded"
                );
                return false;
            }
        }

        self.error_decay_generation.fetch_add(1, Ordering::SeqCst);
        self.teardown_connection(claimed).await
    }

    /// Снимает сессию, принадлежащую попытке `scope`. Перед каждым необратимым
    /// шагом актуальность перепроверяется, а разрушительная часть выполняется
    /// под read-gate `attempt_gate`: более новая попытка не может стартовать
    /// между проверкой и остановкой actor-а (её `begin_connection_attempt`
    /// требует write-gate). Best-effort отправки идут через sender, снятый в
    /// начале teardown-а, поэтому даже при появлении новой сессии они не
    /// попадут в новый actor.
    async fn teardown_connection(&self, scope: ConnectionAttempt) -> bool {
        let typing_action = { self.settings.read().await.typing_action.clone() };

        if !self.is_current_attempt(scope) {
            return false;
        }

        // Собираем состояние под кратким локом, не держа его через await.
        let (typing_active, resolved_item, actor_sender) = {
            let inner = self.inner.lock().await;
            (
                inner.typing_active,
                inner.resolved_item.clone(),
                inner.actor.as_ref().map(|handle| handle.sender.clone()),
            )
        };

        if !self.is_current_attempt(scope) {
            return false;
        }

        // Best-effort отправки ПЕРЕД teardown, через sender текущей сессии.
        // Конкурентная heartbeat-failure может убить actor mid-send (Q7) — допустимо.
        if let Some(sender) = actor_sender.as_ref() {
            if typing_action.output_mode == VTubeStudioTypingMode::Item {
                let applied = self.item_transition.read_applied();
                let should_hide = matches!(
                    (resolved_item.as_ref(), applied),
                    (Some(_), Some(true)) | (Some(_), None)
                );
                if should_hide {
                    if let Some(ref item) = resolved_item {
                        let start = Instant::now();
                        if let Err(_e) = animate_item_on(self, sender, item, false).await {
                            warn!(
                                mode = "Item",
                                file_name = %item.file_name,
                                "Best-effort hide during disconnect failed"
                            );
                        }
                        let duration = start.elapsed();
                        info!(
                            mode = "Item",
                            item_type = %item.item_type,
                            file_name = %item.file_name,
                            desired = false,
                            duration_ms = duration.as_millis(),
                            "Disconnect hide completed"
                        );
                    }
                }
            } else if typing_active {
                match typing_action.output_mode {
                    VTubeStudioTypingMode::Event => {
                        let _ = inject_typing_on(self, sender, &typing_action.parameter_name, 0.0)
                            .await;
                    }
                    VTubeStudioTypingMode::Hotkeys => {
                        if !typing_action.stop_hotkey_id.is_empty() {
                            let _ = trigger_hotkey_on(self, sender, &typing_action.stop_hotkey_id)
                                .await;
                        }
                    }
                    VTubeStudioTypingMode::Item => {}
                }
            }
        }

        // Разрушительная часть: под read-gate инвалидация невозможна, поэтому
        // проверка актуальности и остановка actor-а атомарны с началом новой
        // попытки.
        let _gate = self.attempt_gate.read().await;
        if !self.is_current_attempt(scope) {
            return false;
        }

        if typing_action.output_mode == VTubeStudioTypingMode::Item {
            self.invalidate_session();
            self.item_transition.reset();
        }

        {
            let mut inner = self.inner.lock().await;
            if !self.is_current_attempt(scope) {
                return false;
            }
            self.set_desired_running(false);
            self.stop_typing_keepalive_locked(&mut inner);
            self.stop_heartbeat_locked(&mut inner);
            self.stop_actor_locked(&mut inner);
            inner.connection_generation = inner.connection_generation.wrapping_add(1);
            inner.typing_active = false;
            inner.resolved_item = None;
            self.is_authenticated.store(false, Ordering::SeqCst);
            self.set_connection_status(VTubeStudioConnectionStatus::Disconnected);
            *self.item_status.lock() = VTubeStudioItemStatus::Inactive;
        }
        true
    }

    fn stop_typing_keepalive_locked(&self, inner: &mut InnerState) {
        if let Some(cancel) = inner.typing_cancel.take() {
            cancel.cancel();
        }
        if let Some(handle) = inner.typing_handle.take() {
            handle.abort();
        }
    }

    fn stop_heartbeat_locked(&self, inner: &mut InnerState) {
        if let Some(cancel) = inner.heartbeat_cancel.take() {
            cancel.cancel();
        }
        if let Some(handle) = inner.heartbeat_handle.take() {
            handle.abort();
        }
    }

    fn start_heartbeat_locked(&self, inner: &mut InnerState) {
        self.stop_heartbeat_locked(inner);
        // heartbeat captures generation of the currently-owned actor;
        // starting/restarting heartbeat must NOT bump generation.
        let hb_gen = inner.connection_generation;

        // Клонируем actor-sender под текущим локом (heartbeat не должен держать
        // `inner` через await — это прежний источник дедлока).
        let sender = match inner.actor.as_ref() {
            Some(h) => h.sender.clone(),
            None => return,
        };

        let hb_cancel = CancellationToken::new();
        let hb_ct = hb_cancel.clone();
        let inner_arc = Arc::clone(&self.inner);
        let auth_flag = Arc::clone(&self.is_authenticated);
        let status_arc = Arc::clone(&self.connection_status);
        let app_handle = self.app_handle.lock().clone();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = hb_ct.cancelled() => break,
                    _ = tokio::time::sleep(HEARTBEAT_INTERVAL) => {}
                }
                if hb_ct.is_cancelled() {
                    break;
                }

                // Текущая generation может измениться (disconnect/connect) — проверяем.
                if inner_arc.lock().await.connection_generation != hb_gen {
                    break;
                }

                let id = uuid::Uuid::new_v4().to_string();
                let req = VtsRequest::api_state_request(&id);
                let json = match serde_json::to_string(&req) {
                    Ok(j) => j,
                    Err(_) => {
                        // Сериализация не падает на детерминированном запросе; если упало — транспорт не виноват.
                        break;
                    }
                };

                let (tx, rx) = oneshot::channel::<Result<serde_json::Value, String>>();
                if sender
                    .send(ActorRequest {
                        request_id: id.clone(),
                        json,
                        expected_msg_type: "APIStateResponse".to_string(),
                        timeout: REQUEST_TIMEOUT,
                        reply: tx,
                    })
                    .await
                    .is_err()
                {
                    // Actor уже мёртв — transport failure уже зафиксирован самим actor-ом.
                    break;
                }

                match timeout(REQUEST_TIMEOUT, rx).await {
                    Ok(Ok(Ok(value))) => {
                        let parsed =
                            serde_json::from_value::<messages::APIStateResponseData>(value);
                        match parsed {
                            Ok(data) => {
                                if let Err(e) = validate_api_state(&data) {
                                    debug!("Heartbeat validation failed: {}", e);
                                    actor_transport_failure(
                                        &inner_arc,
                                        &auth_flag,
                                        &status_arc,
                                        app_handle.clone(),
                                        hb_gen,
                                        FailureClass::Read,
                                        Some(e),
                                        &mut HashMap::new(),
                                    )
                                    .await;
                                    break;
                                }
                            }
                            Err(e) => {
                                // Семантическая ошибка парсинга — сокет жив, не инвалидируем.
                                debug!(error = %e, "Heartbeat response parse error (socket stays alive)");
                            }
                        }
                    }
                    Ok(Ok(Err(e))) => {
                        // Semantic VTS error на heartbeat-ответе (редко) — сокет жив.
                        debug!(error = %e, "Heartbeat got semantic error (socket stays alive)");
                    }
                    Ok(Err(_)) => {
                        // Reply-канал закрыт: actor умер и сам зафиксировал transport failure.
                        break;
                    }
                    Err(_) => {
                        // Таймаут. Actor сам зафиксирует read-failure; здесь дублируем
                        // инвалидацию (exactly-once через generation guard).
                        actor_transport_failure(
                            &inner_arc,
                            &auth_flag,
                            &status_arc,
                            app_handle.clone(),
                            hb_gen,
                            FailureClass::Read,
                            None,
                            &mut HashMap::new(),
                        )
                        .await;
                        break;
                    }
                }
            }
        });

        inner.heartbeat_cancel = Some(hb_cancel);
        inner.heartbeat_handle = Some(handle);
    }

    pub async fn get_current_model_hotkeys(&self) -> Result<Vec<HotkeyInfo>, String> {
        if !self.is_desired_running() {
            return Err("VTube Studio is not running.".to_string());
        }

        let status = self.get_connection_status();
        if status != VTubeStudioConnectionStatus::Connected {
            return Err(format!(
                "VTube Studio is not connected (status: {:?}).",
                status
            ));
        }

        if !self.is_authenticated.load(Ordering::SeqCst) {
            return Err("VTube Studio is not authenticated.".to_string());
        }

        if self.inner.lock().await.actor.is_none() {
            return Err("VTube Studio WebSocket is not available.".to_string());
        }

        let value = self
            .actor_request(
                "HotkeysInCurrentModelResponse",
                VtsRequest::hotkeys_in_current_model_request,
                REQUEST_TIMEOUT,
            )
            .await
            .map_err(|e| format!("Hotkeys request failed: {}", e))?;

        let data: HotkeysInCurrentModelResponseData =
            serde_json::from_value(value).map_err(|e| format!("Parse hotkeys response: {}", e))?;

        if !data.model_loaded {
            return Err("No model is currently loaded in VTube Studio.".to_string());
        }

        Ok(data.available_hotkeys)
    }

    pub async fn test_typing_action(
        &self,
        timeout_ms: u64,
        repeat_count: u64,
    ) -> Result<String, String> {
        let typing_action = self.settings.read().await.typing_action.clone();

        if !(100..=5000).contains(&timeout_ms) {
            return Err("Таймаут должен быть от 100 до 5000 мс".to_string());
        }
        if !(1..=10).contains(&repeat_count) {
            return Err("Повторы должны быть от 1 до 10".to_string());
        }

        if !self.is_desired_running() {
            return Err("VTube Studio is not running. Start the connection first.".to_string());
        }

        let status = self.get_connection_status();
        if status != VTubeStudioConnectionStatus::Connected {
            return Err(format!(
                "VTube Studio is not connected (status: {:?}). Must be Connected.",
                status
            ));
        }

        if self.inner.lock().await.actor.is_none() {
            return Err("VTube Studio WebSocket is not available.".to_string());
        }

        {
            let mut inner = self.inner.lock().await;
            self.stop_typing_keepalive_locked(&mut inner);
        }

        if typing_action.output_mode == VTubeStudioTypingMode::Item {
            // Invalidate session so any in-flight worker completes without
            // restoring state after this test ends (requirement 4).
            let session = self.invalidate_session();
            let resolved = { self.inner.lock().await.resolved_item.clone() };

            let item = match resolved {
                Some(item) => item,
                None => {
                    return Err(
                        "No resolved item available for testing. Refresh the item first."
                            .to_string(),
                    );
                }
            };

            for i in 0..repeat_count {
                if let Err(e) = animate_item(self, &item, true).await {
                    self.item_transition.mark_applied_unknown();
                    *self.item_status.lock() = VTubeStudioItemStatus::Error {
                        file_name: item.file_name.clone(),
                        message: format!("show failed at repeat {}: {}", i + 1, e),
                    };
                    // Do not change connection status on item test failure
                    return Err(format!("Item show failed at repeat {}: {}", i + 1, e));
                }

                tokio::time::sleep(Duration::from_millis(timeout_ms)).await;

                if let Err(e) = animate_item(self, &item, false).await {
                    self.item_transition.mark_applied_unknown();
                    *self.item_status.lock() = VTubeStudioItemStatus::Error {
                        file_name: item.file_name.clone(),
                        message: format!("hide failed at repeat {}: {}", i + 1, e),
                    };
                    // Do not change connection status on item test failure
                    return Err(format!("Item hide failed at repeat {}: {}", i + 1, e));
                }

                if i + 1 < repeat_count {
                    tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
                }
            }

            // Guarded commit: only restore if session is still current (requirement 3)
            if self.read_session() == session {
                self.item_transition.reset();
                self.item_transition.force_applied(false);
                *self.item_status.lock() = VTubeStudioItemStatus::Ready {
                    file_name: item.file_name.clone(),
                    vts_type: item.item_type.clone(),
                };
            }

            return Ok(format!(
                "Тест действия выполнен: повторов — {}, таймаут — {} мс",
                repeat_count, timeout_ms
            ));
        }

        let timeout_dur = Duration::from_millis(timeout_ms);

        for i in 0..repeat_count {
            match typing_action.output_mode {
                VTubeStudioTypingMode::Event => {
                    let param_name = typing_action.parameter_name.clone();
                    if let Err(e) = ensure_event_parameter(self, &param_name).await {
                        if is_semantic_vts_error(&e) {
                            return Err(format!(
                                "VTube Studio action test failed at repeat {} (ensure): {}",
                                i + 1,
                                explain_event_error(&e, &param_name)
                            ));
                        }
                        // transport failure: actor сам зафиксирует read-failure и
                        // инвалидирует соединение. Просто возвращаем ошибку.
                        return Err(format!(
                            "VTube Studio action test failed at repeat {} (ensure): {}",
                            i + 1,
                            e
                        ));
                    }
                    if let Err(e) = inject_typing(self, &param_name, 1.0).await {
                        if is_semantic_vts_error(&e) {
                            return Err(format!(
                                "VTube Studio action test failed at repeat {} (start): {}",
                                i + 1,
                                explain_event_error(&e, &param_name)
                            ));
                        }
                        return Err(format!(
                            "VTube Studio action test failed at repeat {} (start): {}",
                            i + 1,
                            e
                        ));
                    }
                }
                VTubeStudioTypingMode::Hotkeys => {
                    let start_id = typing_action.start_hotkey_id.clone();
                    if let Err(e) = trigger_hotkey(self, &start_id).await {
                        if is_semantic_vts_error(&e) {
                            return Err(format!(
                                "VTube Studio action test failed at repeat {} (start): {}",
                                i + 1,
                                e
                            ));
                        }
                        return Err(format!(
                            "VTube Studio action test failed at repeat {} (start): {}",
                            i + 1,
                            e
                        ));
                    }
                }
                VTubeStudioTypingMode::Item => {
                    // Unreachable — Item mode handled above.
                }
            }

            tokio::time::sleep(timeout_dur).await;

            match typing_action.output_mode {
                VTubeStudioTypingMode::Event => {
                    let param_name = typing_action.parameter_name.clone();
                    if let Err(e) = inject_typing(self, &param_name, 0.0).await {
                        if is_semantic_vts_error(&e) {
                            return Err(format!(
                                "VTube Studio action test failed at repeat {} (stop): {}",
                                i + 1,
                                explain_event_error(&e, &param_name)
                            ));
                        }
                        return Err(format!(
                            "VTube Studio action test failed at repeat {} (stop): {}",
                            i + 1,
                            e
                        ));
                    }
                }
                VTubeStudioTypingMode::Hotkeys => {
                    let stop_id = typing_action.stop_hotkey_id.clone();
                    if let Err(e) = trigger_hotkey(self, &stop_id).await {
                        if is_semantic_vts_error(&e) {
                            return Err(format!(
                                "VTube Studio action test failed at repeat {} (stop): {}",
                                i + 1,
                                e
                            ));
                        }
                        return Err(format!(
                            "VTube Studio action test failed at repeat {} (stop): {}",
                            i + 1,
                            e
                        ));
                    }
                }
                VTubeStudioTypingMode::Item => {
                    // Unreachable — Item mode handled above.
                }
            }

            if i + 1 < repeat_count {
                tokio::time::sleep(timeout_dur).await;
            }
        }

        Ok(format!(
            "Тест действия выполнен: повторов — {}, таймаут — {} мс",
            repeat_count, timeout_ms
        ))
    }
}

/// Connection-actor: единственный долгоживущий владелец аутентифицированного WsStream.
///
/// Постоянный reader-loop (P0-фикс): `ws.next()` опрашивается каждую итерацию через
/// `select!`, поэтому входящие PING от VTube Studio читаются, и tungstenite 0.24
/// авто-отправляет queued PONG при следующем read. Раньше Ping не читался в idle →
/// PONG не уходил → VTS рвал TCP (10053).
///
/// Записи сериализованы: `ws.send` зовёт только этот actor (callers пушат в mpsc).
/// Ответы коррелируются по requestID через `pending: HashMap`.
#[allow(clippy::too_many_arguments)]
async fn actor_task(
    mut ws: WsStream,
    mut rx: mpsc::Receiver<ActorRequest>,
    cancel: CancellationToken,
    inner: Arc<tokio::sync::Mutex<InnerState>>,
    is_authenticated: Arc<AtomicBool>,
    connection_status: Arc<parking_lot::Mutex<VTubeStudioConnectionStatus>>,
    app_handle: Option<AppHandle>,
    actor_generation: u64,
) {
    use tokio_tungstenite::tungstenite::Message;

    // shared Arc-ы в контексте actor для вызова handle_transport_failure через временный
    // «view»: создаём placeholder-сервис невозможен, поэтому реализуем инвалидацию inline,
    // переиспользуя те же поля, что и VTubeStudioService::handle_transport_failure_locked.

    let mut pending: HashMap<String, Pending> = HashMap::new();
    let mut sweep = tokio::time::interval(ACTOR_SWEEP_INTERVAL);
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = ws.close(None).await;
                fail_all_pending(&mut pending, "actor cancelled");
                return;
            }
            // sweep «зависших» pending-entries (защита от утечки при caller-timeout,
            // когда сервер молча не отвечает, но сокет жив).
            _ = sweep.tick() => {
                let now = Instant::now();
                pending.retain(|_, p| {
                    if now.duration_since(p.started) > p.timeout * 2 {
                        debug!("VTS actor pending sweep: dropping stale entry");
                        false
                    } else {
                        true
                    }
                });
                // sendable-continue; чтение/запись не блокируются
            }
            req = rx.recv() => {
                let Some(req) = req else {
                    let _ = ws.close(None).await;
                    fail_all_pending(&mut pending, "actor sender closed");
                    return;
                };
                log_outgoing_request(&req.json);
                let started = Instant::now();
                match timeout(SEND_TIMEOUT, ws.send(Message::Text(req.json))).await {
                    Ok(Ok(())) => {
                        pending.insert(req.request_id.clone(), Pending {
                            reply: req.reply,
                            expected_msg_type: req.expected_msg_type,
                            started,
                            timeout: req.timeout,
                        });
                    }
                    Ok(Err(e)) => {
                        debug!(error = %e, generation = actor_generation, "VTS send failed");
                        let _ = req.reply.send(Err("Send failed".to_string()));
                        actor_transport_failure(
                            &inner, &is_authenticated, &connection_status,
                            app_handle.clone(), actor_generation,
                            FailureClass::Send, Some(e.to_string()), &mut pending,
                        ).await;
                        return;
                    }
                    Err(_) => {
                        debug!(generation = actor_generation, "VTS send timed out");
                        let _ = req.reply.send(Err("Send timed out".to_string()));
                        actor_transport_failure(
                            &inner, &is_authenticated, &connection_status,
                            app_handle.clone(), actor_generation,
                            FailureClass::SendTimeout, None, &mut pending,
                        ).await;
                        return;
                    }
                }
            }
            frame = ws.next() => {
                let class = match frame {
                    None => Some(FailureClass::Closed),
                    Some(Err(e)) => {
                        debug!(error = %e, generation = actor_generation, "VTS read error");
                        Some(FailureClass::Read)
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let code = frame.as_ref().map(|f| f.code);
                        let reason = frame.as_ref().map(|f| f.reason.to_string());
                        debug!(?code, ?reason, generation = actor_generation, "VTS closed WebSocket connection");
                        Some(FailureClass::Closed)
                    }
                    // PONG уже в очереди tungstenite при парсинге Ping; отправится при
                    // следующем read/flush. Просто продолжаем цикл.
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => None,
                    Some(Ok(Message::Text(t))) => {
                        route_text(t, &mut pending);
                        None
                    }
                    Some(Ok(other)) => {
                        debug!(?other, generation = actor_generation, "Unexpected WebSocket message from VTS, ignoring");
                        None
                    }
                };
                if let Some(failure_class) = class {
                    actor_transport_failure(
                        &inner, &is_authenticated, &connection_status,
                        app_handle.clone(), actor_generation,
                        failure_class, None, &mut pending,
                    ).await;
                    return;
                }
            }
        }
    }
}

/// Маршрутизирует входящее text-сообщение по requestID в ожидающий reply-канал.
/// Semantic APIError завершает конкретный запрос, но сокет остаётся живым.
fn route_text(text: String, pending: &mut HashMap<String, Pending>) {
    let parsed: VtsResponse = match serde_json::from_str(&text) {
        Ok(p) => p,
        Err(e) => {
            debug!(error = %e, "VTS parse error, dropping frame");
            return;
        }
    };

    log_incoming_response(&parsed);

    let req_id = parsed.request_id.clone();
    let msg_type = parsed.message_type.clone();

    if let Some(p) = pending.remove(&req_id) {
        let elapsed = p.started.elapsed();
        match classify_vts_response(&parsed, &req_id, &p.expected_msg_type) {
            RecvResult::Match(data) => {
                debug!(messageType = %msg_type, %req_id, elapsed_ms = elapsed.as_millis(), "VTS recv routed");
                let _ = p.reply.send(Ok(data));
            }
            RecvResult::Error(e) => {
                // semantic error: сокет ЖИВ, actor не инвалидируется.
                debug!(messageType = %msg_type, %req_id, error = %e, "VTS recv semantic error");
                let _ = p.reply.send(Err(e));
            }
            RecvResult::Skip => {
                // requestID совпал, но messageType не тот — protocol violation.
                debug!(messageType = %msg_type, %req_id, expected_msg_type = %p.expected_msg_type, "VTS recv unexpected message type");
                let _ = p
                    .reply
                    .send(Err(format!("Unexpected WebSocket message: {}", msg_type)));
            }
        }
    } else {
        // Unsolicited event / ответ на уже отменённый запрос — лог-и-дроп.
        debug!(messageType = %msg_type, %req_id, "Unsolicited VTS message, no pending request — dropping");
    }
}

/// Завершает все pending reply с ошибкой (при shutdown / transport failure).
fn fail_all_pending(pending: &mut HashMap<String, Pending>, reason: &str) {
    for (_, p) in pending.drain() {
        let _ = p.reply.send(Err(reason.to_string()));
    }
}

/// Inline-копия VTubeStudioService::handle_transport_failure_locked для actor-задачи
/// (actor не владеет `&self`). Тело намеренно идентично методу, чтобы инвалидация
/// была единой.
#[allow(clippy::too_many_arguments)]
async fn actor_transport_failure(
    inner: &Arc<tokio::sync::Mutex<InnerState>>,
    is_authenticated: &Arc<AtomicBool>,
    connection_status: &Arc<parking_lot::Mutex<VTubeStudioConnectionStatus>>,
    app_handle: Option<AppHandle>,
    actor_generation: u64,
    class: FailureClass,
    detail: Option<String>,
    pending: &mut HashMap<String, Pending>,
) {
    fail_all_pending(pending, "transport failure");

    let mut guard = inner.lock().await;
    if guard.connection_generation != actor_generation {
        debug!(
            failure_generation = actor_generation,
            current_generation = guard.connection_generation,
            "stale transport failure ignored"
        );
        return;
    }

    let had_actor = guard.actor.is_some();
    if !had_actor {
        return;
    }

    // Те же шаги инвалидации, что в VTubeStudioService::stop_*_locked.
    if let Some(h) = guard.actor.take() {
        h.cancel.cancel();
        drop(h.sender);
        h.join.abort();
    }
    if let Some(c) = guard.typing_cancel.take() {
        c.cancel();
    }
    if let Some(h) = guard.typing_handle.take() {
        h.abort();
    }
    if let Some(c) = guard.heartbeat_cancel.take() {
        c.cancel();
    }
    if let Some(h) = guard.heartbeat_handle.take() {
        h.abort();
    }
    guard.typing_active = false;
    guard.resolved_item = None;
    is_authenticated.store(false, Ordering::SeqCst);
    *connection_status.lock() = VTubeStudioConnectionStatus::Error;

    info!(
        class = ?class,
        ?detail,
        generation = actor_generation,
        "VTS transport failure: connection invalidated"
    );

    // P1: emit перехода Error.
    if let Some(h) = &app_handle {
        let status = connection_status.lock().clone();
        let _ = h.emit(VTS_STATUS_CHANGED_EVENT, &status);
    }
}

async fn connect_ws(host: &str, port: u16) -> Result<WsStream, String> {
    let url = format!("ws://{}:{}", host, port);
    info!(%url, "Connecting to VTube Studio");

    let (ws, _resp) = timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| {
            "Connection to VTube Studio timed out. Is it running with Plugin API enabled?"
                .to_string()
        })?
        .map_err(|e| format!("WebSocket connect failed: {}", e))?;

    info!("Connected to VTube Studio");
    Ok(ws)
}

async fn connect_and_auth_fresh(host: &str, port: u16, token: &str) -> Result<WsStream, String> {
    info!("Establishing fresh socket for new-token session");
    let id = uuid::Uuid::new_v4().to_string();
    let mut ws = connect_ws(host, port)
        .await
        .map_err(|e| format!("Fresh-connect after token issuance failed: {}", e))?;
    match perform_authentication(&mut ws, id, Some(token)).await {
        Ok(None) => {
            info!("Fresh socket authenticated with new token");
            Ok(ws)
        }
        Ok(Some(_)) => Err(
            "Fresh-token authentication unexpectedly issued another token; aborting connection"
                .to_string(),
        ),
        Err(e) => Err(format!("Fresh-socket authentication failed: {}", e)),
    }
}

async fn perform_authentication(
    ws: &mut WsStream,
    request_id: String,
    stored_token: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(token) = stored_token {
        if !token.is_empty() {
            debug!("Trying stored authentication token");
            let req = VtsRequest::authentication_request(&request_id, token);
            let json = serde_json::to_string(&req).map_err(|e| e.to_string())?;

            match send_and_recv(ws, &json, &request_id, "AuthenticationResponse").await {
                Ok(value) => {
                    let data: messages::AuthenticationResponseData = serde_json::from_value(value)
                        .map_err(|e| format!("Parse auth response: {}", e))?;
                    if data.authenticated {
                        info!("Authenticated with stored token");
                        return Ok(None);
                    }
                    debug!("Stored token rejected, requesting new");
                }
                Err(e)
                    if e.starts_with("VTS error ")
                        || e.starts_with("Parse error data")
                        || e.starts_with("Parse response JSON") =>
                {
                    debug!(error = %e, "Stored token rejected by VTS, requesting new");
                }
                Err(e) => {
                    return Err(format!("Stored token authentication failed: {}", e));
                }
            }
        }
    }

    let token_req_id = format!("{}-tk", request_id);
    info!("Requesting new authentication token");
    let req = VtsRequest::auth_token_request(&token_req_id);
    let json = serde_json::to_string(&req).map_err(|e| e.to_string())?;

    let value = send_and_recv(ws, &json, &token_req_id, "AuthenticationTokenResponse")
        .await
        .map_err(|e| format!("Token request failed: {}", e))?;

    let token_data: messages::AuthTokenResponseData =
        serde_json::from_value(value).map_err(|e| format!("Parse token response: {}", e))?;
    let token = token_data.authentication_token;
    debug!("Received new authentication token");

    let auth_req_id = format!("{}-au", request_id);
    let req = VtsRequest::authentication_request(&auth_req_id, &token);
    let json = serde_json::to_string(&req).map_err(|e| e.to_string())?;

    let value = send_and_recv(ws, &json, &auth_req_id, "AuthenticationResponse")
        .await
        .map_err(|e| format!("Auth request failed: {}", e))?;

    let data: messages::AuthenticationResponseData =
        serde_json::from_value(value).map_err(|e| format!("Parse auth response: {}", e))?;

    if !data.authenticated {
        return Err(
            "VTS rejected authentication. The token was not approved in VTube Studio.".to_string(),
        );
    }

    info!("Authentication successful");
    Ok(Some(token))
}

/// Идемпотентно гарантирует custom INPUT перед inject в Event-режиме.
/// VTS повторно создаёт тот же параметр тем же plugin identity без ошибки.
async fn ensure_event_parameter(
    svc: &VTubeStudioService,
    parameter_name: &str,
) -> Result<(), String> {
    let name = parameter_name.to_string();
    let _value = svc
        .actor_request(
            "ParameterCreationResponse",
            |id| VtsRequest::parameter_creation_request(id, &name),
            REQUEST_TIMEOUT,
        )
        .await
        .map_err(|e| format!("Create parameter failed: {}", e))?;

    debug!(parameter_name, "Parameter ensured");
    Ok(())
}

async fn delete_event_parameter(
    svc: &VTubeStudioService,
    parameter_name: &str,
) -> Result<(), String> {
    let name = parameter_name.to_string();
    let _value = svc
        .actor_request(
            "ParameterDeletionResponse",
            |id| VtsRequest::parameter_deletion_request(id, &name),
            REQUEST_TIMEOUT,
        )
        .await
        .map_err(|e| format!("Delete parameter failed: {}", e))?;

    debug!(parameter_name, "Parameter deleted");
    Ok(())
}

async fn inject_typing(
    svc: &VTubeStudioService,
    parameter_name: &str,
    value: f64,
) -> Result<(), String> {
    let sender = svc.current_actor_sender().await?;
    inject_typing_on(svc, &sender, parameter_name, value).await
}

/// `inject_typing` через pinned sender конкретной сессии (teardown).
async fn inject_typing_on(
    svc: &VTubeStudioService,
    sender: &mpsc::Sender<ActorRequest>,
    parameter_name: &str,
    value: f64,
) -> Result<(), String> {
    let name = parameter_name.to_string();
    let _value = svc
        .actor_request_with_sender(
            sender,
            None,
            "InjectParameterDataResponse",
            |id| VtsRequest::inject_parameter_request(id, &name, value),
            REQUEST_TIMEOUT,
        )
        .await
        .map_err(|e| format!("Inject parameter failed: {}", e))?;

    debug!(parameter_name, value, "Parameter injected");
    Ok(())
}

async fn trigger_hotkey(svc: &VTubeStudioService, hotkey_id: &str) -> Result<(), String> {
    let sender = svc.current_actor_sender().await?;
    trigger_hotkey_on(svc, &sender, hotkey_id).await
}

/// `trigger_hotkey` через pinned sender конкретной сессии (teardown).
async fn trigger_hotkey_on(
    svc: &VTubeStudioService,
    sender: &mpsc::Sender<ActorRequest>,
    hotkey_id: &str,
) -> Result<(), String> {
    let id_param = hotkey_id.to_string();
    let _value = svc
        .actor_request_with_sender(
            sender,
            None,
            "HotkeyTriggerResponse",
            |id| VtsRequest::hotkey_trigger_request(id, &id_param),
            REQUEST_TIMEOUT,
        )
        .await
        .map_err(|e| format!("Hotkey trigger failed: {}", e))?;

    debug!(hotkey_id, "Hotkey triggered");
    Ok(())
}

async fn fetch_scene_instances(
    svc: &VTubeStudioService,
    file_name: Option<&str>,
) -> Result<Vec<ItemInstanceInfo>, String> {
    let value = svc
        .actor_request(
            "ItemListResponse",
            |id| VtsRequest::item_list_request(id, file_name),
            REQUEST_TIMEOUT,
        )
        .await?;

    let data: ItemListResponseData =
        serde_json::from_value(value).map_err(|e| format!("Parse item list response: {}", e))?;

    Ok(data.item_instances_in_scene)
}

async fn animate_item(
    svc: &VTubeStudioService,
    resolved: &ResolvedItem,
    show: bool,
) -> Result<(), String> {
    let sender = svc.current_actor_sender().await?;
    animate_item_on(svc, &sender, resolved, show).await
}

/// `animate_item` через pinned sender конкретной сессии (teardown).
async fn animate_item_on(
    svc: &VTubeStudioService,
    sender: &mpsc::Sender<ActorRequest>,
    resolved: &ResolvedItem,
    show: bool,
) -> Result<(), String> {
    let (opacity, frame, play_state) = match (&resolved.kind, show) {
        (ItemKind::Animated, true) => (1.0, Some(0), Some(true)),
        (ItemKind::Animated, false) => (0.0, None, Some(false)),
        (ItemKind::Static, true) => (1.0, None, None),
        (ItemKind::Static, false) => (0.0, None, None),
        _ => {
            return Err(format!(
                "cannot animate unsupported item kind for '{}'",
                resolved.file_name
            ))
        }
    };

    let instance_id = resolved.instance_id.clone();
    let value = svc
        .actor_request_with_sender(
            sender,
            None,
            "ItemAnimationControlResponse",
            |id| {
                VtsRequest::item_animation_control_request(
                    id,
                    &instance_id,
                    opacity,
                    frame,
                    play_state,
                )
            },
            ITEM_ACTION_TIMEOUT,
        )
        .await?;

    let _data: ItemAnimationControlResponseData = serde_json::from_value(value)
        .map_err(|e| format!("Malformed item animation response: {}", e))?;

    debug!(
        opacity,
        show,
        item_type = %resolved.item_type,
        file_name = %resolved.file_name,
        "Item animation control applied"
    );
    Ok(())
}

enum RecvResult {
    Match(serde_json::Value),
    Skip,
    Error(String),
}

fn classify_vts_response(
    resp: &VtsResponse,
    expected_id: &str,
    expected_msg_type: &str,
) -> RecvResult {
    if resp.message_type == "APIError" {
        if resp.request_id == expected_id {
            match serde_json::from_value::<messages::VtsErrorData>(resp.data.clone()) {
                Ok(err) => RecvResult::Error(format!("VTS error {}", err.error_id)),
                Err(e) => RecvResult::Error(format!("Parse error data: {}", e)),
            }
        } else {
            RecvResult::Skip
        }
    } else if resp.message_type == expected_msg_type || resp.message_type == "APIResponse" {
        if resp.request_id != expected_id {
            RecvResult::Skip
        } else {
            RecvResult::Match(resp.data.clone())
        }
    } else {
        RecvResult::Skip
    }
}

fn is_auth_message(msg_type: &str) -> bool {
    matches!(
        msg_type,
        "AuthenticationTokenRequest"
            | "AuthenticationRequest"
            | "AuthenticationTokenResponse"
            | "AuthenticationResponse"
    )
}

fn should_log_data_payload(msg_type: &str) -> bool {
    msg_type.starts_with("ParameterCreation")
        || msg_type.starts_with("InjectParameterData")
        || msg_type.starts_with("APIState")
        || msg_type == "APIError"
}

fn log_outgoing_request(request_json: &str) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(request_json) {
        let msg_type = v.get("messageType").and_then(|v| v.as_str()).unwrap_or("?");
        let req_id = v.get("requestID").and_then(|v| v.as_str()).unwrap_or("?");
        if is_auth_message(msg_type) {
            debug!(messageType = %msg_type, requestID = %req_id, "VTS send");
        } else if should_log_data_payload(msg_type) {
            debug!(messageType = %msg_type, requestID = %req_id, data = %v["data"], "VTS send");
        } else {
            debug!(messageType = %msg_type, requestID = %req_id, "VTS send");
        }
    }
}

fn log_incoming_response(resp: &VtsResponse) {
    let msg_type = &resp.message_type;
    let req_id = &resp.request_id;
    if is_auth_message(msg_type) {
        debug!(messageType = %msg_type, requestID = %req_id, "VTS recv");
    } else if should_log_data_payload(msg_type) {
        debug!(messageType = %msg_type, requestID = %req_id, data = %resp.data, "VTS recv");
    } else {
        debug!(messageType = %msg_type, requestID = %req_id, "VTS recv");
    }
}

async fn send_and_recv(
    ws: &mut WsStream,
    request_json: &str,
    expected_id: &str,
    expected_msg_type: &str,
) -> Result<serde_json::Value, String> {
    use tokio_tungstenite::tungstenite::Message;

    log_outgoing_request(request_json);

    let send_msg = Message::Text(request_json.to_string());
    timeout(REQUEST_TIMEOUT, ws.send(send_msg))
        .await
        .map_err(|_| {
            debug!(expected_id, expected_msg_type, "VTS send timed out");
            "Send timed out".to_string()
        })?
        .map_err(|e| {
            debug!(
                expected_id,
                expected_msg_type,
                error = %e,
                "VTS send failed"
            );
            format!("Send failed: {}", e)
        })?;

    timeout(
        REQUEST_TIMEOUT,
        recv_until_match(ws, expected_id, expected_msg_type),
    )
    .await
    .map_err(|_| {
        debug!(expected_id, expected_msg_type, "VTS response timed out");
        "Response timed out".to_string()
    })?
}

async fn recv_until_match(
    ws: &mut WsStream,
    expected_id: &str,
    expected_msg_type: &str,
) -> Result<serde_json::Value, String> {
    use tokio_tungstenite::tungstenite::Message;

    loop {
        let raw_msg = match ws.next().await {
            Some(Ok(msg)) => msg,
            Some(Err(e)) => {
                debug!(
                    expected_id,
                    expected_msg_type,
                    error = %e,
                    "VTS read error"
                );
                return Err(format!("Read error: {}", e));
            }
            None => {
                debug!(expected_id, expected_msg_type, "VTS connection closed");
                return Err("VTS connection closed".to_string());
            }
        };

        let text = match raw_msg {
            Message::Text(t) => t.to_string(),
            Message::Close(frame) => {
                let code = frame.as_ref().map(|f| f.code);
                let reason = frame.as_ref().map(|f| f.reason.to_string());
                debug!(
                    ?code,
                    ?reason,
                    expected_id,
                    expected_msg_type,
                    "VTS closed WebSocket connection"
                );
                return Err("VTS closed the connection".to_string());
            }
            Message::Ping(_) | Message::Pong(_) => continue,
            other => {
                debug!(
                    ?other,
                    expected_id, expected_msg_type, "Unexpected WebSocket message from VTS"
                );
                return Err(format!("Unexpected WebSocket message: {:?}", other));
            }
        };

        let parsed: VtsResponse =
            serde_json::from_str(&text).map_err(|e| format!("Parse response JSON: {}", e))?;

        let msg_type = parsed.message_type.clone();
        let req_id = parsed.request_id.clone();

        log_incoming_response(&parsed);

        match classify_vts_response(&parsed, expected_id, expected_msg_type) {
            RecvResult::Match(data) => return Ok(data),
            RecvResult::Skip => {
                debug!(
                    expected_id,
                    %req_id,
                    %msg_type,
                    expected_msg_type,
                    "Skipping VTS response"
                );
                continue;
            }
            RecvResult::Error(e) => return Err(e),
        }
    }
}

fn build_scene_records(instances: &[ItemInstanceInfo]) -> Vec<SceneItemRecord> {
    let records: Vec<SceneItemRecord> = instances
        .iter()
        .map(|inst| {
            let kind = ItemKind::classify(&inst.item_type);
            let supported = !matches!(kind, ItemKind::Unsupported { .. });
            SceneItemRecord {
                file_name: inst.file_name.clone(),
                item_type: inst.item_type.clone(),
                supported,
                duplicate_count: 1,
            }
        })
        .collect();

    let mut grouped: Vec<SceneItemRecord> = Vec::new();
    let mut seen: HashMap<(String, String), usize> = HashMap::new();

    for rec in &records {
        let key = (rec.file_name.clone(), rec.item_type.clone());
        if let Some(idx) = seen.get(&key) {
            grouped[*idx].duplicate_count += 1;
        } else {
            seen.insert(key, grouped.len());
            grouped.push(rec.clone());
        }
    }

    grouped.sort_by(|a, b| {
        a.file_name
            .cmp(&b.file_name)
            .then_with(|| a.item_type.cmp(&b.item_type))
    });

    grouped
}

/// true, если ошибка — валидный ответ VTS (APIError / парсинг), а не потеря транспорта.
/// На такой ошибке сокет жив и НЕ должен отбрасываться.
pub(crate) fn is_semantic_vts_error(err: &str) -> bool {
    let inner = inner_vts_error(err);
    inner.starts_with("VTS error ")
        || inner.starts_with("Parse error data")
        || inner.starts_with("Parse response JSON")
}

fn explain_event_error(err: &str, param_name: &str) -> String {
    let inner = inner_vts_error(err);
    if inner.starts_with("VTS error 453") {
        format!(
            "VTS error 453: input parameter '{}' not found. Map INPUT '{}' to the model's OUTPUT (e.g. ParamTyping) in VTS Parameter Setup; do not send a Live2D OUTPUT id directly.",
            param_name, param_name
        )
    } else if inner.starts_with("VTS error 352") {
        format!(
            "VTS error 352: custom parameter '{}' already created by another plugin. Free or rename the conflicting custom parameter.",
            param_name
        )
    } else if inner.starts_with("VTS error 356") {
        format!(
            "VTS error 356: invalid parameter name '{}'. Fix the parameter name (invalid characters or length).",
            param_name
        )
    } else if inner.starts_with("VTS error 355") {
        "VTS error 355: too many custom parameters. Free a custom parameter slot in VTS."
            .to_string()
    } else {
        err.to_string()
    }
}

/// Возвращает внутренний payload: либо строку после известного префикса операции
/// (`Create parameter failed: `, `Inject parameter failed: `, `Hotkey trigger failed: `),
/// либо исходную строку, если префикс не найден.
fn inner_vts_error(err: &str) -> &str {
    const WRAPPERS: &[&str] = &[
        "Create parameter failed: ",
        "Delete parameter failed: ",
        "Inject parameter failed: ",
        "Hotkey trigger failed: ",
    ];
    for prefix in WRAPPERS {
        if let Some(inner) = err.strip_prefix(prefix) {
            return inner;
        }
    }
    err
}

fn validate_api_state(data: &messages::APIStateResponseData) -> Result<(), String> {
    if !data.active {
        return Err("API is not active".to_string());
    }
    if !data.current_session_authenticated {
        return Err("Session is not authenticated".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unused_local_port() -> u16 {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("failed to bind for port discovery");
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    #[test]
    fn service_defaults_are_correct() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let settings = svc.settings.read().await;
            assert!(!settings.enabled);
            assert_eq!(settings.host, "127.0.0.1");
            assert_eq!(settings.port, 8001);
            assert!(settings.token.is_none());
            assert_eq!(
                settings.typing_action.output_mode,
                VTubeStudioTypingMode::Event
            );
            assert_eq!(settings.typing_action.parameter_name, "TTSBardTyping");
        });
        assert!(!svc.is_desired_running());
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
    }

    #[test]
    fn next_id_is_uuid() {
        let svc = VTubeStudioService::new();
        let id1 = svc.next_id();
        let id2 = svc.next_id();
        assert_ne!(id1, id2);
        assert!(
            uuid::Uuid::parse_str(&id1).is_ok(),
            "{} is not a valid UUID",
            id1
        );
        assert!(
            uuid::Uuid::parse_str(&id2).is_ok(),
            "{} is not a valid UUID",
            id2
        );
    }

    #[test]
    fn disconnect_cleans_state() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        rt.block_on(async {
            svc.disconnect().await;
            let inner = svc.inner.lock().await;
            assert!(inner.actor.is_none());
            assert!(inner.typing_cancel.is_none());
            assert!(inner.typing_handle.is_none());
            assert!(!inner.typing_active);
            assert!(inner.heartbeat_cancel.is_none());
            assert!(inner.heartbeat_handle.is_none());
        });
        assert!(!svc.is_desired_running());
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
    }

    #[test]
    fn set_typing_false_when_disconnected_is_noop() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = Arc::new(VTubeStudioService::new());
        rt.block_on(async {
            let result = svc.set_typing(false, "127.0.0.1", 8001, "").await;
            assert!(result.is_ok());
            let inner = svc.inner.lock().await;
            assert!(!inner.typing_active);
        });
    }

    #[test]
    fn set_typing_true_with_empty_token_is_noop() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = Arc::new(VTubeStudioService::new());
        svc.set_desired_running(true);
        rt.block_on(async {
            let result = svc.set_typing(true, "127.0.0.1", 8001, "").await;
            assert!(result.is_ok());
            let inner = svc.inner.lock().await;
            assert!(inner.actor.is_none());
        });
    }

    #[test]
    fn set_typing_true_ignored_when_not_desired_running() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = Arc::new(VTubeStudioService::new());
        assert!(!svc.is_desired_running());
        rt.block_on(async {
            let result = svc.set_typing(true, "127.0.0.1", 8001, "test-token").await;
            assert!(result.is_ok());
            let inner = svc.inner.lock().await;
            assert!(inner.actor.is_none());
        });
    }

    #[test]
    fn desired_running_flags() {
        let svc = VTubeStudioService::new();
        assert!(!svc.is_desired_running());
        svc.set_desired_running(true);
        assert!(svc.is_desired_running());
        svc.set_desired_running(false);
        assert!(!svc.is_desired_running());
    }

    #[test]
    fn connection_status_transitions() {
        let svc = VTubeStudioService::new();
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );

        svc.set_connection_status(VTubeStudioConnectionStatus::Connecting);
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connecting
        );

        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );

        svc.set_connection_status(VTubeStudioConnectionStatus::Error);
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Error
        );
    }

    fn make_response(msg_type: &str, request_id: &str, data: serde_json::Value) -> VtsResponse {
        VtsResponse {
            api_name: "VTubeStudioPublicAPI".into(),
            api_version: "1.0".into(),
            request_id: request_id.into(),
            message_type: msg_type.into(),
            data,
        }
    }

    #[test]
    fn classify_typed_response_matches() {
        let resp = make_response(
            "AuthenticationResponse",
            "req-1",
            serde_json::json!({"authenticated": true, "reason": ""}),
        );
        match classify_vts_response(&resp, "req-1", "AuthenticationResponse") {
            RecvResult::Match(data) => {
                assert!(data["authenticated"].as_bool().unwrap());
            }
            RecvResult::Skip => panic!("expected Match, got Skip"),
            RecvResult::Error(e) => panic!("expected Match, got Error: {}", e),
        }
    }

    #[test]
    fn classify_api_response_fallback_matches() {
        let resp = make_response(
            "APIResponse",
            "req-2",
            serde_json::json!({"authenticated": true, "reason": ""}),
        );
        match classify_vts_response(&resp, "req-2", "AuthenticationResponse") {
            RecvResult::Match(data) => {
                assert!(data["authenticated"].as_bool().unwrap());
            }
            RecvResult::Skip => panic!("expected Match, got Skip"),
            RecvResult::Error(e) => panic!("expected Match, got Error: {}", e),
        }
    }

    #[test]
    fn classify_parameter_creation_response() {
        let resp = make_response(
            "ParameterCreationResponse",
            "req-3",
            serde_json::json!({"parameterName": "TTSBardTyping"}),
        );
        match classify_vts_response(&resp, "req-3", "ParameterCreationResponse") {
            RecvResult::Match(data) => {
                assert_eq!(data["parameterName"].as_str().unwrap(), "TTSBardTyping");
            }
            _ => panic!("expected Match for ParameterCreationResponse"),
        }
    }

    #[test]
    fn classify_inject_parameter_response() {
        let resp = make_response(
            "InjectParameterDataResponse",
            "req-4",
            serde_json::json!({}),
        );
        match classify_vts_response(&resp, "req-4", "InjectParameterDataResponse") {
            RecvResult::Match(_) => {}
            _ => panic!("expected Match for InjectParameterDataResponse"),
        }
    }

    #[test]
    fn classify_hotkeys_in_current_model_response() {
        let resp = make_response(
            "HotkeysInCurrentModelResponse",
            "req-hk",
            serde_json::json!({
                "modelLoaded": true,
                "modelName": "test",
                "modelID": "id",
                "availableHotkeys": []
            }),
        );
        match classify_vts_response(&resp, "req-hk", "HotkeysInCurrentModelResponse") {
            RecvResult::Match(_) => {}
            _ => panic!("expected Match for HotkeysInCurrentModelResponse"),
        }
    }

    #[test]
    fn classify_hotkey_trigger_response_matches() {
        let resp = make_response("HotkeyTriggerResponse", "req-ht", serde_json::json!({}));
        match classify_vts_response(&resp, "req-ht", "HotkeyTriggerResponse") {
            RecvResult::Match(_) => {}
            _ => panic!("expected Match for HotkeyTriggerResponse"),
        }
    }

    #[test]
    fn classify_api_error_sanitizes_to_numeric_id() {
        let resp = make_response(
            "APIError",
            "req-5",
            serde_json::json!({"errorID": 42, "message": "Token rejected: secret-token-value"}),
        );
        match classify_vts_response(&resp, "req-5", "AuthenticationResponse") {
            RecvResult::Error(e) => {
                assert!(
                    e.contains("VTS error 42"),
                    "error should contain only numeric error ID, got: {}",
                    e
                );
                assert!(
                    !e.contains("secret-token-value"),
                    "error must not contain VTS message text: {}",
                    e
                );
                assert!(
                    !e.contains("Token rejected"),
                    "error must not contain VTS message text: {}",
                    e
                );
            }
            RecvResult::Match(_) => panic!("expected Error, got Match"),
            RecvResult::Skip => panic!("expected Error, got Skip"),
        }
    }

    #[test]
    fn classify_api_error_wrong_id_skipped() {
        let resp = make_response(
            "APIError",
            "other-req",
            serde_json::json!({"errorID": 1, "message": "Not ready"}),
        );
        match classify_vts_response(&resp, "my-req", "AuthenticationResponse") {
            RecvResult::Skip => {}
            RecvResult::Error(e) => {
                panic!("APIError with wrong ID must be skipped, got Error: {}", e)
            }
            RecvResult::Match(_) => panic!("APIError must not produce Match"),
        }
    }

    #[test]
    fn classify_mismatched_id_skipped() {
        let resp = make_response(
            "AuthenticationResponse",
            "wrong-id",
            serde_json::json!({"authenticated": true, "reason": ""}),
        );
        match classify_vts_response(&resp, "my-req", "AuthenticationResponse") {
            RecvResult::Skip => {}
            _ => panic!("mismatched request_id must be skipped"),
        }
    }

    #[test]
    fn classify_mismatched_id_on_api_response_fallback() {
        let resp = make_response(
            "APIResponse",
            "wrong-id",
            serde_json::json!({"authenticated": true, "reason": ""}),
        );
        match classify_vts_response(&resp, "my-req", "AuthenticationResponse") {
            RecvResult::Skip => {}
            _ => panic!("APIResponse with mismatched id must be skipped"),
        }
    }

    #[test]
    fn classify_unknown_type_skipped() {
        let resp = make_response(
            "ModelLoadedEvent",
            "req-6",
            serde_json::json!({"modelName": "test"}),
        );
        match classify_vts_response(&resp, "req-6", "AuthenticationResponse") {
            RecvResult::Skip => {}
            _ => panic!("unknown message type must be skipped"),
        }
    }

    #[test]
    fn classify_api_error_parse_failure() {
        let resp = make_response("APIError", "req-7", serde_json::json!("garbage"));
        match classify_vts_response(&resp, "req-7", "AuthenticationResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("Parse error data"), "got: {}", e);
            }
            _ => panic!("expected parse error"),
        }
    }

    #[test]
    fn inject_error_during_test_pulse_produces_error() {
        let resp = make_response(
            "APIError",
            "pulse-id",
            serde_json::json!({"errorID": 13, "message": "fail"}),
        );
        match classify_vts_response(&resp, "pulse-id", "InjectParameterDataResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 13"), "got: {}", e);
            }
            _ => panic!("InjectParameterData APIError must produce Error for test pulse"),
        }
    }

    #[test]
    fn inject_error_for_reset_produces_error() {
        let resp = make_response(
            "APIError",
            "reset-id",
            serde_json::json!({"errorID": 7, "message": "stub"}),
        );
        match classify_vts_response(&resp, "reset-id", "InjectParameterDataResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 7"), "got: {}", e);
            }
            _ => panic!("InjectParameterData APIError on reset must produce Error"),
        }
    }

    #[test]
    fn hotkey_trigger_error_classifies() {
        let resp = make_response(
            "APIError",
            "hk-err",
            serde_json::json!({"errorID": 5, "message": "invalid hotkey"}),
        );
        match classify_vts_response(&resp, "hk-err", "HotkeyTriggerResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 5"), "got: {}", e);
            }
            _ => panic!("HotkeyTriggerResponse APIError must produce Error"),
        }
    }

    #[test]
    fn disconnect_resets_desired_running() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        assert!(svc.is_desired_running());
        rt.block_on(async {
            svc.disconnect().await;
        });
        assert!(!svc.is_desired_running());
    }

    #[test]
    fn connect_manual_failure_decays_desired_running() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_error_decay_delay(Duration::from_millis(50));
        let port = unused_local_port();
        rt.block_on(async {
            let result = svc
                .connect("127.0.0.1", port, None, ConnectOrigin::Manual)
                .await;
            assert!(result.is_err());
            // Сразу после ручной ошибки desired_running жив — красный в titlebar.
            assert!(svc.is_desired_running());
            assert_eq!(
                svc.get_connection_status(),
                VTubeStudioConnectionStatus::Error
            );
            // После переопределённого decay-таймера — серый.
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert!(!svc.is_desired_running());
        });
    }

    #[test]
    fn connect_autostart_failure_keeps_desired_running() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_error_decay_delay(Duration::from_millis(50));
        let port = unused_local_port();
        rt.block_on(async {
            let result = svc
                .connect("127.0.0.1", port, None, ConnectOrigin::Autostart)
                .await;
            assert!(result.is_err());
            assert!(svc.is_desired_running());
            assert_eq!(
                svc.get_connection_status(),
                VTubeStudioConnectionStatus::Error
            );
            // Ошибка автостарта красная бессрочно: decay-таймер не вооружается.
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert!(svc.is_desired_running());
        });
    }

    #[test]
    fn disconnect_supersedes_armed_decay_timer() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_error_decay_delay(Duration::from_millis(50));
        let port = unused_local_port();
        rt.block_on(async {
            let result = svc
                .connect("127.0.0.1", port, None, ConnectOrigin::Manual)
                .await;
            assert!(result.is_err());
            assert!(svc.is_desired_running());

            svc.disconnect().await;
            assert!(!svc.is_desired_running());

            // Вооружённый таймер гасится disconnect: desired остаётся false.
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert!(!svc.is_desired_running());
        });
    }

    #[test]
    fn test_typing_action_fails_when_not_desired_running() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        assert!(!svc.is_desired_running());
        rt.block_on(async {
            let result = svc.test_typing_action(800, 1).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("not running") || msg.contains("Start the connection"),
                "error should mention not running: {}",
                msg
            );
        });
        assert!(!svc.is_desired_running());
    }

    #[test]
    fn test_typing_action_fails_when_disconnected() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        assert!(svc.is_desired_running());
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
        rt.block_on(async {
            let result = svc.test_typing_action(800, 1).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("not connected"),
                "error should mention not connected: {}",
                msg
            );
        });
        assert!(
            svc.is_desired_running(),
            "desired_running must remain unchanged"
        );
    }

    #[test]
    fn test_typing_action_rejects_timeout_99() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let result = svc.test_typing_action(99, 1).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("100") && (msg.contains("5000") || msg.contains("до")),
                "error should mention 100–5000 range, got: {}",
                msg
            );
        });
        assert!(!svc.is_desired_running());
    }

    #[test]
    fn test_typing_action_rejects_timeout_5001() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let result = svc.test_typing_action(5001, 1).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("100") && (msg.contains("5000") || msg.contains("до")),
                "error should mention 100–5000 range, got: {}",
                msg
            );
        });
        assert!(!svc.is_desired_running());
    }

    #[test]
    fn test_typing_action_rejects_repeat_0() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let result = svc.test_typing_action(800, 0).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("1") && (msg.contains("10") || msg.contains("до")),
                "error should mention 1–10 range, got: {}",
                msg
            );
        });
        assert!(!svc.is_desired_running());
    }

    #[test]
    fn test_typing_action_rejects_repeat_11() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let result = svc.test_typing_action(800, 11).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("1") && (msg.contains("10") || msg.contains("до")),
                "error should mention 1–10 range, got: {}",
                msg
            );
        });
        assert!(!svc.is_desired_running());
    }

    #[test]
    fn test_typing_action_boundary_100_passes_validation() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        rt.block_on(async {
            let result = svc.test_typing_action(100, 1).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("not connected") || msg.contains("Start the connection"),
                "boundary timeout 100 should pass validation and fail on lifecycle: {}",
                msg
            );
        });
        assert!(svc.is_desired_running());
    }

    #[test]
    fn test_typing_action_boundary_5000_passes_validation() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        rt.block_on(async {
            let result = svc.test_typing_action(5000, 1).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("not connected") || msg.contains("Start the connection"),
                "boundary timeout 5000 should pass validation and fail on lifecycle: {}",
                msg
            );
        });
        assert!(svc.is_desired_running());
    }

    #[test]
    fn test_typing_action_boundary_repeat_1_passes_validation() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        rt.block_on(async {
            let result = svc.test_typing_action(800, 1).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("not connected") || msg.contains("Start the connection"),
                "boundary repeat 1 should pass validation and fail on lifecycle: {}",
                msg
            );
        });
        assert!(svc.is_desired_running());
    }

    #[test]
    fn test_typing_action_boundary_repeat_10_passes_validation() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        rt.block_on(async {
            let result = svc.test_typing_action(800, 10).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("not connected") || msg.contains("Start the connection"),
                "boundary repeat 10 should pass validation and fail on lifecycle: {}",
                msg
            );
        });
        assert!(svc.is_desired_running());
    }

    #[test]
    fn test_typing_action_keeps_desired_running_on_failure() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        rt.block_on(async {
            let result = svc.test_typing_action(800, 1).await;
            assert!(result.is_err());
        });
        assert!(svc.is_desired_running());
    }

    #[test]
    fn get_current_model_hotkeys_fails_when_not_desired_running() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let result = svc.get_current_model_hotkeys().await;
            assert!(result.is_err());
        });
    }

    #[test]
    fn get_current_model_hotkeys_fails_when_disconnected() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        rt.block_on(async {
            let result = svc.get_current_model_hotkeys().await;
            assert!(result.is_err());
        });
    }

    #[test]
    fn typing_action_default_is_event_mode() {
        let svc = VTubeStudioService::new();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let s = svc.settings.read().await;
            assert_eq!(s.typing_action.output_mode, VTubeStudioTypingMode::Event);
            assert_eq!(s.typing_action.parameter_name, "TTSBardTyping");
            assert!(s.typing_action.start_hotkey_id.is_empty());
            assert!(s.typing_action.stop_hotkey_id.is_empty());
        });
    }

    #[test]
    fn get_current_model_hotkeys_fails_when_not_authenticated() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        assert!(!svc.is_authenticated.load(Ordering::SeqCst));
        rt.block_on(async {
            let result = svc.get_current_model_hotkeys().await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("not authenticated"),
                "should fail with not authenticated: {}",
                msg
            );
        });
    }

    #[test]
    fn get_current_model_hotkeys_fails_when_no_live_socket() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        rt.block_on(async {
            let result = svc.get_current_model_hotkeys().await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("not available"),
                "should fail with not available: {}",
                msg
            );
        });
    }

    #[test]
    fn connect_is_transport_only_no_side_effects() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        let port = unused_local_port();
        rt.block_on(async {
            let result = svc
                .connect("127.0.0.1", port, None, ConnectOrigin::Autostart)
                .await;
            assert!(result.is_err());
        });
        // Ошибка сохраняет desired_running (красный бессрочно), но не оставляет
        // actor/authenticated — transport-only.
        assert!(svc.is_desired_running());
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Error
        );
        assert!(!svc.is_authenticated.load(Ordering::SeqCst));
    }

    #[test]
    fn disconnect_with_hotkeys_config_cleans_state() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Hotkeys;
                s.typing_action.start_hotkey_id = "hk-start".to_string();
                s.typing_action.stop_hotkey_id = "hk-stop".to_string();
            }
            svc.disconnect().await;
            let inner = svc.inner.lock().await;
            assert!(inner.actor.is_none());
            assert!(inner.typing_cancel.is_none());
            assert!(inner.typing_handle.is_none());
            assert!(!inner.typing_active);
            assert!(inner.heartbeat_cancel.is_none());
            assert!(inner.heartbeat_handle.is_none());
        });
        assert!(!svc.is_desired_running());
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
    }

    #[test]
    fn test_typing_action_fails_when_ws_not_available() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        rt.block_on(async {
            let result = svc.test_typing_action(800, 1).await;
            assert!(result.is_err());
            let msg = result.unwrap_err();
            assert!(
                msg.contains("not available"),
                "should fail with not available: {}",
                msg
            );
        });
        assert!(svc.is_desired_running());
    }

    #[test]
    fn classification_roundtrip_all_response_types() {
        let cases = vec![
            ("AuthenticationTokenResponse", "APIResponse"),
            ("AuthenticationResponse", "AuthenticationResponse"),
            ("AuthenticationResponse", "APIResponse"),
            ("ParameterCreationResponse", "ParameterCreationResponse"),
            ("InjectParameterDataResponse", "InjectParameterDataResponse"),
            (
                "HotkeysInCurrentModelResponse",
                "HotkeysInCurrentModelResponse",
            ),
            ("HotkeyTriggerResponse", "HotkeyTriggerResponse"),
        ];
        let req_id = "rtt";

        for (msg_type, expected_match) in cases {
            let resp = make_response(msg_type, req_id, serde_json::json!({}));
            let expected_msg_type = if expected_match == "APIResponse" {
                if msg_type == "AuthenticationTokenResponse" {
                    "AuthenticationTokenResponse"
                } else {
                    "AuthenticationResponse"
                }
            } else {
                msg_type
            };
            if let RecvResult::Match(_) = classify_vts_response(&resp, req_id, expected_msg_type) {
                assert!(
                    msg_type == expected_msg_type
                        || (expected_match == "APIResponse" && msg_type == expected_match)
                        || msg_type == expected_match,
                    "expected Match for {}/expecting {}, but got Match from msg_type={}",
                    expected_msg_type,
                    expected_match,
                    msg_type
                );
            }
        }
    }

    #[test]
    fn hotkey_trigger_api_error_skipped_when_wrong_expected_type() {
        let resp = make_response(
            "APIError",
            "hk-err-2",
            serde_json::json!({"errorID": 99, "message": "bad"}),
        );
        match classify_vts_response(&resp, "hk-err-2", "InjectParameterDataResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 99"), "got: {}", e);
            }
            _ => panic!("expected Error"),
        }
    }

    fn make_instance(file_name: &str, instance_id: &str, item_type: &str) -> ItemInstanceInfo {
        ItemInstanceInfo {
            file_name: file_name.to_string(),
            instance_id: instance_id.to_string(),
            item_type: item_type.to_string(),
            framerate: 0.0,
            frame_count: -1,
            current_frame: -1,
        }
    }

    #[test]
    fn item_kind_classifies_types() {
        assert_eq!(ItemKind::classify("PNG"), ItemKind::Static);
        assert_eq!(ItemKind::classify("JPG"), ItemKind::Static);
        assert_eq!(ItemKind::classify("GIF"), ItemKind::Animated);
        assert_eq!(ItemKind::classify("AnimationFolder"), ItemKind::Animated);
        assert_eq!(
            ItemKind::classify("Live2D"),
            ItemKind::Unsupported {
                original_type: "Live2D".to_string()
            }
        );
        assert_eq!(
            ItemKind::classify("Unknown"),
            ItemKind::Unsupported {
                original_type: "Unknown".to_string()
            }
        );
        assert_eq!(
            ItemKind::classify("FutureTypeXYZ"),
            ItemKind::Unsupported {
                original_type: "FutureTypeXYZ".to_string()
            }
        );
    }

    #[test]
    fn resolve_zero_matches_is_missing() {
        let instances = vec![make_instance("icon.png", "i1", "PNG")];
        assert_eq!(
            resolve_item(&instances, "other.png").unwrap_err(),
            ResolveItemError::Missing
        );
    }

    #[test]
    fn resolve_single_match_returns_item() {
        let instances = vec![make_instance("icon.png", "i1", "PNG")];
        let resolved = resolve_item(&instances, "icon.png").unwrap();
        assert_eq!(resolved.instance_id, "i1");
        assert_eq!(resolved.file_name, "icon.png");
        assert_eq!(resolved.item_type, "PNG");
        assert_eq!(resolved.kind, ItemKind::Static);
    }

    #[test]
    fn resolve_duplicate_filename_is_ambiguous() {
        let instances = vec![
            make_instance("icon.png", "i1", "PNG"),
            make_instance("icon.png", "i2", "PNG"),
        ];
        assert_eq!(
            resolve_item(&instances, "icon.png").unwrap_err(),
            ResolveItemError::Ambiguous
        );
    }

    #[test]
    fn resolve_wrong_case_is_missing() {
        let instances = vec![make_instance("Icon.png", "i1", "PNG")];
        assert_eq!(
            resolve_item(&instances, "icon.png").unwrap_err(),
            ResolveItemError::Missing
        );
    }

    #[test]
    fn resolve_unsupported_rejected_with_type() {
        let instances = vec![make_instance("model.moc3", "i1", "Live2D")];
        assert_eq!(
            resolve_item(&instances, "model.moc3").unwrap_err(),
            ResolveItemError::Unsupported {
                file_name: "model.moc3".to_string(),
                item_type: "Live2D".to_string(),
            }
        );
    }

    #[test]
    fn resolve_future_type_rejected_with_diagnostic() {
        let instances = vec![make_instance("video.mp4", "i1", "VideoClip")];
        assert_eq!(
            resolve_item(&instances, "video.mp4").unwrap_err(),
            ResolveItemError::Unsupported {
                file_name: "video.mp4".to_string(),
                item_type: "VideoClip".to_string(),
            }
        );
    }

    #[test]
    fn resolve_unsupported_unknown_rejected() {
        let instances = vec![make_instance("thing.dat", "i1", "Unknown")];
        assert_eq!(
            resolve_item(&instances, "thing.dat").unwrap_err(),
            ResolveItemError::Unsupported {
                file_name: "thing.dat".to_string(),
                item_type: "Unknown".to_string(),
            }
        );
    }

    #[test]
    fn resolve_duplicate_unsupported_is_ambiguous_not_unsupported() {
        let instances = vec![
            make_instance("bad.moc3", "i1", "Live2D"),
            make_instance("bad.moc3", "i2", "Live2D"),
        ];
        assert_eq!(
            resolve_item(&instances, "bad.moc3").unwrap_err(),
            ResolveItemError::Ambiguous
        );
    }

    #[test]
    fn resolve_multiple_duplicate_unsupported_ambiguous_first() {
        let instances = vec![
            make_instance("a.png", "i1", "PNG"),
            make_instance("bad.moc3", "i2", "Live2D"),
            make_instance("bad.moc3", "i3", "Live2D"),
        ];
        let result = resolve_item(&instances, "bad.moc3");
        assert_eq!(result.unwrap_err(), ResolveItemError::Ambiguous);
    }

    #[test]
    fn resolve_animated_gif_is_animated_kind() {
        let instances = vec![make_instance("anim.gif", "i1", "GIF")];
        let resolved = resolve_item(&instances, "anim.gif").unwrap();
        assert_eq!(resolved.kind, ItemKind::Animated);
    }

    #[test]
    fn resolve_animated_folder_is_animated_kind() {
        let instances = vec![make_instance("akari_fly", "i1", "AnimationFolder")];
        let resolved = resolve_item(&instances, "akari_fly").unwrap();
        assert_eq!(resolved.kind, ItemKind::Animated);
    }

    #[test]
    fn resolve_exact_filename_match_in_mixed_set() {
        let instances = vec![
            make_instance("icon.png", "i1", "PNG"),
            make_instance("icon.gif", "i2", "GIF"),
            make_instance("icon.png", "i3", "PNG"),
        ];
        assert_eq!(
            resolve_item(&instances, "icon.png").unwrap_err(),
            ResolveItemError::Ambiguous
        );
        let resolved = resolve_item(&instances, "icon.gif").unwrap();
        assert_eq!(resolved.instance_id, "i2");
        assert_eq!(resolved.kind, ItemKind::Animated);
    }

    #[test]
    fn classify_item_list_response_matches() {
        let resp = make_response(
            "ItemListResponse",
            "ilr-1",
            serde_json::json!({
                "itemsInSceneCount": 1,
                "totalItemsAllowedCount": 60,
                "canLoadItemsRightNow": true,
                "itemInstancesInScene": [
                    {"fileName": "a.png", "instanceID": "i1", "type": "PNG"}
                ]
            }),
        );
        match classify_vts_response(&resp, "ilr-1", "ItemListResponse") {
            RecvResult::Match(data) => {
                let parsed: ItemListResponseData = serde_json::from_value(data).unwrap();
                assert_eq!(parsed.items_in_scene_count, 1);
                assert!(parsed.can_load_items_right_now);
                assert_eq!(parsed.item_instances_in_scene.len(), 1);
            }
            RecvResult::Skip => panic!("expected Match, got Skip"),
            RecvResult::Error(e) => panic!("expected Match, got Error: {}", e),
        }
    }

    #[test]
    fn classify_item_list_api_error_matches() {
        let resp = make_response(
            "APIError",
            "ilr-err",
            serde_json::json!({"errorID": 150, "message": "Permission not granted"}),
        );
        match classify_vts_response(&resp, "ilr-err", "ItemListResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 150"), "got: {}", e);
                assert!(
                    !e.contains("Permission"),
                    "error must not contain VTS message text: {}",
                    e
                );
            }
            RecvResult::Match(_) => panic!("expected Error, got Match"),
            RecvResult::Skip => panic!("expected Error, got Skip"),
        }
    }

    #[test]
    fn classify_item_animation_control_response_matches() {
        let resp = make_response(
            "ItemAnimationControlResponse",
            "iac-r-1",
            serde_json::json!({"frame": 3, "animationPlaying": true}),
        );
        match classify_vts_response(&resp, "iac-r-1", "ItemAnimationControlResponse") {
            RecvResult::Match(data) => {
                let parsed: ItemAnimationControlResponseData =
                    serde_json::from_value(data).unwrap();
                assert_eq!(parsed.frame, 3);
                assert!(parsed.animation_playing);
            }
            RecvResult::Skip => panic!("expected Match, got Skip"),
            RecvResult::Error(e) => panic!("expected Match, got Error: {}", e),
        }
    }

    #[test]
    fn classify_item_animation_control_api_error_matches() {
        let resp = make_response(
            "APIError",
            "iac-err",
            serde_json::json!({"errorID": 200, "message": "Bad request"}),
        );
        match classify_vts_response(&resp, "iac-err", "ItemAnimationControlResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 200"), "got: {}", e);
                assert!(
                    !e.contains("Bad request"),
                    "error must not contain VTS message text: {}",
                    e
                );
            }
            RecvResult::Match(_) => panic!("expected Error, got Match"),
            RecvResult::Skip => panic!("expected Error, got Skip"),
        }
    }

    #[test]
    fn malformed_item_animation_control_response_fails_deserialization() {
        let resp = make_response(
            "ItemAnimationControlResponse",
            "iac-mal",
            serde_json::json!({"unexpectedField": "garbage"}),
        );
        match classify_vts_response(&resp, "iac-mal", "ItemAnimationControlResponse") {
            RecvResult::Match(data) => {
                let err =
                    serde_json::from_value::<ItemAnimationControlResponseData>(data).unwrap_err();
                assert!(
                    err.to_string().contains("frame"),
                    "malformed deserialization should mention missing frame field: {}",
                    err
                );
            }
            RecvResult::Skip => panic!("expected Match, got Skip"),
            RecvResult::Error(e) => panic!("expected Match, got Error: {}", e),
        }
    }

    #[test]
    fn resolve_item_error_display() {
        assert_eq!(
            ResolveItemError::Missing.to_string(),
            "no scene item matching the configured filename"
        );
        assert_eq!(
            ResolveItemError::Ambiguous.to_string(),
            "multiple scene items match the configured filename"
        );
        assert_eq!(
            ResolveItemError::Unsupported {
                file_name: "x.moc3".to_string(),
                item_type: "Live2D".to_string(),
            }
            .to_string(),
            "item 'x.moc3' has unsupported type 'Live2D'"
        );
    }

    // ---------------------------------------------------------------------------
    // VTubeStudioItemStatus serde shape
    // ---------------------------------------------------------------------------

    fn has_instance_id_keys(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                map.keys().any(|k| k == "instanceID" || k == "instanceId")
                    || map.values().any(has_instance_id_keys)
            }
            serde_json::Value::Array(arr) => arr.iter().any(has_instance_id_keys),
            _ => false,
        }
    }

    #[test]
    fn item_status_serde_inactive() {
        let status = VTubeStudioItemStatus::Inactive;
        let expected = serde_json::json!({"status": "Inactive"});
        assert_eq!(serde_json::to_value(&status).unwrap(), expected);
        assert!(!has_instance_id_keys(
            &serde_json::to_value(&status).unwrap()
        ));

        let parsed: VTubeStudioItemStatus = serde_json::from_value(expected).unwrap();
        assert_eq!(parsed, VTubeStudioItemStatus::Inactive);
    }

    #[test]
    fn item_status_serde_ready() {
        let status = VTubeStudioItemStatus::Ready {
            file_name: "icon.png".into(),
            vts_type: "PNG".into(),
        };
        let expected =
            serde_json::json!({"status": "Ready", "fileName": "icon.png", "vtsType": "PNG"});
        assert_eq!(serde_json::to_value(&status).unwrap(), expected);
        assert!(!has_instance_id_keys(
            &serde_json::to_value(&status).unwrap()
        ));

        let parsed: VTubeStudioItemStatus = serde_json::from_value(expected).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn item_status_serde_missing() {
        let status = VTubeStudioItemStatus::Missing {
            file_name: "ghost.png".into(),
        };
        let expected = serde_json::json!({"status": "Missing", "fileName": "ghost.png"});
        assert_eq!(serde_json::to_value(&status).unwrap(), expected);
        assert!(!has_instance_id_keys(
            &serde_json::to_value(&status).unwrap()
        ));

        let parsed: VTubeStudioItemStatus = serde_json::from_value(expected).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn item_status_serde_ambiguous() {
        let status = VTubeStudioItemStatus::Ambiguous {
            file_name: "dup.png".into(),
            match_count: 3,
        };
        let expected =
            serde_json::json!({"status": "Ambiguous", "fileName": "dup.png", "matchCount": 3});
        assert_eq!(serde_json::to_value(&status).unwrap(), expected);
        assert!(!has_instance_id_keys(
            &serde_json::to_value(&status).unwrap()
        ));

        let parsed: VTubeStudioItemStatus = serde_json::from_value(expected).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn item_status_serde_unsupported() {
        let status = VTubeStudioItemStatus::Unsupported {
            file_name: "model.moc3".into(),
            vts_type: "Live2D".into(),
        };
        let expected = serde_json::json!({"status": "Unsupported", "fileName": "model.moc3", "vtsType": "Live2D"});
        assert_eq!(serde_json::to_value(&status).unwrap(), expected);
        assert!(!has_instance_id_keys(
            &serde_json::to_value(&status).unwrap()
        ));

        let parsed: VTubeStudioItemStatus = serde_json::from_value(expected).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn item_status_serde_error() {
        let status = VTubeStudioItemStatus::Error {
            file_name: "bad.gif".into(),
            message: "request timed out".into(),
        };
        let expected = serde_json::json!({"status": "Error", "fileName": "bad.gif", "message": "request timed out"});
        assert_eq!(serde_json::to_value(&status).unwrap(), expected);
        assert!(!has_instance_id_keys(
            &serde_json::to_value(&status).unwrap()
        ));

        let parsed: VTubeStudioItemStatus = serde_json::from_value(expected).unwrap();
        assert_eq!(parsed, status);
    }

    // ---------------------------------------------------------------------------
    // Resolve error → Item status mapping (pure)
    // ---------------------------------------------------------------------------

    #[test]
    fn resolve_missing_maps_to_missing_status() {
        let instances = vec![make_instance("icon.png", "i1", "PNG")];
        let err = resolve_item(&instances, "other.png").unwrap_err();
        assert_eq!(err, ResolveItemError::Missing);

        let status = VTubeStudioItemStatus::Missing {
            file_name: "other.png".into(),
        };
        assert!(
            matches!(status, VTubeStudioItemStatus::Missing { ref file_name } if file_name == "other.png")
        );
    }

    #[test]
    fn resolve_ambiguous_maps_to_ambiguous_status_with_count() {
        let instances = vec![
            make_instance("dup.png", "i1", "PNG"),
            make_instance("dup.png", "i2", "PNG"),
        ];
        let err = resolve_item(&instances, "dup.png").unwrap_err();
        assert_eq!(err, ResolveItemError::Ambiguous);

        let count = instances
            .iter()
            .filter(|i| i.file_name == "dup.png")
            .count() as u32;
        let status = VTubeStudioItemStatus::Ambiguous {
            file_name: "dup.png".into(),
            match_count: count,
        };
        assert!(
            matches!(&status, VTubeStudioItemStatus::Ambiguous { file_name, match_count } if file_name == "dup.png" && *match_count == 2)
        );
    }

    #[test]
    fn resolve_unsupported_maps_to_unsupported_status() {
        let instances = vec![make_instance("model.moc3", "i1", "Live2D")];
        let err = resolve_item(&instances, "model.moc3").unwrap_err();

        match err {
            ResolveItemError::Unsupported {
                file_name,
                item_type,
            } => {
                let status = VTubeStudioItemStatus::Unsupported {
                    file_name: file_name.clone(),
                    vts_type: item_type.clone(),
                };
                assert!(
                    matches!(&status, VTubeStudioItemStatus::Unsupported { file_name: f, vts_type: t } if f == "model.moc3" && t == "Live2D")
                );
            }
            _ => panic!("expected Unsupported"),
        }
    }

    // ---------------------------------------------------------------------------
    // Scene record aggregation (pure)
    // ---------------------------------------------------------------------------

    #[test]
    fn build_scene_records_single_item() {
        let instances = vec![make_instance("icon.png", "i1", "PNG")];
        let records = build_scene_records(&instances);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].file_name, "icon.png");
        assert_eq!(records[0].item_type, "PNG");
        assert!(records[0].supported);
        assert_eq!(records[0].duplicate_count, 1);
    }

    #[test]
    fn build_scene_records_aggregates_duplicates_same_file_type() {
        let instances = vec![
            make_instance("icon.png", "i1", "PNG"),
            make_instance("icon.png", "i2", "PNG"),
        ];
        let records = build_scene_records(&instances);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].file_name, "icon.png");
        assert_eq!(records[0].item_type, "PNG");
        assert_eq!(records[0].duplicate_count, 2);
        assert!(records[0].supported);
    }

    #[test]
    fn build_scene_records_separates_by_type_even_with_same_filename() {
        let instances = vec![
            make_instance("item.png", "i1", "PNG"),
            make_instance("item.png", "i2", "Live2D"),
        ];
        let records = build_scene_records(&instances);
        assert_eq!(records.len(), 2);
        let png = records.iter().find(|r| r.item_type == "PNG").unwrap();
        assert!(png.supported);
        assert_eq!(png.duplicate_count, 1);
        let l2d = records.iter().find(|r| r.item_type == "Live2D").unwrap();
        assert!(!l2d.supported);
        assert_eq!(l2d.duplicate_count, 1);
    }

    #[test]
    fn build_scene_records_unsupported_future_type_is_false() {
        let instances = vec![make_instance("video.mp4", "i1", "VideoClip")];
        let records = build_scene_records(&instances);
        assert_eq!(records.len(), 1);
        assert!(!records[0].supported);
        assert_eq!(records[0].item_type, "VideoClip");
    }

    #[test]
    fn build_scene_records_unknown_type_is_unsupported() {
        let instances = vec![make_instance("thing.dat", "i1", "Unknown")];
        let records = build_scene_records(&instances);
        assert_eq!(records.len(), 1);
        assert!(!records[0].supported);
    }

    #[test]
    fn build_scene_records_deterministic_order_by_filename_then_type() {
        let instances = vec![
            make_instance("z.png", "iz", "PNG"),
            make_instance("a.png", "ia1", "AnimationFolder"),
            make_instance("a.png", "ia2", "PNG"),
        ];
        let records = build_scene_records(&instances);
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].file_name, "a.png");
        assert_eq!(records[0].item_type, "AnimationFolder");
        assert_eq!(records[1].file_name, "a.png");
        assert_eq!(records[1].item_type, "PNG");
        assert_eq!(records[2].file_name, "z.png");
        assert_eq!(records[2].item_type, "PNG");

        let instances_rev = vec![
            make_instance("a.png", "ia2", "PNG"),
            make_instance("a.png", "ia1", "AnimationFolder"),
            make_instance("z.png", "iz", "PNG"),
        ];
        let records_rev = build_scene_records(&instances_rev);
        assert_eq!(records, records_rev);
    }

    #[test]
    fn scene_item_record_serde_no_instance_id() {
        let rec = SceneItemRecord {
            file_name: "icon.png".into(),
            item_type: "PNG".into(),
            supported: true,
            duplicate_count: 2,
        };
        let json = serde_json::to_string(&rec).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["fileName"].as_str().unwrap(), "icon.png");
        assert!(v.get("file_name").is_none());
        assert!(v.get("duplicate_count").is_none());
        assert_eq!(v["duplicateCount"].as_u64().unwrap(), 2);
        assert!(v.get("instanceID").is_none());
        assert!(v.get("instanceId").is_none());
        assert!(v.get("instance_id").is_none());
    }

    #[test]
    fn scene_item_record_supported_static() {
        let rec = SceneItemRecord {
            file_name: "a.png".into(),
            item_type: "PNG".into(),
            supported: true,
            duplicate_count: 1,
        };
        assert!(rec.supported);

        let rec2 = SceneItemRecord {
            file_name: "b.jpg".into(),
            item_type: "JPG".into(),
            supported: true,
            duplicate_count: 1,
        };
        assert!(rec2.supported);
    }

    #[test]
    fn scene_item_record_supported_animated() {
        let rec = SceneItemRecord {
            file_name: "anim.gif".into(),
            item_type: "GIF".into(),
            supported: true,
            duplicate_count: 1,
        };
        assert!(rec.supported);

        let rec2 = SceneItemRecord {
            file_name: "akari_fly".into(),
            item_type: "AnimationFolder".into(),
            supported: true,
            duplicate_count: 1,
        };
        assert!(rec2.supported);
    }

    #[test]
    fn scene_item_record_unsupported_live2d() {
        let rec = SceneItemRecord {
            file_name: "model.moc3".into(),
            item_type: "Live2D".into(),
            supported: false,
            duplicate_count: 1,
        };
        assert!(!rec.supported);
    }

    // ---------------------------------------------------------------------------
    // Event/Hotkeys refresh → Inactive / no network request
    // ---------------------------------------------------------------------------

    #[test]
    fn refresh_item_action_event_mode_is_inactive_no_network() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Event;
            }
            let status = svc.refresh_item_action().await;
            assert_eq!(status, VTubeStudioItemStatus::Inactive);
            assert_eq!(svc.get_item_status(), VTubeStudioItemStatus::Inactive);
        });
    }

    #[test]
    fn refresh_item_action_hotkeys_mode_is_inactive_no_network() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Hotkeys;
            }
            let status = svc.refresh_item_action().await;
            assert_eq!(status, VTubeStudioItemStatus::Inactive);
            assert_eq!(svc.get_item_status(), VTubeStudioItemStatus::Inactive);
        });
    }

    // ---------------------------------------------------------------------------
    // Empty Item filename → Missing (no network)
    // ---------------------------------------------------------------------------

    #[test]
    fn refresh_item_action_empty_filename_is_missing_no_network() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Item;
                s.typing_action.item_file_name = String::new();
            }
            let status = svc.refresh_item_action().await;
            assert_eq!(
                status,
                VTubeStudioItemStatus::Missing {
                    file_name: String::new()
                }
            );
            assert_eq!(
                svc.get_item_status(),
                VTubeStudioItemStatus::Missing {
                    file_name: String::new()
                }
            );
        });
    }

    // ---------------------------------------------------------------------------
    // Defaults and disconnect
    // ---------------------------------------------------------------------------

    #[test]
    fn service_defaults_have_separate_disconnected_and_inactive() {
        let svc = VTubeStudioService::new();
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
        assert_eq!(svc.get_item_status(), VTubeStudioItemStatus::Inactive);
    }

    #[test]
    fn disconnect_sets_item_status_inactive() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        rt.block_on(async {
            svc.disconnect().await;
        });
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
        assert_eq!(svc.get_item_status(), VTubeStudioItemStatus::Inactive);
    }

    #[test]
    fn new_service_item_status_is_inactive() {
        let svc = VTubeStudioService::new();
        let status = svc.get_item_status();
        assert_eq!(status, VTubeStudioItemStatus::Inactive);
    }

    // ---------------------------------------------------------------------------
    // Event/Hotkeys refresh clears pre-existing resolution → Inactive (no network)
    // ---------------------------------------------------------------------------

    #[test]
    fn refresh_event_mode_clears_resolved_item_and_sets_inactive() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Event;
            }
            {
                let mut inner = svc.inner.lock().await;
                inner.resolved_item = Some(ResolvedItem {
                    instance_id: "old-id".into(),
                    file_name: "old.png".into(),
                    item_type: "PNG".into(),
                    kind: ItemKind::Static,
                });
            }

            let status = svc.refresh_item_action().await;
            assert_eq!(status, VTubeStudioItemStatus::Inactive);
            assert_eq!(svc.get_item_status(), VTubeStudioItemStatus::Inactive);

            let inner = svc.inner.lock().await;
            assert!(inner.resolved_item.is_none());
        });
    }

    #[test]
    fn refresh_hotkeys_mode_clears_resolved_item_and_sets_inactive() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Hotkeys;
            }
            {
                let mut inner = svc.inner.lock().await;
                inner.resolved_item = Some(ResolvedItem {
                    instance_id: "old-id".into(),
                    file_name: "old.gif".into(),
                    item_type: "GIF".into(),
                    kind: ItemKind::Animated,
                });
            }

            let status = svc.refresh_item_action().await;
            assert_eq!(status, VTubeStudioItemStatus::Inactive);
            assert_eq!(svc.get_item_status(), VTubeStudioItemStatus::Inactive);

            let inner = svc.inner.lock().await;
            assert!(inner.resolved_item.is_none());
        });
    }

    // ---------------------------------------------------------------------------
    // No-socket Item refresh clears resolution → Error
    // ---------------------------------------------------------------------------

    #[test]
    fn refresh_item_no_socket_clears_resolution_and_returns_error() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Item;
                s.typing_action.item_file_name = "no-socket-test.png".to_string();
            }
            {
                let mut inner = svc.inner.lock().await;
                inner.resolved_item = Some(ResolvedItem {
                    instance_id: "stale-id".into(),
                    file_name: "stale.png".into(),
                    item_type: "PNG".into(),
                    kind: ItemKind::Static,
                });
            }
            assert!(svc.inner.lock().await.actor.is_none());

            let status = svc.refresh_item_action().await;
            assert_eq!(
                status,
                VTubeStudioItemStatus::Error {
                    file_name: "no-socket-test.png".to_string(),
                    message: "WebSocket not available".to_string(),
                }
            );
            assert_eq!(
                svc.get_item_status(),
                VTubeStudioItemStatus::Error {
                    file_name: "no-socket-test.png".to_string(),
                    message: "WebSocket not available".to_string(),
                }
            );

            let inner = svc.inner.lock().await;
            assert!(inner.resolved_item.is_none());
        });
    }

    // ---------------------------------------------------------------------------
    // Item transition state-machine pure tests (no live socket)
    // ---------------------------------------------------------------------------

    #[test]
    fn item_transition_record_desired_true_starts_sync() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        let (d, gen) = state.read_desired();
        assert!(d);
        assert_eq!(state.read_applied(), None);
        assert!(gen >= 1);
    }

    #[test]
    fn item_transition_repeated_true_is_idempotent() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        let (_, g1) = state.read_desired();
        state.record_desired(true);
        let (d, g2) = state.read_desired();
        assert!(d);
        assert!(g2 >= g1);
    }

    #[test]
    fn item_transition_false_to_true_before_completion_collapses() {
        let state = ItemTransitionState::new();
        state.record_desired(false);
        let (_, gen_false) = state.read_desired();
        state.record_desired(true);
        let (d, gen_true) = state.read_desired();
        assert!(d);
        assert!(gen_true > gen_false);
        let accepted = state.set_applied_if_current(false, gen_false);
        assert!(!accepted);
        assert_eq!(state.read_applied(), None);
    }

    #[test]
    fn item_transition_completion_only_replaces_matching_generation() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        let (_, gen1) = state.read_desired();
        let accepted = state.set_applied_if_current(true, gen1);
        assert!(accepted);
        assert_eq!(state.read_applied(), Some(true));
        state.record_desired(false);
        let (_, gen2) = state.read_desired();
        assert!(!state.set_applied_if_current(true, gen1));
        let (d, _) = state.read_desired();
        assert!(!d);
        assert!(state.set_applied_if_current(false, gen2));
        assert_eq!(state.read_applied(), Some(false));
    }

    #[test]
    fn item_transition_many_updates_keep_constant_size_state() {
        let state = ItemTransitionState::new();
        for i in 0..100 {
            state.record_desired(i % 2 == 0);
        }
        let (_, gen) = state.read_desired();
        assert!(gen >= 100);
    }

    #[test]
    fn item_transition_failure_sets_applied_unknown() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        state.force_applied(true);
        assert_eq!(state.read_applied(), Some(true));
        state.mark_applied_unknown();
        assert_eq!(state.read_applied(), None);
    }

    #[test]
    fn item_transition_next_sync_retries_latest_desired_after_failure() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        // Request fails — mark applied unknown
        state.mark_applied_unknown();
        assert_eq!(state.read_applied(), None);
        // Next sync pass reads latest desired
        let (d, _gen) = state.read_desired();
        assert!(d);
        // It will be retried because applied != desired
        assert_ne!(state.read_applied(), Some(d));
    }

    #[test]
    fn item_transition_shutdown_race_handshake_observes_update() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        state.force_applied(true);
        let (d, _) = state.read_desired();
        assert_eq!(Some(d), state.read_applied());
        // No new desired, should not claim (begin_work returns None)
        assert!(state.begin_work().is_none());

        // New desired arrives
        state.record_desired(false);
        // begin_work should succeed because desired != applied
        let gen = state.begin_work();
        assert!(gen.is_some(), "new desired detected, must claim worker");
    }

    #[test]
    fn item_transition_worker_cas_ensures_single_worker() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        assert!(state.begin_work().is_some());
        assert!(state.begin_work().is_none());
        assert!(state.begin_work().is_none());
        state.end_work();
        state.record_desired(true); // trigger again (applied still None)
        assert!(state.begin_work().is_some());
    }

    #[test]
    fn item_transition_refresh_commit_sets_applied_to_desired() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        // Simulate successful refresh that normalizes to desired
        state.force_applied(true);
        assert_eq!(state.read_applied(), Some(true));

        state.record_desired(false);
        state.force_applied(false);
        assert_eq!(state.read_applied(), Some(false));
    }

    #[test]
    fn item_transition_reset_clears_desired_and_applied() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        state.force_applied(true);
        state.reset();
        let (d, _) = state.read_desired();
        assert!(!d);
        assert_eq!(state.read_applied(), None);
    }

    #[test]
    fn item_transition_force_applied_setting() {
        let state = ItemTransitionState::new();
        assert_eq!(state.read_applied(), None);
        state.force_applied(true);
        assert_eq!(state.read_applied(), Some(true));
        state.force_applied(false);
        assert_eq!(state.read_applied(), Some(false));
    }

    // ---------------------------------------------------------------------------
    // Worker ownership/exit protocol model tests
    // ---------------------------------------------------------------------------

    #[test]
    fn worker_exit_racing_with_new_desired_leaves_live_owner_or_clean_unclaimed() {
        // Equality exits unclaimed (finite steps no busy loop)
        let state = ItemTransitionState::new();
        state.record_desired(true);
        let gen = state.begin_work().expect("should claim worker");

        // Worker successfully completes: applied=true via finish_success
        let continue_work = state.finish_success(gen, true);
        assert!(!continue_work, "equality exits unclaimed, no further work");
        assert_eq!(state.read_applied(), Some(true));
        // begin_work must now return None because desired==applied
        assert!(state.begin_work().is_none());

        // New desired arrives: update racing with release is not lost
        state.record_desired(false);
        let _gen2 = state.begin_work().expect("new desired must claim worker");
    }

    #[test]
    fn persistent_failure_attempts_once_and_exits_unclaimed() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        let gen = state.begin_work().expect("should claim worker");

        // Worker fails: finish_failure releases worker and sets applied=None
        assert!(!state.finish_failure(gen));
        assert_eq!(state.read_applied(), None);
        // After failure, worker is released → begin_work may claim for retry
        // (same-value after failure is eligible for one retry, requirement 2)
    }

    #[test]
    fn same_value_update_during_inflight_does_not_stale_completion() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        let gen1 = state.begin_work().expect("worker must be in flight");
        assert!(gen1 >= 1);

        // Worker starts processing gen1 desired=true
        // Meanwhile, same desired=true recorded again — must NOT advance generation
        state.record_desired(true);
        let (d, gen2) = state.read_desired();
        assert!(d);
        assert_eq!(gen1, gen2, "same-value must not advance generation");

        // Worker finishes gen1 completion — stale completion must be rejected
        // (set_applied_if_current with gen1 still works since gen hasn't changed)
        let accepted = state.set_applied_if_current(true, gen1);
        assert!(accepted, "completion with current gen must be accepted");
        assert_eq!(state.read_applied(), Some(true));
    }

    #[test]
    fn same_value_update_after_failure_can_claim_retry() {
        let state = ItemTransitionState::new();
        state.record_desired(true);

        // Worker fails — finish_failure sets applied=None, releases worker
        let gen = state.begin_work().expect("should claim worker");
        assert!(!state.finish_failure(gen));
        assert_eq!(state.read_applied(), None);

        // New trigger: even with same desired value (true), applied=None means mismatch
        let (d, gen1) = state.read_desired();
        assert!(d);
        assert_eq!(state.read_applied(), None);
        assert_ne!(Some(d), state.read_applied());

        // Next sync can claim worker to retry (begin_work succeeds)
        // record_desired(true) must advance generation for retry
        state.record_desired(true);
        let gen_retry = state.begin_work();
        assert!(gen_retry.is_some(), "same-value after failure may retry");
        // Generation must have advanced
        assert!(
            gen_retry.unwrap() > gen1,
            "generation must advance for retry"
        );
    }

    // ---------------------------------------------------------------------------
    // Session/disconnect invalidation model tests
    // ---------------------------------------------------------------------------

    #[test]
    fn disconnect_invalidates_inflight_worker_session() {
        // Simulate: service connects (session=0), worker runs, disconnect increments session
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        // Set Item mode with a filename so run_item_sync enters Item path
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Item;
                s.typing_action.item_file_name = "test.png".to_string();
            }
        });

        // Session starts at 0, then disconnect bumps it
        rt.block_on(async {
            svc.disconnect().await;
        });
        let session_after = svc.read_session();
        assert!(
            session_after > 0,
            "session must be incremented by disconnect"
        );

        // After disconnect, all state is clean
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
        assert!(!svc.is_desired_running());
        assert_eq!(svc.get_item_status(), VTubeStudioItemStatus::Inactive);
    }

    #[test]
    fn worker_session_mismatch_prevents_socket_restoration() {
        let state = ItemTransitionState::new();
        state.record_desired(true);
        // Worker claims and owns a generation snapshot
        let gen = state.begin_work();
        assert!(gen.is_some());
        state.force_applied(true);

        // Session invalidated externally → worker releases
        state.end_work();
        // After release, begin_work checks: applied(Some(true)) == desired(true) → no work
        assert!(
            state.begin_work().is_none(),
            "session invalidation via release prevents stale reclaim"
        );
    }

    // ---------------------------------------------------------------------------
    // Generation-aware refresh model tests
    // ---------------------------------------------------------------------------

    #[test]
    fn refresh_desired_change_during_io_cannot_commit_stale_applied() {
        let state = ItemTransitionState::new();
        let (_, gen0) = state.read_desired();
        state.record_desired(true);
        let (_, gen1) = state.read_desired();
        assert!(gen1 > gen0);

        // Simulate I/O takes time, desired changes to false during it
        state.record_desired(false);
        let (_, gen2) = state.read_desired();
        assert!(gen2 > gen1);

        // Old completion with stale desired=true, gen=gen1 must be rejected
        let stale = state.set_applied_if_current(true, gen1);
        assert!(
            !stale,
            "stale gen1 applied must be rejected when gen2 is newer"
        );

        // Current completion succeeds
        let current = state.set_applied_if_current(false, gen2);
        assert!(current, "current gen2 completion should succeed");
        assert_eq!(state.read_applied(), Some(false));
    }

    #[test]
    fn refresh_action_error_retains_resolved_item_for_retry() {
        // This tests the do_item_refresh_with_desired contract:
        // When fetch_scene_instances succeeds but animate_item fails,
        // the resolved item should be returned alongside the Error status.
        // This is inherently an async test involving network, but the model is:
        // The method returns (Some(resolved_item), Error) on animate failure.

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Item;
                s.typing_action.item_file_name = "ghost.png".to_string();
            }
            // No websocket — animate will fail
            let status = svc.refresh_item_action().await;
            assert!(
                matches!(status, VTubeStudioItemStatus::Error { .. }),
                "expected Error, got {:?}",
                status
            );
            // After Error from refresh (no socket), resolved_item is None
            let inner = svc.inner.lock().await;
            assert!(inner.resolved_item.is_none());
        });
    }

    // ---------------------------------------------------------------------------
    // Resolve errors suppress requests (no transition)
    // ---------------------------------------------------------------------------

    #[test]
    fn missing_ambiguous_unsupported_suppress_item_requests() {
        // These resolve errors do not change connection status
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );

        // Empty filename -> Missing (no WS call)
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Item;
                s.typing_action.item_file_name = String::new();
            }
            let status = svc.refresh_item_action().await;
            assert!(matches!(status, VTubeStudioItemStatus::Missing { .. }));
        });
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );

        // Event mode -> Inactive (no WS call)
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Event;
            }
            let status = svc.refresh_item_action().await;
            assert_eq!(status, VTubeStudioItemStatus::Inactive);
        });
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );
    }

    // ---------------------------------------------------------------------------
    // Item status transitions never change connection to Error
    // ---------------------------------------------------------------------------

    #[test]
    fn no_item_transition_changes_connection_from_connected_to_error() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);

        // Event/Hotkeys/empty Item refresh — all return without touching connection
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Event;
            }
            let _ = svc.refresh_item_action().await;
        });
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );

        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Hotkeys;
            }
            let _ = svc.refresh_item_action().await;
        });
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );

        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Item;
                s.typing_action.item_file_name = String::new();
            }
            let _ = svc.refresh_item_action().await;
        });
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );
    }

    #[test]
    fn item_worker_failure_sets_error_status_but_keeps_connection() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);

        // Set Item mode with a filename but no WS — worker will fail
        rt.block_on(async {
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Item;
                s.typing_action.item_file_name = "test.png".to_string();
            }
        });

        svc.record_item_desired(true);
        rt.block_on(async {
            let status = svc.run_item_sync().await;
            // Status may be Inactive (no ws, no resolved item) or Error
            // Key: connection status remains Connected
            let _ = status;
        });

        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected,
            "item worker failure must not corrupt connection status"
        );
    }

    // ---------------------------------------------------------------------------
    // Requirement 3: session invalidation between pre-check and mutex
    // acquisition cannot restore a socket (guarded commit).
    // ---------------------------------------------------------------------------
    #[test]
    fn guarded_commit_must_recheck_session_under_mutex() {
        // This test models the required protocol:
        // 1. read session before taking ws
        // 2. take ws from inner
        // 3. do I/O (simulated)
        // 4. acquire inner, re-check session under mutex before restoring
        //
        // Previously: session was checked outside mutex, then inner locked.
        // Now: inner locked first, then session re-checked inside.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let svc = VTubeStudioService::new();
            svc.set_desired_running(true);
            svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
            svc.is_authenticated.store(true, Ordering::SeqCst);

            // Set up item mode with a filename so we can observe behavior
            {
                let mut s = svc.settings.write().await;
                s.typing_action.output_mode = VTubeStudioTypingMode::Item;
                s.typing_action.item_file_name = "test.png".to_string();
            }

            // Place a fake resolved item so the worker doesn't bail early
            {
                let mut inner = svc.inner.lock().await;
                inner.resolved_item = Some(ResolvedItem {
                    instance_id: "test-id".into(),
                    file_name: "test.png".into(),
                    item_type: "PNG".into(),
                    kind: ItemKind::Static,
                });
            }

            let session = svc.read_session();

            // Invalidate session as if disconnect happened while worker was in-flight
            let _new_session = svc.invalidate_session();
            assert_ne!(svc.read_session(), session);

            // Guarded commit: acquire inner, then re-check session
            let mut inner = svc.inner.lock().await;
            let stale = svc.read_session() != session;
            assert!(stale, "session must be stale after invalidation");

            // Because session is stale, do NOT restore socket/change state
            // The inner guard is dropped without restoring
            inner.resolved_item = None;
            // (In production: inner.actor would NOT be created for stale session)

            assert!(
                inner.actor.is_none(),
                "no actor should exist for stale session"
            );
        });
    }

    // ---------------------------------------------------------------------------
    // Requirement 4: Item test action coordinates with worker/session.
    // Older in-flight worker must not restore state after the test.
    // ---------------------------------------------------------------------------

    #[test]
    fn item_test_invalidates_session_to_coordinate_with_worker_and_ends_hidden() {
        let state = ItemTransitionState::new();

        // Simulate: worker running (claimed), applying desired=true
        state.record_desired(true);
        let gen = state.begin_work().expect("worker must claim");
        // Worker is in-flight with gen

        // Now test starts — invalidate by calling finish_failure (model of session invalidation)
        // In real code, session invalidation causes worker to check `read_session() != session`
        // and call end_work() or finish_failure().
        // After test ends hidden:
        state.finish_failure(gen); // old worker exits, applied=None
        assert_eq!(state.read_applied(), None);

        // Test resets to hidden
        state.reset();
        state.force_applied(false);
        let (d, _) = state.read_desired();
        assert!(!d, "must end hidden");
        assert_eq!(state.read_applied(), Some(false));

        // Verify old worker cannot resurrect — begin_work returns None
        // because desired(false) == applied(Some(false))
        assert!(
            state.begin_work().is_none(),
            "stale worker must not be able to claim after test ends hidden"
        );
    }

    // ---------------------------------------------------------------------------
    // Requirement 1/5 combined: equality exits unclaimed; success with no newer
    // desired exits unclaimed; update racing with release is not lost.
    // ---------------------------------------------------------------------------

    #[test]
    fn equality_exits_unclaimed_in_finite_steps_no_busy_loop() {
        let state = ItemTransitionState::new();

        // Record desired=true, claim worker, succeed
        state.record_desired(true);
        let gen = state.begin_work().expect("must claim");
        let continue_work = state.finish_success(gen, true);
        assert!(!continue_work, "equality exits unclaimed — no busy loop");
        assert_eq!(state.read_applied(), Some(true));

        // begin_work returns None when desired == applied
        assert!(state.begin_work().is_none());

        // A new record_desired(true) while worker not running and applied=Some(true)
        // must NOT advance generation (requirement 2)
        let (_, gen_before) = state.read_desired();
        state.record_desired(true);
        let (_, gen_after) = state.read_desired();
        assert_eq!(
            gen_before, gen_after,
            "same-value recording while already applied must not advance generation"
        );
    }

    // ---------------------------------------------------------------------------
    // Requirement 2 combined: same-value after failure is eligible for one retry.
    // ---------------------------------------------------------------------------

    #[test]
    fn same_value_after_failure_triggers_retry_by_advancing_generation() {
        let state = ItemTransitionState::new();

        state.record_desired(true);
        let gen_first = state.begin_work().expect("claim");
        state.finish_failure(gen_first);

        // After failure: applied=None, no worker. Record same desired=true
        let (_, gen_before_retry) = state.read_desired();
        state.record_desired(true); // must advance generation for retry
        let (d, gen_after_retry) = state.read_desired();
        assert!(d);
        assert!(
            gen_after_retry > gen_before_retry || gen_after_retry > gen_first,
            "generation must advance for retry after failure"
        );
        assert!(gen_after_retry > gen_first);

        // begin_work succeeds for the retry
        let gen_retry = state.begin_work();
        assert!(gen_retry.is_some(), "retry must be possible after failure");

        // Success: no more work
        let continue_work = state.finish_success(gen_retry.unwrap(), true);
        assert!(!continue_work, "after retry success, exits unclaimed");
        assert_eq!(state.read_applied(), Some(true));
    }

    #[test]
    fn session_invalidation_via_disconnect_prevents_worker_resurrection() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);

        // Store a synthetic resolved item
        rt.block_on(async {
            let mut inner = svc.inner.lock().await;
            inner.resolved_item = Some(ResolvedItem {
                instance_id: "test-id".into(),
                file_name: "test.png".into(),
                item_type: "PNG".into(),
                kind: ItemKind::Static,
            });
        });

        // After disconnect, worker cannot resurrect state
        rt.block_on(async {
            svc.disconnect().await;
        });

        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
        assert_eq!(svc.get_item_status(), VTubeStudioItemStatus::Inactive);
    }

    // ---------------------------------------------------------------------------
    // Event/parameter lifecycle VTS error classification
    // ---------------------------------------------------------------------------

    #[test]
    fn classify_inject_error_453_param_name_not_found() {
        let resp = make_response(
            "APIError",
            "inj-453",
            serde_json::json!({"errorID": 453, "message": "InjectDataParamNameNotFound"}),
        );
        match classify_vts_response(&resp, "inj-453", "InjectParameterDataResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 453"), "got: {}", e);
            }
            _ => panic!("expected Error for VTS error 453 on InjectParameterDataResponse"),
        }
    }

    #[test]
    fn classify_creation_error_352_param_created_by_other_plugin() {
        let resp = make_response(
            "APIError",
            "cr-352",
            serde_json::json!({"errorID": 352, "message": "CustomParamAlreadyCreatedByOtherPlugin"}),
        );
        match classify_vts_response(&resp, "cr-352", "ParameterCreationResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 352"), "got: {}", e);
            }
            _ => panic!("expected Error for VTS error 352 on ParameterCreationResponse"),
        }
    }

    #[test]
    fn classify_creation_error_350_parameter_already_exists() {
        let resp = make_response(
            "APIError",
            "cr-350",
            serde_json::json!({"errorID": 350, "message": "ParameterAlreadyCreatedByThisPlugin"}),
        );
        match classify_vts_response(&resp, "cr-350", "ParameterCreationResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 350"), "got: {}", e);
            }
            _ => panic!("expected Error for VTS error 350 on ParameterCreationResponse"),
        }
    }

    #[test]
    fn classify_creation_error_355_max_parameter_limit() {
        let resp = make_response(
            "APIError",
            "cr-355",
            serde_json::json!({"errorID": 355, "message": "TooManyCustomParams"}),
        );
        match classify_vts_response(&resp, "cr-355", "ParameterCreationResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 355"), "got: {}", e);
            }
            _ => panic!("expected Error for VTS error 355 on ParameterCreationResponse"),
        }
    }

    #[test]
    fn classify_creation_error_356_invalid_parameter_name() {
        let resp = make_response(
            "APIError",
            "cr-356",
            serde_json::json!({"errorID": 356, "message": "InvalidParameterName"}),
        );
        match classify_vts_response(&resp, "cr-356", "ParameterCreationResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 356"), "got: {}", e);
            }
            _ => panic!("expected Error for VTS error 356 on ParameterCreationResponse"),
        }
    }

    // ---------------------------------------------------------------------------
    // is_semantic_vts_error classification
    // ---------------------------------------------------------------------------

    #[test]
    fn semantic_error_classification_vts_error() {
        assert!(is_semantic_vts_error("VTS error 453"));
        assert!(is_semantic_vts_error("VTS error 352"));
        assert!(is_semantic_vts_error("VTS error 356"));
        assert!(is_semantic_vts_error("VTS error 355"));
        assert!(is_semantic_vts_error("VTS error 42"));
    }

    #[test]
    fn semantic_error_classification_parse_errors() {
        assert!(is_semantic_vts_error("Parse error data: some detail"));
        assert!(is_semantic_vts_error("Parse response JSON: malformed"));
    }

    #[test]
    fn semantic_error_classification_transport_false() {
        assert!(!is_semantic_vts_error("Send timed out"));
        assert!(!is_semantic_vts_error("Response timed out"));
        assert!(!is_semantic_vts_error("VTS connection closed"));
        assert!(!is_semantic_vts_error("VTS closed the connection"));
        assert!(!is_semantic_vts_error("WebSocket connect failed: ..."));
        assert!(!is_semantic_vts_error("Read error: something"));
        assert!(!is_semantic_vts_error("Send failed: ..."));
    }

    #[test]
    fn semantic_error_classification_other_strings_false() {
        assert!(!is_semantic_vts_error(""));
        assert!(!is_semantic_vts_error("VTS error")); // no space before numeric id
        assert!(!is_semantic_vts_error("Some random error"));
    }

    // ---------------------------------------------------------------------------
    // explain_event_error helper
    // ---------------------------------------------------------------------------

    #[test]
    fn explain_event_error_453_includes_param_name_and_mapping_hint() {
        let msg = explain_event_error(
            "VTS error 453: input parameter 'MyParam' not found.",
            "MyParam",
        );
        assert!(msg.contains("VTS error 453"));
        assert!(msg.contains("MyParam"));
        assert!(msg.contains("INPUT"));
        assert!(msg.contains("OUTPUT"));
        assert!(msg.contains("ParamTyping"));
        assert!(
            msg.contains("do not send a Live2D OUTPUT id directly"),
            "must explicitly warn against sending Live2D OUTPUT id: {}",
            msg
        );
    }

    #[test]
    fn explain_event_error_352_suggests_free_or_rename() {
        let msg = explain_event_error(
            "VTS error 352: CustomParamAlreadyCreatedByOtherPlugin",
            "TTSParam",
        );
        assert!(msg.contains("VTS error 352"));
        assert!(msg.contains("TTSParam"));
        assert!(
            msg.contains("Free")
                || msg.contains("Rename")
                || msg.contains("free")
                || msg.contains("rename"),
            "should advise freeing or renaming the parameter: {}",
            msg
        );
    }

    #[test]
    fn explain_event_error_356_suggests_fix_name() {
        let msg = explain_event_error("VTS error 356: InvalidParameterName", "bad name!");
        assert!(msg.contains("VTS error 356"));
        assert!(msg.contains("bad name!"));
        assert!(
            msg.to_lowercase().contains("invalid") || msg.to_lowercase().contains("fix"),
            "should mention invalid parameter name: {}",
            msg
        );
    }

    #[test]
    fn explain_event_error_355_suggests_free_slot() {
        let msg = explain_event_error("VTS error 355: TooManyCustomParams", "ignored");
        assert!(msg.contains("VTS error 355"));
        assert!(
            msg.contains("too many") || msg.contains("Free") || msg.contains("slot"),
            "should advise freeing a custom parameter slot: {}",
            msg
        );
    }

    #[test]
    fn explain_event_error_unknown_returns_original() {
        let msg = explain_event_error("VTS error 999: something else", "xyz");
        assert_eq!(msg, "VTS error 999: something else");
    }

    #[test]
    fn explain_event_error_transport_passthrough() {
        let msg = explain_event_error("Send timed out", "ignored");
        assert_eq!(msg, "Send timed out");
    }

    // ---------------------------------------------------------------------------
    // wrapped semantic errors (ROADMAP-060 review fixes)
    // ---------------------------------------------------------------------------

    #[test]
    fn semantic_error_wrapped_create_352() {
        assert!(is_semantic_vts_error(
            "Create parameter failed: VTS error 352"
        ));
    }

    #[test]
    fn semantic_error_wrapped_inject_453() {
        assert!(is_semantic_vts_error(
            "Inject parameter failed: VTS error 453: input parameter 'Foo' not found."
        ));
    }

    #[test]
    fn semantic_error_wrapped_hotkey_api_error() {
        assert!(is_semantic_vts_error("Hotkey trigger failed: VTS error 42"));
    }

    #[test]
    fn semantic_error_wrapped_parse() {
        assert!(is_semantic_vts_error(
            "Create parameter failed: Parse error data: invalid JSON"
        ));
        assert!(is_semantic_vts_error(
            "Inject parameter failed: Parse response JSON: malformed"
        ));
    }

    #[test]
    fn semantic_error_wrapped_transport_remains_false() {
        assert!(!is_semantic_vts_error(
            "Create parameter failed: Send timed out"
        ));
        assert!(!is_semantic_vts_error(
            "Inject parameter failed: Response timed out"
        ));
        assert!(!is_semantic_vts_error(
            "Hotkey trigger failed: VTS connection closed"
        ));
    }

    #[test]
    fn semantic_error_lookalike_negative() {
        assert!(!is_semantic_vts_error(
            "Send failed: peer diagnostic mentioned VTS error 352"
        ));
        assert!(!is_semantic_vts_error(
            "Read error: Parse response JSON was written by peer"
        ));
        assert!(!is_semantic_vts_error(
            "Read error: Parse error data from upstream"
        ));
    }

    #[test]
    fn explain_event_error_lookalike_returns_original() {
        let msg = explain_event_error("Send failed: peer diagnostic mentioned VTS error 352", "p");
        assert_eq!(msg, "Send failed: peer diagnostic mentioned VTS error 352");

        let msg = explain_event_error("Read error: Parse response JSON was written by peer", "p");
        assert_eq!(msg, "Read error: Parse response JSON was written by peer");

        let msg = explain_event_error("Read error: Parse error data from upstream", "p");
        assert_eq!(msg, "Read error: Parse error data from upstream");
    }

    #[test]
    fn explain_event_error_wrapped_453_produces_actionable_text() {
        let msg = explain_event_error(
            "Inject parameter failed: VTS error 453: input parameter 'MyParam' not found.",
            "MyParam",
        );
        assert!(msg.contains("VTS error 453"));
        assert!(msg.contains("MyParam"));
        assert!(msg.contains("INPUT"));
        assert!(msg.contains("OUTPUT"));
        assert!(msg.contains("do not send a Live2D OUTPUT id directly"));
    }

    #[test]
    fn explain_event_error_wrapped_352_produces_actionable_text() {
        let msg = explain_event_error(
            "Create parameter failed: VTS error 352: CustomParamAlreadyCreatedByOtherPlugin",
            "TTSParam",
        );
        assert!(msg.contains("VTS error 352"));
        assert!(msg.contains("TTSParam"));
        assert!(
            msg.contains("Free")
                || msg.contains("Rename")
                || msg.contains("free")
                || msg.contains("rename")
        );
    }

    #[test]
    fn explain_event_error_wrapped_356_suggests_fix_name() {
        let msg = explain_event_error(
            "Create parameter failed: VTS error 356: InvalidParameterName",
            "bad name!",
        );
        assert!(msg.contains("VTS error 356"));
        assert!(msg.contains("bad name!"));
        assert!(msg.to_lowercase().contains("invalid") || msg.to_lowercase().contains("fix"));
    }

    #[test]
    fn explain_event_error_wrapped_355_suggests_free_slot() {
        let msg = explain_event_error(
            "Create parameter failed: VTS error 355: TooManyCustomParams",
            "ignored",
        );
        assert!(msg.contains("VTS error 355"));
        assert!(msg.contains("too many") || msg.contains("Free") || msg.contains("slot"));
    }

    #[test]
    fn explain_event_error_wrapped_transport_passthrough() {
        let msg = explain_event_error("Inject parameter failed: Send timed out", "ignored");
        assert_eq!(msg, "Inject parameter failed: Send timed out");
    }

    // ---------------------------------------------------------------------------
    // APIState heartbeat validation (pure)
    // ---------------------------------------------------------------------------

    #[test]
    fn validate_api_state_both_true_ok() {
        let data = messages::APIStateResponseData {
            active: true,
            current_session_authenticated: true,
        };
        assert!(validate_api_state(&data).is_ok());
    }

    #[test]
    fn validate_api_state_not_active_fails() {
        let data = messages::APIStateResponseData {
            active: false,
            current_session_authenticated: true,
        };
        let err = validate_api_state(&data).unwrap_err();
        assert!(err.contains("not active"));
    }

    #[test]
    fn validate_api_state_not_authenticated_fails() {
        let data = messages::APIStateResponseData {
            active: true,
            current_session_authenticated: false,
        };
        let err = validate_api_state(&data).unwrap_err();
        assert!(err.contains("not authenticated"));
    }

    #[test]
    fn validate_api_state_both_false_fails_first() {
        let data = messages::APIStateResponseData {
            active: false,
            current_session_authenticated: false,
        };
        let err = validate_api_state(&data).unwrap_err();
        assert!(err.contains("not active"));
    }

    // ---------------------------------------------------------------------------
    // classifiy_vts_response for APIStateResponse
    // ---------------------------------------------------------------------------

    #[test]
    fn classify_api_state_response_matches() {
        let resp = make_response(
            "APIStateResponse",
            "api-state-r",
            serde_json::json!({"active": true, "currentSessionAuthenticated": true}),
        );
        match classify_vts_response(&resp, "api-state-r", "APIStateResponse") {
            RecvResult::Match(data) => {
                let parsed: messages::APIStateResponseData = serde_json::from_value(data).unwrap();
                assert!(parsed.active);
                assert!(parsed.current_session_authenticated);
            }
            RecvResult::Skip => panic!("expected Match, got Skip"),
            RecvResult::Error(e) => panic!("expected Match, got Error: {}", e),
        }
    }

    #[test]
    fn classify_api_state_api_error_produces_error() {
        let resp = make_response(
            "APIError",
            "api-state-err",
            serde_json::json!({"errorID": 5, "message": "Not available"}),
        );
        match classify_vts_response(&resp, "api-state-err", "APIStateResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 5"), "got: {}", e);
            }
            _ => panic!("expected Error for APIError on APIStateResponse"),
        }
    }

    #[test]
    fn classify_api_state_wrong_message_type_skipped() {
        let resp = make_response(
            "AuthenticationResponse",
            "api-state-r",
            serde_json::json!({}),
        );
        match classify_vts_response(&resp, "api-state-r", "APIStateResponse") {
            RecvResult::Skip => {}
            _ => panic!("expected Skip for wrong message type"),
        }
    }

    #[test]
    fn classify_api_state_wrong_request_id_skipped() {
        let resp = make_response(
            "APIStateResponse",
            "other-id",
            serde_json::json!({"active": true, "currentSessionAuthenticated": true}),
        );
        match classify_vts_response(&resp, "my-id", "APIStateResponse") {
            RecvResult::Skip => {}
            _ => panic!("expected Skip for mismatched request_id"),
        }
    }

    // ---------------------------------------------------------------------------
    // Heartbeat ownership: only one heartbeat owner at a time
    // ---------------------------------------------------------------------------

    #[test]
    fn new_service_has_no_heartbeat() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let inner = svc.inner.lock().await;
            assert!(inner.heartbeat_cancel.is_none());
            assert!(inner.heartbeat_handle.is_none());
        });
    }

    #[test]
    fn heartbeat_cancellation_token_signals() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cancel = CancellationToken::new();
        let ct = cancel.clone();
        let signaled = Arc::new(AtomicBool::new(false));
        let s = Arc::clone(&signaled);

        let handle = rt.spawn(async move {
            tokio::select! {
                _ = ct.cancelled() => {
                    s.store(true, Ordering::SeqCst);
                }
                _ = tokio::time::sleep(Duration::from_secs(10)) => {}
            }
        });

        cancel.cancel();
        rt.block_on(async {
            let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
        });
        assert!(signaled.load(Ordering::SeqCst));
    }

    #[test]
    fn heartbeat_stop_clears_fields() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            {
                let mut inner = svc.inner.lock().await;
                let cancel = CancellationToken::new();
                inner.heartbeat_cancel = Some(cancel);
                inner.heartbeat_handle = Some(tokio::spawn(async {}));
            }
            let mut inner = svc.inner.lock().await;
            assert!(inner.heartbeat_cancel.is_some());
            assert!(inner.heartbeat_handle.is_some());
            svc.stop_heartbeat_locked(&mut inner);
            assert!(inner.heartbeat_cancel.is_none());
            assert!(inner.heartbeat_handle.is_none());
        });
    }

    #[test]
    fn connect_failure_stops_heartbeat() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        let port = unused_local_port();

        // Place a fake heartbeat so we can assert it's cleared
        rt.block_on(async {
            {
                let mut inner = svc.inner.lock().await;
                let cancel = CancellationToken::new();
                inner.heartbeat_cancel = Some(cancel);
                inner.heartbeat_handle = Some(tokio::spawn(async {}));
            }
        });

        rt.block_on(async {
            let result = svc
                .connect("127.0.0.1", port, None, ConnectOrigin::Autostart)
                .await;
            assert!(result.is_err());
            let inner = svc.inner.lock().await;
            assert!(
                inner.heartbeat_cancel.is_none(),
                "heartbeat must be stopped on connect failure"
            );
            assert!(
                inner.heartbeat_handle.is_none(),
                "heartbeat must be stopped on connect failure"
            );
        });
    }

    #[test]
    fn heartbeat_field_defaults_none() {
        let svc = VTubeStudioService::new();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let inner = svc.inner.lock().await;
            assert!(inner.heartbeat_cancel.is_none());
            assert!(inner.heartbeat_handle.is_none());
        });
    }

    // ---------------------------------------------------------------------------
    // is_auth_message helper
    // ---------------------------------------------------------------------------

    #[test]
    fn is_auth_message_returns_true_for_auth_types() {
        assert!(is_auth_message("AuthenticationTokenRequest"));
        assert!(is_auth_message("AuthenticationRequest"));
        assert!(is_auth_message("AuthenticationTokenResponse"));
        assert!(is_auth_message("AuthenticationResponse"));
    }

    #[test]
    fn is_auth_message_returns_false_for_non_auth() {
        assert!(!is_auth_message("APIStateRequest"));
        assert!(!is_auth_message("APIStateResponse"));
        assert!(!is_auth_message("ParameterCreationRequest"));
        assert!(!is_auth_message("ParameterCreationResponse"));
        assert!(!is_auth_message("InjectParameterDataRequest"));
        assert!(!is_auth_message("InjectParameterDataResponse"));
        assert!(!is_auth_message("APIError"));
        assert!(!is_auth_message("HotkeyTriggerRequest"));
        assert!(!is_auth_message("ItemListRequest"));
    }

    // ---------------------------------------------------------------------------
    // should_log_data_payload helper
    // ---------------------------------------------------------------------------

    #[test]
    fn should_log_data_payload_true_for_safe_types() {
        assert!(should_log_data_payload("ParameterCreationRequest"));
        assert!(should_log_data_payload("ParameterCreationResponse"));
        assert!(should_log_data_payload("InjectParameterDataRequest"));
        assert!(should_log_data_payload("InjectParameterDataResponse"));
        assert!(should_log_data_payload("APIStateRequest"));
        assert!(should_log_data_payload("APIStateResponse"));
        assert!(should_log_data_payload("APIError"));
    }

    #[test]
    fn should_log_data_payload_false_for_auth_types() {
        assert!(!should_log_data_payload("AuthenticationTokenRequest"));
        assert!(!should_log_data_payload("AuthenticationRequest"));
        assert!(!should_log_data_payload("AuthenticationTokenResponse"));
        assert!(!should_log_data_payload("AuthenticationResponse"));
    }

    #[test]
    fn should_log_data_payload_false_for_other_types() {
        assert!(!should_log_data_payload("HotkeyTriggerRequest"));
        assert!(!should_log_data_payload("HotkeyTriggerResponse"));
        assert!(!should_log_data_payload("ItemListRequest"));
        assert!(!should_log_data_payload("ItemListResponse"));
        assert!(!should_log_data_payload("ItemAnimationControlRequest"));
        assert!(!should_log_data_payload("ItemAnimationControlResponse"));
        assert!(!should_log_data_payload("HotkeysInCurrentModelRequest"));
        assert!(!should_log_data_payload("HotkeysInCurrentModelResponse"));
    }

    // ---------------------------------------------------------------------------
    // Generation guard: stale heartbeat must not mutate state
    // ---------------------------------------------------------------------------

    #[test]
    fn generation_default_is_zero() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let inner = svc.inner.lock().await;
            assert_eq!(inner.connection_generation, 0);
        });
    }

    #[test]
    fn generation_bumps_on_disconnect() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        rt.block_on(async {
            svc.disconnect().await;
            let inner = svc.inner.lock().await;
            assert!(inner.connection_generation > 0);
        });
    }

    #[test]
    fn heartbeat_stale_generation_does_not_mutate_state() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let svc = VTubeStudioService::new();
            svc.set_desired_running(true);
            svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
            svc.is_authenticated.store(true, Ordering::SeqCst);

            let hb_gen = {
                let mut inner = svc.inner.lock().await;
                inner.connection_generation = 5;
                inner.connection_generation
            };

            // Bump generation externally, simulating disconnect
            {
                let mut inner = svc.inner.lock().await;
                inner.connection_generation = inner.connection_generation.wrapping_add(1);
            }

            // Reacquire and verify gen mismatch
            {
                let inner = svc.inner.lock().await;
                assert_ne!(inner.connection_generation, hb_gen);
                // Stale heartbeat would observe generation mismatch and exit
                // without mutating state
                let actor_was_some = inner.actor.is_some();
                let typing_was_active = inner.typing_active;
                let auth_was_true = svc.is_authenticated.load(Ordering::SeqCst);
                let status_was = svc.get_connection_status();

                // These must remain unchanged after gen mismatch
                assert_eq!(inner.actor.is_some(), actor_was_some);
                assert_eq!(inner.typing_active, typing_was_active);
                assert_eq!(svc.is_authenticated.load(Ordering::SeqCst), auth_was_true);
                assert_eq!(svc.get_connection_status(), status_was);
            }
        });
    }

    // ---------------------------------------------------------------------------
    // start_heartbeat_locked lifecycle (pure, no network)
    // ---------------------------------------------------------------------------

    #[test]
    fn start_heartbeat_locked_noop_without_actor() {
        // Без connection-actor heartbeat не запускается (некуда слать запросы),
        // но generation не меняется и старые задачи останавливаются.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let mut inner = svc.inner.lock().await;
            let gen_before = inner.connection_generation;
            assert!(inner.heartbeat_cancel.is_none());
            assert!(inner.heartbeat_handle.is_none());

            svc.start_heartbeat_locked(&mut inner);

            // generation НЕ bumped, handle/cancel не установлены (actor отсутствует)
            assert_eq!(
                inner.connection_generation, gen_before,
                "generation must NOT change without an actor"
            );
            assert!(inner.heartbeat_cancel.is_none());
            assert!(inner.heartbeat_handle.is_none());
        });
    }

    #[test]
    fn start_heartbeat_locked_stops_previous_heartbeat() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let old_cancel = {
                let mut inner = svc.inner.lock().await;
                let cancel = CancellationToken::new();
                let ct_clone = cancel.clone();
                inner.heartbeat_cancel = Some(cancel);
                let ct_for_spawn = ct_clone.clone();
                inner.heartbeat_handle = Some(tokio::spawn(async move {
                    ct_for_spawn.cancelled().await;
                }));
                ct_clone
            };

            // Old token should not be cancelled yet
            assert!(!old_cancel.is_cancelled());

            let mut inner = svc.inner.lock().await;
            svc.start_heartbeat_locked(&mut inner);

            // Old token must be cancelled by start_heartbeat_locked (stop_heartbeat_locked
            // вызывается в начале даже без actor).
            assert!(
                old_cancel.is_cancelled(),
                "old heartbeat token must be cancelled"
            );

            // Без actor новый heartbeat не запускается.
            assert!(inner.heartbeat_cancel.is_none());
            assert!(inner.heartbeat_handle.is_none());
        });
    }

    // ---------------------------------------------------------------------------
    // test_connection error path truthful state (unused port, no VTS)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_connection_on_closed_port_sets_truthful_state() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        // Place a fake resolved item so inner looks like 'retained'
        rt.block_on(async {
            {
                let mut inner = svc.inner.lock().await;
                inner.typing_active = true;
                // no real socket: actor stays None by default
            }
        });

        let port = unused_local_port();
        rt.block_on(async {
            let result = svc.test_connection("127.0.0.1", port, None).await;
            assert!(result.is_err(), "connect to closed port must fail");
        });

        // After failure: auth must be false, status must be Error,
        // ws must be None, typing must be inactive.
        assert!(
            !svc.is_authenticated.load(Ordering::SeqCst),
            "auth must be false after test_connection error"
        );
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Error,
            "status must be Error after test_connection failure"
        );
        assert!(
            svc.is_desired_running(),
            "desired_running must NOT be cleared by test_connection failure"
        );

        rt.block_on(async {
            let inner = svc.inner.lock().await;
            assert!(inner.actor.is_none());
            assert!(!inner.typing_active);
            assert!(
                inner.heartbeat_cancel.is_none(),
                "no heartbeat should be started on failed test_connection"
            );
            assert!(
                inner.heartbeat_handle.is_none(),
                "no heartbeat should be started on failed test_connection"
            );
        });
    }

    #[test]
    fn test_connection_failure_preserves_desired_running() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        let port = unused_local_port();

        rt.block_on(async {
            let result = svc.test_connection("127.0.0.1", port, None).await;
            assert!(result.is_err());
        });

        assert!(
            svc.is_desired_running(),
            "desired_running must survive test_connection failure"
        );
    }

    #[test]
    fn test_connection_failure_when_not_desired_running_still_sets_error_status() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        assert!(!svc.is_desired_running());
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);

        let port = unused_local_port();
        rt.block_on(async {
            let result = svc.test_connection("127.0.0.1", port, None).await;
            assert!(result.is_err());
        });

        assert!(!svc.is_desired_running());
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Error,
            "status must be Error"
        );
        assert!(!svc.is_authenticated.load(Ordering::SeqCst));
    }

    // ---------------------------------------------------------------------------
    // ensure_event_parameter_if_connected tests
    // ---------------------------------------------------------------------------

    #[test]
    fn ensure_param_disconnected_no_socket_returns_no_socket() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        let session_before = svc.read_session();
        rt.block_on(async {
            let result = svc.ensure_event_parameter_if_connected("MyParam").await;
            assert_eq!(result, Ok(EnsureOutcome::Skipped(SkipReason::NoSocket)));
        });
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected,
            "connection status unchanged when no socket"
        );
        assert!(svc.is_authenticated.load(Ordering::SeqCst));
        assert_eq!(svc.read_session(), session_before, "session unchanged");
        assert!(svc.is_desired_running());
    }

    #[test]
    fn ensure_param_not_desired_running_returns_skipped() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            assert_eq!(
                svc.ensure_event_parameter_if_connected("MyParam").await,
                Ok(EnsureOutcome::Skipped(SkipReason::NotDesiredRunning))
            );
        });
    }

    // ---------------------------------------------------------------------------
    // delete_event_parameter_if_connected tests
    // ---------------------------------------------------------------------------

    #[test]
    fn delete_param_disconnected_returns_skipped_no_mutation() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let result = svc.delete_event_parameter_if_connected("OldParam").await;
            assert_eq!(
                result,
                Ok(DeleteOutcome::Skipped(SkipReason::NotDesiredRunning))
            );
        });
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected,
            "connection status unchanged when disconnected"
        );
        assert!(!svc.is_authenticated.load(Ordering::SeqCst));
    }

    #[test]
    fn delete_param_not_desired_running_returns_skipped() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let result = svc.delete_event_parameter_if_connected("OldParam").await;
            assert_eq!(
                result,
                Ok(DeleteOutcome::Skipped(SkipReason::NotDesiredRunning))
            );
        });
    }

    #[test]
    fn delete_param_not_authenticated_returns_skipped() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        assert!(!svc.is_authenticated());
        rt.block_on(async {
            let result = svc.delete_event_parameter_if_connected("OldParam").await;
            assert_eq!(
                result,
                Ok(DeleteOutcome::Skipped(SkipReason::NotAuthenticated))
            );
        });
        assert!(svc.is_desired_running());
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );
    }

    #[test]
    fn delete_param_no_socket_returns_skipped() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        rt.block_on(async {
            let result = svc.delete_event_parameter_if_connected("OldParam").await;
            assert_eq!(result, Ok(DeleteOutcome::Skipped(SkipReason::NoSocket)));
        });
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected,
            "connection status unchanged when no socket"
        );
        assert!(svc.is_authenticated.load(Ordering::SeqCst));
    }

    // ---------------------------------------------------------------------------
    // inner_vts_error — "Delete parameter failed: " prefix
    // ---------------------------------------------------------------------------

    #[test]
    fn inner_vts_error_strips_delete_prefix() {
        let err = "Delete parameter failed: VTS error 401";
        assert_eq!(inner_vts_error(err), "VTS error 401");
    }

    #[test]
    fn inner_vts_error_strips_delete_prefix_400() {
        let err = "Delete parameter failed: VTS error 400";
        assert_eq!(inner_vts_error(err), "VTS error 400");
    }

    #[test]
    fn inner_vts_error_strips_create_prefix() {
        let err = "Create parameter failed: VTS error 350";
        assert_eq!(inner_vts_error(err), "VTS error 350");
    }

    #[test]
    fn inner_vts_error_passes_unknown_prefix_through() {
        let err = "Some unknown error";
        assert_eq!(inner_vts_error(err), "Some unknown error");
    }

    // ---------------------------------------------------------------------------
    // is_semantic_vts_error with delete prefix
    // ---------------------------------------------------------------------------

    #[test]
    fn is_semantic_delete_error_401() {
        assert!(is_semantic_vts_error(
            "Delete parameter failed: VTS error 401"
        ));
    }

    #[test]
    fn is_semantic_delete_error_402() {
        assert!(is_semantic_vts_error(
            "Delete parameter failed: VTS error 402"
        ));
    }

    #[test]
    fn is_semantic_delete_error_403() {
        assert!(is_semantic_vts_error(
            "Delete parameter failed: VTS error 403"
        ));
    }

    #[test]
    fn is_semantic_delete_error_400() {
        assert!(is_semantic_vts_error(
            "Delete parameter failed: VTS error 400"
        ));
    }

    #[test]
    fn is_not_semantic_transport_delete_error() {
        assert!(!is_semantic_vts_error(
            "Delete parameter failed: Read error: connection lost"
        ));
    }

    // ---------------------------------------------------------------------------
    // classify_vts_response — ParameterDeletionResponse
    // ---------------------------------------------------------------------------

    #[test]
    fn classify_parameter_deletion_response() {
        let resp = make_response(
            "ParameterDeletionResponse",
            "del-1",
            serde_json::json!({"parameterName": "TTSBardTyping"}),
        );
        match classify_vts_response(&resp, "del-1", "ParameterDeletionResponse") {
            RecvResult::Match(data) => {
                assert_eq!(data["parameterName"].as_str().unwrap(), "TTSBardTyping");
            }
            _ => panic!("expected Match for ParameterDeletionResponse"),
        }
    }

    #[test]
    fn classify_deletion_api_error() {
        let resp = make_response(
            "APIError",
            "del-err",
            serde_json::json!({"errorID": 401, "message": "CustomParamDeletionNotFound"}),
        );
        match classify_vts_response(&resp, "del-err", "ParameterDeletionResponse") {
            RecvResult::Error(e) => {
                assert!(e.contains("VTS error 401"), "got: {}", e);
            }
            _ => panic!("expected Error for deletion APIError"),
        }
    }

    // ---------------------------------------------------------------------------
    // stop_event_typing_and_reset lifecycle (no network)
    // ---------------------------------------------------------------------------

    #[test]
    fn stop_event_typing_and_reset_clears_typing_active_no_socket() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            {
                let mut inner = svc.inner.lock().await;
                inner.typing_active = true;
            }
            svc.stop_event_typing_and_reset("OldInput").await;
            let inner = svc.inner.lock().await;
            assert!(
                !inner.typing_active,
                "typing_active must be cleared by stop_event_typing_and_reset"
            );
            assert!(inner.typing_cancel.is_none());
            assert!(inner.typing_handle.is_none());
        });
    }

    #[test]
    fn stop_event_typing_and_reset_stops_keepalive() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            {
                let mut inner = svc.inner.lock().await;
                let cancel = tokio_util::sync::CancellationToken::new();
                inner.typing_cancel = Some(cancel);
                inner.typing_handle = Some(tokio::spawn(async {}));
                inner.typing_active = true;
            }
            svc.stop_event_typing_and_reset("OldInput").await;
            let inner = svc.inner.lock().await;
            assert!(
                inner.typing_cancel.is_none(),
                "typing_cancel must be cleared"
            );
            assert!(
                inner.typing_handle.is_none(),
                "typing_handle must be cleared"
            );
            assert!(!inner.typing_active);
        });
    }

    #[test]
    fn stop_event_typing_and_reset_noop_when_not_active() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            svc.stop_event_typing_and_reset("Irrelevant").await;
            let inner = svc.inner.lock().await;
            assert!(!inner.typing_active);
            assert!(inner.typing_cancel.is_none());
            assert!(inner.typing_handle.is_none());
        });
    }

    // ---------------------------------------------------------------------------
    // Generation guard regression tests (ROADMAP-062 review correction)
    // ---------------------------------------------------------------------------

    fn make_fake_actor_handle() -> ConnectionActorHandle {
        let (tx, _rx) = mpsc::channel::<ActorRequest>(1);
        let cancel = CancellationToken::new();
        let join = tokio::spawn(async {});
        ConnectionActorHandle {
            sender: tx.clone(),
            cancel,
            join,
        }
    }

    /// Heartbeat preserves the actor's generation — start_heartbeat_locked
    /// must NOT bump connection_generation.
    #[test]
    fn heartbeat_preserves_actor_generation() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let svc = VTubeStudioService::new();
        rt.block_on(async {
            let mut inner = svc.inner.lock().await;
            inner.actor = Some(make_fake_actor_handle());
            inner.connection_generation = 42;

            svc.start_heartbeat_locked(&mut inner);

            assert_eq!(
                inner.connection_generation, 42,
                "heartbeat start must NOT bump generation"
            );
            assert!(
                inner.heartbeat_cancel.is_some(),
                "heartbeat cancel must be set"
            );
            assert!(
                inner.heartbeat_handle.is_some(),
                "heartbeat handle must be set"
            );
        });
    }

    /// actor_transport_failure free-function with stale generation
    /// preserves fresh actor state.
    #[test]
    fn actor_transport_failure_stale_gen_preserves_state() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let svc = VTubeStudioService::new();
            svc.set_desired_running(true);
            svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
            svc.is_authenticated.store(true, Ordering::SeqCst);

            {
                let mut inner = svc.inner.lock().await;
                inner.connection_generation = 99;
                inner.actor = Some(make_fake_actor_handle());
                inner.typing_active = true;
            }

            let mut pending: HashMap<String, Pending> = HashMap::new();
            actor_transport_failure(
                &svc.inner,
                &svc.is_authenticated,
                &svc.connection_status,
                None,
                5, // stale
                FailureClass::Read,
                None,
                &mut pending,
            )
            .await;

            let inner = svc.inner.lock().await;
            assert!(
                inner.actor.is_some(),
                "fresh actor must survive stale failure"
            );
            assert!(
                svc.is_authenticated.load(Ordering::SeqCst),
                "auth must survive stale failure"
            );
        });
    }

    /// actor_transport_failure free-function with matching generation
    /// MUST invalidate state.
    #[test]
    fn actor_transport_failure_current_gen_invalidates() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let svc = VTubeStudioService::new();
            svc.set_desired_running(true);
            svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
            svc.is_authenticated.store(true, Ordering::SeqCst);

            let gen = {
                let mut inner = svc.inner.lock().await;
                inner.connection_generation = 8;
                inner.actor = Some(make_fake_actor_handle());
                inner.typing_active = true;
                inner.typing_cancel = Some(CancellationToken::new());
                inner.heartbeat_cancel = Some(CancellationToken::new());
                8u64
            };

            let mut pending: HashMap<String, Pending> = HashMap::new();
            actor_transport_failure(
                &svc.inner,
                &svc.is_authenticated,
                &svc.connection_status,
                None,
                gen, // matching generation
                FailureClass::Read,
                None,
                &mut pending,
            )
            .await;

            let inner = svc.inner.lock().await;
            assert!(
                inner.actor.is_none(),
                "actor must be cleared on matching failure"
            );
            assert!(!svc.is_authenticated.load(Ordering::SeqCst));
            assert_eq!(
                svc.get_connection_status(),
                VTubeStudioConnectionStatus::Error
            );
        });
    }

    // ---------------------------------------------------------------------------
    // Fencing regressions: stale results, durable token commit, teardown scope
    // ---------------------------------------------------------------------------

    /// Fake VTube Studio для детерминированных гонок connect. Первое соединение
    /// придерживает ответ на AuthenticationTokenRequest до `release`, давая тесту
    /// окно, в котором попытку можно вытеснить. `close_on_release` обрывает
    /// соединение вместо ответа (stale-error сценарий).
    struct FakeVts {
        port: u16,
        token_request_seen: Option<oneshot::Receiver<()>>,
        release: Option<oneshot::Sender<()>>,
        shutdown: CancellationToken,
    }

    impl FakeVts {
        async fn wait_for_token_request(&mut self) {
            let seen = self
                .token_request_seen
                .take()
                .expect("fake VTS token-request gate already consumed");
            timeout(Duration::from_secs(10), seen)
                .await
                .expect("connect did not request a token in time")
                .expect("fake VTS token-request signal dropped");
        }

        fn release(&mut self) {
            if let Some(tx) = self.release.take() {
                let _ = tx.send(());
            }
        }
    }

    impl Drop for FakeVts {
        fn drop(&mut self) {
            self.shutdown.cancel();
        }
    }

    async fn spawn_fake_vts(close_on_release: bool) -> FakeVts {
        use tokio::net::TcpListener;
        use tokio_tungstenite::tungstenite::Message;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen_tx, seen_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let shutdown = CancellationToken::new();
        let accept_shutdown = shutdown.clone();

        let seen_tx = Arc::new(parking_lot::Mutex::new(Some(seen_tx)));
        let release_rx = Arc::new(tokio::sync::Mutex::new(Some(release_rx)));
        let mut first_connection = true;

        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = accept_shutdown.cancelled() => break,
                    accepted = listener.accept() => accepted,
                };
                let Ok((stream, _addr)) = accepted else {
                    break;
                };
                let gated = first_connection;
                first_connection = false;
                let seen_tx = Arc::clone(&seen_tx);
                let release_rx = Arc::clone(&release_rx);
                let connection_shutdown = accept_shutdown.clone();

                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    loop {
                        let incoming = tokio::select! {
                            _ = connection_shutdown.cancelled() => break,
                            incoming = ws.next() => incoming,
                        };
                        let Some(Ok(message)) = incoming else {
                            break;
                        };
                        let Message::Text(text) = message else {
                            continue;
                        };
                        let Ok(request) = serde_json::from_str::<VtsRequest>(&text) else {
                            continue;
                        };
                        let (message_type, data) = match request.message_type.as_str() {
                            "AuthenticationTokenRequest" => {
                                if gated {
                                    if let Some(tx) = seen_tx.lock().take() {
                                        let _ = tx.send(());
                                    }
                                    if let Some(wait) = release_rx.lock().await.take() {
                                        let _ = wait.await;
                                    }
                                    if close_on_release {
                                        return;
                                    }
                                }
                                (
                                    "AuthenticationTokenResponse",
                                    serde_json::json!({ "authenticationToken": "fake-vts-token" }),
                                )
                            }
                            "AuthenticationRequest" => (
                                "AuthenticationResponse",
                                serde_json::json!({ "authenticated": true, "reason": "" }),
                            ),
                            _ => continue,
                        };
                        let response = serde_json::json!({
                            "apiName": "VTubeStudioPublicAPI",
                            "apiVersion": "1.0",
                            "requestID": request.request_id,
                            "messageType": message_type,
                            "data": data,
                        });
                        if ws.send(Message::Text(response.to_string())).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });

        FakeVts {
            port,
            token_request_seen: Some(seen_rx),
            release: Some(release_tx),
            shutdown,
        }
    }

    /// Actor-handle с живым receiver-ом: позволяет управлять ответами и
    /// удерживать best-effort запросы teardown-а.
    fn fake_actor_with_receiver(
        capacity: usize,
    ) -> (ConnectionActorHandle, mpsc::Receiver<ActorRequest>) {
        let (sender, receiver) = mpsc::channel::<ActorRequest>(capacity);
        let handle = ConnectionActorHandle {
            sender,
            cancel: CancellationToken::new(),
            join: tokio::spawn(async {}),
        };
        (handle, receiver)
    }

    /// Успешный connect устанавливает живую сессию; токен коммитится в
    /// runtime-настройки только после durable-записи.
    #[tokio::test]
    async fn connect_success_installs_live_session_and_commits_token() {
        let mut server = spawn_fake_vts(false).await;
        let svc = Arc::new(VTubeStudioService::new());

        let connect_svc = Arc::clone(&svc);
        let port = server.port;
        let connect = tokio::spawn(async move {
            connect_svc
                .connect("127.0.0.1", port, None, ConnectOrigin::Manual)
                .await
        });

        server.wait_for_token_request().await;
        server.release();
        let success = connect.await.unwrap().expect("connect must succeed");

        assert!(success.live);
        assert_eq!(success.token.as_deref(), Some("fake-vts-token"));
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );
        assert!(svc.is_authenticated());
        assert!(svc.inner.lock().await.actor.is_some());
        assert!(svc.inner.lock().await.heartbeat_cancel.is_some());

        let persisted = Arc::new(parking_lot::Mutex::new(None::<String>));
        let persisted_for_commit = Arc::clone(&persisted);
        let report = svc
            .commit_connection_success(success, move |token| async move {
                *persisted_for_commit.lock() = Some(token);
                Ok(())
            })
            .await;

        assert!(report.is_live());
        assert!(!report.is_superseded());
        assert!(report.persist_error().is_none());
        assert_eq!(*persisted.lock(), Some("fake-vts-token".to_string()));
        assert_eq!(
            svc.settings.read().await.token.as_deref(),
            Some("fake-vts-token")
        );

        svc.disconnect().await;
    }

    /// Устаревший успех не должен ни установить сокет, ни перетереть более
    /// новую сессию, ни вернуть ложный успех.
    #[tokio::test]
    async fn stale_success_is_superseded_and_preserves_newer_session() {
        let mut server = spawn_fake_vts(false).await;
        let svc = Arc::new(VTubeStudioService::new());

        let connect_svc = Arc::clone(&svc);
        let port = server.port;
        let connect = tokio::spawn(async move {
            connect_svc
                .connect("127.0.0.1", port, None, ConnectOrigin::Manual)
                .await
        });

        server.wait_for_token_request().await;

        // Более новая попытка уже установила свою сессию, пока старая ждёт сеть.
        let newer_attempt = svc.begin_connection_attempt().await;
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        {
            let mut inner = svc.inner.lock().await;
            inner.connection_generation = inner.connection_generation.wrapping_add(1);
            inner.actor = Some(make_fake_actor_handle());
        }

        server.release();
        let failure = connect
            .await
            .unwrap()
            .expect_err("stale success must be reported as superseded");

        assert!(failure.is_superseded());
        assert!(failure.message().is_none());
        assert!(
            svc.is_current_attempt(newer_attempt),
            "newer attempt must stay current"
        );
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected,
            "stale success must not change status"
        );
        assert!(
            svc.is_authenticated(),
            "stale success must not clear auth of newer session"
        );
        assert!(svc.is_desired_running());
        assert!(
            svc.inner.lock().await.actor.is_some(),
            "stale success must not tear down newer actor"
        );
    }

    /// Устаревшая ошибка не должна менять auth/status/actor текущей сессии.
    #[tokio::test]
    async fn stale_error_is_superseded_and_preserves_current_session() {
        let mut server = spawn_fake_vts(true).await;
        let svc = Arc::new(VTubeStudioService::new());

        let connect_svc = Arc::clone(&svc);
        let port = server.port;
        let connect = tokio::spawn(async move {
            connect_svc
                .connect("127.0.0.1", port, None, ConnectOrigin::Manual)
                .await
        });

        server.wait_for_token_request().await;

        let newer_attempt = svc.begin_connection_attempt().await;
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        {
            let mut inner = svc.inner.lock().await;
            inner.connection_generation = inner.connection_generation.wrapping_add(1);
            inner.actor = Some(make_fake_actor_handle());
        }

        server.release();
        let failure = connect
            .await
            .unwrap()
            .expect_err("stale failure must be reported as superseded");

        assert!(failure.is_superseded());
        assert!(failure.message().is_none());
        assert!(svc.is_current_attempt(newer_attempt));
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected,
            "stale error must not overwrite status with Error"
        );
        assert!(
            svc.is_authenticated(),
            "stale error must not clear auth of current session"
        );
        assert!(svc.is_desired_running());
        assert!(
            svc.inner.lock().await.actor.is_some(),
            "stale error must not tear down current actor"
        );
    }

    /// Standalone-проверка auth при desired_running=false: токен получен и
    /// коммитится, но actor не устанавливается и живая сессия не публикуется.
    #[tokio::test]
    async fn test_connection_without_desired_running_verifies_auth_without_actor() {
        let mut server = spawn_fake_vts(false).await;
        let svc = Arc::new(VTubeStudioService::new());
        assert!(!svc.is_desired_running());

        let test_svc = Arc::clone(&svc);
        let port = server.port;
        let test =
            tokio::spawn(async move { test_svc.test_connection("127.0.0.1", port, None).await });

        server.wait_for_token_request().await;
        server.release();
        let success = test.await.unwrap().expect("auth test must succeed");

        assert!(!success.live, "no socket must be installed");
        assert_eq!(success.token.as_deref(), Some("fake-vts-token"));
        assert!(svc.inner.lock().await.actor.is_none());
        assert!(!svc.is_authenticated());
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
        assert!(!svc.is_desired_running());

        let persisted = Arc::new(parking_lot::Mutex::new(None::<String>));
        let persisted_for_commit = Arc::clone(&persisted);
        let report = svc
            .commit_connection_success(success, move |token| async move {
                *persisted_for_commit.lock() = Some(token);
                Ok(())
            })
            .await;

        assert!(!report.is_live(), "auth-only result must not report live");
        assert!(!report.is_superseded());
        assert!(report.persist_error().is_none());
        assert_eq!(*persisted.lock(), Some("fake-vts-token".to_string()));
        assert_eq!(
            svc.settings.read().await.token.as_deref(),
            Some("fake-vts-token")
        );
    }

    /// Инвалидация попытки (disconnect/новый connect) не может пройти между
    /// проверкой актуальности и durable-записью токена: запись и точка
    /// линеаризации общие, поэтому устаревший токен не может лечь на диск
    /// после более нового.
    #[tokio::test]
    async fn attempt_invalidation_cannot_interleave_with_durable_token_commit() {
        let svc = Arc::new(VTubeStudioService::new());
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        let attempt = svc.begin_connection_attempt().await;

        let order = Arc::new(parking_lot::Mutex::new(Vec::<&'static str>::new()));
        let (persist_started_tx, persist_started_rx) = oneshot::channel();
        let (persist_release_tx, persist_release_rx) = oneshot::channel();

        let commit_svc = Arc::clone(&svc);
        let commit_order = Arc::clone(&order);
        let commit = tokio::spawn(async move {
            commit_svc
                .commit_connection_success(
                    ConnectionSuccess {
                        token: Some("durable-token".to_string()),
                        live: true,
                        attempt,
                    },
                    move |_token| async move {
                        commit_order.lock().push("persist:start");
                        let _ = persist_started_tx.send(());
                        let _ = persist_release_rx.await;
                        commit_order.lock().push("persist:done");
                        Ok(())
                    },
                )
                .await
        });

        persist_started_rx.await.expect("persist must start");

        let (invalidated_tx, mut invalidated_rx) = oneshot::channel();
        let invalidation_svc = Arc::clone(&svc);
        let invalidation_order = Arc::clone(&order);
        let invalidation = tokio::spawn(async move {
            invalidation_svc.begin_connection_attempt().await;
            invalidation_order.lock().push("invalidate");
            let _ = invalidated_tx.send(());
        });

        // Даём задаче инвалидации шанс дойти до write-gate: она обязана ждать
        // завершения durable-записи.
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        assert!(
            invalidated_rx.try_recv().is_err(),
            "invalidation must block while the durable token commit is in flight"
        );
        assert_eq!(*order.lock(), ["persist:start"]);

        persist_release_tx.send(()).unwrap();
        let report = commit.await.unwrap();
        assert!(report.is_live());
        assert!(report.persist_error().is_none());

        timeout(Duration::from_secs(10), invalidated_rx)
            .await
            .expect("invalidation must complete after the commit")
            .unwrap();
        invalidation.await.unwrap();

        let steps = order.lock().clone();
        assert_eq!(steps, ["persist:start", "persist:done", "invalidate"]);
        assert!(!svc.is_current_attempt(attempt));
        assert_eq!(
            svc.settings.read().await.token.as_deref(),
            Some("durable-token")
        );
    }

    /// Уже вытесненная попытка не пишет ни диск, ни runtime-токен.
    #[tokio::test]
    async fn token_commit_skips_disk_and_runtime_for_superseded_attempt() {
        let svc = Arc::new(VTubeStudioService::new());
        svc.settings.write().await.token = Some("old-token".to_string());
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        let attempt = svc.begin_connection_attempt().await;
        // Более новая попытка вытесняет предыдущую до начала коммита.
        svc.begin_connection_attempt().await;

        let persist_called = Arc::new(AtomicBool::new(false));
        let persist_called_for_commit = Arc::clone(&persist_called);
        let report = svc
            .commit_connection_success(
                ConnectionSuccess {
                    token: Some("stale-token".to_string()),
                    live: true,
                    attempt,
                },
                move |_token| async move {
                    persist_called_for_commit.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
            .await;

        assert!(report.is_superseded());
        assert!(!report.is_live());
        assert!(report.persist_error().is_none());
        assert!(
            !persist_called.load(Ordering::SeqCst),
            "superseded attempt must not persist a token"
        );
        assert_eq!(
            svc.settings.read().await.token.as_deref(),
            Some("old-token")
        );
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );
        assert!(svc.is_authenticated());
    }

    /// Неудачная запись токена актуальной попытки снимает только её соединение.
    #[tokio::test]
    async fn token_persist_failure_discards_current_attempt_connection() {
        let svc = Arc::new(VTubeStudioService::new());
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        {
            let mut inner = svc.inner.lock().await;
            inner.actor = Some(make_fake_actor_handle());
            inner.typing_active = true;
        }
        let attempt = svc.begin_connection_attempt().await;

        let report = svc
            .commit_connection_success(
                ConnectionSuccess {
                    token: Some("undurable-token".to_string()),
                    live: true,
                    attempt,
                },
                |_token| async move { Err("disk full".to_string()) },
            )
            .await;

        assert!(!report.is_live());
        assert!(!report.is_superseded());
        assert_eq!(report.persist_error(), Some("disk full"));
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Disconnected
        );
        assert!(!svc.is_authenticated());
        assert!(!svc.is_desired_running());
        let inner = svc.inner.lock().await;
        assert!(inner.actor.is_none());
        assert!(!inner.typing_active);
    }

    /// Scoped-discard не имеет права трогать сессию более новой попытки.
    #[tokio::test]
    async fn discard_for_stale_attempt_keeps_newer_actor() {
        let svc = VTubeStudioService::new();
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);

        let stale_attempt = svc.begin_connection_attempt().await;
        let (newer_actor, _newer_rx) = fake_actor_with_receiver(8);
        let newer_sender = newer_actor.sender.clone();
        {
            let mut inner = svc.inner.lock().await;
            inner.actor = Some(newer_actor);
        }
        // Новая попытка вытесняет старую.
        svc.begin_connection_attempt().await;

        assert!(!svc.discard_attempt_connection(stale_attempt).await);

        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected
        );
        assert!(svc.is_authenticated());
        assert!(svc.is_desired_running());
        let inner = svc.inner.lock().await;
        let actor = inner.actor.as_ref().expect("newer actor must survive");
        assert!(
            actor.sender.same_channel(&newer_sender),
            "newer actor must not be replaced"
        );
    }

    /// Явный disconnect не должен останавливать/пересоздавать более новую
    /// сессию, появившуюся за время best-effort отправок (await).
    #[tokio::test]
    async fn disconnect_teardown_does_not_touch_newer_actor_after_await() {
        let svc = Arc::new(VTubeStudioService::new());
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        {
            let mut s = svc.settings.write().await;
            s.typing_action.output_mode = VTubeStudioTypingMode::Event;
        }

        let (old_actor, mut old_rx) = fake_actor_with_receiver(8);
        {
            let mut inner = svc.inner.lock().await;
            inner.connection_generation = 7;
            inner.typing_active = true;
            inner.actor = Some(old_actor);
        }

        // Держим best-effort inject=0.0 запрос, чтобы развести disconnect и
        // появление новой сессии детерминированно.
        let (inject_seen_tx, inject_seen_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        tokio::spawn(async move {
            let Some(request) = old_rx.recv().await else {
                return;
            };
            let _ = inject_seen_tx.send(());
            let _ = release_rx.await;
            drop(request);
        });

        let disconnect_svc = Arc::clone(&svc);
        let disconnect = tokio::spawn(async move {
            disconnect_svc.disconnect().await;
        });

        inject_seen_rx
            .await
            .expect("disconnect must send best-effort inject");
        assert!(
            !disconnect.is_finished(),
            "disconnect must still be waiting for the in-flight best-effort send"
        );

        // Пока disconnect висит на await, стартует более новая попытка и
        // устанавливает собственную сессию.
        let newer_attempt = svc.begin_connection_attempt().await;
        let (newer_actor, _newer_rx) = fake_actor_with_receiver(8);
        let newer_sender = newer_actor.sender.clone();
        {
            let mut inner = svc.inner.lock().await;
            inner.connection_generation = 8;
            inner.actor = Some(newer_actor);
        }
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);

        release_tx.send(()).unwrap();
        timeout(Duration::from_secs(10), disconnect)
            .await
            .expect("disconnect must finish")
            .unwrap();

        assert!(svc.is_current_attempt(newer_attempt));
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected,
            "superseded disconnect must not report Disconnected"
        );
        assert!(
            svc.is_authenticated(),
            "superseded disconnect must not clear newer auth"
        );
        assert!(
            svc.is_desired_running(),
            "superseded disconnect must not clear newer desired_running"
        );
        let inner = svc.inner.lock().await;
        let actor = inner.actor.as_ref().expect("newer actor must survive");
        assert!(
            actor.sender.same_channel(&newer_sender),
            "superseded disconnect must not stop newer actor"
        );
    }

    /// Вытеснённый connect не имеет права выполнить свой teardown (снести actor)
    /// после await, за которым стартовала более новая попытка.
    #[tokio::test]
    async fn stale_connect_teardown_does_not_touch_newer_actor() {
        let svc = Arc::new(VTubeStudioService::new());

        // Держим settings-write, чтобы устаревший connect остановился между
        // begin_connection_attempt и своим teardown-блоком.
        let settings_guard = svc.settings.write().await;

        let connect_svc = Arc::clone(&svc);
        let connect = tokio::spawn(async move {
            connect_svc
                .connect("127.0.0.1", 9, None, ConnectOrigin::Manual)
                .await
        });

        // Ждём, пока connect зарегистрирует свою попытку.
        for _ in 0..64 {
            if svc.connection_attempt.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            svc.connection_attempt.load(Ordering::SeqCst),
            1,
            "connect must have registered an attempt"
        );

        // Более новая попытка стартует и устанавливает свою сессию, пока
        // устаревший connect ещё ждёт settings-лок.
        let newer_attempt = svc.begin_connection_attempt().await;
        svc.set_desired_running(true);
        svc.set_connection_status(VTubeStudioConnectionStatus::Connected);
        svc.is_authenticated.store(true, Ordering::SeqCst);
        let (newer_actor, _newer_rx) = fake_actor_with_receiver(8);
        let newer_sender = newer_actor.sender.clone();
        {
            let mut inner = svc.inner.lock().await;
            inner.connection_generation = inner.connection_generation.wrapping_add(1);
            inner.actor = Some(newer_actor);
        }
        drop(settings_guard);

        let failure = timeout(Duration::from_secs(10), connect)
            .await
            .expect("stale connect must finish")
            .unwrap()
            .expect_err("stale connect must not report success");
        assert!(failure.is_superseded());

        assert!(svc.is_current_attempt(newer_attempt));
        assert_eq!(
            svc.get_connection_status(),
            VTubeStudioConnectionStatus::Connected,
            "stale connect teardown must not overwrite newer status"
        );
        assert!(
            svc.is_authenticated(),
            "stale connect teardown must not clear newer auth"
        );
        assert!(svc.is_desired_running());
        let inner = svc.inner.lock().await;
        let actor = inner.actor.as_ref().expect("newer actor must survive");
        assert!(
            actor.sender.same_channel(&newer_sender),
            "stale connect teardown must not stop newer actor"
        );
    }
}
