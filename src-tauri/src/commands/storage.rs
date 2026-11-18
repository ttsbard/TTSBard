//! Пользовательские пути хранения (ROADMAP-117): каталог аудио-кеша, корень
//! данных программы, перенос данных и открытие папки локальных данных.

use std::sync::{Arc, OnceLock};

use tauri::{AppHandle, Emitter, State};

use crate::config::SettingsManager;
use crate::ipc::CommandError;
use crate::updates::install::InstallState;
use crate::updates::UpdatesState;

enum TransferCommandFailure {
    Message(String),
    Transfer(anyhow::Error),
}

impl From<String> for TransferCommandFailure {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<anyhow::Error> for TransferCommandFailure {
    fn from(error: anyhow::Error) -> Self {
        Self::Transfer(error)
    }
}

impl TransferCommandFailure {
    fn into_command_error(self) -> CommandError {
        match self {
            Self::Transfer(error)
                if error.is::<crate::storage_transfer::ManualTransferRequired>() =>
            {
                CommandError::new("storage.manual_transfer_required", error.to_string(), false)
            }
            Self::Transfer(error) => transfer_failed(error),
            Self::Message(message) => transfer_failed(message),
        }
    }
}

fn transfer_failed(error: impl std::fmt::Display) -> CommandError {
    CommandError::new("storage.transfer_failed", error.to_string(), true)
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn manual_transfer_error_survives_command_boundary() {
        let error = TransferCommandFailure::from(anyhow::Error::new(
            crate::storage_transfer::ManualTransferRequired,
        ))
        .into_command_error();
        assert_eq!(error.code, "storage.manual_transfer_required");
        assert!(!error.retryable);
    }
}

/// Эффективный корень данных программы и признак дефолтного пути.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DataInfo {
    pub path: String,
    pub is_default: bool,
}

/// Результат подготовки переноса данных.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TransferPrepareInfo {
    pub source_path: String,
    pub target_path: String,
    pub total_bytes: u64,
}

/// Результат успешного переноса данных.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TransferDataResult {
    pub path: String,
    pub is_default: bool,
    /// Живые инстансы моделей могут удерживать старые пути: после переноса
    /// моделей рекомендуется перезапуск. Не утверждает runtime-миграцию.
    pub restart_required: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
struct TransferProgressPayload {
    operation_id: String,
    phase: &'static str,
    completed_bytes: u64,
    total_bytes: u64,
}

/// Глобальный owner-guard: допускает только один одновременный перенос.
static TRANSFER_GUARD: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

fn transfer_guard() -> &'static std::sync::Mutex<()> {
    TRANSFER_GUARD.get_or_init(|| std::sync::Mutex::new(()))
}

/// Текущий корень данных программы (дефолт `%LOCALAPPDATA%\ttsbard` или
/// пользовательский `storage.data_dir`).
#[tauri::command]
pub async fn storage_get_data_info() -> Result<DataInfo, String> {
    tokio::task::spawn_blocking(move || {
        let root = crate::paths::data_root()
            .map_err(|e| format!("Не удалось определить корень данных: {}", e))?;
        Ok(DataInfo {
            path: root.to_string_lossy().into_owned(),
            // A legacy custom cache still needs a reset/migration even when
            // the program root itself uses the default location.
            is_default: crate::paths::data_root_is_default()
                && crate::history::audio_cache_dir_is_default(),
        })
    })
    .await
    .map_err(|e| format!("Операция была прервана: {}", e))?
}

/// Разобрать `path: Option<String>` в целевой корень и persisted-значение
/// `storage.data_dir` (`None` — сброс на дефолт).
fn resolve_transfer_target(
    path: Option<String>,
) -> Result<(std::path::PathBuf, Option<String>), String> {
    match path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()) {
        None => {
            let root = crate::paths::default_data_root()
                .map_err(|e| format!("Не удалось определить дефолтный корень данных: {}", e))?;
            Ok((root, None))
        }
        Some(p) => Ok((std::path::PathBuf::from(&p), Some(p))),
    }
}

fn current_transfer_inputs() -> Result<(std::path::PathBuf, std::path::PathBuf, bool), String> {
    let source_root = crate::paths::data_root()
        .map_err(|e| format!("Не удалось определить корень данных: {}", e))?;
    let audio_cache_source = crate::history::resolved_audio_cache_dir()
        .map_err(|e| format!("Не удалось определить каталог аудио-кеша: {}", e))?;
    let legacy_audio_cache = !crate::history::audio_cache_dir_is_default();
    Ok((source_root, audio_cache_source, legacy_audio_cache))
}

