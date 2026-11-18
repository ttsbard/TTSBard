//! In-app installation of signed NSIS updates from GitHub Releases.
//!
//! The pipeline fetches the latest release, downloads its installer and detached
//! signature, verifies them, and stages the verified installer for launch.
//! A successful independent installer launch triggers graceful app shutdown.
//! A launch failure keeps the app running and lets the user retry.
//!
//! Cancellation is only possible during `checking`/`downloading`: the
//! download future is raced against a [`tokio_util::sync::CancellationToken`]
//! in a `select!`, so a cancel drops the request mid-flight without side
//! effects (nothing is written to disk before the signature is verified).
//! The cancel command only signals the token — the worker keeps owning the
//! job slot until it clears it on its own cancelled path, so a replacement
//! pipeline can never start while the old one is winding down. Freezing the
//! job (the point of no return after the signature check) re-checks the
//! token under the same lock, making a cancel-vs-finish race deterministic.
//!
//! Installing requires a configured updater public key, an NSIS installation,
//! and a matching installer and signature in the release assets.
//!
//!
//! Commands live in [`crate::commands::updates`]; progress travels through
//! the [`UPDATE_INSTALL_PROGRESS_EVENT`] wire event.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

use crate::config::SettingsManager;
use crate::updates::UpdateStatusDto;

/// Wire event carrying [`UpdateInstallProgressDto`]. Must match
/// `UPDATE_INSTALL_PROGRESS_EVENT` in `src/ipc/appUpdates.ts`; written as a
/// literal so the IPC contract inventory picks it up.
pub const UPDATE_INSTALL_PROGRESS_EVENT: &str = "update-install-progress";

/// NSIS uninstall registry key written by the Tauri NSIS bundle (product
/// name from `tauri.conf.json`).
#[cfg(windows)]
const NSIS_UNINSTALL_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\TTSBard";

/// Uninstall-key value storing the install directory. The Tauri NSIS
/// template writes it quoted: `WriteRegStr SHCTX "${UNINSTKEY}"
/// "InstallLocation" "$\"$INSTDIR$\""` — hence the quote stripping.
#[cfg(windows)]
const NSIS_INSTALL_LOCATION_VALUE: &str = "InstallLocation";

// ============================================================================
// Wire DTO
// ============================================================================

/// Progress of the in-app update installation, pushed to the UI through
/// [`UPDATE_INSTALL_PROGRESS_EVENT`]. `stage` is one of the stable names
/// below; a `failed` stage carries a stable error code from
/// [`InstallErrorKind`].
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UpdateInstallProgressDto {
    /// Pipeline stage: `checking`, `downloading`, `saving`, `launching`,
    /// `cancelled` or `failed`.
    pub stage: &'static str,
    /// Version of the release being installed; `None` before the check.
    pub version: Option<String>,
    /// Bytes received so far during `downloading`.
    pub downloaded: u64,
    /// Total download size when the server announced one.
    pub total: Option<u64>,
    /// Stable code of the failure, only for the `failed` stage.
    pub error_code: Option<&'static str>,
}

/// Stable code of a failed installation. The frontend maps each variant to a
/// fixed locale key; raw backend text never crosses the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallErrorKind {
    /// The updater is not configured (no public key) or the running app is
    /// not an NSIS installation.
    Unavailable,
    /// The check or the download request failed (DNS/connect/timeout/status).
    Network,
    /// The endpoint returned no usable manifest, or no artifact for this
    /// platform.
    NotFound,
    /// The download did not match its minisign signature.
    Signature,
    /// Writing or extracting the installer failed.
    Filesystem,
    /// The installer could not be launched.
    Launch,
    /// The endpoint announced nothing newer than the running version.
    NoUpdate,
}

impl InstallErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InstallErrorKind::Unavailable => "unavailable",
            InstallErrorKind::Network => "network",
            InstallErrorKind::NotFound => "not_found",
            InstallErrorKind::Signature => "signature",
            InstallErrorKind::Filesystem => "filesystem",
            InstallErrorKind::Launch => "launch",
            InstallErrorKind::NoUpdate => "no_update",
        }
    }
}

// ============================================================================
// Capability detection
// ============================================================================

fn updater_public_key(config: &tauri::Config) -> Option<&str> {
    let key = config
        .plugins
        .0
        .get("updater")?
        .get("pubkey")?
        .as_str()?
        .trim();
    (!key.is_empty()).then_some(key)
}

