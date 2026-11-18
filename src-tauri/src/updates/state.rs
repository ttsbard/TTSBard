//! Persisted update-check state (ROADMAP-141): `<data_root>/updates/state.json`.
//!
//! The schedule switches live in `settings.json` (see
//! `crate::config::UpdateSettings`); everything a restart must restore about
//! completed checks — the found release, the success time, the last attempt
//! record and the dismissed version — lives here instead, so a check attempt
//! never rewrites `settings.json`. Per the user's decision (2026-11-18) there
//! is no migration: legacy state keys inside an old `settings.json` are
//! ignored, and the new file starts empty.
//!
//! Durability rules: writes reuse the shared atomic JSON replace under one
//! in-process lock, so concurrent read-modify-write transactions serialize and
//! the file is never observed half-written. Reads are tolerant: a missing file
//! is a normal empty state, and a broken or unavailable file logs a warning
//! and yields the empty state instead of failing startup; the next successful
//! write replaces the damaged file.
//!
//! The path is resolved per operation against the effective
//! [`crate::paths::data_root`], because the settings manager (and therefore
//! this store) is constructed before the configured data root override is
//! applied at startup. Changing «Данные программы» mid-run switches subsequent
//! reads and writes to the new root without migrating anything: the cached
//! release, the attempt records and the dismissed version are regenerable and
//! simply reset (see `commands::storage`).
//!
//! The resolved directory is also the *identity* of the state file: an
//! in-flight check pins it at its start (see
//! `SettingsManager::set_updates_check_result_in`), so a completion is
//! persisted into the original root even when the effective root changed
//! while the request ran, and in-memory attempt records carry the same
//! identity, so a late completion of a previous root can never show in or
//! drive the current root's card or schedule.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tracing::warn;

use crate::config::UpdateRelease;

/// File name inside the state directory.
const STATE_FILE_NAME: &str = "state.json";
/// Subdirectory of the data root holding the state file.
const STATE_DIR_NAME: &str = "updates";

/// Serializes all readers and read-modify-write transactions of update state
/// files within this process, mirroring `config::persistence`'s write lock:
/// independent store instances pointing at one file still see a consistent
/// file, and the atomic replace never interleaves with a read.
static STATE_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn state_write_lock() -> &'static Mutex<()> {
    STATE_WRITE_LOCK.get_or_init(|| Mutex::new(()))
}

/// The persisted outcome of update checks: the fields a restart must restore,
/// stored in [`STATE_FILE_NAME`] (see the module docs). Every field is
/// optional and defaults to `None`, so a partial or older file loads as
/// "unknown" rather than failing; unknown extra keys are ignored for forward
/// compatibility. Kept in memory only through the [`UpdateStateStore`] —
/// consumers read a fresh snapshot per access.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UpdateStateRecord {
    /// Last found stable release; `None` — no successful check has completed.
    /// The card and the panel are rebuilt from it after a restart.
    #[serde(default)]
    pub last_release: Option<UpdateRelease>,
    /// RFC3339 (UTC) of the last successful check, including manual ones.
    /// Unchanged by a failed attempt.
    #[serde(default)]
    pub last_check_at: Option<String>,
    /// RFC3339 (UTC) of the last completed attempt (success or failure,
    /// manual included); the periodic schedule counts from it.
    #[serde(default)]
    pub last_attempt_at: Option<String>,
    /// Stable code of the last failed attempt; `None` — the last attempt
    /// succeeded or no attempt has completed yet.
    #[serde(default)]
    pub last_attempt_error: Option<String>,
    /// Version whose update panel the user closed; a newer release shows the
    /// panel again.
    #[serde(default)]
    pub dismissed_version: Option<String>,
}

/// Persistence owner of [`UpdateStateRecord`].
///
/// Production instances resolve the file under the effective
/// [`crate::paths::data_root`] on every operation; an explicit directory
/// replaces the resolution for isolated test paths and for the pinned
/// directory of an in-flight check. The directory is interior-mutable so a
/// test can retarget one shared store (all [`SettingsManager`] clones share
/// one instance) to simulate the mid-run root switch.
#[derive(Default)]
pub struct UpdateStateStore {
    /// Explicit state directory (tests, pinned in-flight checks); `None` —
    /// resolve per operation against the effective data root.
    state_dir: Mutex<Option<PathBuf>>,
}