/// Подготовить перенос данных в новый корень (`None` — сброс на дефолт).
/// Возвращает source/target пути и объём данных. Advisory: не мутирует ФС и
/// не блокирует выполнение параллельных переносов.
#[tauri::command]
pub async fn storage_prepare_data_transfer(
    path: Option<String>,
) -> Result<TransferPrepareInfo, CommandError> {
    tokio::task::spawn_blocking(
        move || -> Result<TransferPrepareInfo, TransferCommandFailure> {
            let (target_root, _) = resolve_transfer_target(path)?;
            let (source_root, audio_cache_source, legacy_audio_cache) = current_transfer_inputs()?;
            let config_root = crate::paths::config_root()
                .map_err(|e| format!("Не удалось определить корень конфигурации: {}", e))?;

            let plan = crate::storage_transfer::build_transfer_plan(
                &source_root,
                &target_root,
                &audio_cache_source,
                legacy_audio_cache,
                &config_root,
            )?;

            Ok(TransferPrepareInfo {
                source_path: source_root.to_string_lossy().into_owned(),
                target_path: target_root.to_string_lossy().into_owned(),
                total_bytes: plan.total_bytes,
            })
        },
    )
    .await
    .map_err(transfer_failed)?
    .map_err(TransferCommandFailure::into_command_error)
}

/// Выполнить перенос данных в новый корень (`None` — сброс на дефолт).
/// Эмитирует `storage-transfer-progress` и возвращает инфо после успешного
/// копирования и сохранения настроек. После публикации нового корня
/// обновляет и контекст проверок обновлений: свежий снимок через
/// `update-status-changed` (состояние теперь читается из нового каталога) и
/// пробуждение планировщика для пересчёта периодического таймера
/// (`crate::updates::notify_data_root_changed`).
#[tauri::command]
pub async fn storage_transfer_data(
    path: Option<String>,
    operation_id: String,
    settings_manager: State<'_, SettingsManager>,
    updates_state: State<'_, Arc<UpdatesState>>,
    install_state: State<'_, Arc<InstallState>>,
    app_handle: AppHandle,
) -> Result<TransferDataResult, CommandError> {
    let manager = settings_manager.inner().clone();
    let updates = updates_state.inner().clone();
    let install = install_state.inner().clone();
    let app = app_handle.clone();
    let operation_id = operation_id.clone();
    let manager_for_notify = manager.clone();

    tokio::task::spawn_blocking(
        move || -> Result<TransferDataResult, TransferCommandFailure> {
            let _guard = transfer_guard()
                .try_lock()
                .map_err(|_| "Передача данных уже выполняется".to_string())?;
            let _cache_guard = crate::history::cache_io_lock().write();

            let (target_root, new_data_dir) = resolve_transfer_target(path)?;
            let (source_root, audio_cache_source, legacy_audio_cache) = current_transfer_inputs()?;
            let config_root = crate::paths::config_root()
                .map_err(|e| format!("Не удалось определить корень конфигурации: {}", e))?;

            let plan = crate::storage_transfer::build_transfer_plan(
                &source_root,
                &target_root,
                &audio_cache_source,
                legacy_audio_cache,
                &config_root,
            )?;

            let op = operation_id.clone();
            let app_emit = app.clone();
            let mut progress =
                move |phase: crate::storage_transfer::TransferPhase, completed: u64, total: u64| {
                    let _ = app_emit.emit(
                        "storage-transfer-progress",
                        TransferProgressPayload {
                            operation_id: op.clone(),
                            phase: phase.as_str(),
                            completed_bytes: completed,
                            total_bytes: total,
                        },
                    );
                };

            let mgr = manager.clone();
            let new_data_dir_clone = new_data_dir.clone();
            let mut persist = move || {
                mgr.set_storage_data_dir(new_data_dir_clone.clone(), None)?;
                crate::paths::publish_data_root(
                    new_data_dir_clone.as_ref().map(std::path::PathBuf::from),
                );
                crate::history::init_audio_cache_dir(None);
                Ok(())
            };

            let outcome =
                crate::storage_transfer::execute_transfer(plan, &mut progress, &mut persist)?;

            let root = crate::paths::data_root()
                .map_err(|e| format!("Не удалось определить корень данных: {}", e))?;
            Ok(TransferDataResult {
                path: root.to_string_lossy().into_owned(),
                is_default: crate::paths::data_root_is_default(),
                restart_required: outcome.models_migrated || source_root != target_root,
            })
        },
    )
    .await
    .map_err(transfer_failed)?
    .map_err(TransferCommandFailure::into_command_error)
    .inspect(|_| {
        crate::commands::emit_settings_changed(&app_handle);
        // `settings-changed` does not refresh the updates context: publish
        // the fresh snapshot of the new root and wake the scheduler here.
        crate::updates::notify_data_root_changed(
            &app_handle,
            &manager_for_notify,
            &updates,
            &install,
        );
    })
}

/// Открыть эффективный корень данных программы (по умолчанию
/// `%LOCALAPPDATA%\ttsbard`) в файловом менеджере ОС.
#[tauri::command]
pub async fn open_local_data_folder() -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let app_dir = crate::paths::data_root()
            .map_err(|_| "Не удалось определить каталог локальных данных приложения".to_string())?;

        std::fs::create_dir_all(&app_dir)
            .map_err(|e| format!("Не удалось создать папку приложения: {}", e))?;

        let app_dir = app_dir
            .canonicalize()
            .map_err(|e| format!("Некорректный путь к папке приложения: {}", e))?;

        let path = app_dir
            .to_str()
            .ok_or("Некорректный путь к папке приложения")?;

        super::open_in_file_manager(path)
    })
    .await
    .map_err(|e| format!("Операция открытия папки была прервана: {}", e))?
}