/// `true` when the running app itself was installed through the NSIS
/// installer: the uninstall registry key written by the Tauri NSIS bundle
/// (product name, per-user HKCU or per-machine HKLM hive) must record an
/// `InstallLocation` that matches the directory of the currently running
/// executable. A standalone `ttsbard.exe` copied next to an existing
/// installation (or anywhere else) reads a different directory and keeps the
/// manual GitHub route, so it can never update the installed copy — or be
/// replaced by the installer aimed at it.
#[cfg(windows)]
pub fn nsis_install_detected() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ, REG_VALUE_TYPE,
    };

    /// Read the `InstallLocation` string value the NSIS template writes into
    /// the uninstall key (`WriteRegStr SHCTX "${UNINSTKEY}" "InstallLocation"
    /// "$\"$INSTDIR$\""` — the path is wrapped in literal quote characters).
    fn install_location(root: HKEY, subkey: &[u16], value: &[u16]) -> Option<String> {
        unsafe {
            let mut hkey = HKEY::default();
            let status = RegOpenKeyExW(root, PCWSTR(subkey.as_ptr()), 0, KEY_READ, &mut hkey);
            if status.is_err() {
                return None;
            }
            // Install paths live well below 2048 bytes; a larger value is
            // treated as missing rather than being truncated.
            let mut buffer = [0u8; 2048];
            let mut len = buffer.len() as u32;
            let mut value_type = REG_VALUE_TYPE::default();
            let status = RegQueryValueExW(
                hkey,
                PCWSTR(value.as_ptr()),
                None,
                Some(&mut value_type),
                Some(buffer.as_mut_ptr()),
                Some(&mut len),
            );
            let _ = RegCloseKey(hkey);
            if status.is_err() {
                return None;
            }
            let raw = &buffer[..(len as usize).min(buffer.len())];
            // REG_SZ is UTF-16; trim the terminator and the template's quotes.
            let wide: Vec<u16> = raw
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let text = String::from_utf16_lossy(&wide)
                .trim_end_matches('\0')
                .trim_matches('"')
                .trim()
                .to_string();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
    }

    /// Normalize a directory path for comparison: resolve to its real casing
    /// via canonicalize and drop the extended-length prefix both sides get.
    fn normalized_dir(path: &std::path::Path) -> Option<String> {
        let canonical = path.canonicalize().ok()?;
        let text = canonical
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string();
        Some(text.to_lowercase())
    }

    let running_dir = match std::env::current_exe()
        .ok()
        .as_deref()
        .map(std::path::Path::parent)
    {
        Some(Some(dir)) => normalized_dir(dir),
        _ => None,
    };
    let Some(running_dir) = running_dir else {
        return false;
    };

    let mut subkey: Vec<u16> = NSIS_UNINSTALL_SUBKEY.encode_utf16().collect();
    subkey.push(0);
    let mut value: Vec<u16> = NSIS_INSTALL_LOCATION_VALUE.encode_utf16().collect();
    value.push(0);

    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .any(|root| {
            install_location(root, &subkey, &value)
                .and_then(|recorded| normalized_dir(std::path::Path::new(&recorded)))
                .is_some_and(|recorded| recorded == running_dir)
        })
}

#[cfg(not(windows))]
pub fn nsis_install_detected() -> bool {
    false
}

// ============================================================================
// Artifact availability probe
// ============================================================================

/// Outcome of the signed asset availability probe for one release version.
/// Only `Available` enables the «Обновить» button: a transport failure or a
/// not-yet-probed version leaves the button hidden until the artifact is
/// confirmed (releases published before the updater need the manual GitHub
/// route anyway).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactProbe {
    Unknown,
    Available,
    Missing,
}

async fn probe_signed_installer(version: &str) -> ArtifactProbe {
    match super::fetch_github_release().await {
        Ok(release)
            if release
                .tag_name
                .as_deref()
                .and_then(super::parse_version)
                .is_some_and(|tag| tag.to_string() == version) =>
        {
            if release.signed_installer().is_some() {
                ArtifactProbe::Available
            } else {
                ArtifactProbe::Missing
            }
        }
        Err(super::UpdateErrorKind::NotFound) => ArtifactProbe::Missing,
        _ => ArtifactProbe::Unknown,
    }
}

// ============================================================================
// Runtime state
// ============================================================================

/// The active install pipeline. `cancel` aborts the download while the
/// frontend modal (or its close button) requests it; `cancellable` mirrors
/// the pipeline stage so a cancel racing a finished download is a no-op.
struct InstallJob {
    version: String,
    cancellable: bool,
    cancel: tokio_util::sync::CancellationToken,
}

