//! Update checks through GitHub Releases (ROADMAP-141, stage 2).
//!
//! The module owns three things:
//! - the wire DTOs shared with the frontend ([`UpdateStatusDto`],
//!   [`ReleaseInfoDto`]);
//! - the GitHub "latest release" fetch with SemVer comparison and plain-text
//!   notes sanitization;
//! - the background scheduler driving two independent saved switches: one
//!   startup attempt per process and the periodic interval. Every completed
//!   attempt (success or failure, manual included) moves the periodic base;
//!   failed checks are never retried automatically.
//!
//! The found release is persisted next to the attempt outcome in
//! `<data_root>/updates/state.json` (see [`state::UpdateStateRecord`] and
//! [`crate::config::UpdateRelease`]; only the schedule switches live in
//! `settings.json`), so a restart restores the card, the panel and the last
//! attempt result without mistaking a saved time for a fresh success. An
//! in-flight check is bound to the data root captured at its start: if the
//! «Данные программы» root changes mid-run (see `commands::storage`), the
//! completion lands in the original root and never touches the new root's
//! file, card or periodic base; every snapshot and schedule computation
//! pairs the state file of the *current* root with in-memory records of
//! that same root only. Command wrappers live in
//! [`crate::commands::updates`]. All command errors of a check are reported
//! as stable codes inside [`UpdateStatusDto`], so raw backend text never
//! reaches the UI.
//!
//! In-app update installation (the Tauri updater plugin, signature
//! verification and the NSIS installer launch) lives in [`install`]; the
//! Windows-only handoff to the launched installer lives in [`launch`].

mod artifacts;
pub mod install;
pub mod state;

/// Launching the signature-verified NSIS installer as an independent
/// process, without the plugin's forced `process::exit(0)` (see [`install`]).
#[cfg(windows)]
pub mod launch;

use std::future::Future;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::SettingsManager;

/// GitHub repository that publishes TTSBard releases.
const RELEASES_API_URL: &str = "https://api.github.com/repos/ttsbard/TTSBard/releases/latest";
/// Timeout of a single GitHub API request.
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Release notes travel as plain text; cap them so a pathological release
/// body cannot bloat IPC payloads.
const MAX_NOTES_CHARS: usize = 20_000;
/// Wire event carrying [`UpdateStatusDto`]. Must match
/// `UPDATE_STATUS_CHANGED_EVENT` in `src/ipc/appUpdates.ts`; written as a
/// literal so the IPC contract inventory picks it up.
pub const UPDATE_STATUS_CHANGED_EVENT: &str = "update-status-changed";

// ============================================================================
// Wire DTOs
// ============================================================================

/// A stable GitHub release offered to the user. One type for the wire and the
/// persisted copy: `UpdateStatusDto.latest` and `updates/state.json →
/// last_release` share this exact shape.
pub type ReleaseInfoDto = crate::config::UpdateRelease;

/// Stable code of the last failed check. The frontend maps each variant to a
/// fixed locale key; arbitrary backend error text never crosses the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateErrorKind {
    /// DNS/connect/timeout failure or a non-success, non-rate-limit status.
    Network,
    /// GitHub answered 403/429: rate limit or blocked request.
    RateLimited,
    /// 404 or a non-stable payload: no usable latest stable release.
    NotFound,
    /// The response was not the expected release JSON.
    Parse,
    /// The result could not be saved to the local update state file.
    Save,
}

impl UpdateErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            UpdateErrorKind::Network => "network",
            UpdateErrorKind::RateLimited => "rate_limited",
            UpdateErrorKind::NotFound => "not_found",
            UpdateErrorKind::Parse => "parse",
            UpdateErrorKind::Save => "save",
        }
    }
}

/// Full update state delivered to the UI: the persisted schedule plus the
/// result of the most recent attempt (release and times come from
/// `updates/state.json`, so they survive a restart). `last_attempt_*` and
/// `last_check_at` are kept apart, so the UI can distinguish "never
/// checked", a completed success and a completed failure, show when the
/// last attempt ran and never present an old success as a new one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateStatusDto {
    /// Version of the running application.
    pub current_version: String,
    /// The known latest release is newer than the running version.
    pub update_available: bool,
    /// The most recent successful check result; kept across failed re-checks.
    pub latest: Option<ReleaseInfoDto>,
    /// RFC3339 (UTC) of the last completed attempt (success or failure,
    /// manual included); `None` — no attempt has completed yet.
    pub last_attempt_at: Option<String>,
    /// Stable code of the last failed attempt; `None` — the last attempt
    /// succeeded or no attempt has completed yet.
    pub last_attempt_error: Option<String>,
    /// RFC3339 (UTC) of the last successful check, including manual ones.
    /// Unchanged by a failed attempt.
    pub last_check_at: Option<String>,
    /// A check is currently in flight.
    pub checking: bool,
    /// Persisted one-shot check-at-process-startup switch.
    pub startup_check_enabled: bool,
    /// Persisted periodic check switch.
    pub periodic_check_enabled: bool,
    /// Persisted periodic check interval in days (1..=7).
    pub check_interval_days: u32,
    /// Version whose update panel the user closed.
    pub dismissed_version: Option<String>,
    /// The app has an updater public key, runs from its NSIS install directory,
    /// and the latest release contains the matching installer and signature.
    pub install_available: bool,
}

// ============================================================================
// Version comparison
// ============================================================================

/// Parse a release tag (`1.9.0`, `v1.9.0`) as SemVer, ignoring build metadata.
fn parse_version(raw: &str) -> Option<semver::Version> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    semver::Version::parse(trimmed).ok()
}

/// `true` when `candidate` is a strictly newer SemVer than `current`. An
/// unparseable version never reports an update.
fn version_is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

// ============================================================================
// Notes sanitization
// ============================================================================

/// Reduce a release body to displayable text: drop control characters
/// (except line breaks and tabs) and cap the length. The UI renders this as
/// Markdown with raw HTML disabled and images/links stripped (see
/// `SettingsGeneral.vue`), so no remote markup becomes active content.
fn sanitize_notes(body: Option<&str>) -> String {
    let trimmed = body.unwrap_or("").trim();
    let mut notes = String::with_capacity(trimmed.len().min(MAX_NOTES_CHARS * 2));
    for (total, ch) in trimmed.chars().enumerate() {
        if total >= MAX_NOTES_CHARS {
            notes.push('…');
            break;
        }
        let keep = ch == '\n' || ch == '\r' || ch == '\t' || !ch.is_control();
        if keep {
            notes.push(ch);
        }
    }
    notes
}

// ============================================================================
// Runtime state
// ============================================================================

/// Outcome of one completed check attempt, kept in memory for the current
/// process. The error is the stable wire code (`UpdateErrorKind::as_str`);
/// an unknown persisted code is carried through unchanged, so a seeded
/// record never turns a stored failure into a success. It always pairs the
/// attempt time with the outcome, so the UI never sees a new error next to
/// an old attempt time or vice versa. `dir` is the identity of the state
/// file the attempt was persisted under (the state directory captured at
/// the attempt's start): the record only ever applies to that root. The
/// same pair is persisted (best effort) for restarts.
#[derive(Debug, Clone, PartialEq)]
struct AttemptRecord {
    at: DateTime<Utc>,
    error: Option<String>,
    dir: Option<PathBuf>,
}

/// Runtime update state shared between the commands, the scheduler and the UI
/// event. Everything the UI needs across restarts (schedule, attempt outcome,
/// check time, the found release, the dismissed version) lives on disk via
/// [`SettingsManager`] — the schedule in `settings.json`, the rest in
/// `updates/state.json` (see [`state`]); the freshest attempt of the
/// current process is additionally held here, so a failed persistence does
/// not hide this session's result. An in-memory record applies only to the
/// root it was produced under (see [`AttemptRecord`]); readers fall back to
/// the persisted record of the current root.
pub struct UpdatesState {
    /// The most recent completed attempt of this process (fetch finished,
    /// outcome included), tagged with the state directory captured at its
    /// start. `None` until the first attempt of this process. The mutex is
    /// also the publish lock ([`UpdatesState::publish_attempt`],
    /// [`UpdatesState::read_attempt_with`]): a completion's persistence and
    /// its runtime record are published as one critical section, and every
    /// combined read of the runtime record plus the persisted record of the
    /// same root holds it too. Synchronous work only inside — the lock is
    /// never held across an await, a network call or an emit. Shared by
    /// handle so the blocking worker of a completion can own a clone while
    /// the transaction runs off the async workers.
    last_attempt: Arc<Mutex<Option<AttemptRecord>>>,
    /// Wakes the scheduler when the persisted schedule changes or a manual
    /// check moved the schedule base.
    schedule_changed: tokio::sync::Notify,
    /// Wakes the scheduler when an in-flight check finishes, so a due
    /// scheduled check waits instead of spinning against the guard.
    check_finished: tokio::sync::Notify,
    /// One-shot signal that the frontend is ready (settings loaded and the
    /// main window mounted). The spawned scheduler waits for it before its
    /// delayed startup pass; cancelling is idempotent, so a repeated
    /// `mark_update_startup_ready` call has no effect.
    startup_ready: CancellationToken,
    /// Set before the first await of a manual check. The scheduler's startup
    /// branch skips its automatic attempt when a manual check already started
    /// in this process.
    manual_check_started: AtomicBool,
    /// Single check at a time across the scheduler and manual commands.
    checking: AtomicBool,
    #[cfg(debug_assertions)]
    preview_dismissal: Option<Mutex<Option<String>>>,
}

impl Default for UpdatesState {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdatesState {
    pub fn new() -> Self {
        Self {
            last_attempt: Arc::new(Mutex::new(None)),
            schedule_changed: tokio::sync::Notify::new(),
            check_finished: tokio::sync::Notify::new(),
            startup_ready: CancellationToken::new(),
            manual_check_started: AtomicBool::new(false),
            checking: AtomicBool::new(false),
            #[cfg(debug_assertions)]
            preview_dismissal: std::env::var("TTSBARD_UPDATE_PREVIEW_BUBBLE")
                .is_ok_and(|value| value == "1")
                .then(|| Mutex::new(None)),
        }
    }

    /// Wake the scheduler so it recomputes the next deadline from the current
    /// persisted settings.
    pub fn notify_schedule_changed(&self) {
        self.schedule_changed.notify_one();
    }

    /// One-shot frontend readiness signal for the scheduler's delayed
    /// startup pass. Cancelling is idempotent: repeated calls have no
    /// additional effect.
    pub fn mark_startup_ready(&self) {
        self.startup_ready.cancel();
    }

    /// Token the spawned scheduler waits on before its startup pass.
    fn startup_ready(&self) -> &CancellationToken {
        &self.startup_ready
    }

    /// Record that a manual check started in this process. The scheduler's
    /// startup branch reads it to skip its automatic attempt when the user
    /// checked first.
    fn mark_manual_check_started(&self) {
        self.manual_check_started.store(true, Ordering::SeqCst);
    }

    fn manual_check_started(&self) -> bool {
        self.manual_check_started.load(Ordering::SeqCst)
    }

    fn is_checking(&self) -> bool {
        self.checking.load(Ordering::SeqCst)
    }

    /// The in-memory attempt record when it belongs to `current` — the state
    /// directory of the effective data root, as captured at the attempt's
    /// start. Records produced under another root (the root changed after
    /// the attempt began or completed) are ignored: a completed check of a
    /// previous data root never shows in or drives the current one.
    /// Test-only: production readers use [`UpdatesState::read_attempt_with`]
    /// so the record and the persisted record are observed as one pair.
    #[cfg(test)]
    fn attempt_of_root(&self, current: Option<&Path>) -> Option<AttemptRecord> {
        let attempt = self
            .last_attempt
            .lock()
            .expect("updates last_attempt lock")
            .clone();
        attempt.filter(|record| record.dir.as_deref() == current)
    }

    /// One transaction on the blocking worker: the synchronous persistence
    /// of a completed attempt and the publication of the matching runtime
    /// record run together inside one `spawn_blocking` closure under the
    /// publish lock, so the disk write stays off the async workers while a
    /// concurrent reader holding the lock either sees both steps or neither
    /// — a fresh persisted release can never appear next to the previous
    /// attempt's stale error, and the scheduler base never regresses behind
    /// an already-visible persisted completion. The lock is held inside the
    /// worker only: no await, network or emit happens under it. The caller
    /// awaits the join handle; a join failure means the worker never ran,
    /// and the caller falls back to [`UpdatesState::set_last_attempt`].
    fn publish_attempt<T, F>(&self, transaction: F) -> tokio::task::JoinHandle<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Option<AttemptRecord>) -> T + Send + 'static,
    {
        let last_attempt = Arc::clone(&self.last_attempt);
        tokio::task::spawn_blocking(move || {
            let mut last_attempt = last_attempt.lock().expect("updates last_attempt lock");
            transaction(&mut last_attempt)
        })
    }

