//! VBAN network output commands (ROADMAP-138, ROADMAP-139).
//!
//! Единая точка IPC для сетевого аудиовыхода VBAN:
//! - [`set_vban_settings`] — валидация, persist-first сохранение секции
//!   `audio.vban` и применение цели к общему runtime;
//! - [`test_vban_output`] — отправка встроенного тестового звука на параметры
//!   из формы (независимо от флага `enabled`), без локального звука, истории
//!   и TTS;
//! - [`get_vban_status`] — диагностика локальной отправки;
//! - [`get_vban_health`] — доступность приёмника (Receptor) по VBAN-PING;
//!   проверка живёт в отдельном потоке мониторинга и не зависит от вкладки.

use crate::audio::vban::health::{VbanHealthState, VbanPingTarget};
use crate::audio::vban::{VbanPlaybackStatus, VbanTarget};
use crate::config::{SettingsManager, VbanSettings, VbanSettingsDto};
use crate::state::AppState;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::State;
use tracing::{info, warn};

/// Состояние production-отправки VBAN для UI.
///
/// `disabled` — VBAN выключен в сохранённых настройках; `idle` — включён и
/// ничего не отправляет без ошибок; `sending` — идёт локальная отправка;
/// `error` — последний production-этап завершился ошибкой (диагностика).
#[derive(Debug, Clone, serde::Serialize)]
pub struct VbanStatusDto {
    pub state: String,
    pub error: Option<String>,
}

/// Построить `VbanTarget` из проверенных настроек формы.
pub(crate) fn build_vban_target(settings: &VbanSettings) -> Result<VbanTarget, String> {
    let addr = std::net::SocketAddrV4::new(settings.parse_address()?, settings.validate_port()?);
    Ok(VbanTarget {
        addr,
        stream_name: settings.validate_stream_name()?,
        gain: settings.validate_volume()? as f32 / 100.0,
    })
}

/// Цель из сохранённых настроек для старта runtime.
///
/// Возвращает `None` при выключенном VBAN или невалидных сохранённых полях
/// (загрузочный путь уже нормализовал их через `AppSettings::validate`, здесь
/// последняя линия защиты).
pub(crate) fn saved_vban_target(settings: &VbanSettings) -> Option<VbanTarget> {
    if !settings.enabled {
        return None;
    }
    match build_vban_target(settings) {
        Ok(target) => Some(target),
        Err(error) => {
            warn!(
                error = %error,
                "Saved VBAN settings are invalid; network output stays disabled"
            );
            None
        }
    }
}

/// Цель проверки доступности из сохранённых настроек (ROADMAP-139).
///
/// Следует сохранённому состоянию, а не несохранённому черновику формы:
/// пока VBAN выключен или сохранённое назначение невалидно, проверка не
/// ведётся. `None` останавливает монитор и освобождает локальный порт.
pub(crate) fn saved_health_target(settings: &VbanSettings) -> Option<VbanPingTarget> {
    let target = saved_vban_target(settings)?;
    let interval_secs = settings.validate_ping_interval().ok()?;
    Some(VbanPingTarget {
        addr: target.addr,
        interval: Duration::from_secs(u64::from(interval_secs)),
    })
}

/// Применить сохранённые настройки к обоим рантаймам: аудио-воркеру и
/// монитору доступности. Монитор следует тем же сохранённым полям, что и
/// отправка, но с собственным интервалом.
fn apply_saved_runtimes(state: &AppState) {
    let settings = state.settings_cache.read().audio.vban.clone();
    state.vban.configure(saved_vban_target(&settings));
    state
        .vban
        .health()
        .set_target(saved_health_target(&settings));
}

/// Чистое отображение состояния runtime в DTO статуса.
fn vban_status_dto(enabled: bool, sending: bool, last_error: Option<String>) -> VbanStatusDto {
    if !enabled {
        return VbanStatusDto {
            state: "disabled".to_string(),
            error: None,
        };
    }
    if sending {
        return VbanStatusDto {
            state: "sending".to_string(),
            error: None,
        };
    }
    match last_error {
        Some(error) => VbanStatusDto {
            state: "error".to_string(),
            error: Some(error),
        },
        None => VbanStatusDto {
            state: "idle".to_string(),
            error: None,
        },
    }
}

/// Сохранить настройки VBAN: валидация до применения, persist-first запись
/// секции `audio/vban` и применение цели к общему runtime после успешной
/// записи. Выключение и смена назначения отменяют текущие сетевые дорожки
/// (семантика `VbanOutput::configure`).
#[tauri::command]
pub async fn set_vban_settings(
    settings: VbanSettingsDto,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    info!(
        enabled = settings.enabled,
        port = settings.port,
        volume = settings.volume,
        "Saving VBAN settings"
    );

    // Валидация до применения: адрес нужен только включённому VBAN,
    // структурные поля проверяются всегда.
    settings.validate(settings.enabled)?;

    let settings_for_persist = settings.clone();
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_vban_settings(settings_for_persist)
    })
    .await
    .map_err(|e| format!("Не удалось сохранить настройки VBAN: {e}"))?;

    // Применяем к runtime только после успешного persist. При выключении
    // цель снимается: configure(None) отменяет production-дорожки, монитор
    // доступности прекращает проверку.
    let target = if settings.enabled {
        match build_vban_target(&settings) {
            Ok(target) => Some(target),
            Err(e) => {
                // Валидация выше уже отсеяла этот случай; безопасный fallback.
                warn!(error = %e, "Validated VBAN settings failed target build; disabling");
                None
            }
        }
    } else {
        None
    };
    state.vban.configure(target);
    state
        .vban
        .health()
        .set_target(saved_health_target(&settings));

    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Apply the durable snapshot, never an unsaved connection draft.