/// Install capability and runtime state, managed in `setup.rs` next to
/// [`crate::updates::UpdatesState`].
pub struct InstallState {
    /// The public key is configured and the app runs from an NSIS
    /// installation; static for the process lifetime.
    ready: bool,
    /// Probe result per found release version. Only *confirmed* results
    /// (`Available`/`Missing`) are recorded here: a transient `Unknown`
    /// transport failure must stay re-requestable.
    probes: Mutex<HashMap<String, ArtifactProbe>>,
    /// Versions with a probe in flight (dedupes repeated snapshots).
    probing: Mutex<HashSet<String>>,
    /// The active install pipeline, if any.
    job: Mutex<Option<InstallJob>>,
    /// Serializes status broadcasts: the snapshot is built and emitted while
    /// holding this mutex. Probe completions, normal checks (scheduled and
    /// manual) and the storage-root switch all publish through it, so the
    /// last emitted snapshot is always computed after every state change
    /// (probe result insert, attempt completion, root switch) that completed
    /// before it acquired the mutex — a stale event can never overtake a
    /// newer one.
    broadcast_order: Mutex<()>,
}

impl InstallState {
    pub fn new(ready: bool) -> Self {
        Self {
            ready,
            probes: Mutex::new(HashMap::new()),
            probing: Mutex::new(HashSet::new()),
            job: Mutex::new(None),
            broadcast_order: Mutex::new(()),
        }
    }

    pub fn ready(&self) -> bool {
        self.ready
    }

