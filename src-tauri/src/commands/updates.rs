//! Update check and install commands (ROADMAP-141, stages 1–2).
//!
//! Every status command returns the full [`UpdateStatusDto`] so the card and
//! the banner are always driven by one snapshot. Check failures are reported
//! as stable codes inside the DTO, never as command rejections, so the UI
//! maps them to fixed locale keys instead of showing raw backend text.
//!
//! The install commands ([`install_update`], [`cancel_update_install`]) drive
//! the backend pipeline from `updates::install`; progress travels through the
//! `update-install-progress` event, and command rejections carry stable
//! [`InstallErrorKind`]-compatible code strings.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::config::SettingsManager;
use crate::updates::install::{self, InstallState, UpdateInstallProgressDto};
use crate::updates::{status_snapshot, UpdateStatusDto, UpdatesState};

/// Read the current update state without starting a check. The cached
/// release from a previous process is probed and broadcast by the
/// scheduler's delayed startup pass instead of here.
#[tauri::command]
pub async fn get_update_status(
    settings_manager: State<'_, SettingsManager>,
    state: State<'_, Arc<UpdatesState>>,
    install: State<'_, Arc<InstallState>>,
) -> Result<UpdateStatusDto, String> {
    Ok(status_snapshot(&settings_manager, &state, &install))
}

/// One-shot frontend readiness signal: the settings snapshot is loaded and
/// the main window is mounted, so the scheduler may run its delayed startup
/// pass. Repeated calls have no effect (idempotent cancel).
#[tauri::command]
pub async fn mark_update_startup_ready(
    state: State<'_, Arc<UpdatesState>>,
) -> Result<(), String> {
    state.mark_startup_ready();
    Ok(())
}

/// Manual "check now": performs a fetch and records the attempt (time and
/// outcome). Like any completed attempt, a manual one moves the periodic
/// schedule base (`last_attempt_at`) — to itself on success, together with
/// the last success time.
#[tauri::command]
pub async fn check_for_updates(
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
    state: State<'_, Arc<UpdatesState>>,
    install: State<'_, Arc<InstallState>>,
) -> Result<UpdateStatusDto, String> {
    let manager = settings_manager.inner().clone();
    let status =
        crate::updates::perform_manual_check(&app_handle, &manager, state.inner(), install.inner())
            .await;
    // The schedule base may have moved; wake the scheduler so it re-computes
    // the next due time from the fresh check time.
    state.notify_schedule_changed();
    Ok(status)
}

/// Start the in-app update installation (ROADMAP-141 stage 2): check the
/// latest GitHub release, download and verify the NSIS installer, launch it
/// and coordinate graceful exit. The
/// pipeline runs detached; progress arrives through the
/// `update-install-progress` event, so a dropped invoke promise never
/// cancels a download. Rejections carry stable code strings.
#[tauri::command]
pub async fn install_update(
    app_handle: AppHandle,
    install: State<'_, Arc<InstallState>>,
) -> Result<UpdateInstallProgressDto, String> {
    install::begin_install(&app_handle, install.inner())
        .await
        .map_err(|code| code.to_string())
}

/// Cancel the running installation while it is still downloading (the modal
/// close button included). Returns the `cancelled` progress, or `None` when
/// nothing cancellable is running anymore.
#[tauri::command]
pub async fn cancel_update_install(
    install: State<'_, Arc<InstallState>>,
) -> Result<Option<UpdateInstallProgressDto>, String> {
    Ok(install::cancel_install(install.inner()))
}

/// Enable or disable the one-shot check at process startup. Mid-run changes
/// never fire a startup attempt: the attempt belongs to the process start
/// only.
#[tauri::command]
pub async fn set_updates_startup_check(
    settings_manager: State<'_, SettingsManager>,
    state: State<'_, Arc<UpdatesState>>,
    install: State<'_, Arc<InstallState>>,
    enabled: bool,
) -> Result<UpdateStatusDto, String> {
    let manager = settings_manager.inner().clone();
    super::persist_blocking(&manager, move |mgr| {
        mgr.set_updates_startup_check_enabled(enabled)
    })
    .await
    .map_err(|e| format!("Failed to save update startup check setting: {}", e))?;
    state.notify_schedule_changed();
    Ok(status_snapshot(&manager, state.inner(), &install))
}

/// Enable or disable the periodic check. The selected interval is kept, so
/// re-enabling restores the previous schedule; manual checks stay available.
#[tauri::command]
pub async fn set_updates_periodic_check(
    settings_manager: State<'_, SettingsManager>,
    state: State<'_, Arc<UpdatesState>>,
    install: State<'_, Arc<InstallState>>,
    enabled: bool,
) -> Result<UpdateStatusDto, String> {
    let manager = settings_manager.inner().clone();
    super::persist_blocking(&manager, move |mgr| {
        mgr.set_updates_periodic_check_enabled(enabled)
    })
    .await
    .map_err(|e| format!("Failed to save update periodic check setting: {}", e))?;
    state.notify_schedule_changed();
    Ok(status_snapshot(&manager, state.inner(), &install))
}

/// Change the periodic check interval (days); out-of-range values are
/// clamped to 1..=7 before persisting. The deadline is re-counted from the
/// last completed attempt with the new interval.
#[tauri::command]
pub async fn set_updates_check_interval(
    settings_manager: State<'_, SettingsManager>,
    state: State<'_, Arc<UpdatesState>>,
    install: State<'_, Arc<InstallState>>,
    days: u32,
) -> Result<UpdateStatusDto, String> {
    let manager = settings_manager.inner().clone();
    super::persist_blocking(&manager, move |mgr| {
        mgr.set_updates_check_interval_days(days)
    })
    .await
    .map_err(|e| format!("Failed to save update check interval: {}", e))?;
    state.notify_schedule_changed();
    Ok(status_snapshot(&manager, state.inner(), &install))
}

/// Hide the update panel for this release version, including after repeated
/// checks and restarts. The card keeps showing the release.
#[tauri::command]
pub async fn dismiss_update_panel(
    settings_manager: State<'_, SettingsManager>,
    state: State<'_, Arc<UpdatesState>>,
    install: State<'_, Arc<InstallState>>,
    version: String,
) -> Result<UpdateStatusDto, String> {
    #[cfg(debug_assertions)]
    if state.dismiss_preview_bubble(&version) {
        return Ok(status_snapshot(&settings_manager, &state, &install));
    }

    let manager = settings_manager.inner().clone();
    // Pin the state directory before the blocking persistence is dispatched:
    // the closure runs on another thread and must write the dismissal to the
    // root the caller saw, not to whichever root is effective by then (a
    // «Данные программы» switch in between would otherwise move the record).
    // A failed resolution is rejected up front — the command never writes to
    // a dynamically re-resolved path.
    let state_dir = manager.update_state_dir().ok_or_else(|| {
        "Failed to resolve the update state directory for the dismissal".to_string()
    })?;
    super::persist_blocking(&manager, move |mgr| {
        mgr.set_updates_dismissed_version_in(&state_dir, Some(version))
    })
    .await
    .map_err(|e| format!("Failed to save dismissed update version: {}", e))?;
    Ok(status_snapshot(&manager, state.inner(), &install))
}