fn apply_saved_vban(state: &AppState) {
    apply_saved_runtimes(state);
}

/// Save only address, port and stream name, preserving the live controls.
#[tauri::command]
pub async fn set_vban_connection_settings(
    settings: VbanSettingsDto,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_vban_connection_settings(settings)
    })
    .await?;
    apply_saved_vban(&state);
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Persist and apply output gain without changing the saved destination.
#[tauri::command]
pub async fn set_vban_volume(
    volume: u8,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_vban_volume(volume)
    })
    .await?;
    apply_saved_vban(&state);
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Enable the saved destination or stop production output immediately.
#[tauri::command]
pub async fn set_vban_enabled(
    enabled: bool,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    super::persist_blocking(settings_manager.inner(), move |manager| {
        manager.set_vban_enabled(enabled)
    })
    .await?;
    apply_saved_vban(&state);
    super::emit_settings_changed(&app_handle);
    Ok(())
}

/// Отправить тестовый звук на параметры из формы.
///
/// Требует корректный IPv4 назначения независимо от флага `enabled` в
/// payload. Сохранённые настройки не меняются, runtime не переконфигурируется.
/// Тест идёт на отдельной дорожке: занятость тем же production-потоком
/// возвращает ошибку. IPC-вызов завершается после локального завершения
/// отправки (успех или ошибка дорожки); приём звука проверяется в Receptor.
#[tauri::command]
pub async fn test_vban_output(
    settings: VbanSettingsDto,
    state: State<'_, AppState>,
) -> Result<(), String> {
    info!(
        address = %settings.address,
        port = settings.port,
        stream_name = %settings.stream_name,
        volume = settings.volume,
        "Testing VBAN output"
    );

    // Тест всегда требует корректное назначение, даже если в форме VBAN
    // выключен: он отправляет именно на параметры из формы.
    settings.validate(true)?;
    let target = build_vban_target(&settings)?;

    // Декодируем встроенный тестовый звук вне IPC-потока; локально ничего
    // не звучит, история и TTS не затрагиваются.
    let pcm = tokio::task::spawn_blocking(|| {
        crate::audio::decode_audio(crate::assets::TEST_SOUND_MP3).map(Arc::new)
    })
    .await
    .map_err(|e| format!("Test sound decode task failed: {e}"))?
    .map_err(|e| format!("Failed to decode test sound: {e}"))?;

    // Ограничение ожидания: длительность звука + запас на старт дорожки.
    let deadline =
        Instant::now() + Duration::from_secs_f64(pcm.duration_secs()) + Duration::from_secs(10);

    // Отдельный handle тестовой дорожки: отправка выполняется в фоне
    // worker-а, здесь мы только ждём локальный терминальный статус. Занятость
    // тем же production-потоком возвращает ошибку.
    let vban = state.vban.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let handle = vban
            .test(target, pcm)
            .map_err(|e| format!("VBAN test rejected: {e}"))?;
        loop {
            match handle.status() {
                VbanPlaybackStatus::Finished => return Ok(()),
                VbanPlaybackStatus::Failed(error) => return Err(error),
                VbanPlaybackStatus::Stopped => {
                    return Err("VBAN test was stopped".to_string());
                }
                VbanPlaybackStatus::Pending
                | VbanPlaybackStatus::Playing
                | VbanPlaybackStatus::Paused => {}
            }
            if Instant::now() >= deadline {
                handle.stop();
                return Err("VBAN test timed out waiting for completion".to_string());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    })
    .await
    .map_err(|e| format!("VBAN test wait task failed: {e}"))?;

    match result {
        Ok(()) => {
            info!("VBAN test completed");
            Ok(())
        }
        Err(error) => {
            warn!(error = %error, "VBAN test failed");
            Err(format!("VBAN test failed: {error}"))
        }
    }
}

/// Диагностика production-отправки VBAN. Доступность приёмника не
/// определяется: статус отражает только локальную отправку и ошибки.
#[tauri::command]
pub fn get_vban_status(state: State<'_, AppState>) -> VbanStatusDto {
    let enabled = state.settings_cache.read().audio.vban.enabled;
    vban_status_dto(enabled, state.vban.is_sending(), state.vban.last_error())
}

/// Доступность приёмника VBAN (ROADMAP-139): снапшот монитора VBAN-PING.
///
/// `unknown` — результата ещё нет (серый), `alive` — приёмник ответил
/// (зелёный), `failed` — установлена ошибка (красный) с причиной `reason`:
/// `no_reply` — приёмник не отвечает, `local_socket` — локальная проверка
/// не смогла запуститься (например, занят UDP-порт), отдельное объяснение.
/// Проверка идёт в фоне независимо от открытой вкладки.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VbanHealthDto {
    pub state: String,
    pub reason: Option<String>,
}