    /// Enter the shared broadcast critical section (see
    /// [`Self::broadcast_order`]): build the snapshot and emit while holding
    /// the guard, never across an await. The guard is deliberately exposed
    /// instead of wrapping emit callbacks so callers keep their own closure
    /// signatures.
    pub(crate) fn lock_broadcast(&self) -> MutexGuard<'_, ()> {
        self.broadcast_order.lock().expect("broadcast order lock")
    }

    /// `true` when the «Обновить» button may be shown for the given release
    /// version: the capability is present and the updater artifact is
    /// *confirmed* available (a completed probe returned `Available`). No
    /// probe yet, a transport failure (`Unknown`) and a proven-missing
    /// release all keep the button hidden — the manual GitHub route stays
    /// for those.
    pub fn install_available_for(&self, version: Option<&str>) -> bool {
        if !self.ready() {
            return false;
        }
        let Some(version) = version else {
            return false;
        };
        let probes = self.probes.lock().expect("artifact probes lock");
        probes.get(version) == Some(&ArtifactProbe::Available)
    }

    /// Spawn the signed asset probe for the announced release until a
    /// *confirmed* result is recorded. Every status refresh — an explicit
    /// "check now", a status read or the scheduled loop — reaches this
    /// initiator, so a probe that ended with a transient transport failure
    /// (`Unknown`) is re-requested on the next refresh instead of being
    /// wedged forever. A definitive result (`Available`/`Missing`) is final
    /// for the version and, on success, re-broadcasts the refreshed status
    /// snapshot so the card flips the button without waiting for the next
    /// check.
    pub fn ensure_artifact_probe(
        self: &Arc<Self>,
        app: &AppHandle,
        updates: &Arc<crate::updates::UpdatesState>,
        settings_manager: &SettingsManager,
        status: &UpdateStatusDto,
    ) {
        if !self.ready() || !status.update_available {
            return;
        }
        let Some(release) = status.latest.as_ref() else {
            return;
        };
        let version = release.version.clone();
        if !self.begin_probe(&version) {
            return;
        }
        let app = app.clone();
        let install = Arc::clone(self);
        let updates = Arc::clone(updates);
        let settings_manager = settings_manager.clone();
        tauri::async_runtime::spawn(async move {
            let probe = probe_signed_installer(&version).await;
            install.finish_artifact_probe(version, &settings_manager, &updates, probe, |status| {
                let _ = app.emit(crate::updates::UPDATE_STATUS_CHANGED_EVENT, &status);
            });
        });
    }

    /// Whether a probe for `version` may start: a confirmed result is final
    /// for the version, everything else — no probe yet, or an `Unknown`
    /// entry that a stale producer could have left behind — allows a fresh
    /// request. The in-flight set dedupes concurrent callers: `false` means
    /// a probe is not needed or one is already running.
    fn begin_probe(&self, version: &str) -> bool {
        {
            let probes = self.probes.lock().expect("artifact probes lock");
            if matches!(
                probes.get(version),
                Some(ArtifactProbe::Available) | Some(ArtifactProbe::Missing)
            ) {
                return false;
            }
        }
        let mut probing = self.probing.lock().expect("artifact probing lock");
        probing.insert(version.to_string())
    }

    /// Commit one finished probe. A confirmed result is stored under the
    /// version and re-broadcast through a fresh status snapshot so the card
    /// flips the button; the snapshot is built and emitted while holding the
    /// broadcast-order mutex, so a probe finishing concurrently can never
    /// publish an early status over this result. A transient `Unknown` is
    /// neither stored nor broadcast — the button stays hidden (it was hidden
    /// before) and a fresh request of the same version stays possible for the
    /// next initiator (explicit check, status read, scheduled loop).
    pub(crate) fn finish_artifact_probe<E>(
        &self,
        version: String,
        settings_manager: &SettingsManager,
        updates: &crate::updates::UpdatesState,
        probe: ArtifactProbe,
        emit: E,
    ) where
        E: Fn(UpdateStatusDto),
    {
        if probe != ArtifactProbe::Unknown {
            self.probes
                .lock()
                .expect("artifact probes lock")
                .insert(version.clone(), probe);
        }
        self.probing
            .lock()
            .expect("artifact probing lock")
            .remove(&version);
        info!(version = %version, probe = ?probe, "Updater artifact probe finished");
        if probe != ArtifactProbe::Unknown {
            let _order = self.lock_broadcast();
            let status = crate::updates::status_snapshot(settings_manager, updates, self);
            emit(status);
        }
    }

    /// Register a fresh pipeline job; `Err(())` when one is already active
    /// (single-flight guard). The returned token cancels the download.
    fn begin_job(&self) -> Result<tokio_util::sync::CancellationToken, ()> {
        let mut job = self.job.lock().expect("install job lock");
        if job.is_some() {
            return Err(());
        }
        let cancel = tokio_util::sync::CancellationToken::new();
        *job = Some(InstallJob {
            version: String::new(),
            cancellable: true,
            cancel: cancel.clone(),
        });
        Ok(cancel)
    }

    /// Record the release version of the running pipeline.
    fn set_job_version(&self, version: &str) {
        let mut job = self.job.lock().expect("install job lock");
        if let Some(active) = job.as_mut() {
            active.version = version.to_owned();
        }
    }

    /// Freeze the job against cancellation once the verified bytes are in
    /// hand — the point of no return. The cancellation check happens under
    /// the same lock as [`Self::cancel_job`], so a cancel racing the finished
    /// download wins atomically: `false` means the job was already cancelled
    /// and the pipeline must take its cancelled path instead of installing.
    fn freeze_job(&self) -> bool {
        let mut job = self.job.lock().expect("install job lock");
        match job.as_mut() {
            Some(active) if active.cancel.is_cancelled() => false,
            Some(active) => {
                active.cancellable = false;
                true
            }
            None => false,
        }
    }

    /// Clear the job slot on every terminal pipeline path. Only the owning
    /// worker calls this, so a cancelled-but-still-running worker cannot wipe
    /// a freshly registered replacement job.
    fn clear_job(&self) {
        *self.job.lock().expect("install job lock") = None;
    }

    /// Signal cancellation to the running pipeline. The job slot stays
    /// occupied until the worker itself wakes on the token and clears it, so
    /// a new pipeline can never start while the old one is still winding down
    /// (and can therefore never have its slot wiped or receive the old
    /// worker's events). Returns `None` when nothing cancellable is running
    /// (already committed to installing, already finished, or never started).
    fn cancel_job(&self) -> Option<UpdateInstallProgressDto> {
        let job = self.job.lock().expect("install job lock");
        let active = job.as_ref()?;
        if !active.cancellable {
            return None;
        }
        // Signal first: the pipeline's select! sees the token and drops the
        // download future mid-flight.
        active.cancel.cancel();
        let version = active.version.clone();
        Some(UpdateInstallProgressDto {
            stage: "cancelled",
            version: if version.is_empty() {
                None
            } else {
                Some(version)
            },
            downloaded: 0,
            total: None,
            error_code: None,
        })
    }

    /// Try to register a fresh pipeline job, waiting out a cancelled worker
    /// that is still winding down. Returns the cancel token of the new job,
    /// or `None` when the slot stayed busy for the whole grace window.
    async fn begin_job_with_grace(
        &self,
        grace: std::time::Duration,
    ) -> Option<tokio_util::sync::CancellationToken> {
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            match self.begin_job() {
                Ok(cancel) => return Some(cancel),
                Err(()) if tokio::time::Instant::now() >= deadline => return None,
                Err(()) => {
                    // The previous worker is cancelling: it clears its own
                    // slot, normally within a single scheduler tick.
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        }
    }
}

// ============================================================================
// Plugin registration
// ============================================================================