    /// Fallback after a failed publication (the blocking worker never ran,
    /// so the persisted file was not touched): record the outcome in memory
    /// only, so a failed persistence still shows this session's result.
    fn set_last_attempt(&self, record: AttemptRecord) {
        *self.last_attempt.lock().expect("updates last_attempt lock") = Some(record);
    }

    /// One transaction read under the publish lock: the current root's
    /// directory and its persisted record, resolved together with the
    /// in-memory attempt record filtered to that same root. Because a
    /// completion publishes its persistence and its runtime record under
    /// the same lock, a snapshot or a scheduler base derived from this pair
    /// is always coherent — both from before a completion or both from
    /// after it, never the persisted half of one with the runtime half of
    /// the other. `read` must be synchronous.
    fn read_attempt_with<R>(
        &self,
        read: impl FnOnce() -> (Option<PathBuf>, R),
    ) -> (Option<AttemptRecord>, (Option<PathBuf>, R)) {
        let guard = self.last_attempt.lock().expect("updates last_attempt lock");
        let (current_dir, persisted) = read();
        let attempt = guard
            .clone()
            .filter(|record| record.dir.as_deref() == current_dir.as_deref());
        (attempt, (current_dir, persisted))
    }

    /// The (time, error) pair the UI shows for the last attempt: the
    /// in-memory record of the current root when present (freshest), the
    /// persisted pair of the same root's state file otherwise (fresh
    /// process, or the root switched after the in-memory attempt
    /// completed).
    fn display_attempt(
        attempt: Option<AttemptRecord>,
        persisted: &state::UpdateStateRecord,
    ) -> (Option<String>, Option<String>) {
        match attempt {
            Some(record) => (
                Some(record.at.to_rfc3339_opts(SecondsFormat::Secs, true)),
                record.error,
            ),
            None => (
                persisted.last_attempt_at.clone(),
                persisted.last_attempt_error.clone(),
            ),
        }
    }

    #[cfg(debug_assertions)]
    pub fn dismiss_preview_bubble(&self, version: &str) -> bool {
        let Some(dismissal) = &self.preview_dismissal else {
            return false;
        };
        *dismissal.lock().expect("update preview dismissal lock") = Some(version.to_owned());
        true
    }

    fn dismissed_version(&self, persisted: Option<String>) -> Option<String> {
        #[cfg(debug_assertions)]
        if let Some(dismissal) = &self.preview_dismissal {
            return dismissal
                .lock()
                .expect("update preview dismissal lock")
                .clone();
        }
        persisted
    }
}

/// Version of the running application, as reported to the UI.
pub fn current_version() -> &'static str {
    #[cfg(debug_assertions)]
    {
        static PREVIEW_VERSION: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        if let Some(version) = PREVIEW_VERSION
            .get_or_init(|| {
                std::env::var("TTSBARD_UPDATE_PREVIEW_VERSION")
                    .ok()
                    .and_then(|value| parse_version(&value))
                    .map(|version| version.to_string())
            })
            .as_deref()
        {
            return version;
        }
    }
    env!("CARGO_PKG_VERSION")
}

/// Build the full status DTO from the persisted schedule (settings.json) plus
/// the persisted check state of the current data root (`updates/state.json`:
/// attempt outcome, check time, the stored release, the dismissed version)
/// and the install capability. The in-memory attempt record applies only
/// when it was produced under the current root's state file; the persisted
/// record of the same file is the fallback, so the card always reflects one
/// root. `update_available` is derived here so the UI never repeats the
/// SemVer comparison.
pub fn status_snapshot(
    settings_manager: &SettingsManager,
    state: &UpdatesState,
    install: &install::InstallState,
) -> UpdateStatusDto {
    let updates = settings_manager.get_update_settings();
    // One transaction for the directory, its persisted record and the
    // runtime fallback: a mid-run root switch must not pair the new root's
    // directory with the old root's JSON, and a completing check must never
    // be observed as its persisted half with the previous attempt's runtime
    // record (a fresh release next to a stale error) — see
    // [`UpdatesState::read_attempt_with`].
    let (attempt, (_current_dir, persisted)) =
        state.read_attempt_with(|| -> (Option<PathBuf>, state::UpdateStateRecord) {
            settings_manager.get_update_state_with_dir()
        });
    let (last_attempt_at, last_attempt_error) = UpdatesState::display_attempt(attempt, &persisted);
    let update_available = persisted
        .last_release
        .as_ref()
        .map(|release| version_is_newer(&release.version, current_version()))
        .unwrap_or(false);
    let latest_version = persisted
        .last_release
        .as_ref()
        .filter(|_| update_available)
        .map(|release| release.version.clone());
    UpdateStatusDto {
        current_version: current_version().to_string(),
        update_available,
        latest: persisted.last_release,
        last_attempt_at,
        last_attempt_error,
        last_check_at: persisted.last_check_at,
        checking: state.is_checking(),
        startup_check_enabled: updates.startup_check_enabled,
        periodic_check_enabled: updates.periodic_check_enabled,
        check_interval_days: updates.check_interval_days,
        dismissed_version: state.dismissed_version(persisted.dismissed_version),
        install_available: install.install_available_for(latest_version.as_deref()),
    }
}

// ============================================================================
// GitHub fetch
// ============================================================================

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<artifacts::ReleaseAsset>,
}

async fn fetch_github_release() -> Result<GitHubRelease, UpdateErrorKind> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|error| {
            warn!(error = %error, "Failed to build the update check HTTP client");
            UpdateErrorKind::Network
        })?;
    let response = client
        .get(RELEASES_API_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header(
            reqwest::header::USER_AGENT,
            format!("TTSBard/{}", current_version()),
        )
        .send()
        .await
        .map_err(|error| {
            debug!(error = %error, "Update check request failed");
            UpdateErrorKind::Network
        })?;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(UpdateErrorKind::NotFound);
    }
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Err(UpdateErrorKind::RateLimited);
    }
    if !status.is_success() {
        return Err(UpdateErrorKind::Network);
    }

    let release: GitHubRelease = response.json().await.map_err(|error| {
        debug!(error = %error, "Update check response is not a release object");
        UpdateErrorKind::Parse
    })?;
    // `/releases/latest` never returns drafts or pre-releases; treat such a
    // payload as "no stable release" rather than offering it to the user.
    if release.draft || release.prerelease {
        return Err(UpdateErrorKind::NotFound);
    }
    Ok(release)
}

/// Fetch the latest stable release from GitHub and reduce it to the wire DTO.
async fn fetch_latest_release() -> Result<ReleaseInfoDto, UpdateErrorKind> {
    let release = fetch_github_release().await?;
    let tag = release.tag_name.ok_or(UpdateErrorKind::Parse)?;
    let version = parse_version(&tag)
        .ok_or(UpdateErrorKind::Parse)?
        .to_string();
    let url = release.html_url.ok_or(UpdateErrorKind::Parse)?;
    // Only GitHub release pages may reach the UI as the "open release" link.
    if !url.starts_with("https://github.com/") {
        return Err(UpdateErrorKind::Parse);
    }

    Ok(ReleaseInfoDto {
        version,
        name: release.name.filter(|name| !name.trim().is_empty()),
        notes: sanitize_notes(release.body.as_deref()),
        url,
        published_at: release.published_at,
    })
}

// ============================================================================
// Check orchestration
// ============================================================================

/// Run exactly one check (fetch + persistence + event). `None` means another
/// check was already in flight and this call did nothing; the caller decides
/// how to wait ([`run_attempt_or_wait`] listens on `check_finished`).
///
/// Two broadcasts per attempt: a `checking=true` snapshot right after the
/// guard is taken (an open card sees the spinner and disables the manual
/// button even for scheduled checks) and the final snapshot with the
/// outcome.
///
/// Every completed attempt moves the periodic base: a success persists the
/// release plus the new attempt time (clearing the error), a failure
/// persists the new attempt time with its stable code while keeping the last
/// success and the known release. Both writes are atomic, so a restart never
/// reconstructs a false success from a saved time and a forgotten error. The
/// persistence and the matching runtime record are published as one
/// transaction under the publish lock ([`UpdatesState::publish_attempt`]),
/// so a concurrent snapshot or scheduler base read always observes the pair
/// coherently — never a fresh persisted release next to the previous
/// attempt's stale error, and never a regressed base. A failed persistence
/// still records the outcome in memory (`save` on a success), so this
/// session's result is not hidden.
///
/// The whole attempt is bound to the data root captured at its start: the
/// persistence writes go to that root's state file and the in-memory record
/// carries the same identity, so a «Данные программы» switch mid-run cannot
/// leak this completion into the new root's file, card or schedule. The
/// caller pins that root: `check_dir` is the root the attempt belongs to —
/// the scheduler passes the directory its due decision was derived from
/// (the decision and the write always concern the same root), while manual
/// and startup callers pass `None` to attempt the current root.
/// Build a status snapshot and broadcast it inside the shared
/// broadcast-order critical section (see
/// [`install::InstallState::broadcast_order`]): normal checks, installer
/// probe completions and the storage-root switch all publish through this
/// mutex, so a snapshot built before a concurrent state change can never be
/// emitted after that change's own broadcast — the switch's event lands
/// last and reflects the new root. Both the build and the emit are
/// synchronous, so the guard is never held across an await.
fn broadcast_ordered<T, B, E>(install: &install::InstallState, build: B, emit: E) -> T
where
    B: FnOnce() -> T,
    E: FnOnce(&T),
{
    let _order = install.lock_broadcast();
    let payload = build();
    emit(&payload);
    payload
}

/// Publish the fresh snapshot of the effective data root after a storage
/// root switch, in the shared broadcast order. The snapshot is computed
/// inside the critical section, so any check or probe that was mid-emit
/// when the switch happened finishes first, and this event — carrying the
/// new root's state — lands last.
fn broadcast_root_switch_snapshot<E>(
    settings_manager: &SettingsManager,
    updates: &UpdatesState,
    install: &std::sync::Arc<install::InstallState>,
    emit: E,
) where
    E: FnOnce(&UpdateStatusDto),
{
    broadcast_ordered(
        install,
        || status_snapshot(settings_manager, updates, install),
        emit,
    );
}