impl From<&VbanHealthState> for VbanHealthDto {
    fn from(state: &VbanHealthState) -> Self {
        Self {
            state: state.state_key().to_string(),
            reason: state.reason_key().map(str::to_string),
        }
    }
}

#[tauri::command]
pub fn get_vban_health(state: State<'_, AppState>) -> VbanHealthDto {
    VbanHealthDto::from(&state.vban.health().state())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VbanSettings;

    #[test]
    fn vban_status_dto_disabled_ignores_runtime() {
        let dto = vban_status_dto(false, true, Some("stale".to_string()));
        assert_eq!(dto.state, "disabled");
        assert!(dto.error.is_none());
    }

    #[test]
    fn vban_status_dto_sending_suppresses_stale_error() {
        let dto = vban_status_dto(true, true, Some("old socket error".to_string()));
        assert_eq!(dto.state, "sending");
        assert!(dto.error.is_none());
    }

    #[test]
    fn vban_status_dto_error_carries_last_error() {
        let dto = vban_status_dto(true, false, Some("socket send failed".to_string()));
        assert_eq!(dto.state, "error");
        assert_eq!(dto.error.as_deref(), Some("socket send failed"));
    }

    #[test]
    fn vban_status_dto_idle_without_error() {
        let dto = vban_status_dto(true, false, None);
        assert_eq!(dto.state, "idle");
        assert!(dto.error.is_none());
    }

    #[test]
    fn vban_status_dto_serializes_contract_fields() {
        let dto = vban_status_dto(true, false, Some("x".to_string()));
        let value = serde_json::to_value(&dto).unwrap();
        assert!(value.get("state").is_some());
        assert!(value.get("error").is_some());
    }

    #[test]
    fn set_command_validation_requires_address_for_enabled() {
        let mut settings = VbanSettings {
            enabled: true,
            ..Default::default()
        };
        assert!(settings.validate(settings.enabled).is_err());

        settings.address = "127.0.0.1".to_string();
        assert!(settings.validate(settings.enabled).is_ok());
    }

    #[test]
    fn build_vban_target_maps_all_form_fields() {
        let settings = VbanSettings {
            enabled: true,
            address: "192.168.10.7".to_string(),
            port: 7001,
            stream_name: "Stream1".to_string(),
            volume: 60,
            ..Default::default()
        };
        let target = build_vban_target(&settings).unwrap();
        assert_eq!(target.addr.ip().to_string(), "192.168.10.7");
        assert_eq!(target.addr.port(), 7001);
        assert_eq!(target.stream_name, "Stream1");
        assert!((target.gain - 0.6).abs() < 0.0001);
    }

    #[test]
    fn saved_health_target_follows_the_saved_snapshot() {
        use std::net::{Ipv4Addr, SocketAddrV4};
        use std::time::Duration;

        // Disabled VBAN: no check at all.
        let disabled = VbanSettings {
            enabled: false,
            address: "192.168.1.50".to_string(),
            ..Default::default()
        };
        assert!(saved_health_target(&disabled).is_none());

        // Enabled with the agreed default interval.
        let enabled = VbanSettings {
            enabled: true,
            address: "192.168.1.50".to_string(),
            port: 6980,
            stream_name: "TTSBard".to_string(),
            volume: 100,
            ping_interval: 30,
        };
        let target = saved_health_target(&enabled).unwrap();
        assert_eq!(
            target.addr,
            SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 50), 6980)
        );
        assert_eq!(target.interval, Duration::from_secs(30));

        // An out-of-range interval keeps the check off (the load path
        // normalizes this; here it is the last line of defence).
        let damaged = VbanSettings {
            ping_interval: 0,
            ..enabled
        };
        assert!(saved_health_target(&damaged).is_none());
    }

    #[test]
    fn vban_health_dto_maps_state_and_reason_keys() {
        let dto = VbanHealthDto::from(&VbanHealthState::Unknown);
        assert_eq!(dto.state, "unknown");
        assert_eq!(dto.reason, None);

        let dto = VbanHealthDto::from(&VbanHealthState::Alive);
        assert_eq!(dto.state, "alive");
        assert_eq!(dto.reason, None);

        let dto = VbanHealthDto::from(&VbanHealthState::Failed(
            crate::audio::vban::health::VbanHealthFailure::NoReply,
        ));
        assert_eq!(dto.state, "failed");
        assert_eq!(dto.reason.as_deref(), Some("no_reply"));

        let dto = VbanHealthDto::from(&VbanHealthState::Failed(
            crate::audio::vban::health::VbanHealthFailure::LocalSocket("busy".to_string()),
        ));
        assert_eq!(dto.state, "failed");
        assert_eq!(dto.reason.as_deref(), Some("local_socket"));
        let value = serde_json::to_value(&dto).unwrap();
        assert!(value.get("state").is_some());
        assert!(value.get("reason").is_some());
    }
}