impl UpdateStateStore {
    /// Store persisting under the effective data root.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store persisting under an explicit directory: isolated test paths and
    /// the root pinned at an in-flight check's start.
    pub(crate) fn with_state_dir(state_dir: PathBuf) -> Self {
        Self {
            state_dir: Mutex::new(Some(state_dir)),
        }
    }

    /// The state directory this store resolves to right now: the explicit
    /// directory when pinned, the effective data root's `updates`
    /// subdirectory otherwise. This is the identity in-memory attempt
    /// records are compared against.
    pub(crate) fn resolved_dir(&self) -> Option<PathBuf> {
        let pinned = self
            .state_dir
            .lock()
            .expect("update state dir lock")
            .clone();
        pinned.or_else(|| {
            crate::paths::data_root()
                .ok()
                .map(|root| root.join(STATE_DIR_NAME))
        })
    }

    /// Retarget the pinned directory (tests simulate the mid-run root
    /// switch of `commands::storage`).
    #[cfg(test)]
    pub(crate) fn set_dir(&self, dir: Option<PathBuf>) {
        *self.state_dir.lock().expect("update state dir lock") = dir;
    }

    /// Path of the state file for this operation.
    fn state_path(&self) -> Result<PathBuf> {
        match self.resolved_dir() {
            Some(dir) => Ok(dir.join(STATE_FILE_NAME)),
            None => crate::paths::data_root()
                .map(|root| root.join(STATE_DIR_NAME).join(STATE_FILE_NAME)),
        }
    }