async fn try_run_check<F, Fut, E, C>(
    settings_manager: &SettingsManager,
    state: &UpdatesState,
    install: &install::InstallState,
    check_dir: Option<Option<PathBuf>>,
    fetch: F,
    emit: &E,
    now: C,
) -> Option<UpdateStatusDto>
where
    F: Fn() -> Fut + Send,
    Fut: Future<Output = Result<ReleaseInfoDto, UpdateErrorKind>> + Send,
    E: Fn(&UpdateStatusDto) + Sync,
    C: Fn() -> DateTime<Utc> + Send,
{
    if state
        .checking
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        debug!("Update check already in flight; keeping the current status");
        return None;
    }

    // Bind this attempt to the pinned root before any await: fetches and
    // persistence writes of this attempt land there even if the effective
    // root changes mid-flight. Manual and startup callers resolve the
    // current root here instead.
    let check_dir = check_dir.unwrap_or_else(|| settings_manager.update_state_dir());

    // Publish the in-flight flag right away, so an already open card shows
    // the spinner and keeps the manual button disabled while a scheduled
    // check runs; the completion below broadcasts the final snapshot. Both
    // broadcasts share the install broadcast order with probe completions
    // and the storage-root switch, so no snapshot can overtake another.
    broadcast_ordered(
        install,
        || status_snapshot(settings_manager, state, install),
        |started| emit(started),
    );

    // The attempt counts as completed when the fetch returns: the periodic
    // base and the shown attempt time are the completion time, not the
    // start. The clock is injected together with the scheduler's deadline
    // clock, so tests see one consistent time grid; production passes
    // `Utc::now` for both.
    let outcome = fetch().await;
    let attempted_at = now();
    // One transaction (see [`UpdatesState::publish_attempt`]): the
    // synchronous persistence of the completed attempt and the publication
    // of the matching runtime record run together inside one blocking
    // worker under the publish lock, so a concurrent snapshot or scheduler
    // base read always observes the pair coherently — never a fresh
    // persisted release next to the previous attempt's stale error, and
    // never a regressed base. Only the manager clone and the
    // `last_attempt` handle move into the worker; no lock, await, network
    // or emit is held by the async side while it runs.
    let manager = settings_manager.clone();
    let state_dir = check_dir.clone();
    let persisted = state.publish_attempt(move |last_attempt| {
        let error = match outcome {
            Ok(release) => {
                info!(
                    version = %release.version,
                    update_available = version_is_newer(&release.version, current_version()),
                    "Update check succeeded"
                );
                // The success result (release + check time) and the attempt
                // record (same time, error cleared) are one atomic write: the
                // schedule counts from any completed attempt, and the card
                // keeps a consistent success snapshot across restarts.
                let checked_at = attempted_at.to_rfc3339_opts(SecondsFormat::Secs, true);
                match manager.set_updates_check_result_in(state_dir.as_deref(), checked_at, release)
                {
                    Ok(()) => None,
                    Err(error) => {
                        warn!(error = %error, "Failed to persist the update check result");
                        Some(UpdateErrorKind::Save.as_str().to_string())
                    }
                }
            }
            Err(kind) => {
                warn!(code = kind.as_str(), "Update check failed");
                // The attempt failure moves the periodic base without touching
                // the last success or the known release; it is persisted so a
                // restart shows the failure instead of an older success.
                let attempted = attempted_at.to_rfc3339_opts(SecondsFormat::Secs, true);
                if let Err(error) = manager.set_updates_attempt_failure_in(
                    state_dir.as_deref(),
                    attempted,
                    kind.as_str(),
                ) {
                    warn!(
                        error = %error,
                        "Failed to persist the failed update attempt"
                    );
                }
                Some(kind.as_str().to_string())
            }
        };
        *last_attempt = Some(AttemptRecord {
            at: attempted_at,
            error: error.clone(),
            dir: state_dir,
        });
        error
    });
    // A join failure means the worker never ran, so the persisted file was
    // not touched: publish the runtime record directly as the `save`
    // fallback. The outcome itself is carried by the published record.
    if let Err(join_error) = persisted.await {
        warn!(error = %join_error, "Update persistence task failed");
        state.set_last_attempt(AttemptRecord {
            at: attempted_at,
            error: Some(UpdateErrorKind::Save.as_str().to_string()),
            dir: check_dir.clone(),
        });
    }
    state.checking.store(false, Ordering::SeqCst);
    // Wake a scheduled check that is waiting out this in-flight one. The flag
    // above must be cleared first so the woken waiter sees the guard free.
    state.check_finished.notify_waiters();
    // The final snapshot is built and emitted inside the shared broadcast
    // order: a root switch that happens after this build waits here, and a
    // switch that already broadcast makes this completion wait — its
    // snapshot is then computed from the new root instead of overtaking the
    // switch's event with stale content.
    let status = broadcast_ordered(
        install,
        || status_snapshot(settings_manager, state, install),
        |status| emit(status),
    );
    Some(status)
}

/// Manual check entry point used by the `check_for_updates` command. When a
/// scheduled check is already in flight, the current snapshot is returned:
/// the card keeps its spinner through `checking` and updates from the
/// `update-status-changed` event the running check will broadcast.
pub async fn perform_manual_check(
    app: &tauri::AppHandle,
    settings_manager: &SettingsManager,
    updates: &std::sync::Arc<UpdatesState>,
    install: &std::sync::Arc<install::InstallState>,
) -> UpdateStatusDto {
    let state: &UpdatesState = updates;
    // Before the first await: the scheduler's startup branch must see a
    // manual check that already started in this process and skip its
    // automatic attempt.
    state.mark_manual_check_started();
    let emit = |status: &UpdateStatusDto| {
        install.ensure_artifact_probe(app, updates, settings_manager, status);
        let _ = app.emit(UPDATE_STATUS_CHANGED_EVENT, status);
    };
    try_run_check(
        settings_manager,
        state,
        install,
        // Manual checks target the current root.
        None,
        fetch_latest_release,
        &emit,
        Utc::now,
    )
    .await
    .unwrap_or_else(|| status_snapshot(settings_manager, state, install))
}

/// One attempt on behalf of the scheduler. When another check (manual) is in
/// flight, wait for its completion instead of spinning against the guard —
/// the finished check already moved the base, so the outer loop recomputes
/// the deadline from the fresh state and the due attempt stays satisfied.
/// The attempt runs in the root the caller's due decision was derived from:
/// `check_dir` pins it (see [`try_run_check`]), so a root switch between the
/// decision and the guard cannot redirect the write to another root.
#[allow(
    clippy::too_many_arguments,
    reason = "Explicit dependencies keep the check and wait testable"
)]
async fn run_attempt_or_wait<F, Fut, E, C>(
    settings_manager: &SettingsManager,
    state: &UpdatesState,
    install: &install::InstallState,
    check_dir: Option<Option<PathBuf>>,
    shutdown: &CancellationToken,
    fetch: F,
    emit: &E,
    now: C,
) where
    F: Fn() -> Fut + Send,
    Fut: Future<Output = Result<ReleaseInfoDto, UpdateErrorKind>> + Send,
    E: Fn(&UpdateStatusDto) + Sync,
    C: Fn() -> DateTime<Utc> + Send,
{
    if try_run_check(
        settings_manager,
        state,
        install,
        check_dir,
        fetch,
        emit,
        now,
    )
    .await
    .is_some()
    {
        return;
    }
    let finished = state.check_finished.notified();
    // The in-flight check may complete between the failed guard and the
    // waiter registration above; re-check the flag so the wake-up is not
    // lost.
    if !state.is_checking() {
        return;
    }
    tokio::select! {
        _ = shutdown.cancelled() => {}
        _ = state.schedule_changed.notified() => {}
        _ = finished => {}
    }
}

/// Duration after which the next periodic attempt is due, given the time of
/// the last completed attempt (success or failure, manual included) and the
/// persisted interval. `None` means "attempt right now": no completed
/// attempt yet or a deadline already in the past.
fn next_periodic_delay(
    last_attempt: Option<DateTime<Utc>>,
    interval_days: u32,
    now: DateTime<Utc>,
) -> Option<Duration> {
    let deadline = last_attempt? + chrono::Duration::days(i64::from(interval_days));
    if deadline <= now {
        None
    } else {
        Some((deadline - now).to_std().unwrap_or(Duration::ZERO))
    }
}

/// Parse the persisted attempt time into the scheduler base.
fn persisted_attempt_time(state: &state::UpdateStateRecord) -> Option<DateTime<Utc>> {
    state.last_attempt_at.as_deref().and_then(|value| {
        DateTime::parse_from_rfc3339(value)
            .map(|time| time.with_timezone(&Utc))
            .map_err(|error| {
                warn!(value = %value, error = %error, "Unparsable persisted update attempt time; treating as never attempted")
            })
            .ok()
    })
}

/// The scheduler core: one startup attempt per process when the switch is on
/// (merged with an overdue periodic schedule into this single attempt), then
/// periodic attempts on the persisted interval — including while the main
/// window is closed to the tray. With periodic checks off the loop parks
/// until the schedule changes or the app exits. Generic over the clock and
/// the sleeper so tests drive time without real-day waits. Takes owned
/// state so the future stays `'static` for the runtime spawn.
#[allow(
    clippy::too_many_arguments,
    reason = "The scheduler takes injectable time, fetch, and event functions"
)]
async fn scheduler_loop<C, SF, S, F, Fut, E>(
    settings_manager: SettingsManager,
    state: Arc<UpdatesState>,
    install: Arc<install::InstallState>,
    shutdown: CancellationToken,
    fetch: F,
    emit: E,
    clock: C,
    sleep: S,
) where
    C: Fn() -> DateTime<Utc> + Send + Sync,
    S: Fn(Duration) -> SF + Send,
    SF: Future<Output = ()> + Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Result<ReleaseInfoDto, UpdateErrorKind>> + Send,
    E: Fn(&UpdateStatusDto) + Send + Sync,
{
    let (state, install, settings_manager, shutdown) =
        (&state, &install, &settings_manager, &shutdown);
    // The startup attempt belongs to the process start: it is decided once,
    // regardless of when the window is opened, and re-enabling the switch
    // mid-run must not fire a second (or a delayed first) startup attempt.
    let mut startup_decided = false;
    while !shutdown.is_cancelled() {
        let updates = settings_manager.get_update_settings();
        if !startup_decided {
            startup_decided = true;
            if updates.startup_check_enabled && !state.manual_check_started() {
                // One startup attempt; when periodic checks are also on and
                // their deadline is overdue (or absent), this same attempt
                // covers the schedule — the base moves to its completion
                // time, success or failure alike.
                run_attempt_or_wait(
                    settings_manager,
                    state,
                    install,
                    // Startup decides for the current root; the attempt
                    // resolves it like a manual check would.
                    None,
                    shutdown,
                    &fetch,
                    &emit,
                    &clock,
                )
                .await;
                continue;
            }
        }
        if !updates.periodic_check_enabled {
            // Park until the user enables the schedule (or any schedule
            // change) or the app exits; no automatic checks in the meantime.
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = state.schedule_changed.notified() => continue,
            }
        }
        // The periodic base is derived per iteration: the in-memory attempt
        // of the current data root when present (freshest), otherwise the
        // persisted record of the same root's state file. The fallback keeps
        // the schedule across restarts and recomputes it after a root
        // switch, where in-memory records of the previous root are ignored.
        // The directory, its record and the runtime fallback come from one
        // transaction (see `read_attempt_with`), so the base never mixes two
        // roots and never observes a completed check's persistence without
        // its runtime record (or vice versa) — the base cannot regress.
        let (attempt, (current_dir, persisted)) =
            state.read_attempt_with(|| -> (Option<PathBuf>, state::UpdateStateRecord) {
                settings_manager.get_update_state_with_dir()
            });
        let base = attempt
            .map(|record| record.at)
            .or_else(|| persisted_attempt_time(&persisted));
        let delay = next_periodic_delay(base, updates.check_interval_days, clock());
        match delay {
            Some(delay) => {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = state.schedule_changed.notified() => continue,
                    _ = sleep(delay) => {
                        // The due decision above belongs to the root captured
                        // before the wait. If that root changed while the
                        // timer was pending (without a schedule wake-up, the
                        // decision is stale), running the attempt now would
                        // check the *current* root on the old root's base —
                        // too early for it. Loop back instead: the next
                        // iteration recomputes the base and the due decision
                        // from the same root it would start the attempt in.
                        continue;
                    }
                }
            }
            None => {
                // Overdue or never attempted: one attempt now, then the next
                // deadline is a full interval away — failed checks are never
                // retried early.
                run_attempt_or_wait(
                    settings_manager,
                    state,
                    install,
                    // The attempt belongs to the root the due decision was
                    // derived from, not to whichever root is effective when
                    // the guard is finally taken: re-resolving here would let
                    // a «Данные программы» switch between the decision and
                    // the attempt write A's overdue check into B.
                    Some(current_dir.clone()),
                    shutdown,
                    &fetch,
                    &emit,
                    &clock,
                )
                .await;
            }
        }
    }
    info!("Update check scheduler stopped");
}

/// Background scheduler entry point: spawns the loop on the async runtime
/// with the real clock and real sleeps.
pub fn spawn_update_scheduler(
    app: tauri::AppHandle,
    settings_manager: SettingsManager,
    state: Arc<UpdatesState>,
    install: Arc<install::InstallState>,
    shutdown: CancellationToken,
) {
    let emit_app = app.clone();
    let emit_state = Arc::clone(&state);
    let emit_install = Arc::clone(&install);
    let emit_manager = settings_manager.clone();
    let emit = move |status: &UpdateStatusDto| {
        emit_install.ensure_artifact_probe(&emit_app, &emit_state, &emit_manager, status);
        let _ = emit_app.emit(UPDATE_STATUS_CHANGED_EVENT, status);
    };
    tauri::async_runtime::spawn(async move {
        // Wait until the frontend is ready (settings loaded and the main
        // window mounted, signalled by `mark_update_startup_ready`) or the
        // app exits; then give the UI an extra settle delay so the startup
        // pass never competes with the initial render.
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Update check scheduler stopped before startup readiness");
                return;
            }
            _ = state.startup_ready().cancelled() => {}
        }
        tokio::select! {
            _ = shutdown.cancelled() => {}
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
        }
        if shutdown.is_cancelled() {
            info!("Update check scheduler stopped during the startup delay");
            return;
        }
        // Publish the snapshot built from the persisted state once, so a
        // release cached from a previous process reaches the UI (and its
        // artifact probe) without waiting for the first attempt — inside
        // the shared broadcast order, like every other status broadcast.
        broadcast_ordered(
            &install,
            || status_snapshot(&settings_manager, &state, &install),
            |cached| emit(cached),
        );
        scheduler_loop(
            settings_manager,
            state,
            install,
            shutdown,
            fetch_latest_release,
            emit,
            Utc::now,
            tokio::time::sleep,
        )
        .await;
    });
}