/// Set up in-app installation for builds with a public key and an NSIS installation.
pub fn setup_install_state(app: &tauri::App) -> Arc<InstallState> {
    let ready = updater_public_key(app.config()).is_some() && nsis_install_detected();
    let state = Arc::new(InstallState::new(ready));
    app.manage(state.clone());
    state
}

// ============================================================================
// Install pipeline
// ============================================================================

fn emit_progress(
    app: &AppHandle,
    stage: &'static str,
    version: Option<&str>,
    downloaded: u64,
    total: Option<u64>,
    error_code: Option<&'static str>,
) {
    let payload = UpdateInstallProgressDto {
        stage,
        version: version.map(str::to_owned),
        downloaded,
        total,
        error_code,
    };
    let _ = app.emit(UPDATE_INSTALL_PROGRESS_EVENT, payload);
}

/// Grace window for a cancelled worker to release the job slot before a fresh
/// pipeline rejects with `busy`.
const JOB_SLOT_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Start the install pipeline (single-flight guard). `Ok` returns the
/// initial `checking` progress; `Err` carries a stable [`InstallErrorKind`]
/// code for an immediate rejection. The pipeline itself runs detached: all
/// progress travels through [`UPDATE_INSTALL_PROGRESS_EVENT`], so a dropped
/// invoke promise never cancels a download.
pub async fn begin_install(
    app: &AppHandle,
    install: &Arc<InstallState>,
) -> Result<UpdateInstallProgressDto, &'static str> {
    if !install.ready() {
        return Err(InstallErrorKind::Unavailable.as_str());
    }
    // A just-cancelled worker releases its own slot; wait out that wind-down
    // instead of rejecting the retry with a confusing `busy`.
    let Some(cancel) = install.begin_job_with_grace(JOB_SLOT_GRACE).await else {
        return Err("busy");
    };

    let task_app = app.clone();
    let task_install = Arc::clone(install);
    tauri::async_runtime::spawn(async move {
        run_install_pipeline(task_app, task_install, cancel).await;
    });

    Ok(UpdateInstallProgressDto {
        stage: "checking",
        version: None,
        downloaded: 0,
        total: None,
        error_code: None,
    })
}

/// Request cancellation of the running pipeline. Returns the `cancelled`
/// progress when a cancellable job was aborted, or `None` when nothing
/// cancellable is running anymore (already committed to installing, already
/// finished, or never started).
pub fn cancel_install(install: &Arc<InstallState>) -> Option<UpdateInstallProgressDto> {
    let cancelled = install.cancel_job()?;
    info!(version = ?cancelled.version, "Update install cancelled by the user");
    Some(cancelled)
}