    /// Tolerant read of one file: missing — the normal empty state; broken or
    /// unreadable — a warning and the empty state, never an error, so a
    /// damaged file cannot break startup.
    fn read_file(path: &Path) -> UpdateStateRecord {
        if !path.exists() {
            return UpdateStateRecord::default();
        }
        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) => {
                warn!(
                    path = %crate::secret_log::safe_path_for_log(path),
                    error = %error,
                    "Failed to read the update state file; treating as empty"
                );
                return UpdateStateRecord::default();
            }
        };
        match serde_json::from_str(&content) {
            Ok(record) => record,
            Err(error) => {
                warn!(
                    path = %crate::secret_log::safe_path_for_log(path),
                    error = %error,
                    "Broken update state file; treating as empty"
                );
                UpdateStateRecord::default()
            }
        }
    }

    /// Read the persisted state. Never fails: see [`Self::read_file`].
    #[cfg(test)]
    pub fn load(&self) -> UpdateStateRecord {
        self.load_with_dir().1
    }

    /// Read the persisted state together with the directory the record was
    /// read from, resolved once: the returned directory is the identity of
    /// the returned record. Consumers that need the record and its root (the
    /// status snapshot, the periodic schedule base) must take this pair
    /// instead of resolving the directory and loading the record separately —
    /// between two resolutions a «Данные программы» switch would pair one
    /// root's directory with the other root's JSON.
    pub fn load_with_dir(&self) -> (Option<PathBuf>, UpdateStateRecord) {
        let _guard = state_write_lock()
            .lock()
            .expect("update state write lock");
        match self.resolved_dir() {
            Some(dir) => {
                let path = dir.join(STATE_FILE_NAME);
                (Some(dir), Self::read_file(&path))
            }
            None => {
                warn!(
                    error = "the data root cannot be resolved",
                    "Failed to resolve the update state path; treating as empty"
                );
                (None, UpdateStateRecord::default())
            }
        }
    }

    /// One serialized read-modify-write transaction: read the current record
    /// (tolerantly), apply the mutation, replace the file atomically. The
    /// shared lock serializes concurrent transactions, so interleaved writers
    /// cannot lose one another's fields. A write failure returns `Err` (the
    /// caller's existing save-error semantics) and leaves the previous file
    /// untouched; a damaged file read as empty is replaced by this write.
    pub fn update(&self, mutate: impl FnOnce(&mut UpdateStateRecord)) -> Result<()> {
        let _guard = state_write_lock()
            .lock()
            .expect("update state write lock");
        let path = self.state_path()?;
        let mut record = Self::read_file(&path);
        mutate(&mut record);
        let content =
            serde_json::to_string_pretty(&record).context("Failed to serialize update state")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create the update state directory")?;
        }
        crate::config::replace_file_atomically(&path, content.as_bytes())
            .context("Failed to write the update state file")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state_dir(tag: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-update-state-{tag}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_release(version: &str) -> UpdateRelease {
        UpdateRelease {
            version: version.to_string(),
            name: Some(format!("TTSBard {version}")),
            notes: "What is new".to_string(),
            url: format!("https://github.com/ttsbard/TTSBard/releases/tag/v{version}"),
            published_at: Some("2026-11-16T08:00:00Z".to_string()),
        }
    }

    /// A missing file is the normal empty state: `load` returns defaults,
    /// never fails and never creates anything on disk.
    #[test]
    fn missing_state_file_loads_empty_without_side_effects() {
        let dir = temp_state_dir("missing");
        let store = UpdateStateStore::with_state_dir(dir.join("updates"));

        assert_eq!(store.load(), UpdateStateRecord::default());
        assert!(!dir.join("updates").exists(), "load must not create anything");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A broken (unparsable) or unreadable file must not break anything:
    /// `load` warns and yields the empty state, and the next successful
    /// `update` replaces the damaged file.
    #[test]
    fn broken_state_file_is_safe_and_heals_on_next_write() {
        let dir = temp_state_dir("broken");
        let state_dir = dir.join("updates");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join(STATE_FILE_NAME), "{ not json !!!").unwrap();
        let store = UpdateStateStore::with_state_dir(state_dir.clone());

        assert_eq!(store.load(), UpdateStateRecord::default());

        // The heal: a successful transaction overwrites the garbage.
        store
            .update(|state| {
                state.last_release = Some(sample_release("1.9.0"));
                state.last_check_at = Some("2026-11-18T10:00:00Z".to_string());
            })
            .unwrap();
        let healed = store.load();
        assert_eq!(
            healed.last_release.as_ref().map(|r| r.version.as_str()),
            Some("1.9.0")
        );
        assert_eq!(healed.last_check_at.as_deref(), Some("2026-11-18T10:00:00Z"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fresh store instance (a restarted process) reads what a previous
    /// instance persisted: the saved result and the attempt time survive.
    #[test]
    fn new_store_instance_reads_saved_result_and_attempt_time() {
        let dir = temp_state_dir("restart");
        let state_dir = dir.join("updates");
        let first = UpdateStateStore::with_state_dir(state_dir.clone());
        first
            .update(|state| {
                state.last_release = Some(sample_release("1.9.0"));
                state.last_check_at = Some("2026-11-18T09:00:00Z".to_string());
                state.last_attempt_at = Some("2026-11-18T12:30:00Z".to_string());
                state.last_attempt_error = Some("network".to_string());
                state.dismissed_version = Some("1.9.0".to_string());
            })
            .unwrap();

        let reloaded = UpdateStateStore::with_state_dir(state_dir);
        let record = reloaded.load();
        assert_eq!(
            record.last_release.as_ref().map(|r| r.version.as_str()),
            Some("1.9.0")
        );
        assert_eq!(record.last_check_at.as_deref(), Some("2026-11-18T09:00:00Z"));
        assert_eq!(record.last_attempt_at.as_deref(), Some("2026-11-18T12:30:00Z"));
        assert_eq!(record.last_attempt_error.as_deref(), Some("network"));
        assert_eq!(record.dismissed_version.as_deref(), Some("1.9.0"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The paired read resolves the directory once and reads the record from
    /// exactly that directory: the pair is always one root's (directory,
    /// JSON), never a mix of two roots. This is the seam the status snapshot
    /// and the scheduler depend on after the resolve-then-read split let a
    /// mid-run «Данные программы» switch pair one root's directory with the
    /// other root's record.
    #[test]
    fn paired_load_returns_dir_and_record_of_one_root() {
        let dir = temp_state_dir("paired");
        let a = dir.join("root-a").join(STATE_DIR_NAME);
        let b = dir.join("root-b").join(STATE_DIR_NAME);
        for (state_dir, version) in [(&a, "1.8.0"), (&b, "1.9.0")] {
            std::fs::create_dir_all(state_dir).unwrap();
            let record = UpdateStateRecord {
                last_release: Some(sample_release(version)),
                ..Default::default()
            };
            std::fs::write(
                state_dir.join(STATE_FILE_NAME),
                serde_json::to_string(&record).unwrap(),
            )
            .unwrap();
        }
        let store = UpdateStateStore::with_state_dir(a.clone());

        let (resolved, record) = store.load_with_dir();
        assert_eq!(resolved.as_deref(), Some(a.as_path()));
        assert_eq!(
            record.last_release.as_ref().map(|r| r.version.as_str()),
            Some("1.8.0")
        );

        // The mid-run root switch (every manager clone shares one store):
        // the pair follows the new root as one unit, and `load` (the plain
        // consumers) reads the same record.
        store.set_dir(Some(b.clone()));
        let (resolved, record) = store.load_with_dir();
        assert_eq!(resolved.as_deref(), Some(b.as_path()));
        assert_eq!(
            record.last_release.as_ref().map(|r| r.version.as_str()),
            Some("1.9.0")
        );
        assert_eq!(store.load(), record);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A write failure surfaces as `Err` and leaves the previously stored
    /// record untouched (the state directory is a file, so no write can
    /// succeed and no state file appears).
    #[test]
    fn write_failure_surfaces_error_and_creates_nothing() {
        let dir = temp_state_dir("write-fail");
        // A file where the state directory should be breaks every write.
        std::fs::write(dir.join(STATE_DIR_NAME), b"not a directory").unwrap();
        let store = UpdateStateStore::with_state_dir(dir.join(STATE_DIR_NAME));

        // Reads stay safe (missing state file inside an unusable directory).
        assert_eq!(store.load(), UpdateStateRecord::default());
        assert!(
            store.update(|state| state.last_attempt_at = Some("x".to_string())).is_err(),
            "a write into an unusable directory must fail"
        );
        assert!(!dir.join(STATE_DIR_NAME).join(STATE_FILE_NAME).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Concurrent transactions on one shared file serialize: after parallel
    /// writers the file parses and carries one of the written values — no
    /// torn writes, no lost fields from interleaved read-modify-write.
    #[test]
    fn concurrent_updates_serialize_into_a_consistent_file() {
        let dir = temp_state_dir("concurrent");
        let state_dir = dir.join("updates");
        std::fs::create_dir_all(&state_dir).unwrap();
        let store = UpdateStateStore::with_state_dir(state_dir.clone());
        store
            .update(|state| {
                state.last_release = Some(sample_release("1.8.0"));
                state.dismissed_version = Some("1.8.0".to_string());
            })
            .unwrap();

        const THREADS: usize = 6;
        const ROUNDS: usize = 25;
        let mut handles = Vec::new();
        for thread in 0..THREADS {
            let store = UpdateStateStore::with_state_dir(state_dir.clone());
            handles.push(std::thread::spawn(move || {
                for round in 0..ROUNDS {
                    store
                        .update(|state| {
                            state.last_attempt_at =
                                Some(format!("2026-11-18T{thread:02}:{round:02}:00Z"));
                            state.last_attempt_error = Some("network".to_string());
                        })
                        .expect("every concurrent update must succeed");
                }
            }));
        }
        for handle in handles {
            handle.join().expect("writer thread must not panic");
        }

        let record = store.load();
        assert!(
            record.last_attempt_at.is_some(),
            "the final record carries one of the written attempt times"
        );
        assert_eq!(record.last_attempt_error.as_deref(), Some("network"));
        // Fields untouched by the concurrent writers survive every write.
        assert_eq!(
            record.last_release.as_ref().map(|r| r.version.as_str()),
            Some("1.8.0")
        );
        assert_eq!(record.dismissed_version.as_deref(), Some("1.8.0"));
        // The file on disk is valid JSON matching the final in-memory read.
        let raw = std::fs::read_to_string(state_dir.join(STATE_FILE_NAME)).unwrap();
        let on_disk: UpdateStateRecord = serde_json::from_str(&raw).unwrap();
        assert_eq!(on_disk, record);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