/// React to a successful «Данные программы» root change (called from
/// `commands::storage` after the new root is published). The snapshot is
/// rebuilt from the new root's state file — the card of an already open
/// window updates even with both auto-check switches off — and the
/// scheduler is woken so an already-waiting periodic timer recomputes its
/// deadline from the new root's base (an empty root follows the existing
/// "no base → attempt now" rule; the startup attempt stays one-shot per
/// process and is not re-fired). In-memory attempt records of the previous
/// root are ignored by identity, so nothing stale leaks into the new
/// root's snapshot.
pub fn notify_data_root_changed(
    app: &tauri::AppHandle,
    settings_manager: &SettingsManager,
    updates: &std::sync::Arc<UpdatesState>,
    install: &std::sync::Arc<install::InstallState>,
) {
    broadcast_root_switch_snapshot(settings_manager, updates, install, |status| {
        install.ensure_artifact_probe(app, updates, settings_manager, status);
        let _ = app.emit(UPDATE_STATUS_CHANGED_EVENT, status);
    });
    updates.notify_schedule_changed();
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn sample_release(version: &str) -> ReleaseInfoDto {
        ReleaseInfoDto {
            version: version.to_string(),
            name: Some(format!("TTSBard {version}")),
            notes: "What is new".to_string(),
            url: format!("https://github.com/ttsbard/TTSBard/releases/tag/v{version}"),
            published_at: Some("2026-11-16T08:00:00Z".to_string()),
        }
    }

    fn temp_config_dir(tag: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-updates-{tag}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn noop_emit(status: &UpdateStatusDto) {
        let _ = status;
    }

    /// Capability-less install state for check tests (no updater key → the
    /// `install_available` flag is always false).
    fn test_install() -> install::InstallState {
        install::InstallState::new(false)
    }

    fn test_install_arc() -> Arc<install::InstallState> {
        Arc::new(test_install())
    }

    fn datetime(raw: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(raw)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Poll a condition in real time with a generous timeout, so tests never
    /// hang on a scheduler step that should have happened.
    async fn wait_until(mut condition: impl FnMut() -> bool, what: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Wait until the loop has parked in its Nth re-arm sleep; recording the
    /// sleep is the synchronization point that proves the previous step was
    /// fully processed.
    async fn wait_sleep_count(clock: &TestClock, n: usize) {
        wait_until(
            || clock.sleep_count() >= n,
            "the re-arm sleep to be recorded",
        )
        .await;
    }

    /// A fetch stub counting calls and answering with a fixed release.
    fn fetch_ok(
        calls: &Arc<AtomicUsize>,
        version: &'static str,
    ) -> impl Fn() -> std::pin::Pin<
        Box<dyn Future<Output = Result<ReleaseInfoDto, UpdateErrorKind>> + Send>,
    > + Send
           + Sync
           + Clone {
        let calls = Arc::clone(calls);
        move || {
            let calls = Arc::clone(&calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(sample_release(version))
            })
        }
    }

    /// A fetch stub counting calls and always failing with one code.
    fn fetch_failing(
        calls: &Arc<AtomicUsize>,
        kind: UpdateErrorKind,
    ) -> impl Fn() -> std::pin::Pin<
        Box<dyn Future<Output = Result<ReleaseInfoDto, UpdateErrorKind>> + Send>,
    > + Send
           + Sync
           + Clone {
        let calls = Arc::clone(calls);
        move || {
            let calls = Arc::clone(&calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(kind)
            })
        }
    }

    /// A fetch stub that counts calls, models the network duration by
    /// advancing the test clock, and answers with a fixed release: the
    /// completion time lands on the same fake grid as every deadline, so
    /// re-arm delays are exact.
    fn fetch_ok_advancing(
        calls: &Arc<AtomicUsize>,
        version: &'static str,
        clock: &TestClock,
        advance_secs: u64,
    ) -> impl Fn() -> std::pin::Pin<
        Box<dyn Future<Output = Result<ReleaseInfoDto, UpdateErrorKind>> + Send>,
    > + Send
           + Sync
           + Clone {
        let calls = Arc::clone(calls);
        let clock = clock.clone();
        move || {
            let calls = Arc::clone(&calls);
            let clock = clock.clone();
            Box::pin(async move {
                clock.advance_secs(advance_secs);
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(sample_release(version))
            })
        }
    }

    /// Deterministic time for scheduler tests: a fixed clock the test
    /// advances manually, and a sleeper that records the requested delays
    /// and parks until the test releases it — no real-day waits.
    #[derive(Clone)]
    struct TestClock {
        now: Arc<Mutex<DateTime<Utc>>>,
        release: Arc<tokio::sync::Notify>,
        sleeps: Arc<Mutex<Vec<Duration>>>,
    }

    impl TestClock {
        fn starting_at(now: DateTime<Utc>) -> Self {
            Self {
                now: Arc::new(Mutex::new(now)),
                release: Arc::new(tokio::sync::Notify::new()),
                sleeps: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn now(&self) -> DateTime<Utc> {
            *self.now.lock().expect("test clock lock")
        }

        fn advance(&self, by: chrono::Duration) {
            *self.now.lock().expect("test clock lock") += by;
        }

        fn advance_secs(&self, secs: u64) {
            self.advance(chrono::Duration::seconds(
                i64::try_from(secs).unwrap_or(i64::MAX),
            ));
        }

        /// Wake the loop's current sleep. `notify_one` stores a permit when
        /// the sleeper has not registered yet, so an early release is never
        /// lost.
        fn release_sleeps(&self) {
            self.release.notify_one();
        }

        fn sleeps(&self) -> Vec<Duration> {
            self.sleeps.lock().expect("test sleeps lock").clone()
        }

        fn sleep_count(&self) -> usize {
            self.sleeps().len()
        }

        fn sleep_fn(
            &self,
        ) -> impl Fn(Duration) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Send + Clone
        {
            let clock = self.clone();
            move |duration: Duration| {
                let clock = clock.clone();
                Box::pin(async move {
                    clock
                        .sleeps
                        .lock()
                        .expect("test sleeps lock")
                        .push(duration);
                    clock.release.notified().await;
                })
            }
        }
    }

    fn spawn_loop(
        manager: &SettingsManager,
        state: &Arc<UpdatesState>,
        clock: &TestClock,
        shutdown: &CancellationToken,
        fetch: impl Fn() -> std::pin::Pin<
                Box<dyn Future<Output = Result<ReleaseInfoDto, UpdateErrorKind>> + Send>,
            > + Send
            + Sync
            + Clone
            + 'static,
        emit: impl Fn(&UpdateStatusDto) + Send + Sync + 'static,
    ) -> tokio::task::JoinHandle<()> {
        let clock2 = clock.clone();
        tokio::spawn(scheduler_loop(
            manager.clone(),
            Arc::clone(state),
            test_install_arc(),
            shutdown.clone(),
            fetch,
            emit,
            move || clock2.now(),
            clock.sleep_fn(),
        ))
    }

    #[test]
    fn version_is_newer_compares_semver() {
        let current = "1.8.0";
        assert!(version_is_newer("1.9.0", current));
        assert!(version_is_newer("v1.9.0", current));
        assert!(version_is_newer("2.0.0", current));
        assert!(version_is_newer("1.8.1", current));
        assert!(!version_is_newer("1.8.0", current));
        assert!(!version_is_newer("1.7.9", current));
        assert!(!version_is_newer("v1.8.0", current));
        // Pre-release ordering: 1.9.0-rc.1 < 1.9.0.
        assert!(version_is_newer("1.9.0", current));
        assert!(!version_is_newer("1.9.0-rc.1", "1.9.0"));
        // Unparseable input never reports an update.
        assert!(!version_is_newer("not-a-version", current));
        assert!(!version_is_newer("1.9", current));
        assert!(!version_is_newer("1.9.0", "broken"));
    }

    #[test]
    fn sanitize_notes_strips_control_characters_and_caps_length() {
        assert_eq!(sanitize_notes(None), "");
        assert_eq!(sanitize_notes(Some("  \n  Hello\n")), "Hello");
        // Line breaks and tabs survive; other control characters are dropped.
        assert_eq!(sanitize_notes(Some("a\u{0}b\nc\td")), "ab\nc\td");
        // Over-long notes are capped with an ellipsis marker.
        let long = "x".repeat(MAX_NOTES_CHARS + 100);
        let notes = sanitize_notes(Some(&long));
        assert_eq!(notes.chars().count(), MAX_NOTES_CHARS + 1);
        assert!(notes.ends_with('…'));
    }

    #[test]
    fn update_error_kind_codes_are_stable() {
        assert_eq!(UpdateErrorKind::Network.as_str(), "network");
        assert_eq!(UpdateErrorKind::RateLimited.as_str(), "rate_limited");
        assert_eq!(UpdateErrorKind::NotFound.as_str(), "not_found");
        assert_eq!(UpdateErrorKind::Parse.as_str(), "parse");
        assert_eq!(UpdateErrorKind::Save.as_str(), "save");
    }

    /// The periodic deadline counts from any completed attempt (success or
    /// failure alike): never attempted → due now, a recent attempt → a full
    /// interval ahead, a stale one → due now, and the interval changes the
    /// deadline.
    #[test]
    fn next_periodic_delay_counts_from_last_completed_attempt() {
        let now = datetime("2026-11-17T12:00:00Z");

        // Never attempted: due immediately.
        assert_eq!(next_periodic_delay(None, 1, now), None);

        // Attempted 2 hours ago with a 1 day interval: ~22 hours left.
        let recent = Some(now - chrono::Duration::hours(2));
        let delay = next_periodic_delay(recent, 1, now).expect("deadline in the future");
        assert_eq!(delay, Duration::from_secs(22 * 3600));

        // The deadline passed: due immediately (failed attempts included).
        let stale = Some(now - chrono::Duration::days(2));
        assert_eq!(next_periodic_delay(stale, 1, now), None);

        // A changed interval re-counts the deadline from the same attempt.
        let delay = next_periodic_delay(recent, 7, now).expect("deadline in the future");
        assert_eq!(delay, Duration::from_secs(6 * 24 * 3600 + 22 * 3600));
    }

    /// The persisted attempt string parses into the scheduler base; garbage
    /// is treated as "never attempted" without panicking.
    #[test]
    fn persisted_attempt_time_parses_or_rejects() {
        let mut record = state::UpdateStateRecord::default();
        assert_eq!(persisted_attempt_time(&record), None);

        record.last_attempt_at = Some("2026-11-17T10:00:00Z".to_string());
        assert_eq!(
            persisted_attempt_time(&record),
            Some(datetime("2026-11-17T10:00:00Z"))
        );

        record.last_attempt_at = Some("not-a-date".to_string());
        assert_eq!(persisted_attempt_time(&record), None);
    }

    /// A successful attempt persists the found release, the check time and
    /// the matching attempt record (same time, no error) in one atomic
    /// write; a fresh state plus a reloaded manager (a restarted process)
    /// restores the card and the panel from the state file alone.
    #[tokio::test]
    async fn successful_attempt_persists_result_and_restores_after_restart() {
        let dir = temp_config_dir("persist");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let state = UpdatesState::new();

        let current = parse_version(current_version()).expect("current app version is valid");
        let next_version = semver::Version::new(current.major + 1, 0, 0).to_string();
        let release = sample_release(&next_version);
        let status = try_run_check(
            &manager,
            &state,
            &test_install(),
            None,
            move || {
                let release = release.clone();
                async move { Ok(release) }
            },
            &noop_emit,
            Utc::now,
        )
        .await
        .expect("the check ran");
        assert_eq!(status.last_attempt_error, None);
        assert!(status.last_attempt_at.is_some());

        // Persisted together: the attempt record feeds the schedule, the
        // release feeds the card and the panel.
        let persisted = manager.get_update_state();
        assert!(persisted.last_check_at.is_some(), "check time persisted");
        assert_eq!(
            persisted.last_attempt_at, persisted.last_check_at,
            "a success is also the fresh attempt record"
        );
        assert_eq!(persisted.last_attempt_error, None);
        assert_eq!(
            persisted.last_release.as_ref().map(|r| r.version.as_str()),
            Some(next_version.as_str()),
            "found release persisted"
        );

        // The UI snapshot is built from the persisted state, not a runtime
        // cache: a fresh state plus a reloaded manager sees the same result
        // without any false failure.
        let reloaded = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let fresh_state = UpdatesState::new();
        let status = status_snapshot(&reloaded, &fresh_state, &test_install());
        assert_eq!(
            status.latest.as_ref().map(|r| r.version.as_str()),
            Some(next_version.as_str()),
            "release restored after restart"
        );
        assert!(status.update_available);
        assert_eq!(status.last_attempt_error, None);
        assert!(status.last_attempt_at.is_some());
        assert_eq!(status.last_check_at, status.last_attempt_at);

        // Dismissal keeps hiding the panel for this version across restarts,
        // while the card still receives the release through the same fields.
        reloaded
            .set_updates_dismissed_version(Some(next_version.clone()))
            .unwrap();
        let status = status_snapshot(&reloaded, &fresh_state, &test_install());
        assert_eq!(
            status.dismissed_version.as_deref(),
            Some(next_version.as_str())
        );
        assert_eq!(
            status.latest.as_ref().map(|r| r.version.as_str()),
            Some(next_version.as_str())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed attempt keeps the previously stored release and success time
    /// untouched, but persists its own time and stable code: the card shows
    /// the error next to the known release, and the new attempt time never
    /// masquerades as a success.
    #[tokio::test]
    async fn failed_attempt_keeps_release_and_success_time_and_persists_outcome() {
        let dir = temp_config_dir("failed-keeps-release");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        manager
            .set_updates_check_result("2026-11-15T08:00:00Z".to_string(), sample_release("1.8.9"))
            .unwrap();
        let state = UpdatesState::new();
        let install = test_install();
        let status = try_run_check(
            &manager,
            &state,
            &install,
            None,
            || async { Err(UpdateErrorKind::RateLimited) },
            &noop_emit,
            Utc::now,
        )
        .await
        .expect("the failed attempt ran");

        // The snapshot separates the failed attempt from the old success.
        assert_eq!(status.last_attempt_error.as_deref(), Some("rate_limited"));
        assert!(status.last_attempt_at.is_some());
        assert_ne!(
            status.last_attempt_at.as_deref(),
            Some("2026-11-15T08:00:00Z"),
            "the attempt time moved past the old success"
        );
        assert_eq!(
            status.last_check_at.as_deref(),
            Some("2026-11-15T08:00:00Z"),
            "the last successful check time is untouched"
        );
        assert_eq!(
            status.latest.as_ref().map(|r| r.version.as_str()),
            Some("1.8.9")
        );

        // The failure outcome is persisted for restarts.
        let persisted = manager.get_update_state();
        assert!(persisted.last_attempt_at.is_some());
        assert_eq!(
            persisted.last_attempt_error.as_deref(),
            Some("rate_limited")
        );
        assert_eq!(
            persisted.last_check_at.as_deref(),
            Some("2026-11-15T08:00:00Z")
        );
        assert_eq!(
            persisted.last_release.as_ref().map(|r| r.version.as_str()),
            Some("1.8.9")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// After a failed attempt a fresh process (reloaded manager, empty
    /// runtime state) shows the stored failure — never a fresh-looking
    /// success rebuilt from the saved success time.
    #[tokio::test]
    async fn restart_after_failure_shows_error_without_false_success() {
        let dir = temp_config_dir("restart-failure");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        manager
            .set_updates_check_result("2026-11-15T08:00:00Z".to_string(), sample_release("1.8.9"))
            .unwrap();
        manager
            .set_updates_attempt_failure("2026-11-18T12:30:00Z".to_string(), "network")
            .unwrap();

        let reloaded = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let fresh_state = UpdatesState::new();
        let status = status_snapshot(&reloaded, &fresh_state, &test_install());
        assert_eq!(status.last_attempt_error.as_deref(), Some("network"));
        assert_eq!(
            status.last_attempt_at.as_deref(),
            Some("2026-11-18T12:30:00Z"),
            "the failed attempt time is restored"
        );
        assert_eq!(
            status.last_check_at.as_deref(),
            Some("2026-11-15T08:00:00Z"),
            "the old success keeps its own time"
        );
        assert_eq!(
            status.latest.as_ref().map(|r| r.version.as_str()),
            Some("1.8.9"),
            "the known release survives the failure"
        );
        assert!(!status.checking);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A later success replaces the stored error: the attempt record and the
    /// success move together in one write.
    #[tokio::test]
    async fn success_replaces_previous_error() {
        let dir = temp_config_dir("success-replaces-error");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        manager
            .set_updates_attempt_failure("2026-11-18T08:00:00Z".to_string(), "network")
            .unwrap();
        let state = UpdatesState::new();

        let release = sample_release("1.9.0");
        let status = try_run_check(
            &manager,
            &state,
            &test_install(),
            None,
            move || {
                let release = release.clone();
                async move { Ok(release) }
            },
            &noop_emit,
            Utc::now,
        )
        .await
        .expect("the check ran");
        assert_eq!(status.last_attempt_error, None, "success clears the error");

        let persisted = manager.get_update_state();
        assert_eq!(persisted.last_attempt_error, None);
        assert_eq!(
            persisted.last_attempt_at, persisted.last_check_at,
            "attempt and success share the fresh time"
        );
        assert_eq!(
            persisted.last_release.as_ref().map(|r| r.version.as_str()),
            Some("1.9.0")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// While a manual check is in flight, the due scheduled attempt must not
    /// spin against the guard — no fetch happens for the whole real-time
    /// window below (a busy loop would call fetch thousands of times) — and
    /// it returns promptly once the manual check finishes, leaving the
    /// deadline recomputation to the outer loop.
    #[tokio::test]
    async fn attempt_waiter_snoozes_while_manual_in_flight_without_busy_loop() {
        let dir = temp_config_dir("collision");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();

        // A manual check is in flight and holds the guard.
        state.checking.store(true, Ordering::SeqCst);

        let calls = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let task_calls = Arc::clone(&calls);
        let task_done = Arc::clone(&done);
        let task_state = Arc::clone(&state);
        let task_manager = manager.clone();
        let task_install = test_install_arc();
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move {
            run_attempt_or_wait(
                &task_manager,
                &task_state,
                &task_install,
                None,
                &task_shutdown,
                fetch_ok(&task_calls, "1.9.0"),
                &noop_emit,
                Utc::now,
            )
            .await;
            task_done.store(true, Ordering::SeqCst);
        });

        // While the guard is held, real time passes without a single fetch
        // and without the scheduler giving up.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "no fetch while a manual check is in flight"
        );
        assert!(!done.load(Ordering::SeqCst), "scheduler keeps waiting");

        // The manual check finishes: clear the guard and broadcast completion.
        state.checking.store(false, Ordering::SeqCst);
        state.check_finished.notify_waiters();

        let finished = tokio::time::timeout(Duration::from_secs(5), task).await;
        assert!(
            finished.is_ok(),
            "scheduler returns once the in-flight check finished"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "the finished manual check satisfied the due attempt; deadline is recomputed by the outer loop"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Startup and an overdue periodic deadline merge into one attempt on a
    /// fresh process, even after the previous attempt failed. Everything
    /// runs on the one injected clock: the union attempt at T0 re-arms
    /// exactly a full interval; waking the loop 5 and 10 minutes later
    /// produces no retry; when the deadline passes, exactly one more attempt
    /// fires and the next deadline is a full interval from that second
    /// completion.
    #[tokio::test]
    async fn scheduler_startup_and_overdue_periodic_are_one_attempt() {
        const DAY: u64 = 24 * 3600;
        let dir = temp_config_dir("union");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        // A stale failure three days before T0 makes the 1-day schedule
        // overdue on the test grid.
        manager
            .set_updates_attempt_failure(
                datetime("2026-11-15T12:00:00Z").to_rfc3339_opts(SecondsFormat::Secs, true),
                "network",
            )
            .unwrap();
        manager.set_updates_periodic_check_enabled(true).unwrap();

        let clock = TestClock::starting_at(datetime("2026-11-18T12:00:00Z"));
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_ok(&calls, "9.9.9"),
            noop_emit,
        );

        // The startup attempt covers the overdue periodic deadline: exactly
        // one fetch at T0, then a full 24-hour re-arm.
        wait_until(|| calls.load(Ordering::SeqCst) == 1, "the startup attempt").await;
        wait_sleep_count(&clock, 1).await;
        assert_eq!(
            clock.sleeps()[0],
            Duration::from_secs(DAY),
            "the union attempt re-arms a full interval from its completion"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "no duplicate attempt for the overdue periodic schedule"
        );

        // Five minutes pass and a setting change wakes the loop (the same
        // `notify_schedule_changed` the setters fire): it recomputes without
        // a retry, and the deadline is unchanged.
        clock.advance_secs(5 * 60);
        state.notify_schedule_changed();
        wait_sleep_count(&clock, 2).await;
        assert_eq!(clock.sleeps()[1], Duration::from_secs(DAY - 5 * 60));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no retry five minutes in");

        // Ten minutes in: still no retry.
        clock.advance_secs(5 * 60);
        state.notify_schedule_changed();
        wait_sleep_count(&clock, 3).await;
        assert_eq!(clock.sleeps()[2], Duration::from_secs(DAY - 10 * 60));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no retry ten minutes in");

        // The interval elapses (the sleeper is released for real): exactly
        // one periodic attempt at T0 + 24h + 1s, and the next deadline is a
        // full interval from that second completion.
        clock.advance_secs(DAY - 10 * 60 + 1);
        clock.release_sleeps();
        wait_until(|| calls.load(Ordering::SeqCst) == 2, "the periodic attempt").await;
        wait_sleep_count(&clock, 4).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "exactly one periodic attempt"
        );
        assert_eq!(
            clock.sleeps()[3],
            Duration::from_secs(DAY),
            "the next deadline is a full interval from the second completion"
        );

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        // The union attempt recorded a success over the stale failure.
        let persisted = manager.get_update_state();
        assert_eq!(persisted.last_attempt_error, None);
        assert_eq!(
            persisted.last_release.as_ref().map(|r| r.version.as_str()),
            Some("9.9.9")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Startup-only: the single attempt runs even when it fails, and after
    /// the failure no automatic attempt happens for the rest of the process
    /// (no five-minute retry) — neither on schedule changes nor anything
    /// else; only shutdown ends the loop. The attempt completion is awaited
    /// explicitly through the final broadcast (`checking == false` with the
    /// failure code), because the fetch counter increments before the
    /// persistence write — reading the state off the counter alone raced it.
    #[tokio::test]
    async fn scheduler_startup_only_failure_parks_until_next_process() {
        let dir = temp_config_dir("startup-only");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        // Fresh defaults: startup=true, periodic=false.

        let clock = TestClock::starting_at(Utc::now());
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let events = Arc::new(Mutex::new(Vec::new()));
        let emit = {
            let events = Arc::clone(&events);
            move |status: &UpdateStatusDto| {
                events
                    .lock()
                    .expect("events lock")
                    .push((status.checking, status.last_attempt_error.clone()));
            }
        };
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_failing(&calls, UpdateErrorKind::Network),
            emit,
        );

        // The attempt is complete once a broadcast reports checking=false
        // and carries the failure: persistence has finished by then.
        wait_until(
            || {
                events
                    .lock()
                    .expect("events lock")
                    .iter()
                    .any(|(checking, error)| !*checking && error.as_deref() == Some("network"))
            },
            "the startup attempt to complete",
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Give the loop time to park; no retry sleep is ever requested.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a failed startup attempt is not retried in this process"
        );
        assert!(
            clock.sleeps().is_empty(),
            "no periodic timer is armed with the schedule off"
        );

        // Schedule changes must not revive the startup attempt either.
        state.notify_schedule_changed();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // The failure outcome is persisted for the card and restarts; the
        // completion broadcast above guarantees the write has landed.
        let persisted = manager.get_update_state();
        assert_eq!(persisted.last_attempt_error.as_deref(), Some("network"));
        assert!(persisted.last_attempt_at.is_some());

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Periodic-only with no previous attempt: one attempt at process start;
    /// a failed attempt still moves the base, so — on the one injected clock
    /// — the loop re-arms exactly a full interval, recomputes without retry
    /// at +5 and +10 minutes, then fires exactly one more attempt when the
    /// deadline passes and re-arms a full interval from that failure.
    #[tokio::test]
    async fn scheduler_periodic_failure_moves_base_by_full_interval() {
        const INTERVAL: u64 = 2 * 24 * 3600;
        let dir = temp_config_dir("periodic-failure");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        manager.set_updates_startup_check_enabled(false).unwrap();
        manager.set_updates_periodic_check_enabled(true).unwrap();
        manager.set_updates_check_interval_days(2).unwrap();

        let clock = TestClock::starting_at(datetime("2026-11-18T12:00:00Z"));
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_failing(&calls, UpdateErrorKind::RateLimited),
            noop_emit,
        );

        // Never attempted: the due schedule fires exactly once at startup
        // and the failed attempt re-arms a full 2-day interval.
        wait_until(
            || calls.load(Ordering::SeqCst) == 1,
            "the first periodic attempt",
        )
        .await;
        wait_sleep_count(&clock, 1).await;
        assert_eq!(
            clock.sleeps()[0],
            Duration::from_secs(INTERVAL),
            "the failed attempt moves the base by a full interval"
        );

        // +5 minutes: a setting change wakes the loop, it recomputes without
        // a retry.
        clock.advance_secs(5 * 60);
        state.notify_schedule_changed();
        wait_sleep_count(&clock, 2).await;
        assert_eq!(clock.sleeps()[1], Duration::from_secs(INTERVAL - 5 * 60));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no five-minute retry");

        // +10 minutes: still no retry.
        clock.advance_secs(5 * 60);
        state.notify_schedule_changed();
        wait_sleep_count(&clock, 3).await;
        assert_eq!(clock.sleeps()[2], Duration::from_secs(INTERVAL - 10 * 60));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no ten-minute retry");

        // The interval passes: exactly one more attempt; the second failure
        // also re-arms a full interval from its completion.
        clock.advance_secs(INTERVAL - 10 * 60 + 1);
        clock.release_sleeps();
        wait_until(
            || calls.load(Ordering::SeqCst) == 2,
            "the second periodic attempt",
        )
        .await;
        wait_sleep_count(&clock, 4).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2, "exactly two attempts");
        assert_eq!(
            clock.sleeps()[3],
            Duration::from_secs(INTERVAL),
            "the deadline after the second failure is a full interval"
        );

        let persisted = manager.get_update_state();
        assert_eq!(
            persisted.last_attempt_error.as_deref(),
            Some("rate_limited")
        );

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A manual check — the same `try_run_check` the `check_for_updates`
    /// command runs, followed by the same `notify_schedule_changed` — moves
    /// the running scheduler's deadline from the manual completion time,
    /// for a success and for a failure alike. One injected clock everywhere.
    #[tokio::test]
    async fn manual_check_shifts_running_scheduler_from_completion() {
        const DAY: u64 = 24 * 3600;
        let dir = temp_config_dir("manual-shift");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        manager.set_updates_startup_check_enabled(false).unwrap();
        manager.set_updates_periodic_check_enabled(true).unwrap();
        // A failed attempt one hour before T0 is the seeded base: the loop
        // parks 23 hours, not due.
        manager
            .set_updates_attempt_failure(
                datetime("2026-11-18T11:00:00Z").to_rfc3339_opts(SecondsFormat::Secs, true),
                "network",
            )
            .unwrap();

        let clock = TestClock::starting_at(datetime("2026-11-18T12:00:00Z"));
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        // The scheduler's own fetch stub: never invoked while manual checks
        // shift the deadline.
        let calls = Arc::new(AtomicUsize::new(0));
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_failing(&calls, UpdateErrorKind::NotFound),
            noop_emit,
        );

        wait_sleep_count(&clock, 1).await;
        assert_eq!(clock.sleeps()[0], Duration::from_secs(23 * 3600));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // Manual SUCCESS at T0 + 2h (completion on the test grid), then the
        // command's wake-up call.
        clock.advance_secs(2 * 3600);
        let manual_clock = clock.clone();
        let release = sample_release("9.9.9");
        let status = try_run_check(
            &manager,
            &state,
            &test_install(),
            None,
            move || {
                let release = release.clone();
                async move { Ok(release) }
            },
            &noop_emit,
            move || manual_clock.now(),
        )
        .await
        .expect("the manual check ran");
        assert_eq!(status.last_attempt_error, None);
        state.notify_schedule_changed();

        // The deadline moved to the manual completion: a full interval from
        // T0 + 2h, and the scheduler itself has not fetched.
        wait_sleep_count(&clock, 2).await;
        assert_eq!(
            clock.sleeps()[1],
            Duration::from_secs(DAY),
            "the deadline re-arms a full interval from the manual completion"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "no scheduler fetch after the manual check"
        );

        // When that deadline passes, exactly one scheduler attempt runs.
        clock.advance_secs(DAY + 1);
        clock.release_sleeps();
        wait_until(
            || calls.load(Ordering::SeqCst) == 1,
            "the scheduler attempt",
        )
        .await;
        wait_sleep_count(&clock, 3).await;
        assert_eq!(clock.sleeps()[2], Duration::from_secs(DAY));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // A setting change ten minutes later recomputes the remainder
        // without a fetch; the release stays reserved for the real expiry.
        clock.advance_secs(10 * 60);
        state.notify_schedule_changed();
        wait_sleep_count(&clock, 4).await;
        assert_eq!(clock.sleeps()[3], Duration::from_secs(DAY - 10 * 60));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Manual FAILURE two hours later also shifts the deadline: the next
        // try is a full interval from the failed completion.
        clock.advance_secs(2 * 3600);
        let failed_clock = clock.clone();
        let status = try_run_check(
            &manager,
            &state,
            &test_install(),
            None,
            || async { Err(UpdateErrorKind::RateLimited) },
            &noop_emit,
            move || failed_clock.now(),
        )
        .await
        .expect("the failed manual check ran");
        assert_eq!(status.last_attempt_error.as_deref(), Some("rate_limited"));
        state.notify_schedule_changed();

        wait_sleep_count(&clock, 5).await;
        assert_eq!(
            clock.sleeps()[4],
            Duration::from_secs(DAY),
            "the failed manual check re-arms a full interval from its completion"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the scheduler stayed parked"
        );

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        let persisted = manager.get_update_state();
        assert_eq!(
            persisted.last_attempt_error.as_deref(),
            Some("rate_limited"),
            "the failed manual attempt is persisted"
        );
        assert_eq!(
            persisted.last_release.as_ref().map(|r| r.version.as_str()),
            Some("9.9.9"),
            "the manual success release survived the later failure"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With both switches off there are no automatic checks at all. Enabling
    /// the startup switch mid-run does not fire a startup attempt — that
    /// attempt belongs to the process start only — while enabling the
    /// periodic switch wakes the loop into its one due attempt.
    #[tokio::test]
    async fn scheduler_both_disabled_and_late_startup_enable_never_auto_check() {
        let dir = temp_config_dir("both-disabled");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        manager.set_updates_startup_check_enabled(false).unwrap();
        manager.set_updates_periodic_check_enabled(false).unwrap();

        let clock = TestClock::starting_at(Utc::now());
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_ok(&calls, "9.9.9"),
            noop_emit,
        );

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0, "no checks with both off");
        assert!(clock.sleeps().is_empty(), "no timer is armed");

        // The startup switch re-enabled mid-run must not fire a startup
        // attempt.
        manager.set_updates_startup_check_enabled(true).unwrap();
        state.notify_schedule_changed();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "a startup attempt is never fired mid-run"
        );

        // Enabling the periodic schedule wakes the loop: no previous attempt
        // means one due attempt now, and the re-arm is a full interval from
        // its completion on the test grid.
        manager.set_updates_periodic_check_enabled(true).unwrap();
        state.notify_schedule_changed();
        wait_until(
            || calls.load(Ordering::SeqCst) == 1,
            "the due periodic attempt",
        )
        .await;
        wait_sleep_count(&clock, 1).await;
        assert_eq!(
            clock.sleeps()[0],
            Duration::from_secs(24 * 3600),
            "the due attempt re-arms a full interval"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Changing the interval recomputes the deadline from the last completed
    /// attempt without firing an attempt. Fixed fixture times, exact values.
    #[tokio::test]
    async fn scheduler_interval_change_recomputes_deadline() {
        let dir = temp_config_dir("interval-change");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        manager.set_updates_startup_check_enabled(false).unwrap();
        manager.set_updates_periodic_check_enabled(true).unwrap();
        // A failed attempt two hours before T0 is the schedule base.
        manager
            .set_updates_attempt_failure(
                datetime("2026-11-18T10:00:00Z").to_rfc3339_opts(SecondsFormat::Secs, true),
                "network",
            )
            .unwrap();

        let clock = TestClock::starting_at(datetime("2026-11-18T12:00:00Z"));
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_failing(&calls, UpdateErrorKind::Network),
            noop_emit,
        );

        wait_sleep_count(&clock, 1).await;
        assert_eq!(
            clock.sleeps()[0],
            Duration::from_secs(22 * 3600),
            "22 hours left of the 1-day interval after an attempt 2 hours ago"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // Switching the interval to 7 days wakes the loop: the deadline is
        // re-counted from the same attempt, without any fetch.
        manager.set_updates_check_interval_days(7).unwrap();
        state.notify_schedule_changed();
        wait_sleep_count(&clock, 2).await;
        assert_eq!(
            clock.sleeps()[1],
            Duration::from_secs(7 * 24 * 3600 - 2 * 3600),
            "the new interval re-counts from the same completion"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "no attempt on interval change"
        );

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A persistence failure surfaces as the stable `save` code but must not
    /// cause an immediate refetch: the in-memory attempt base still moves,
    /// so even after a wake-up the loop re-arms a full interval.
    #[tokio::test]
    async fn scheduler_save_error_reported_without_immediate_refetch() {
        let dir = temp_config_dir("save-error");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        manager.set_updates_periodic_check_enabled(true).unwrap();
        // Break persistence of the check state: `updates` is now a file where
        // the state directory should be, so every read-modify-write of
        // `updates/state.json` fails while settings.json stays healthy.
        std::fs::write(dir.join("updates"), b"not a directory").unwrap();

        let clock = TestClock::starting_at(datetime("2026-11-18T12:00:00Z"));
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_ok(&calls, "9.9.9"),
            noop_emit,
        );

        wait_until(|| calls.load(Ordering::SeqCst) == 1, "the startup attempt").await;
        wait_sleep_count(&clock, 1).await;
        assert_eq!(
            clock.sleeps()[0],
            Duration::from_secs(24 * 3600),
            "the in-memory base moved despite the save error"
        );

        // The session shows the save error; nothing was persisted.
        let status = status_snapshot(&manager, &state, &test_install());
        assert_eq!(status.last_attempt_error.as_deref(), Some("save"));
        assert_eq!(
            status.last_attempt_at.as_deref(),
            Some("2026-11-18T12:00:00Z")
        );
        let persisted = manager.get_update_state();
        assert_eq!(
            persisted.last_attempt_at, None,
            "a failed persistence leaves the stored record alone"
        );

        // A schedule wake-up recomputes the deadline from the in-memory
        // base: a full interval again, still exactly one fetch.
        state.notify_schedule_changed();
        wait_sleep_count(&clock, 2).await;
        assert_eq!(clock.sleeps()[1], Duration::from_secs(24 * 3600));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "no immediate refetch after a save error"
        );

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression: the persisted attempt record drives the scheduler's
    /// deadline and stays visible in the snapshot with its error, not just
    /// its time (startup off, periodic on with a not-yet-due schedule — the
    /// loop derives the base from the state file and parks without making
    /// any attempt).
    #[tokio::test]
    async fn persisted_error_drives_deadline_and_stays_visible() {
        let dir = temp_config_dir("seed-error");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        manager.set_updates_startup_check_enabled(false).unwrap();
        manager.set_updates_periodic_check_enabled(true).unwrap();
        manager
            .set_updates_attempt_failure(
                datetime("2026-11-18T11:00:00Z").to_rfc3339_opts(SecondsFormat::Secs, true),
                "network",
            )
            .unwrap();

        let clock = TestClock::starting_at(datetime("2026-11-18T12:00:00Z"));
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_ok(&calls, "9.9.9"),
            noop_emit,
        );

        // The persisted record is proven consumed once the loop has computed
        // its first deadline from it: 23 hours left of the 1-day interval
        // after the attempt an hour before T0.
        wait_sleep_count(&clock, 1).await;
        assert_eq!(
            clock.sleeps()[0],
            Duration::from_secs(23 * 3600),
            "the deadline counts from the persisted failed attempt"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "the loop stays parked");

        // The snapshot shows the stored failure — not a false success
        // rebuilt from the persisted time.
        let status = status_snapshot(&manager, &state, &test_install());
        assert_eq!(
            status.last_attempt_error.as_deref(),
            Some("network"),
            "the persisted record keeps its error visible"
        );
        assert_eq!(
            status.last_attempt_at.as_deref(),
            Some("2026-11-18T11:00:00Z")
        );
        assert!(!status.checking);

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The scheduler's periodic base: the in-memory attempt of the current
    /// root when present, the persisted record of the same root's file
    /// otherwise — resolved as one transaction, like the loop does.
    fn scheduler_base(state: &UpdatesState, manager: &SettingsManager) -> Option<DateTime<Utc>> {
        let (attempt, (_current_dir, persisted)) =
            state.read_attempt_with(|| manager.get_update_state_with_dir());
        attempt
            .map(|record| record.at)
            .or_else(|| persisted_attempt_time(&persisted))
    }

    /// A pinned update state store for direct file assertions on an
    /// isolated test root.
    fn state_store_at(dir: &Path) -> state::UpdateStateStore {
        state::UpdateStateStore::with_state_dir(dir.to_path_buf())
    }

    /// A fetch stub held until the test releases the gate, then answering
    /// with a fixed release: models a network request spanning a root
    /// switch.
    fn fetch_gated(
        gate: Arc<tokio::sync::Notify>,
        version: &'static str,
    ) -> impl Fn() -> std::pin::Pin<
        Box<dyn Future<Output = Result<ReleaseInfoDto, UpdateErrorKind>> + Send>,
    > + Send
           + Sync
           + Clone {
        move || {
            let gate = Arc::clone(&gate);
            Box::pin(async move {
                gate.notified().await;
                Ok(sample_release(version))
            })
        }
    }

    /// Regression: the attempt time is the fetch completion time — a delayed
    /// fetch moves the periodic base (in memory and persisted) by its full
    /// duration, and the deadline counts from that same completion on the
    /// one injected time grid.
    #[tokio::test]
    async fn attempt_time_is_fetch_completion_time() {
        let dir = temp_config_dir("completion-time");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        let state = UpdatesState::new();
        let clock = TestClock::starting_at(datetime("2026-11-18T12:00:00Z"));

        // The fetch "takes" 90 seconds on the test grid.
        let fetch = fetch_ok_advancing(&Arc::new(AtomicUsize::new(0)), "9.9.9", &clock, 90);
        let now_clock = clock.clone();
        let status = try_run_check(
            &manager,
            &state,
            &test_install(),
            None,
            fetch,
            &noop_emit,
            move || now_clock.now(),
        )
        .await
        .expect("the check ran");
        assert_eq!(status.last_attempt_error, None);

        // The completion time — not the start — is the attempt time in
        // memory, in the persisted record and in the shown snapshot.
        let expected = datetime("2026-11-18T12:01:30Z");
        assert_eq!(
            state
                .attempt_of_root(manager.update_state_dir().as_deref())
                .map(|record| record.at),
            Some(expected),
            "the in-memory base is the fetch completion time"
        );
        let persisted = manager.get_update_state();
        assert_eq!(
            persisted.last_attempt_at.as_deref(),
            Some("2026-11-18T12:01:30Z"),
            "the persisted attempt time is the fetch completion time"
        );
        assert_eq!(persisted_attempt_time(&persisted), Some(expected));
        assert_eq!(persisted.last_attempt_error, None);
        assert_eq!(
            status.last_attempt_at.as_deref(),
            Some("2026-11-18T12:01:30Z")
        );

        // On the one grid, the next periodic deadline is exactly a full
        // interval from that completion.
        assert_eq!(
            next_periodic_delay(scheduler_base(&state, &manager), 1, clock.now()),
            Some(Duration::from_secs(24 * 3600)),
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression: a scheduled attempt broadcasts its start (checking=true)
    /// and its finish, so an already open card shows the spinner and keeps
    /// the manual button disabled while the automatic check runs.
    #[tokio::test]
    async fn scheduled_attempt_broadcasts_start_and_finish_events() {
        let dir = temp_config_dir("start-finish-events");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        // Fresh defaults: startup=true → the startup attempt is scheduled.

        let events = Arc::new(Mutex::new(Vec::new()));
        let emit = {
            let events = Arc::clone(&events);
            move |status: &UpdateStatusDto| {
                events
                    .lock()
                    .expect("events lock")
                    .push((status.checking, status.last_attempt_error.clone()));
            }
        };

        let clock = TestClock::starting_at(datetime("2026-11-18T12:00:00Z"));
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        // The fetch "takes" 150 seconds on the test grid, so the in-flight
        // broadcast happens strictly before the completion one.
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_ok_advancing(&Arc::new(AtomicUsize::new(0)), "9.9.9", &clock, 150),
            emit,
        );

        wait_until(
            || {
                let events = events.lock().expect("events lock");
                events.len() >= 2 && !events[events.len() - 1].0
            },
            "the start and finish broadcasts",
        )
        .await;

        {
            let events = events.lock().expect("events lock");
            assert_eq!(
                events.len(),
                2,
                "exactly one start and one finish broadcast"
            );
            assert!(
                events[0].0,
                "the start broadcast carries checking=true before the fetch completes"
            );
            assert!(!events[1].0, "the finish broadcast clears checking");
            assert_eq!(events[1].1, None, "the finished attempt succeeded");
        }

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ============ data root switch (see commands::storage) ============

    /// After a «Данные программы» switch the snapshot shows the new root's
    /// own state only: an existing state file of the new root is restored
    /// (the in-memory record of the previous root never wins), and an empty
    /// new root yields the empty state — the old root's release, check time
    /// and attempt record are invisible there.
    #[tokio::test]
    async fn snapshot_after_root_switch_uses_new_roots_state_only() {
        let dir_a = temp_config_dir("switch-a");
        let dir_b = temp_config_dir("switch-b");
        let dir_c = temp_config_dir("switch-c-empty");
        let manager = SettingsManager::with_config_dir(dir_a.clone()).unwrap();
        let state = Arc::new(UpdatesState::new());

        // Root A: a completed success — fresh release and attempt in memory
        // and on disk.
        let release = sample_release("1.8.0");
        let status = try_run_check(
            &manager,
            &state,
            &test_install(),
            None,
            move || {
                let release = release.clone();
                async move { Ok(release) }
            },
            &noop_emit,
            Utc::now,
        )
        .await
        .expect("the check ran");
        assert_eq!(
            status.latest.as_ref().map(|r| r.version.as_str()),
            Some("1.8.0")
        );

        // Root B pre-exists with a different result.
        let manager_b = SettingsManager::with_config_dir(dir_b.clone()).unwrap();
        manager_b
            .set_updates_check_result("2026-11-18T09:00:00Z".to_string(), sample_release("2.0.0"))
            .unwrap();
        manager_b
            .set_updates_attempt_failure("2026-11-18T10:00:00Z".to_string(), "rate_limited")
            .unwrap();

        // Switch A → B (the `publish_data_root` equivalent) and look at the
        // card: B's own record is restored, A's in-memory success does not
        // leak into the attempt pair.
        manager.set_update_state_dir_for_tests(dir_b.join("updates"));
        let status = status_snapshot(&manager, &state, &test_install());
        assert_eq!(
            status.latest.as_ref().map(|r| r.version.as_str()),
            Some("2.0.0"),
            "the new root's existing file is restored"
        );
        assert!(status.update_available);
        assert_eq!(
            status.last_attempt_error.as_deref(),
            Some("rate_limited"),
            "the attempt pair comes from the new root's file"
        );
        assert_eq!(
            status.last_attempt_at.as_deref(),
            Some("2026-11-18T10:00:00Z")
        );
        assert_eq!(
            status.last_check_at.as_deref(),
            Some("2026-11-18T09:00:00Z")
        );

        // Switch to an empty root: nothing of A or B is visible there.
        manager.set_update_state_dir_for_tests(dir_c.join("updates"));
        let status = status_snapshot(&manager, &state, &test_install());
        assert_eq!(status.latest, None, "an empty root has no release");
        assert!(!status.update_available);
        assert_eq!(status.last_attempt_at, None);
        assert_eq!(status.last_attempt_error, None);
        assert_eq!(status.last_check_at, None);
        assert_eq!(status.dismissed_version, None);

        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
        let _ = std::fs::remove_dir_all(&dir_c);
    }

    /// A check whose fetch is delayed completes after the root switched: the
    /// result lands in the original root's file (captured at the attempt's
    /// start), never in the new root's file or snapshot, and the new root's
    /// periodic base ignores the in-memory record of the old root.
    #[tokio::test]
    async fn late_check_completion_lands_in_its_original_root() {
        let dir_a = temp_config_dir("late-a");
        let dir_b = temp_config_dir("late-b-empty");
        let manager = SettingsManager::with_config_dir(dir_a.clone()).unwrap();
        let state = Arc::new(UpdatesState::new());
        let a_dir = manager.update_state_dir().expect("pinned store resolves");
        let b_dir = dir_b.join("updates");

        // A fetch held until the test releases it.
        let gate = Arc::new(tokio::sync::Notify::new());
        let task_gate = Arc::clone(&gate);
        let task = {
            let manager = manager.clone();
            let state = Arc::clone(&state);
            tokio::spawn(async move {
                try_run_check(
                    &manager,
                    &state,
                    &test_install_arc(),
                    None,
                    fetch_gated(task_gate, "9.9.9"),
                    &noop_emit,
                    Utc::now,
                )
                .await
            })
        };
        wait_until(|| state.is_checking(), "the delayed check to start").await;

        // The root switches while the request is in flight.
        manager.set_update_state_dir_for_tests(b_dir.clone());
        let status = status_snapshot(&manager, &state, &test_install());
        assert_eq!(
            status.latest, None,
            "the empty root hides the in-flight result"
        );

        gate.notify_one();
        let status = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the delayed check completes")
            .expect("the check task does not panic")
            .expect("the check ran");
        // The completion snapshot reflects the current (empty) root: the old
        // result does not change the new root's card.
        assert_eq!(status.latest, None);
        assert_eq!(status.last_attempt_at, None);

        // The result was persisted into the original root only.
        let a_state = state_store_at(&a_dir).load();
        assert_eq!(
            a_state.last_release.as_ref().map(|r| r.version.as_str()),
            Some("9.9.9"),
            "the completion lands in the root captured at the attempt's start"
        );
        assert!(
            !b_dir.join("state.json").exists(),
            "the new root's file is not written"
        );

        // The in-memory record is bound to the old root: it never drives the
        // new root's snapshot or periodic base.
        assert!(state.attempt_of_root(Some(&a_dir)).is_some());
        assert!(state.attempt_of_root(Some(&b_dir)).is_none());
        assert_eq!(scheduler_base(&state, &manager), None);

        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    /// Broadcast ordering: a check completion that is mid-emit when the
    /// storage root switches must hold the shared broadcast mutex, so the
    /// root switch's fresh snapshot cannot be published in the middle of it
    /// — and a switch that broadcast first makes the check's snapshot wait
    /// and be computed from the new root. Either way the switch's event
    /// lands last and carries the new root's state: a stale A event never
    /// follows the B root-switch event.
    #[test]
    fn root_switch_broadcast_is_serialized_with_check_completion() {
        let dir_a = temp_config_dir("order-switch-a");
        let dir_b = temp_config_dir("order-switch-b");
        let manager = SettingsManager::with_config_dir(dir_a.clone()).unwrap();
        // Root A carries an update, so the check's events carry release
        // content that the empty root B must not show after the switch.
        manager
            .set_updates_check_result("2026-11-18T08:00:00Z".to_string(), sample_release("9.9.9"))
            .unwrap();
        let state = Arc::new(UpdatesState::new());
        let b_dir = dir_b.join("updates");

        // Shared log of emitted events, in emission order.
        let log = Arc::new(std::sync::Mutex::new(
            Vec::<(&'static str, UpdateStatusDto)>::new(),
        ));

        // The check's emit: the started broadcast passes through; the final
        // broadcast blocks mid-emit until the test releases it, holding the
        // broadcast mutex. The gate receiver sits in a Mutex: `try_run_check`
        // requires the emit to be Sync, and a bare `mpsc::Receiver` is not.
        let (final_in_emit, final_in_emit_rx) = std::sync::mpsc::channel();
        let (final_release, final_release_rx) = std::sync::mpsc::channel::<()>();
        let final_gate = Arc::new(std::sync::Mutex::new(final_release_rx));
        let broadcast_count = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&broadcast_count);
        let log_check = Arc::clone(&log);
        let emit = move |status: &UpdateStatusDto| {
            let tag = if count.fetch_add(1, Ordering::SeqCst) == 0 {
                "check-started"
            } else {
                "check-final"
            };
            log_check.lock().unwrap().push((tag, status.clone()));
            if tag == "check-final" {
                final_in_emit
                    .send(())
                    .expect("the final-emit signal is sent");
                final_gate
                    .lock()
                    .expect("the final gate lock")
                    .recv()
                    .expect("the release signal arrives");
            }
        };

        // One shared InstallState: the broadcast-order mutex must be the
        // same instance on both sides for the serialization to hold.
        let install = test_install_arc();
        let task_manager = manager.clone();
        let task_state = Arc::clone(&state);
        let task_install = Arc::clone(&install);
        // The check runs on a separate thread so the test can release its
        // emitter while it holds the broadcast mutex. Completion is
        // signalled on a channel so the wait stays bounded.
        let (final_done, final_done_rx) = std::sync::mpsc::channel();
        let task = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .build()
                .expect("the check runtime builds");
            runtime.block_on(async {
                try_run_check(
                    &task_manager,
                    &task_state,
                    &task_install,
                    None,
                    || async { Ok(sample_release("9.9.9")) },
                    &emit,
                    Utc::now,
                )
                .await
                .expect("the check ran");
            });
            final_done.send(()).expect("the completion signal is sent");
        });
        // The completion entered its emit while holding the mutex.
        final_in_emit_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the check completion entered its emit");

        // The root switches while the check is mid-emit; the switch's
        // snapshot publisher must wait for the mutex.
        manager.set_update_state_dir_for_tests(b_dir.clone());

        let (switch_in_emit, switch_in_emit_rx) = std::sync::mpsc::channel();
        let (switch_release, switch_release_rx) = std::sync::mpsc::channel::<()>();
        let switch_manager = manager.clone();
        let switch_state = Arc::clone(&state);
        let switch_install = Arc::clone(&install);
        let log_switch = Arc::clone(&log);
        let switcher = std::thread::spawn(move || {
            broadcast_root_switch_snapshot(
                &switch_manager,
                &switch_state,
                &switch_install,
                move |status: &UpdateStatusDto| {
                    log_switch
                        .lock()
                        .unwrap()
                        .push(("root-switch", status.clone()));
                    switch_in_emit
                        .send(())
                        .expect("the switch-emit signal is sent");
                    switch_release_rx
                        .recv()
                        .expect("the switch release arrives");
                },
            );
        });
        assert!(
            switch_in_emit_rx
                .recv_timeout(Duration::from_millis(150))
                .is_err(),
            "the root switch broadcast must wait while the check is mid-emit"
        );

        // Finish the check: its emit completes, then the switch publishes.
        final_release.send(()).expect("the release is sent");
        final_done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the check completes");
        task.join().expect("the check thread does not panic");
        switch_in_emit_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the switch broadcast emits after the check released the mutex");
        switch_release.send(()).expect("the switch release is sent");
        switcher.join().expect("the switch publisher finishes");

        let events = log.lock().unwrap().clone();
        let tags: Vec<&'static str> = events.iter().map(|(tag, _)| *tag).collect();
        assert_eq!(
            tags,
            vec!["check-started", "check-final", "root-switch"],
            "the root switch event lands last"
        );
        let switch_status = &events.last().unwrap().1;
        assert!(
            !switch_status.update_available && switch_status.latest.is_none(),
            "the last event reflects the new (empty) root, not the old root's release"
        );
        assert!(
            !switch_status.checking,
            "the switch snapshot is post-completion"
        );

        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    /// After a switch into an empty root the already-waiting periodic timer
    /// recomputes on the wake-up: no base → the existing rule fires one
    /// attempt now, the completion persists into the new root's file and
    /// re-arms a full interval — while the previous root's record stays
    /// untouched.
    #[tokio::test]
    async fn scheduler_recomputes_timer_after_root_switch_into_empty_root() {
        const DAY: u64 = 24 * 3600;
        let dir_a = temp_config_dir("rearm-a");
        let dir_b = temp_config_dir("rearm-b-empty");
        let manager = SettingsManager::with_config_dir(dir_a.clone()).unwrap();
        manager.set_updates_startup_check_enabled(false).unwrap();
        manager.set_updates_periodic_check_enabled(true).unwrap();
        // A failed attempt one hour before T0 parks the loop 23 hours.
        manager
            .set_updates_attempt_failure(
                datetime("2026-11-18T11:00:00Z").to_rfc3339_opts(SecondsFormat::Secs, true),
                "network",
            )
            .unwrap();

        let clock = TestClock::starting_at(datetime("2026-11-18T12:00:00Z"));
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_ok(&calls, "9.9.9"),
            noop_emit,
        );

        wait_sleep_count(&clock, 1).await;
        assert_eq!(clock.sleeps()[0], Duration::from_secs(23 * 3600));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "parked on the old root's base"
        );

        // The switch (publish) plus the storage command's wake-up: the
        // waiting timer is recomputed from the new root's (empty) file.
        manager.set_update_state_dir_for_tests(dir_b.join("updates"));
        state.notify_schedule_changed();

        // Empty root → no base → one due attempt now, persisted into the
        // new root and re-armed a full interval from its completion.
        wait_until(
            || calls.load(Ordering::SeqCst) == 1,
            "the due attempt in the new root",
        )
        .await;
        wait_sleep_count(&clock, 2).await;
        assert_eq!(
            clock.sleeps()[1],
            Duration::from_secs(DAY),
            "the deadline re-arms from the attempt in the new root, not from the old base"
        );

        let a_state = state_store_at(&dir_a.join("updates")).load();
        assert_eq!(
            a_state.last_attempt_error.as_deref(),
            Some("network"),
            "the old root's record stays untouched"
        );
        assert_eq!(a_state.last_release, None);
        let b_state = state_store_at(&dir_b.join("updates")).load();
        assert_eq!(
            b_state.last_release.as_ref().map(|r| r.version.as_str()),
            Some("9.9.9"),
            "the new root's file owns the new result"
        );
        assert_eq!(b_state.last_attempt_error, None);

        let status = status_snapshot(&manager, &state, &test_install());
        assert_eq!(
            status.latest.as_ref().map(|r| r.version.as_str()),
            Some("9.9.9")
        );
        assert_eq!(status.last_attempt_error, None);

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    /// Regression: an expired periodic timer must not run the current root
    /// on a stale due decision. Root A armed the timer (its base goes due in
    /// an hour); mid-wait the root switches to B, whose own recent attempt
    /// keeps it not due. The timer expires without a schedule wake-up
    /// (`release_sleeps` only): the loop recomputes from B's base instead of
    /// checking B early on A's overdue decision — no fetch, and the timer
    /// re-arms on B's deadline.
    #[tokio::test]
    async fn expired_timer_recomputes_due_after_root_switch_instead_of_early_check() {
        const DAY: u64 = 24 * 3600;
        let dir_a = temp_config_dir("expiry-a");
        let dir_b = temp_config_dir("expiry-b");
        let manager = SettingsManager::with_config_dir(dir_a.clone()).unwrap();
        manager.set_updates_startup_check_enabled(false).unwrap();
        manager.set_updates_periodic_check_enabled(true).unwrap();
        // A failed attempt 23 hours before T0 arms A's timer for one hour.
        manager
            .set_updates_attempt_failure(
                datetime("2026-11-18T13:00:00Z").to_rfc3339_opts(SecondsFormat::Secs, true),
                "network",
            )
            .unwrap();

        // Root B pre-exists with a fresh attempt at T0: not due for a day.
        let manager_b = SettingsManager::with_config_dir(dir_b.clone()).unwrap();
        manager_b
            .set_updates_attempt_failure(
                datetime("2026-11-19T12:00:00Z").to_rfc3339_opts(SecondsFormat::Secs, true),
                "network",
            )
            .unwrap();

        let clock = TestClock::starting_at(datetime("2026-11-19T12:00:00Z"));
        let state = Arc::new(UpdatesState::new());
        let shutdown = CancellationToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let task = spawn_loop(
            &manager,
            &state,
            &clock,
            &shutdown,
            fetch_ok(&calls, "9.9.9"),
            noop_emit,
        );

        // The loop parks one hour on A's base.
        wait_sleep_count(&clock, 1).await;
        assert_eq!(clock.sleeps()[0], Duration::from_secs(3600));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // The switch happens mid-wait, without notifying the scheduler:
        // only the timer expiry can wake the loop.
        manager.set_update_state_dir_for_tests(dir_b.join("updates"));

        // The timer expires: no attempt fires in B, the loop re-arms on B's
        // deadline (a full day from B's T0 attempt minus the 1h+1s elapsed).
        clock.advance_secs(3601);
        clock.release_sleeps();
        wait_sleep_count(&clock, 2).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "no early check of the new root on the old root's due decision"
        );
        assert_eq!(
            clock.sleeps()[1],
            Duration::from_secs(DAY - 3601),
            "the timer re-arms from the new root's own recent attempt"
        );

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop exits on shutdown")
            .expect("the loop task does not panic");

        // Neither root's file was touched by the scheduler.
        assert_eq!(
            state_store_at(&dir_a.join("updates"))
                .load()
                .last_attempt_at
                .as_deref(),
            Some("2026-11-18T13:00:00Z"),
        );
        assert_eq!(
            state_store_at(&dir_b.join("updates"))
                .load()
                .last_attempt_at
                .as_deref(),
            Some("2026-11-19T12:00:00Z"),
        );

        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    /// Interleaving regression: a completed attempt publishes its persisted
    /// write and its runtime record as one pair under the publish lock, so a
    /// concurrent reader crossing the completion never observes a mixed
    /// state — the fresh persisted release next to the previous attempt's
    /// stale error, or a scheduler base regressed to the stale seeded record
    /// while the fresh release is already visible.
    #[tokio::test]
    async fn concurrent_reader_never_sees_mixed_persisted_and_runtime_state() {
        let dir = temp_config_dir("publish-pair");
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();
        // A seeded stale success and a stale failure: both the stale runtime
        // record a two-step completion could leave in memory and the stale
        // persisted release the mixed observation would pair it with.
        manager
            .set_updates_check_result("2026-11-15T08:00:00Z".to_string(), sample_release("1.0.0"))
            .unwrap();
        manager
            .set_updates_attempt_failure("2026-11-15T09:00:00Z".to_string(), "network")
            .unwrap();
        let state = Arc::new(UpdatesState::new());
        // Seed the stale runtime record too (same root, the stale failure
        // time and code): the mixed observation a two-step completion could
        // produce is exactly this record surviving beside the fresh
        // persisted release.
        state.set_last_attempt(AttemptRecord {
            at: datetime("2026-11-15T09:00:00Z"),
            error: Some("network".to_string()),
            dir: manager.update_state_dir(),
        });

        let stop = Arc::new(AtomicBool::new(false));
        let observations = Arc::new(AtomicUsize::new(0));
        let reader = {
            let manager = manager.clone();
            let state = Arc::clone(&state);
            let stop = Arc::clone(&stop);
            let observations = Arc::clone(&observations);
            tokio::spawn(async move {
                while !stop.load(Ordering::SeqCst) {
                    let status = status_snapshot(&manager, &state, &test_install());
                    let fresh_release = status
                        .latest
                        .as_ref()
                        .map(|release| release.version == "9.9.9")
                        .unwrap_or(false);
                    if fresh_release {
                        assert_ne!(
                            status.last_attempt_error.as_deref(),
                            Some("network"),
                            "a fresh release must never be published next to the older error"
                        );
                        assert_ne!(
                            scheduler_base(&state, &manager),
                            Some(datetime("2026-11-15T09:00:00Z")),
                            "the base must not regress to the stale seeded record while the fresh release is visible"
                        );
                    }
                    observations.fetch_add(1, Ordering::SeqCst);
                    // Yield so the reader runs between the checker's await
                    // points — exactly where a two-step completion could be
                    // observed half-applied.
                    tokio::task::yield_now().await;
                }
            })
        };

        // Repeated checks keep crossing the reader with the publish
        // critical section.
        for _ in 0..30 {
            let release = sample_release("9.9.9");
            try_run_check(
                &manager,
                &state,
                &test_install(),
                None,
                move || {
                    let release = release.clone();
                    async move { Ok(release) }
                },
                &noop_emit,
                Utc::now,
            )
            .await
            .expect("the check ran");
        }

        stop.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(5), reader)
            .await
            .expect("the reader stops")
            .expect("the reader does not panic");
        assert!(
            observations.load(Ordering::SeqCst) > 0,
            "the concurrent reader actually sampled snapshots"
        );

        // After the dust settles the pair is the fresh one everywhere.
        let status = status_snapshot(&manager, &state, &test_install());
        assert_eq!(status.last_attempt_error, None);
        assert_eq!(
            status
                .latest
                .as_ref()
                .map(|release| release.version.as_str()),
            Some("9.9.9")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