async fn run_install_pipeline(
    app: AppHandle,
    install: Arc<InstallState>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let finish =
        |version: Option<String>, stage: &'static str, error_code: Option<&'static str>| {
            install.clear_job();
            emit_progress(&app, stage, version.as_deref(), 0, None, error_code);
        };

    emit_progress(&app, "checking", None, 0, None, None);

    let Some(public_key) = updater_public_key(app.config()) else {
        finish(None, "failed", Some(InstallErrorKind::Unavailable.as_str()));
        return;
    };
    let release = tokio::select! {
        result = super::fetch_github_release() => match result {
            Ok(release) => release,
            Err(error) => {
                let kind = match error {
                    super::UpdateErrorKind::NotFound | super::UpdateErrorKind::Parse => InstallErrorKind::NotFound,
                    _ => InstallErrorKind::Network,
                };
                finish(None, "failed", Some(kind.as_str()));
                return;
            }
        },
        _ = cancel.cancelled() => {
            finish(None, "cancelled", None);
            return;
        }
    };
    let Some(version) = release.tag_name.as_deref().and_then(super::parse_version) else {
        finish(None, "failed", Some(InstallErrorKind::NotFound.as_str()));
        return;
    };
    let version = version.to_string();
    if !super::version_is_newer(&version, super::current_version()) {
        finish(None, "failed", Some(InstallErrorKind::NoUpdate.as_str()));
        return;
    }
    let Some(installer) = release.signed_installer() else {
        finish(
            Some(version),
            "failed",
            Some(InstallErrorKind::NotFound.as_str()),
        );
        return;
    };
    install.set_job_version(&version);
    emit_progress(&app, "downloading", Some(&version), 0, None, None);

    let bytes = tokio::select! {
        result = installer.download(public_key, |downloaded, total| {
            emit_progress(&app, "downloading", Some(&version), downloaded, total, None);
        }) => match result {
            Ok(bytes) => bytes,
            Err(kind) => {
                warn!(?kind, "Update download or signature verification failed");
                finish(Some(version), "failed", Some(kind.as_str()));
                return;
            }
        },
        _ = cancel.cancelled() => {
            finish(Some(version), "cancelled", None);
            return;
        }
    };

    // Point of no return: freeze atomically against a racing cancel (same
    // lock), then the verified artifact is committed to installation.
    if !install.freeze_job() {
        finish(Some(version), "cancelled", None);
        return;
    }

    #[cfg(windows)]
    {
        use crate::updates::launch;
        let current_exe_args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
        let parameters =
            launch::join_parameters(&launch::nsis_parameters(app.config(), &current_exe_args));
        let app_name = app.package_info().name.clone();
        let install_app = app.clone();
        let install_version = version.clone();
        let handoff = tokio::task::spawn_blocking(move || {
            emit_progress(
                &install_app,
                "saving",
                Some(&install_version),
                0,
                None,
                None,
            );
            let staged = launch::prepare_installer_file(&app_name, &install_version, &bytes)
                .map_err(|error| (InstallErrorKind::Filesystem, error))?;
            emit_progress(
                &install_app,
                "launching",
                Some(&install_version),
                0,
                None,
                None,
            );
            let result = launch::launch_nsis_installer(&staged, &parameters, &bytes);
            if result.is_err() {
                launch::discard_staged_installer(&staged);
            }
            result.map_err(|error| (InstallErrorKind::Launch, error))
        })
        .await;
        let failure = match handoff {
            Ok(Ok(())) => None,
            Ok(Err((kind, error))) => {
                warn!(%error, ?kind, "Update installer handoff failed");
                Some(kind)
            }
            Err(error) => {
                warn!(%error, "Update installer handoff task failed");
                Some(InstallErrorKind::Launch)
            }
        };
        if let Some(kind) = failure {
            finish(Some(version), "failed", Some(kind.as_str()));
            return;
        }

        info!(version = %version, "Update installer launched; starting the graceful shutdown");
        // This saves the editor and window state once. Keep the install slot
        // occupied until the process exits.
        crate::shutdown::coordinate_shutdown(app).await;
    }

    #[cfg(not(windows))]
    {
        drop(bytes);
        finish(
            Some(version),
            "failed",
            Some(InstallErrorKind::Unavailable.as_str()),
        );
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_error_kind_codes_are_stable() {
        assert_eq!(InstallErrorKind::Unavailable.as_str(), "unavailable");
        assert_eq!(InstallErrorKind::Network.as_str(), "network");
        assert_eq!(InstallErrorKind::NotFound.as_str(), "not_found");
        assert_eq!(InstallErrorKind::Signature.as_str(), "signature");
        assert_eq!(InstallErrorKind::Filesystem.as_str(), "filesystem");
        assert_eq!(InstallErrorKind::Launch.as_str(), "launch");
        assert_eq!(InstallErrorKind::NoUpdate.as_str(), "no_update");
    }

    #[test]
    fn updater_requires_a_configured_public_key() {
        let mut config = tauri::Config::default();
        assert!(updater_public_key(&config).is_none());
        config.plugins.0.insert(
            "updater".into(),
            serde_json::json!({"pubkey": "  example-key  "}),
        );
        assert_eq!(updater_public_key(&config), Some("example-key"));
    }

    /// The install button requires a *confirmed* updater artifact: no probe,
    /// a transport failure (`Unknown`) and a proven-missing release all hide
    /// it; only an explicit `Available` shows it, and ready=false hides it
    /// everywhere.
    #[test]
    fn install_available_requires_confirmed_artifact() {
        let state = InstallState::new(false);
        assert!(!state.install_available_for(Some("1.9.0")));
        assert!(!state.install_available_for(None));

        let ready = InstallState::new(true);
        // No probe yet: hidden until confirmed.
        assert!(!ready.install_available_for(Some("1.9.0")));
        assert!(!ready.install_available_for(None));
        // Transport failure: still hidden.
        ready
            .probes
            .lock()
            .expect("artifact probes lock")
            .insert("1.8.0".to_string(), ArtifactProbe::Unknown);
        assert!(!ready.install_available_for(Some("1.8.0")));
        // Proven missing: hidden.
        ready
            .probes
            .lock()
            .expect("artifact probes lock")
            .insert("1.7.0".to_string(), ArtifactProbe::Missing);
        assert!(!ready.install_available_for(Some("1.7.0")));
        // Confirmed available: the only visible case.
        ready
            .probes
            .lock()
            .expect("artifact probes lock")
            .insert("1.9.0".to_string(), ArtifactProbe::Available);
        assert!(ready.install_available_for(Some("1.9.0")));
    }

    /// Single-flight, cancellation and freeze semantics:
    ///
    /// - a running job rejects a second pipeline;
    /// - cancelling only signals the token — the slot stays occupied until
    ///   the owning worker clears it, so a replacement pipeline can never
    ///   start while the old one is winding down;
    /// - a frozen job (verified bytes in hand) can no longer be cancelled;
    /// - `freeze_job` atomically re-checks cancellation under the same lock,
    ///   so a cancel racing the finished download wins deterministically.
    #[test]
    fn job_slot_is_single_flight_and_cancellation_aware() {
        let install = InstallState::new(true);
        let cancel = install.begin_job().expect("first job starts");
        assert!(install.begin_job().is_err(), "second job is rejected");

        // Cancelling a non-frozen job signals the token but the slot stays
        // with the worker: a fresh begin_job is still rejected while the old
        // worker winds down.
        let cancelled = install.cancel_job().expect("cancellable job is cancelled");
        assert_eq!(cancelled.stage, "cancelled");
        assert!(cancelled.version.is_none(), "version not yet recorded");
        assert!(cancel.is_cancelled());
        assert!(
            install.begin_job().is_err(),
            "the slot belongs to the worker until it clears it"
        );

        // The worker's cancelled path clears the slot; only then a new job
        // can start (mirrored here by the manual clear the pipeline does).
        install.clear_job();
        assert!(
            install.begin_job().is_ok(),
            "slot freed by the owning worker"
        );

        // A frozen job reached the point of no return: cancel is a no-op.
        install.clear_job();
        let _ = install.begin_job().expect("second job starts");
        install.set_job_version("1.9.0");
        assert!(install.freeze_job(), "a live job freezes");
        assert!(
            install.cancel_job().is_none(),
            "frozen job cannot be cancelled"
        );
        assert!(
            install.begin_job().is_err(),
            "frozen job stays until the pipeline ends"
        );
    }

    /// Cancel-vs-freeze race: the check happens under the same lock as
    /// cancel, so a cancelled token always makes freeze fail — the pipeline
    /// takes the cancelled path instead of installing.
    #[test]
    fn freeze_loses_against_a_cancelled_token() {
        let install = InstallState::new(true);
        let cancel = install.begin_job().expect("job starts");
        install.set_job_version("1.9.0");

        // The user cancels right after the download finished.
        assert!(install.cancel_job().is_some());
        assert!(!install.freeze_job(), "cancelled job never freezes");
        // The pipeline clears the slot on its cancelled path.
        install.clear_job();

        // A fresh job freezes normally.
        assert!(
            install.begin_job().is_ok(),
            "slot freed for the replacement"
        );
        assert!(install.freeze_job());
        let _ = cancel;
    }

    /// `begin_job_with_grace` waits out a cancelling worker instead of
    /// rejecting immediately, but still gives up after the grace window.
    #[tokio::test]
    async fn begin_job_with_grace_waits_for_wind_down() {
        let install = InstallState::new(true);
        let cancel = install.begin_job().expect("job starts");

        // With the slot busy, a zero grace window reports the busy state.
        let busy = install
            .begin_job_with_grace(std::time::Duration::ZERO)
            .await;
        assert!(busy.is_none(), "no grace: busy is reported");

        // Once the worker clears the slot, the wait returns the new token.
        let wait = install.begin_job_with_grace(std::time::Duration::from_secs(2));
        cancel.cancel();
        install.clear_job();
        let token = tokio::time::timeout(std::time::Duration::from_secs(2), wait)
            .await
            .expect("grace wait resolves after the slot is freed")
            .expect("a freed slot yields a fresh token");
        assert!(!token.is_cancelled());
    }

    // ============ artifact probe retry and ordering (race fix) ============

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A ready manager with a persisted release the probe flow can announce.
    fn probe_settings_manager(tag: &str) -> (SettingsManager, std::path::PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-probe-{tag}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        manager
            .set_updates_check_result(
                "2026-11-17T08:00:00Z".to_string(),
                crate::config::UpdateRelease {
                    version: "9.9.9".to_string(),
                    name: Some("TTSBard 9.9.9".to_string()),
                    notes: "What is new".to_string(),
                    url: "https://github.com/ttsbard/TTSBard/releases/tag/v9.9.9".to_string(),
                    published_at: None,
                },
            )
            .unwrap();
        (manager, dir)
    }

    /// A transient `Unknown` must leave the version re-requestable: the next
    /// initiator (explicit check, status read, scheduled loop) probes the
    /// same version again, a later confirmed result lands and is broadcast,
    /// and only a *confirmed* result becomes final.
    #[test]
    fn unknown_probe_is_retried_and_a_confirmed_result_is_final() {
        let (manager, dir) = probe_settings_manager("retry");
        let updates = crate::updates::UpdatesState::new();
        let install = InstallState::new(true);
        let broadcasts = AtomicUsize::new(0);

        // First probe fails transiently: nothing stored, nothing broadcast.
        assert!(install.begin_probe("9.9.9"), "the first request starts");
        install.finish_artifact_probe(
            "9.9.9".to_string(),
            &manager,
            &updates,
            ArtifactProbe::Unknown,
            |_status| {
                broadcasts.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(
            broadcasts.load(Ordering::SeqCst),
            0,
            "an Unknown is never broadcast"
        );
        assert!(
            install.probes.lock().unwrap().is_empty(),
            "an Unknown is not stored as a confirmed result"
        );
        assert!(
            !install.install_available_for(Some("9.9.9")),
            "the button stays hidden until confirmed"
        );

        // The next initiator probes the same version again.
        assert!(
            install.begin_probe("9.9.9"),
            "a fresh request of the same version must be allowed after an Unknown"
        );
        install.finish_artifact_probe(
            "9.9.9".to_string(),
            &manager,
            &updates,
            ArtifactProbe::Available,
            |_status| {
                broadcasts.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(
            broadcasts.load(Ordering::SeqCst),
            1,
            "the confirmed result is broadcast"
        );
        assert!(install.install_available_for(Some("9.9.9")));

        // A confirmed result is final for the version.
        assert!(
            !install.begin_probe("9.9.9"),
            "a confirmed result must not be re-probed"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// In-flight dedup is preserved: while a probe is running, a concurrent
    /// request of the same version is rejected; once it completes with a
    /// transient failure, fresh requests are allowed again.
    #[test]
    fn concurrent_probe_requests_are_deduped_while_in_flight() {
        let (manager, dir) = probe_settings_manager("dedup");
        let updates = crate::updates::UpdatesState::new();
        let install = InstallState::new(true);

        assert!(install.begin_probe("9.9.9"), "the first request starts");
        assert!(
            !install.begin_probe("9.9.9"),
            "a concurrent request of an in-flight version is deduped"
        );

        install.finish_artifact_probe(
            "9.9.9".to_string(),
            &manager,
            &updates,
            ArtifactProbe::Unknown,
            |_status| {},
        );
        assert!(
            install.begin_probe("9.9.9"),
            "after the transient failure a fresh request is allowed"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Broadcast ordering: a probe finishing while another is mid-emit must
    /// wait for the broadcast mutex — it can neither emit out of turn nor
    /// publish an early snapshot over the newer confirmed result. Its own
    /// snapshot is computed after the earlier insert and therefore carries it.
    #[test]
    fn probe_broadcasts_are_serialized_and_carry_prior_results() {
        let (manager, dir) = probe_settings_manager("order");
        let updates = std::sync::Arc::new(crate::updates::UpdatesState::new());
        let install = std::sync::Arc::new(InstallState::new(true));

        // Probe A enters the broadcast critical section and holds it mid-emit.
        let (in_emit, in_emit_rx) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel::<()>();
        let install_a = std::sync::Arc::clone(&install);
        let manager_a = manager.clone();
        let updates_a = std::sync::Arc::clone(&updates);
        let probe_a = std::thread::spawn(move || {
            install_a.finish_artifact_probe(
                "9.9.9".to_string(),
                &manager_a,
                &updates_a,
                ArtifactProbe::Available,
                move |_status: UpdateStatusDto| {
                    let _ = in_emit.send(());
                    release_rx.recv().expect("the release signal arrives");
                },
            );
        });
        in_emit_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("probe A entered its emit");

        // Probe B finishes while A holds the mutex: it must not emit yet.
        let (b_emitted, b_emitted_rx) = std::sync::mpsc::channel();
        let install_b = std::sync::Arc::clone(&install);
        let manager_b = manager.clone();
        let updates_b = updates;
        let probe_b = std::thread::spawn(move || {
            install_b.finish_artifact_probe(
                "9.9.8".to_string(),
                &manager_b,
                &updates_b,
                ArtifactProbe::Available,
                move |status: UpdateStatusDto| {
                    let _ = b_emitted.send(status);
                },
            );
        });
        assert!(
            b_emitted_rx
                .recv_timeout(Duration::from_millis(150))
                .is_err(),
            "a concurrent broadcast must wait while the order mutex is held"
        );

        release.send(()).expect("the release is sent");
        probe_a.join().expect("probe A finishes");
        let status = b_emitted_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("probe B emits after A released the mutex");
        probe_b.join().expect("probe B finishes");
        assert!(
            status.install_available,
            "B's snapshot was computed after A's confirmed insert and carries it"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
