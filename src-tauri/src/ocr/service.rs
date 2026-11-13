//! OCR service: desired settings, runtime lifecycle status, and conditional
//! global hotkey registration.
//!
//! Mirrors the Input Server feature seam: persisted `OcrSettings` express only
//! desired state, while the actual runtime status (`OcrStatus`) is tracked
//! here and published to the frontend through a pluggable emit seam.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use image::{RgbImage, RgbaImage};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::capture::{
    clamp_selection, crop_to_rgb, normalize_selection, validate_selection, CaptureError,
    VirtualScreenGeometry,
};
use super::packs::{scan_ocr_packs_across_roots, OcrPackDescriptor};
use super::runtime::{OcrResult, OcrRuntime};
use super::settings::OcrSettings;
use crate::config::Hotkey;

/// Runtime lifecycle status of the OCR runtime.
///
/// Uses the same externally tagged camelCase wire shape as
/// [`crate::input_server::InputServerStatus`]:
/// `{"state": "disabled" | "starting" | "ready" | "error", "message"?: ...}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum OcrStatus {
    Disabled,
    /// Published before scanning/loading; replaced with `Ready` only after the
    /// runtime is built and the capture shortcut is registered.
    Starting,
    Ready,
    /// The one-shot capture session is open and the selection overlay is
    /// visible; no recognition runs until a valid selection is submitted.
    SelectingArea,
    /// The selection was submitted and recognition is running on the stored
    /// frame; the overlay is already hidden.
    Recognizing,
    Error {
        message: String,
    },
}

/// Lifecycle delta to apply after an OCR settings save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrTransition {
    Noop,
    Start,
    Stop,
    /// Model changed while already enabled: shut down and start again.
    Restart,
    /// Capture binding changed while `Ready`: re-register with rollback.
    ReregisterHotkey,
    /// Unchanged settings while the runtime is in `Error`: retry the start.
    Retry,
}

/// Decide the lifecycle delta for an OCR settings save.
///
/// Pure and free of any shortcut-plugin or ONNX dependency. `old_hotkey` is the
/// binding the service currently has registered (if any); `new_hotkey` is the
/// binding from the current hotkey settings.
pub fn decide_transition(
    old: &OcrSettings,
    new: &OcrSettings,
    old_hotkey: Option<&Hotkey>,
    new_hotkey: Option<&Hotkey>,
    status: &OcrStatus,
) -> OcrTransition {
    match (old.enabled, new.enabled) {
        (false, true) => OcrTransition::Start,
        (true, false) => OcrTransition::Stop,
        (true, true) => {
            if old.model_id != new.model_id {
                OcrTransition::Restart
            } else if matches!(status, OcrStatus::Ready) && old_hotkey != new_hotkey {
                OcrTransition::ReregisterHotkey
            } else if matches!(status, OcrStatus::Error { .. }) {
                OcrTransition::Retry
            } else {
                OcrTransition::Noop
            }
        }
        (false, false) => OcrTransition::Noop,
    }
}

/// Reconcile decision for an OCR pack refresh (`refresh_ocr_packs`).
///
/// Pure and free of any shortcut-plugin or ONNX dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrRefreshReconcile {
    /// Nothing to change: the selection is still valid, or the runtime holds the
    /// removed model in memory.
    None,
    /// The selected model is confirmed gone everywhere: disable the feature,
    /// auto-selecting the single remaining model when exactly one is left and
    /// retaining the missing id when none remain.
    Disable { model_id: Option<String> },
    /// There was no (or a dangling) selection and exactly one pack exists:
    /// select it without enabling.
    AutoSelect(String),
}

/// Decide the reconcile action for an OCR pack refresh.
///
/// Rules (order matters):
/// - `enabled` with a saved id that is no longer in `pack_ids` and not held in
///   memory → `Disable` (auto-select the single remaining pack, retain the
///   missing id when no packs remain, otherwise `None`);
/// - disabled with exactly one pack whose id differs from the saved one →
///   `AutoSelect`;
/// - otherwise `None` (a removed model still held in memory is left untouched).
pub fn decide_ocr_refresh_reconcile(
    enabled: bool,
    saved_id: Option<&str>,
    pack_ids: &[String],
    holds_in_memory: bool,
) -> OcrRefreshReconcile {
    if enabled
        && saved_id.is_some()
        && !pack_ids.iter().any(|id| Some(id.as_str()) == saved_id)
        && !holds_in_memory
    {
        return OcrRefreshReconcile::Disable {
            model_id: match pack_ids.len() {
                0 => saved_id.map(str::to_string),
                1 => Some(pack_ids[0].clone()),
                _ => None,
            },
        };
    }

    if !enabled && pack_ids.len() == 1 && saved_id != Some(pack_ids[0].as_str()) {
        return OcrRefreshReconcile::AutoSelect(pack_ids[0].clone());
    }

    OcrRefreshReconcile::None
}

/// Result of [`OcrService::begin_capture_session`].
#[derive(Debug)]
pub enum BeginOutcome {
    /// The frame is saved, `SelectingArea` is published and the show-overlay
    /// callback has already run. Carries the identity of the freshly created
    /// session so the caller can arm its guards (preview timeout, ready,
    /// submit) without another `selection_snapshot` round trip.
    Started { session_id: String },
    /// The status was not `Ready`; nothing was captured or shown.
    Ignored,
    /// The injected capture failed; no overlay was shown and status stays
    /// `Ready`.
    CaptureFailed(CaptureError),
    /// The frame was saved and `SelectingArea` was published, but the
    /// show-overlay callback reported a missing/broken overlay. The session is
    /// cleared and status is back to `Ready`.
    OverlayFailed(String),
}

/// Result of submitting a selection against the in-flight capture session.
#[derive(Debug, Clone, PartialEq)]
pub enum FinishOutcome {
    /// The selection was empty or smaller than [`capture::MIN_SELECTION_SIDE`].
    /// The session is kept and the status stays `SelectingArea` so the user can
    /// drag again.
    TooSmall,
    /// Recognition finished with the given result; status is back to `Ready`.
    Recognized(OcrResult),
    /// Recognition failed with a safe reason; status is back to `Ready`.
    RecognizeFailed(String),
    /// No active capture session; a no-op.
    NoSession,
}

/// A single in-flight one-shot capture.
///
/// Owns the virtual-screen geometry and the full frame saved BEFORE the
/// selection overlay was shown, so cropping never contains overlay pixels.
/// A random [`Uuid`] identity lets the command layer route late overlay events
/// (present/cancel/submit) to the exact session that produced them, and
/// `presented` records whether the stored frame has been revealed. A session
/// starts unpresented: it is stored at capture time, revealed later by
/// [`OcrService::present_selection`] only once the overlay is ready for it.
struct CaptureSession {
    id: String,
    /// Capture epoch this session belongs to (see
    /// [`OcrService::session_generation`]). A finished recognition publishes
    /// `Ready` only while this value is still current.
    generation: u64,
    geometry: VirtualScreenGeometry,
    frame: Arc<RgbaImage>,
    presented: bool,
}

/// RAII marker for an in-flight recognition (frame crop + runtime call).
///
/// [`OcrService::finish_session_locked`] releases the transition lock before that
/// phase, so a shutdown stop can no longer notice it through a failed
/// `try_lock`; [`OcrService::stop_for_shutdown`] reads this counter instead of
/// blocking app exit for up to the recognition timeout.
struct RecognitionInFlight<'a>(&'a AtomicUsize);

impl<'a> RecognitionInFlight<'a> {
    fn new(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for RecognitionInFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Read-only snapshot of the in-flight `SelectingArea` session for the overlay.
///
/// Clone shares the pixel buffer by [`Arc`]; no pixels are copied, so the
/// overlay render and the later crop read the exact same saved frame. Geometry
/// is deliberately not exposed here: the overlay is positioned by the backend
/// through the guarded `present_selection` callback, never by the frontend.
#[derive(Debug, Clone)]
pub struct SelectionSnapshot {
    /// Session identity, used to guard overlay events against staleness.
    pub id: String,
    pub frame: Arc<RgbaImage>,
}

/// Two overlay-local physical corners of a dragged selection.
///
/// The corners are reported by the selection overlay in physical pixels local
/// to its own client area (whose top-left sits on the stored geometry origin)
/// and may arrive in any drag order. They travel whole through the submit seam
/// as one small [`Copy`] value and are shifted into virtual-screen coordinates
/// only inside `OcrService::finish_session_locked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SelectionCorners {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

/// Map overlay-local physical corners into virtual-screen coordinates.
///
/// The selection overlay sits with its client-area origin on the stored
/// geometry origin, so a local corner `(lx, ly)` denotes the virtual-screen
/// pixel `(origin.x + lx, origin.y + ly)`. Addition is checked: a corner that
/// would overflow `i32` yields `None` so the caller can keep the retryable
/// selection instead of panicking.
fn translate_by_session_origin(
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    origin: (i32, i32),
) -> Option<(i32, i32, i32, i32)> {
    let (ox, oy) = origin;
    Some((
        x1.checked_add(ox)?,
        y1.checked_add(oy)?,
        x2.checked_add(ox)?,
        y2.checked_add(oy)?,
    ))
}

/// Backend-owned domain state for the one-shot screen OCR feature.
///
/// Owns the desired settings snapshot, the runtime lifecycle status, the
/// resolved `OcrRuntime` (if `Ready`) and the currently registered capture
/// shortcut. The search roots and the managed packs root are injectable so
/// tests can point at temp dirs.
pub struct OcrService {
    /// Desired settings snapshot, updated by the save command.
    pub settings: Arc<tokio::sync::RwLock<OcrSettings>>,
    status: Arc<Mutex<OcrStatus>>,
    /// Ordered pack search roots (ROADMAP-117): the process cwd first, then
    /// the effective data root. Discovery merges valid packs by ID across
    /// roots; a missing root is normal and stays read-only.
    search_roots: Vec<PathBuf>,
    /// The managed packs root — the effective data root («Данные программы»).
    /// The target of `open_ocr_packs_folder` and the only root where
    /// directories are ever created; deliberately not the priority cwd.
    packs_root: PathBuf,
    runtime: tokio::sync::Mutex<Option<OcrRuntime>>,
    registered_hotkey: Mutex<Option<Hotkey>>,
    /// True while an OCR runtime is installed, even if the capture shortcut was
    /// released because the per-hotkey switch is off. Kept separate from
    /// `registered_hotkey` so OCR stays logically enabled without its shortcut.
    runtime_active: Mutex<bool>,
    /// The single in-flight capture session (`SelectingArea`/`Recognizing`), if
    /// any. Async mutex so the frame is never read while another task mutates
    /// the session; guards are never held across the status/settings locks.
    session: tokio::sync::Mutex<Option<CaptureSession>>,
    /// Serialises transitions so only one start/stop runs at a time. Held across
    /// the async ONNX load; a concurrent stop waits instead of deadlocking.
    /// Deliberately NOT held across the crop/recognize phase of a submitted
    /// selection (see [`Self::finish_session_locked`]), which can take up to the
    /// recognition timeout.
    transition_lock: tokio::sync::Mutex<()>,
    /// Monotonic capture epoch. Bumped when a session begins and when the
    /// runtime is stopped, so a recognition that finishes late can never publish
    /// `Ready` over a newer session or a disabled runtime.
    session_generation: Mutex<u64>,
    /// Number of recognitions currently in flight (see [`RecognitionInFlight`]).
    recognition_in_flight: AtomicUsize,
    /// Serialises the settings-writing commands (`save_ocr_settings` and
    /// `refresh_ocr_packs`): both read the persisted OCR section, persist a
    /// reconcile result and drive a lifecycle transition, so their
    /// read-modify-write sequences must not interleave.
    settings_write_lock: tokio::sync::Mutex<()>,
}

impl OcrService {
    /// Production constructor: the shared model search roots (cwd → data root)
    /// for discovery and the effective data root for the managed packs folder.
    pub fn new() -> Self {
        Self::with_search_roots(
            crate::paths::model_search_root_paths(),
            crate::paths::data_root().unwrap_or_default(),
        )
    }

    /// Single-root constructor kept for existing tests: the injected root
    /// serves both discovery and the managed packs root.
    #[cfg(test)]
    pub fn with_packs_root(packs_root: PathBuf) -> Self {
        Self::with_search_roots(vec![packs_root.clone()], packs_root)
    }

    /// Injectable constructor: ordered discovery roots plus the managed packs
    /// root. Resolves nothing on disk.
    pub fn with_search_roots(search_roots: Vec<PathBuf>, packs_root: PathBuf) -> Self {
        Self {
            settings: Arc::new(tokio::sync::RwLock::new(OcrSettings::default())),
            status: Arc::new(Mutex::new(OcrStatus::Disabled)),
            search_roots,
            packs_root,
            runtime: tokio::sync::Mutex::new(None),
            registered_hotkey: Mutex::new(None),
            runtime_active: Mutex::new(false),
            session: tokio::sync::Mutex::new(None),
            transition_lock: tokio::sync::Mutex::new(()),
            session_generation: Mutex::new(0),
            recognition_in_flight: AtomicUsize::new(0),
            settings_write_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// The managed packs root («Данные программы») — the folder opened by
    /// `open_ocr_packs_folder`, not the higher-priority cwd root.
    pub fn packs_root(&self) -> &Path {
        &self.packs_root
    }

    /// Merged pack discovery across the ordered search roots. The runtime
    /// start, the pack list command and refresh/reconcile all use this single
    /// source, so a selected ID always resolves to the descriptor the user
    /// saw, including a model that only exists in the cwd.
    pub fn discover_packs(&self) -> Vec<OcrPackDescriptor> {
        scan_ocr_packs_across_roots(&self.search_roots)
    }

    /// [`Self::discover_packs`] on the blocking pool.
    ///
    /// Scanning the pack roots is blocking file-system I/O, so the async paths
    /// (`start`, `refresh_ocr_packs`) must not run it on a Tokio worker. A
    /// panicking scan worker yields an empty list: callers then behave exactly
    /// like "no packs found" — an `Error` status for `start`, an empty list for
    /// a refresh — instead of surfacing a panic payload.
    pub async fn discover_packs_blocking(&self) -> Vec<OcrPackDescriptor> {
        let roots = self.search_roots.clone();
        match tokio::task::spawn_blocking(move || scan_ocr_packs_across_roots(&roots)).await {
            Ok(packs) => packs,
            Err(join_error) => {
                tracing::warn!(error = %join_error, "OCR pack discovery worker panicked");
                Vec::new()
            }
        }
    }

    /// Serialisation seam for the OCR settings-writing commands.
    ///
    /// `save_ocr_settings` and `refresh_ocr_packs` both read the persisted OCR
    /// section, persist a reconcile result and drive a lifecycle transition;
    /// holding this lock across that whole sequence keeps those
    /// read-modify-write pairs from interleaving and makes an
    /// [`OcrTransition::Restart`] stop+start atomic with respect to the other
    /// command.
    pub fn settings_write_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.settings_write_lock
    }

    /// Advance the capture epoch, invalidating every in-flight finish.
    fn bump_session_generation(&self) -> u64 {
        let mut generation = self.session_generation.lock();
        *generation = generation.wrapping_add(1);
        *generation
    }

    /// Publish `Ready` after a finished recognition only when `generation` still
    /// identifies the current capture epoch and the runtime is still installed.
    ///
    /// The epoch guard stays held across the status write: a concurrent stop
    /// bumps the epoch before publishing `Disabled`, so the check and the
    /// `Ready` write are ordered against that stop.
    fn publish_ready_if_generation_current<E>(&self, generation: u64, mut emit: E)
    where
        E: FnMut(&OcrStatus),
    {
        let current = self.session_generation.lock();
        if *current != generation || !self.is_runtime_active() {
            return;
        }
        self.publish_status(OcrStatus::Ready, &mut emit);
    }

    pub fn status(&self) -> OcrStatus {
        self.status.lock().clone()
    }

    /// Replace the runtime status, returning whether it actually changed.
    pub fn set_status(&self, status: OcrStatus) -> bool {
        let mut guard = self.status.lock();
        if *guard == status {
            return false;
        }
        *guard = status;
        true
    }

    /// Set the runtime status and invoke `emit` with the new value only when it
    /// actually changed.
    ///
    /// The emitter is a plain callback so this helper stays testable without a
    /// live Tauri app; the command layer forwards it to
    /// `app_handle.emit("ocr-status-changed", status)`.
    pub fn publish_status<E>(&self, status: OcrStatus, emit: E)
    where
        E: FnOnce(&OcrStatus),
    {
        if self.set_status(status.clone()) {
            emit(&status);
        }
    }

    /// Snapshot of the capture binding currently registered (if any).
    pub fn registered_hotkey(&self) -> Option<Hotkey> {
        self.registered_hotkey.lock().clone()
    }

    /// True while the OCR runtime is live, regardless of the current one-shot
    /// session phase (`Ready`, `SelectingArea` or `Recognizing`) and regardless
    /// of whether the capture shortcut is currently held.
    ///
    /// The shortcut may be deliberately released when the per-hotkey `enabled`
    /// switch is off; OCR itself stays enabled then, so this must not be derived
    /// from `registered_hotkey` alone. The fallback keeps hand-built test
    /// services (status/registration only) behaving as active.
    pub fn is_runtime_active(&self) -> bool {
        *self.runtime_active.lock() || self.registered_hotkey.lock().is_some()
    }

    /// The runtime storage slot.
    ///
    /// Crate-private so the OCR command layer can run recognition against the
    /// stored runtime without going through Tauri. The guard must never be held
    /// across the settings/status locks or another async mutex.
    pub(crate) fn runtime_slot(&self) -> &tokio::sync::Mutex<Option<OcrRuntime>> {
        &self.runtime
    }

    /// Start the OCR runtime: scan packs, resolve the selected model, build the
    /// runtime and register the capture shortcut. Any failure lands in a
    /// terminal `Error` status.
    pub async fn start<E, R>(&self, hotkey: &Hotkey, mut emit: E, register: R)
    where
        E: FnMut(&OcrStatus),
        R: FnOnce(&Hotkey) -> Result<(), String>,
    {
        let _guard = self.transition_lock.lock().await;

        // A concurrent or already-complete start is a no-op.
        if matches!(self.status(), OcrStatus::Ready | OcrStatus::Starting) {
            return;
        }

        self.publish_status(OcrStatus::Starting, &mut emit);

        let model_id = self.settings.read().await.model_id.clone();

        let Some(model_id) = model_id else {
            self.publish_status(
                OcrStatus::Error {
                    message: "No OCR model selected".to_string(),
                },
                &mut emit,
            );
            return;
        };

        let packs = self.discover_packs_blocking().await;
        let Some(descriptor) = packs.into_iter().find(|p| p.id == model_id) else {
            self.publish_status(
                OcrStatus::Error {
                    message: format!("OCR model pack not found: {model_id}"),
                },
                &mut emit,
            );
            return;
        };

        let runtime = match OcrRuntime::new(&descriptor).await {
            Ok(runtime) => runtime,
            Err(error) => {
                self.publish_status(
                    OcrStatus::Error {
                        message: error.to_string(),
                    },
                    &mut emit,
                );
                return;
            }
        };

        if let Err(message) = register(hotkey) {
            // A failed registration must not leak a live runtime.
            let _ = runtime.shutdown().await;
            self.publish_status(OcrStatus::Error { message }, &mut emit);
            return;
        }

        *self.runtime.lock().await = Some(runtime);
        *self.registered_hotkey.lock() = Some(hotkey.clone());
        *self.runtime_active.lock() = true;

        self.publish_status(OcrStatus::Ready, &mut emit);
    }

    /// Stop the OCR runtime: release the shortcut, shut the runtime down, drop
    /// any in-flight capture session, then report `Disabled`. A failed shutdown
    /// still lands in `Disabled` but is logged.
    pub async fn stop<E, U>(&self, emit: E, unregister: U)
    where
        E: FnMut(&OcrStatus),
        U: Fn(&Hotkey) -> Result<(), String>,
    {
        let _guard = self.transition_lock.lock().await;
        self.stop_locked(emit, unregister).await;
    }

    /// Stop the runtime for app shutdown without blocking on an in-flight
    /// recognition.
    ///
    /// A recognition (frame crop + ONNX call, up to 15 s) runs with the
    /// transition lock released but registers itself in
    /// [`Self::recognition_in_flight`]; when either a transition or a recognition
    /// is in flight the stop is detached to a background task and this returns
    /// immediately, so app exit is not delayed by OCR — the process exit reclaims
    /// the runtime thread. Otherwise the stop runs inline under the transition
    /// lock.
    pub async fn stop_for_shutdown<E, U>(self: Arc<Self>, emit: E, unregister: U)
    where
        E: FnMut(&OcrStatus) + Send + 'static,
        U: Fn(&Hotkey) -> Result<(), String> + Send + 'static,
    {
        match self.transition_lock.try_lock() {
            Ok(_guard) if self.recognition_in_flight.load(Ordering::SeqCst) == 0 => {
                self.stop_locked(emit, unregister).await;
            }
            // Either another transition owns the lock or a recognition is
            // awaiting the runtime slot: both would make an inline stop wait.
            _ => self.spawn_stop(emit, unregister),
        }
    }

    /// Run a deferred [`Self::stop`] on the app runtime.
    fn spawn_stop<E, U>(self: &Arc<Self>, emit: E, unregister: U)
    where
        E: FnMut(&OcrStatus) + Send + 'static,
        U: Fn(&Hotkey) -> Result<(), String> + Send + 'static,
    {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            this.stop(emit, unregister).await;
        });
    }

    async fn stop_locked<E, U>(&self, mut emit: E, unregister: U)
    where
        E: FnMut(&OcrStatus),
        U: Fn(&Hotkey) -> Result<(), String>,
    {
        let runtime = self.runtime.lock().await.take();
        let hotkey = self.registered_hotkey.lock().take();
        *self.runtime_active.lock() = false;
        // Drop any in-flight capture session so a stale overlay submit/cancel
        // can never resurrect `Ready` from the disabled state.
        self.session.lock().await.take();

        if let Some(hotkey) = &hotkey {
            if let Err(error) = unregister(hotkey) {
                tracing::warn!(error = %error, "Failed to unregister OCR hotkey during stop");
            }
        }

        if let Some(runtime) = runtime {
            if let Err(error) = runtime.shutdown().await {
                tracing::warn!(error = %error, "OCR runtime shutdown failed");
            }
        }

        // Invalidate any recognition still in flight: its guarded
        // `Recognizing -> Ready` transition must not resurrect `Ready` from the
        // disabled state.
        self.bump_session_generation();
        self.publish_status(OcrStatus::Disabled, &mut emit);
    }

    /// Re-register the capture shortcut for a changed binding while the OCR
    /// feature is enabled (`Ready`, `SelectingArea` or `Recognizing`).
    ///
    /// The new binding is registered first and the old one released only after
    /// that succeeds. On registration failure the old binding stays active and
    /// `Error` is surfaced, keeping the persisted setting and the runtime in
    /// agreement (roadmap «Настройки и runtime truth»).
    pub async fn reregister_hotkey<E, R, U>(
        &self,
        new_hotkey: &Hotkey,
        mut emit: E,
        register: R,
        unregister: U,
    ) where
        E: FnMut(&OcrStatus),
        R: FnOnce(&Hotkey) -> Result<(), String>,
        U: Fn(&Hotkey) -> Result<(), String>,
    {
        let _guard = self.transition_lock.lock().await;

        // A rebind is valid while the feature is enabled, including during an
        // active one-shot session (`SelectingArea`/`Recognizing`): the new
        // binding is registered and the old released without disturbing the
        // stored frame. Any other status (disabled, starting, error) is a no-op.
        match self.status() {
            OcrStatus::Ready | OcrStatus::SelectingArea | OcrStatus::Recognizing => {}
            _ => return,
        }

        let old_hotkey = self.registered_hotkey.lock().clone();
        if old_hotkey.as_ref() == Some(new_hotkey) {
            return;
        }

        if let Err(message) = register(new_hotkey) {
            self.publish_status(OcrStatus::Error { message }, &mut emit);
            return;
        }

        if let Some(old) = &old_hotkey {
            if let Err(error) = unregister(old) {
                tracing::warn!(
                    error = %error,
                    "Failed to unregister old OCR hotkey after rebind"
                );
            }
        }

        *self.registered_hotkey.lock() = Some(new_hotkey.clone());
    }

    /// Release the capture shortcut while keeping the OCR runtime enabled.
    ///
    /// Used when the per-hotkey `enabled` switch is turned off: OCR stays
    /// enabled (and its status is left unchanged), but Windows must stop
    /// intercepting the combination immediately. Clearing `registered_hotkey`
    /// is not enough — the injected `unregister` releases the OS registration
    /// owned by [`crate::ocr::OcrService`]. Errors are logged, never surfaced as
    /// an OCR runtime `Error`: this is an expected part of a normal disable.
    pub async fn unregister_hotkey<U>(&self, mut unregister: U)
    where
        U: FnMut(&Hotkey) -> Result<(), String>,
    {
        let _guard = self.transition_lock.lock().await;
        let hotkey = self.registered_hotkey.lock().take();
        if let Some(hotkey) = &hotkey {
            if let Err(error) = unregister(hotkey) {
                tracing::warn!(
                    error = %error,
                    "Failed to release OCR hotkey after it was disabled"
                );
            }
        }
    }

    /// Register the capture shortcut again while the OCR runtime is active.
    ///
    /// Used when the per-hotkey `enabled` switch is turned back on without
    /// restarting OCR. A disabled/empty binding or an inactive runtime is a
    /// no-op. On registration failure the old binding (if any) stays untouched
    /// and the error is returned so the command can keep the switch off.
    pub async fn register_hotkey<R, U>(
        &self,
        new_hotkey: &Hotkey,
        register: R,
        unregister: U,
    ) -> Result<(), String>
    where
        R: FnOnce(&Hotkey) -> Result<(), String>,
        U: Fn(&Hotkey) -> Result<(), String>,
    {
        if !new_hotkey.enabled || new_hotkey.key.is_empty() {
            return Ok(());
        }

        let _guard = self.transition_lock.lock().await;

        if !self.is_runtime_active() {
            return Ok(());
        }
        match self.status() {
            OcrStatus::Ready | OcrStatus::SelectingArea | OcrStatus::Recognizing => {}
            _ => return Ok(()),
        }

        let old_hotkey = self.registered_hotkey.lock().clone();
        if old_hotkey.as_ref() == Some(new_hotkey) {
            return Ok(());
        }

        register(new_hotkey)?;

        if let Some(old) = &old_hotkey {
            if let Err(error) = unregister(old) {
                tracing::warn!(
                    error = %error,
                    "Failed to unregister old OCR hotkey after re-enable"
                );
            }
        }

        *self.registered_hotkey.lock() = Some(new_hotkey.clone());
        Ok(())
    }

    /// Begin the one-shot capture session from `Ready`.
    ///
    /// The transition lock is the serialization boundary: exactly one session
    /// can exist, and it can only start from `Ready` (a re-entry while
    /// `SelectingArea`/`Recognizing` — or any other non-`Ready` status — is
    /// ignored). The frame is captured THROUGH the injected closure on Tokio's
    /// blocking pool — the production closure blocks on `xcap` and must never
    /// run on a Tokio worker — and stored BEFORE the show-overlay callback
    /// runs, so the overlay never appears in OCR input. A panicking capture
    /// closure surfaces as a safe `CaptureFailed`, never a panic payload. All
    /// work stays Tauri-free; the command layer supplies the capture and window
    /// plumbing.
    ///
    /// The show-overlay callback receives the captured geometry (physical
    /// origin/size in virtual-screen coordinates) and returns
    /// `Result<(), String>`. A failed show is not a runtime failure: the stored
    /// session is dropped, `Ready` is published and `OverlayFailed` is
    /// returned so the runtime can start a fresh session.
    pub async fn begin_capture_session<E, C, S>(
        &self,
        mut emit: E,
        capture: C,
        show_overlay: S,
    ) -> BeginOutcome
    where
        E: FnMut(&OcrStatus),
        C: FnOnce() -> Result<(VirtualScreenGeometry, RgbaImage), CaptureError> + Send + 'static,
        S: FnOnce(&VirtualScreenGeometry) -> Result<(), String>,
    {
        let _guard = self.transition_lock.lock().await;

        if self.status() != OcrStatus::Ready {
            return BeginOutcome::Ignored;
        }

        // Capture the full virtual desktop BEFORE any overlay UI is shown. The
        // blocking capture must not occupy a Tokio worker, so it runs on the
        // blocking pool instead.
        let captured = tokio::task::spawn_blocking(capture).await;
        let (geometry, frame) = match captured {
            Ok(Ok(captured)) => captured,
            Ok(Err(error)) => return BeginOutcome::CaptureFailed(error),
            Err(join_error) => {
                // Never surface panic payloads or JoinError text to the
                // frontend; log the detail for developers only.
                tracing::warn!(error = %join_error, "Capture worker panicked");
                return BeginOutcome::CaptureFailed(CaptureError::CaptureFailed(
                    "capture worker failed".to_string(),
                ));
            }
        };

        let session = CaptureSession {
            id: Uuid::new_v4().to_string(),
            generation: self.bump_session_generation(),
            geometry: geometry.clone(),
            frame: Arc::new(frame),
            presented: false,
        };
        let session_id = session.id.clone();
        *self.session.lock().await = Some(session);

        self.publish_status(OcrStatus::SelectingArea, &mut emit);

        // Only now is it safe to reveal the overlay over the saved frame. A
        // missing or broken overlay is not a runtime failure: drop the session
        // and return to `Ready` so the runtime can start again.
        if let Err(reason) = show_overlay(&geometry) {
            self.session.lock().await.take();
            self.publish_status(OcrStatus::Ready, &mut emit);
            return BeginOutcome::OverlayFailed(reason);
        }

        BeginOutcome::Started { session_id }
    }

    /// Snapshot of the current `SelectingArea` session for the overlay to
    /// render, serialized by the transition lock.
    ///
    /// Returns `None` when no session is selecting. The snapshot clones only
    /// the [`Arc`] of the stored frame — never its pixels — so the overlay
    /// render and the later crop read the exact same saved buffer.
    pub async fn selection_snapshot(&self) -> Option<SelectionSnapshot> {
        let _guard = self.transition_lock.lock().await;

        if self.status() != OcrStatus::SelectingArea {
            return None;
        }

        let session = self.session.lock().await;
        session.as_ref().map(|session| SelectionSnapshot {
            id: session.id.clone(),
            frame: Arc::clone(&session.frame),
        })
    }

    /// Reveal the overlay over the stored frame of a specific session.
    ///
    /// The transition lock is acquired and the current session must exist and
    /// match `id` while the status is `SelectingArea`. A stale or missing
    /// session returns `Ok(false)` without running `show` (or any other
    /// callback). The `show` closure is invoked only after the session mutex is
    /// released (the transition lock stays held) and receives the session
    /// geometry for window placement; a successful reveal marks the session
    /// `presented`. A failed reveal drops the matching session, restores
    /// `Ready` while the runtime is active and returns the error. Re-showing an
    /// already-presented matching session is allowed so a too-small retry can
    /// reveal the same frame again.
    pub async fn present_selection<E, S>(
        &self,
        id: &str,
        mut emit: E,
        show: S,
    ) -> Result<bool, String>
    where
        E: FnMut(&OcrStatus),
        S: FnOnce(&VirtualScreenGeometry) -> Result<(), String>,
    {
        let _guard = self.transition_lock.lock().await;

        if self.status() != OcrStatus::SelectingArea {
            return Ok(false);
        }

        let geometry = {
            let session = self.session.lock().await;
            let Some(session) = session.as_ref() else {
                return Ok(false);
            };
            if session.id != id {
                return Ok(false);
            }
            session.geometry.clone()
        };

        if let Err(reason) = show(&geometry) {
            // Reveal failed: drop the matching session and return to `Ready` so
            // the runtime can start a fresh one.
            self.session.lock().await.take();
            if self.is_runtime_active() {
                self.publish_status(OcrStatus::Ready, &mut emit);
            }
            return Err(reason);
        }

        // The transition lock keeps the session identity stable between the
        // check above and this update, so re-locking finds the same session.
        let mut session = self.session.lock().await;
        if let Some(current) = session.as_mut() {
            if current.id == id {
                current.presented = true;
            }
        }
        Ok(true)
    }

    /// Submit a selection rectangle for the in-flight session.
    ///
    /// Test-only legacy unguarded entry point, compiled under `#[cfg(test)]`
    /// and kept for pre-identity tests: it accepts whatever session is current
    /// regardless of identity or presentation. The presented-overlay flow uses
    /// [`Self::finish_selection_for_session`] instead; both share
    /// [`Self::finish_session_locked`].
    ///
    /// The overlay reports the corners as physical coordinates local to its own
    /// client area (its top-left sits on the stored geometry origin), so both
    /// corners are first shifted by that origin into virtual-screen
    /// coordinates. The translated rectangle is then normalized, clamped to the
    /// saved virtual-screen bounds and validated against
    /// [`capture::MIN_SELECTION_SIDE`]. A too-small or empty selection — or one
    /// whose translation would overflow `i32` — keeps the session and stays
    /// `SelectingArea` so the user can drag again. A valid selection drops the
    /// session, publishes `Recognizing`, crops the SAVED frame via
    /// [`crop_to_rgb`] and runs recognition through the injected closure (the
    /// command layer locks [`Self::runtime_slot`]); the status is published
    /// back to `Ready` either way. No pixel work happens under any lock.
    #[cfg(test)]
    pub async fn finish_selection<E, F, Fut>(
        &self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        emit: E,
        recognize: F,
    ) -> FinishOutcome
    where
        E: FnMut(&OcrStatus),
        F: FnOnce(Arc<RgbImage>) -> Fut,
        Fut: Future<Output = Result<OcrResult, String>>,
    {
        let guard = self.transition_lock.lock().await;

        let Some(session) = self.session.lock().await.take() else {
            return FinishOutcome::NoSession;
        };

        let selection = SelectionCorners { x1, y1, x2, y2 };
        self.finish_session_locked(guard, session, selection, emit, recognize)
            .await
    }

    /// Submit a selection for one specific, already-presented session.
    ///
    /// Guarded entry for the presented-overlay flow. Under the transition lock
    /// the current session must exist, match `id` and be marked `presented`; a
    /// stale, missing or still-unpresented session is a `Ok(NoSession)` that
    /// never hides or recognizes. A matching session first runs the injected
    /// `hide` callback while still serialized (the session mutex is released
    /// first); when hiding fails the session is dropped, `Ready` is restored
    /// while the runtime is active and the error is returned. Otherwise the
    /// shared [`Self::finish_session_locked`] flow runs unchanged. A too-small
    /// selection keeps the same session (id, frame and `presented`) under
    /// `SelectingArea` so the command layer can re-show it via
    /// [`Self::present_selection`].
    pub async fn finish_selection_for_session<E, H, F, Fut>(
        &self,
        id: &str,
        selection: SelectionCorners,
        mut emit: E,
        hide: H,
        recognize: F,
    ) -> Result<FinishOutcome, String>
    where
        E: FnMut(&OcrStatus),
        H: FnOnce() -> Result<(), String>,
        F: FnOnce(Arc<RgbImage>) -> Fut,
        Fut: Future<Output = Result<OcrResult, String>>,
    {
        let guard = self.transition_lock.lock().await;

        // Identity + presentation guard: only the current presented session may
        // submit. The session is checked in place (and only then taken) so a
        // stale or unpresented submit leaves the live session untouched.
        let session = {
            let mut guard = self.session.lock().await;
            let Some(current) = guard.as_ref() else {
                return Ok(FinishOutcome::NoSession);
            };
            if current.id != id || !current.presented {
                return Ok(FinishOutcome::NoSession);
            }
            guard.take().expect("matching presented session present")
        };

        // Hide the overlay while still serialized but without the session
        // mutex held. A failed hide abandons the session entirely.
        if let Err(reason) = hide() {
            drop(session);
            if self.is_runtime_active() {
                self.publish_status(OcrStatus::Ready, &mut emit);
            }
            return Err(reason);
        }

        Ok(self
            .finish_session_locked(guard, session, selection, emit, recognize)
            .await)
    }

    /// Shared selection-processing body for a submitted selection; the caller
    /// hands over the transition lock it already holds and has removed the
    /// session from the slot. Reached through
    /// [`Self::finish_selection_for_session`] and the test-only legacy
    /// `Self::finish_selection` (`#[cfg(test)]`).
    ///
    /// `selection` carries the two overlay-local corners in physical px relative
    /// to the overlay's own client area; the saved frame lives in
    /// virtual-screen coordinates, so both corners are first shifted by the
    /// session geometry origin. A representational overflow, an empty/outside
    /// rectangle or a selection below [`capture::MIN_SELECTION_SIDE`] is
    /// retryable: the session is put back (keeping its id/frame/`presented`)
    /// and `TooSmall` is returned. A valid selection publishes `Recognizing` and
    /// then RELEASES the transition lock before cropping the SAVED frame on the
    /// blocking pool and running recognition (up to the recognition timeout), so
    /// a concurrent stop/restart is never blocked by it. No pixel work happens
    /// under any lock.
    ///
    /// Because the lock is gone by then, the final `Recognizing -> Ready`
    /// transition is guarded by the session's capture epoch
    /// ([`Self::publish_ready_if_generation_current`]): a result that lands after
    /// the runtime was stopped, or after a newer capture session began, is
    /// dropped instead of resurrecting `Ready`.
    async fn finish_session_locked<E, F, Fut>(
        &self,
        transition: tokio::sync::MutexGuard<'_, ()>,
        session: CaptureSession,
        selection: SelectionCorners,
        mut emit: E,
        recognize: F,
    ) -> FinishOutcome
    where
        E: FnMut(&OcrStatus),
        F: FnOnce(Arc<RgbImage>) -> Fut,
        Fut: Future<Output = Result<OcrResult, String>>,
    {
        let SelectionCorners { x1, y1, x2, y2 } = selection;
        let Some((x1, y1, x2, y2)) =
            translate_by_session_origin(x1, y1, x2, y2, session.geometry.origin)
        else {
            *self.session.lock().await = Some(session);
            return FinishOutcome::TooSmall;
        };

        let rect = normalize_selection(x1, y1, x2, y2);
        let rect = match clamp_selection(rect, &session.geometry) {
            None => {
                // Empty/outside selection: retryable, keep the session.
                *self.session.lock().await = Some(session);
                return FinishOutcome::TooSmall;
            }
            Some(clamped) => match validate_selection(clamped) {
                Ok(rect) => rect,
                Err(CaptureError::SelectionTooSmall) => {
                    // Retryable: keep the session, stay SelectingArea.
                    *self.session.lock().await = Some(session);
                    return FinishOutcome::TooSmall;
                }
                Err(
                    CaptureError::NoMonitors
                    | CaptureError::CaptureFailed(_)
                    | CaptureError::MonitorUnavailable,
                ) => {
                    unreachable!("validate_selection only ever returns SelectionTooSmall")
                }
            },
        };

        self.publish_status(OcrStatus::Recognizing, &mut emit);

        // The crop + recognition phase below can take up to the recognition
        // timeout. Drop the transition lock for it: a stop, a restart or a
        // shutdown must not wait for OCR. The session's capture epoch is kept so
        // the final `Ready` can be validated once the lock is gone, and the
        // in-flight counter tells `stop_for_shutdown` to defer instead of
        // waiting for the runtime slot.
        let generation = session.generation;
        let _in_flight = RecognitionInFlight::new(&self.recognition_in_flight);
        drop(transition);

        // The RGBA→RGB crop is a pixel conversion (up to ~100 MB) and must not
        // occupy a Tokio worker: run it on the blocking pool. A panicking crop
        // worker surfaces as a safe RecognizeFailed (recognition path), never a
        // panic payload.
        let frame = session.frame.clone();
        let geometry = session.geometry.clone();
        let cropped =
            match tokio::task::spawn_blocking(move || crop_to_rgb(&frame, &geometry, rect)).await {
                Ok(cropped) => Arc::new(cropped),
                Err(join_error) => {
                    tracing::warn!(error = %join_error, "OCR crop worker panicked");
                    self.publish_ready_if_generation_current(generation, &mut emit);
                    return FinishOutcome::RecognizeFailed("recognitionFailed".to_string());
                }
            };

        let outcome = match recognize(cropped).await {
            Ok(result) => FinishOutcome::Recognized(result),
            Err(reason) => FinishOutcome::RecognizeFailed(reason),
        };

        // Return to `Ready` only while this exact capture epoch is still current
        // and the runtime is still installed. A stop (or a newer session) that
        // happened during recognition wins over this late result.
        self.publish_ready_if_generation_current(generation, &mut emit);

        outcome
    }

    /// Cancel the in-flight capture session.
    ///
    /// Test-only legacy unguarded helper, compiled under `#[cfg(test)]` and kept
    /// for pre-identity tests: it drops whatever session is current regardless
    /// of identity or presentation. The command layer exclusively uses the
    /// guarded [`Self::cancel_selection`]. Drops the stored frame and returns
    /// to `Ready` when a session was active; a session-less call is a no-op.
    /// Returns whether a session was dropped.
    #[cfg(test)]
    pub async fn cancel_session<E>(&self, mut emit: E) -> bool
    where
        E: FnMut(&OcrStatus),
    {
        let _guard = self.transition_lock.lock().await;

        let had_session = self.session.lock().await.take().is_some();
        // Publish `Ready` only while the runtime is still installed: a stop
        // drops the session first, so this guard is defensive against a stale
        // cancel resurrecting `Ready` from a disabled state.
        if had_session && self.is_runtime_active() {
            self.publish_status(OcrStatus::Ready, &mut emit);
        }
        had_session
    }

    /// Cancel a specific capture session, hiding the overlay first.
    ///
    /// Guards against stale lifecycle events: under the transition lock the
    /// current session must exist and match `id`, and — when `only_unpresented`
    /// is set — must not yet be `presented`. Anything stale, missing or an
    /// unpresented-only timeout against an already-presented session is a
    /// `Ok(false)` that never runs `hide` or touches the status. Otherwise the
    /// injected `hide` closure runs and the session is dropped even when hiding
    /// fails; `Ready` is restored while the runtime is active and `Ok(true)` or
    /// the hide error is returned. This covers user cancel, a preview decode
    /// failure and a guarded presentation timeout.
    pub async fn cancel_selection<E, H>(
        &self,
        id: &str,
        only_unpresented: bool,
        mut emit: E,
        hide: H,
    ) -> Result<bool, String>
    where
        E: FnMut(&OcrStatus),
        H: FnOnce() -> Result<(), String>,
    {
        let _guard = self.transition_lock.lock().await;

        // Identity guard: a stale or missing id, or a presented session behind
        // an unpresented-only timeout, is a no-op.
        {
            let session = self.session.lock().await;
            let Some(session) = session.as_ref() else {
                return Ok(false);
            };
            if session.id != id || (only_unpresented && session.presented) {
                return Ok(false);
            }
        }

        let hidden = hide();

        // Drop the session unconditionally, even when the hide callback fails:
        // a cancelled selection must never linger.
        self.session.lock().await.take();

        if self.is_runtime_active() {
            self.publish_status(OcrStatus::Ready, &mut emit);
        }

        match hidden {
            Ok(()) => Ok(true),
            Err(reason) => Err(reason),
        }
    }
}

impl Default for OcrService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Hotkey;
    use crate::ocr::capture::{MonitorRect, MIN_SELECTION_SIDE};
    use crate::ocr::settings::OcrSettings;
    use image::Rgba;
    use std::time::Duration;

    fn unique_test_root(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ttsbard-ocr-service-test-{}-{}-{}",
            std::process::id(),
            unique,
            name
        ))
    }

    fn write_valid_pack(root: &Path, id: &str) {
        let dir = root.join("models").join("ocr").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::json!({
                "id": id,
                "display_name": "Dummy Pack",
                "languages": ["ru", "en"],
                "family": "pp_ocr_v5",
                "det_file": "det.onnx",
                "rec_file": "rec.onnx",
                "dict_file": "dict.txt"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(dir.join("det.onnx"), b"dummy det").unwrap();
        std::fs::write(dir.join("rec.onnx"), b"dummy rec").unwrap();
        std::fs::write(dir.join("dict.txt"), b"dummy dict").unwrap();
    }

    fn dummy_hotkey() -> Hotkey {
        Hotkey::default_ocr_capture()
    }

    fn noop_register() -> impl FnOnce(&Hotkey) -> Result<(), String> {
        |_| Ok(())
    }

    fn noop_unregister() -> impl Fn(&Hotkey) -> Result<(), String> {
        |_| Ok(())
    }

    #[test]
    fn status_wire_shape_matches_input_server_convention() {
        assert_eq!(
            serde_json::to_value(OcrStatus::Disabled).unwrap(),
            serde_json::json!({ "state": "disabled" })
        );
        assert_eq!(
            serde_json::to_value(OcrStatus::Starting).unwrap(),
            serde_json::json!({ "state": "starting" })
        );
        assert_eq!(
            serde_json::to_value(OcrStatus::Ready).unwrap(),
            serde_json::json!({ "state": "ready" })
        );
        assert_eq!(
            serde_json::to_value(OcrStatus::SelectingArea).unwrap(),
            serde_json::json!({ "state": "selectingArea" })
        );
        assert_eq!(
            serde_json::to_value(OcrStatus::Recognizing).unwrap(),
            serde_json::json!({ "state": "recognizing" })
        );
        assert_eq!(
            serde_json::to_value(OcrStatus::Error {
                message: "boom".to_string()
            })
            .unwrap(),
            serde_json::json!({ "state": "error", "message": "boom" })
        );
    }

    #[test]
    fn publish_status_emits_only_on_change() {
        let service = OcrService::with_packs_root(PathBuf::new());
        let mut emitted: Vec<OcrStatus> = Vec::new();

        service.publish_status(OcrStatus::Disabled, |status| {
            emitted.push(status.clone());
        });
        assert!(emitted.is_empty(), "no emit when status did not change");

        service.publish_status(OcrStatus::Starting, |status| {
            emitted.push(status.clone());
        });
        service.publish_status(OcrStatus::Starting, |status| {
            emitted.push(status.clone());
        });
        service.publish_status(
            OcrStatus::Error {
                message: "boom".into(),
            },
            |status| {
                emitted.push(status.clone());
            },
        );

        assert_eq!(
            emitted,
            vec![
                OcrStatus::Starting,
                OcrStatus::Error {
                    message: "boom".into()
                }
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_without_model_reports_error_never_ready() {
        let service = OcrService::with_packs_root(PathBuf::new());
        service.settings.write().await.model_id = None;

        let mut transitions: Vec<OcrStatus> = Vec::new();
        service
            .start(
                &dummy_hotkey(),
                |s| transitions.push(s.clone()),
                noop_register(),
            )
            .await;

        assert!(
            matches!(service.status(), OcrStatus::Error { .. }),
            "expected Error, got {:?}",
            service.status()
        );
        assert!(
            !transitions.contains(&OcrStatus::Ready),
            "must never reach Ready without a model"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_with_unknown_model_names_missing_pack_without_substitution() {
        let root = unique_test_root("unknown-model");
        write_valid_pack(&root, "good-pack");

        let service = OcrService::with_packs_root(root.clone());
        service.settings.write().await.model_id = Some("missing-pack".to_string());

        service
            .start(&dummy_hotkey(), |_| {}, noop_register())
            .await;

        match service.status() {
            OcrStatus::Error { message } => {
                assert!(message.contains("missing-pack"), "message: {message}");
                assert!(
                    !message.contains("good-pack"),
                    "must not substitute: {message}"
                );
            }
            other => panic!("expected Error, got {other:?}"),
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_with_dummy_pack_maps_init_failure_to_error() {
        let root = unique_test_root("dummy-init");
        write_valid_pack(&root, "dummy-pack");

        let service = OcrService::with_packs_root(root.clone());
        service.settings.write().await.model_id = Some("dummy-pack".to_string());

        service
            .start(&dummy_hotkey(), |_| {}, noop_register())
            .await;

        match service.status() {
            OcrStatus::Error { message } => assert!(!message.is_empty()),
            other => panic!("expected Error from dummy ONNX, got {other:?}"),
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_without_runtime_reports_disabled() {
        let service = OcrService::with_packs_root(PathBuf::new());
        service.stop(|_| {}, noop_unregister()).await;
        assert_eq!(service.status(), OcrStatus::Disabled);
    }

    // --- Two-root discovery (cwd → data root, ROADMAP-117) ---

    fn write_broken_pack(root: &Path, id: &str) {
        let dir = root.join("models").join("ocr").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::json!({
                "id": id,
                "display_name": "Broken Pack",
                "languages": ["ru"],
                "family": "pp_ocr_v5",
                "det_file": "det.onnx",
                "rec_file": "rec.onnx",
                "dict_file": "dict.txt"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(dir.join("det.onnx"), b"dummy det").unwrap();
        std::fs::write(dir.join("dict.txt"), b"dummy dict").unwrap();
    }

    /// Merged discovery through the service: packs from both roots are
    /// available, a shared ID resolves to the cwd descriptor. The runtime
    /// start, the pack list and refresh all call `discover_packs`, so this is
    /// the same selection every consumer makes.
    #[test]
    fn discover_packs_merges_roots_and_cwd_wins_shared_id() {
        let cwd = unique_test_root("svc-merge-cwd");
        let data = unique_test_root("svc-merge-data");
        write_valid_pack(&cwd, "shared-pack");
        write_valid_pack(&cwd, "cwd-only-pack");
        write_valid_pack(&data, "shared-pack");
        write_valid_pack(&data, "data-only-pack");

        let service = OcrService::with_search_roots(vec![cwd.clone(), data.clone()], data.clone());
        let packs = service.discover_packs();

        let ids: Vec<&str> = packs.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["cwd-only-pack", "data-only-pack", "shared-pack"]);
        let shared = packs.iter().find(|p| p.id == "shared-pack").unwrap();
        assert!(
            shared.pack_root.starts_with(&cwd),
            "cwd descriptor must win: {:?}",
            shared.pack_root
        );
        assert!(
            shared.rec_path.starts_with(&cwd),
            "file paths must come from the descriptor's own root"
        );

        std::fs::remove_dir_all(&cwd).ok();
        std::fs::remove_dir_all(&data).ok();
    }

    /// A broken same-ID candidate in the cwd does not hide the valid data-root
    /// pack; the runtime would load the data-root files.
    #[test]
    fn discover_packs_broken_cwd_pack_falls_back_to_data_root() {
        let cwd = unique_test_root("svc-fallback-cwd");
        let data = unique_test_root("svc-fallback-data");
        write_broken_pack(&cwd, "shared-pack");
        write_valid_pack(&data, "shared-pack");

        let service = OcrService::with_search_roots(vec![cwd.clone(), data.clone()], data.clone());
        let packs = service.discover_packs();

        assert_eq!(packs.len(), 1);
        assert!(
            packs[0].pack_root.starts_with(&data),
            "fallback must resolve to the valid data-root pack: {:?}",
            packs[0].pack_root
        );

        std::fs::remove_dir_all(&cwd).ok();
        std::fs::remove_dir_all(&data).ok();
    }

    /// The managed packs root stays the effective data root even when the cwd
    /// has priority in discovery: opening the folder must not open the cwd.
    #[test]
    fn packs_root_is_managed_data_root_not_priority_cwd() {
        let cwd = unique_test_root("svc-open-cwd");
        let data = unique_test_root("svc-open-data");

        let service = OcrService::with_search_roots(vec![cwd.clone(), data.clone()], data.clone());
        assert_eq!(service.packs_root(), data.as_path());

        // Single-root constructor keeps the root for both roles.
        let single = OcrService::with_packs_root(cwd.clone());
        assert_eq!(single.packs_root(), cwd.as_path());
        assert_eq!(single.discover_packs().len(), 0);

        std::fs::remove_dir_all(&cwd).ok();
        std::fs::remove_dir_all(&data).ok();
    }

    /// A missing cwd root (e.g. deleted working directory) must not break
    /// discovery of the data-root packs.
    #[test]
    fn discover_packs_missing_cwd_root_still_finds_data_root() {
        let cwd = unique_test_root("svc-missing-cwd");
        assert!(!cwd.exists());
        let data = unique_test_root("svc-missing-data");
        write_valid_pack(&data, "data-pack");

        let service = OcrService::with_search_roots(vec![cwd.clone(), data.clone()], data.clone());
        let packs = service.discover_packs();

        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id, "data-pack");
        assert!(!cwd.exists(), "discovery must not create the cwd root");

        std::fs::remove_dir_all(&data).ok();
    }

    /// The blocking-pool discovery used by the async paths must report exactly
    /// what the synchronous seam reports, so `start` and `refresh_ocr_packs`
    /// never disagree with the pack list about the same roots.
    #[tokio::test]
    async fn discover_packs_blocking_matches_sync_discovery() {
        let root = unique_test_root("svc-blocking-discovery");
        write_valid_pack(&root, "shared-pack");

        let service = OcrService::with_packs_root(root.clone());

        let blocking = service.discover_packs_blocking().await;
        let inline = service.discover_packs();

        assert_eq!(blocking.len(), 1);
        assert_eq!(
            blocking.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
            inline.iter().map(|p| p.id.clone()).collect::<Vec<_>>()
        );
        assert_eq!(blocking[0].id, "shared-pack");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn stop_from_starting_without_shortcut_lands_disabled_without_ready() {
        let service = OcrService::with_packs_root(PathBuf::new());

        // A start has begun (`Starting`) but has not yet installed a runtime or
        // registered the capture shortcut — exactly the window a concurrent
        // `refresh_ocr_packs` disable can observe and must stop against.
        service.set_status(OcrStatus::Starting);
        assert!(!service.is_runtime_active());

        let mut published = Vec::new();
        service
            .stop(|status| published.push(status.clone()), noop_unregister())
            .await;

        assert_eq!(service.status(), OcrStatus::Disabled);
        assert_eq!(
            published,
            vec![OcrStatus::Disabled],
            "a stop from Starting must land Disabled directly"
        );
        assert!(
            !published.contains(&OcrStatus::Ready),
            "a stop from Starting must never publish Ready"
        );
        assert!(!service.is_runtime_active());
    }

    #[test]
    fn decide_enable_flip_starts_and_stops() {
        let old = OcrSettings::default();
        let on = OcrSettings {
            enabled: true,
            ..Default::default()
        };

        assert_eq!(
            decide_transition(&old, &on, None, None, &OcrStatus::Disabled),
            OcrTransition::Start
        );
        assert_eq!(
            decide_transition(&on, &old, None, None, &OcrStatus::Ready),
            OcrTransition::Stop
        );
    }

    #[test]
    fn decide_model_change_while_enabled_restarts() {
        let mut old = OcrSettings {
            enabled: true,
            model_id: Some("a".to_string()),
            ..Default::default()
        };
        let mut new = OcrSettings {
            enabled: true,
            model_id: Some("b".to_string()),
            ..Default::default()
        };

        assert_eq!(
            decide_transition(&old, &new, None, None, &OcrStatus::Ready),
            OcrTransition::Restart
        );

        old.model_id = None;
        new.model_id = None;
        assert_eq!(
            decide_transition(&old, &new, None, None, &OcrStatus::Ready),
            OcrTransition::Noop
        );
    }

    #[test]
    fn decide_hotkey_change_while_ready_reregisters() {
        let settings = OcrSettings {
            enabled: true,
            model_id: Some("a".to_string()),
            ..Default::default()
        };
        let old_hotkey = Hotkey::default_ocr_capture();
        let mut new_hotkey = old_hotkey.clone();
        new_hotkey.key = "O".to_string();

        assert_eq!(
            decide_transition(
                &settings,
                &settings,
                Some(&old_hotkey),
                Some(&new_hotkey),
                &OcrStatus::Ready
            ),
            OcrTransition::ReregisterHotkey
        );

        // Same binding while Ready is a no-op.
        assert_eq!(
            decide_transition(
                &settings,
                &settings,
                Some(&old_hotkey),
                Some(&old_hotkey),
                &OcrStatus::Ready
            ),
            OcrTransition::Noop
        );
    }

    #[test]
    fn decide_error_retries_unchanged_settings() {
        let settings = OcrSettings {
            enabled: true,
            model_id: Some("a".to_string()),
            ..Default::default()
        };
        let hotkey = Hotkey::default_ocr_capture();
        let error = OcrStatus::Error {
            message: "boom".to_string(),
        };

        // Unchanged settings (and unchanged binding) while Error retry.
        assert_eq!(
            decide_transition(&settings, &settings, Some(&hotkey), Some(&hotkey), &error),
            OcrTransition::Retry
        );

        // Retry also covers a not-yet-registered binding (the common Error case).
        assert_eq!(
            decide_transition(&settings, &settings, None, Some(&hotkey), &error),
            OcrTransition::Retry
        );

        // A model change while Error still restarts (higher priority).
        let mut new_model = settings.clone();
        new_model.model_id = Some("b".to_string());
        assert_eq!(
            decide_transition(&settings, &new_model, Some(&hotkey), Some(&hotkey), &error),
            OcrTransition::Restart
        );
    }

    #[test]
    fn decide_disabled_to_disabled_is_noop() {
        let settings = OcrSettings::default();
        assert_eq!(
            decide_transition(&settings, &settings, None, None, &OcrStatus::Disabled),
            OcrTransition::Noop
        );
    }

    #[test]
    fn decide_target_only_change_is_noop() {
        use crate::ocr::monitors::CaptureTarget;

        let hotkey = Hotkey::default_ocr_capture();
        let old = OcrSettings {
            enabled: true,
            model_id: Some("a".to_string()),
            capture_target: CaptureTarget::All,
        };
        let mut new = old.clone();
        new.capture_target = CaptureTarget::Monitor {
            device_path: r"\\.\DISPLAY2\Monitor0".to_string(),
        };

        // A target-only change must never restart the model, re-register the
        // hotkey or disturb an in-flight selection.
        assert_eq!(
            decide_transition(&old, &new, Some(&hotkey), Some(&hotkey), &OcrStatus::Ready),
            OcrTransition::Noop
        );
        assert_eq!(
            decide_transition(
                &old,
                &new,
                Some(&hotkey),
                Some(&hotkey),
                &OcrStatus::SelectingArea
            ),
            OcrTransition::Noop
        );
    }

    // --- OCR refresh reconcile decision ---

    #[test]
    fn refresh_enabled_missing_model_not_in_memory_disables_with_none() {
        let packs = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            decide_ocr_refresh_reconcile(true, Some("gone"), &packs, false),
            OcrRefreshReconcile::Disable { model_id: None }
        );
    }

    #[test]
    fn refresh_enabled_missing_model_with_single_remaining_autoselects_on_disable() {
        let packs = vec!["a".to_string()];
        assert_eq!(
            decide_ocr_refresh_reconcile(true, Some("gone"), &packs, false),
            OcrRefreshReconcile::Disable {
                model_id: Some("a".to_string())
            }
        );
    }

    #[test]
    fn refresh_enabled_missing_model_held_in_memory_is_none() {
        let packs = vec!["a".to_string()];
        assert_eq!(
            decide_ocr_refresh_reconcile(true, Some("gone"), &packs, true),
            OcrRefreshReconcile::None
        );
    }

    #[test]
    fn refresh_enabled_model_still_present_is_none() {
        let packs = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            decide_ocr_refresh_reconcile(true, Some("a"), &packs, false),
            OcrRefreshReconcile::None
        );
    }

    #[test]
    fn refresh_disabled_single_pack_autoselects_when_different() {
        let packs = vec!["a".to_string()];
        assert_eq!(
            decide_ocr_refresh_reconcile(false, None, &packs, false),
            OcrRefreshReconcile::AutoSelect("a".to_string())
        );
        assert_eq!(
            decide_ocr_refresh_reconcile(false, Some("gone"), &packs, false),
            OcrRefreshReconcile::AutoSelect("a".to_string())
        );
    }

    #[test]
    fn refresh_disabled_single_pack_already_selected_is_none() {
        let packs = vec!["a".to_string()];
        assert_eq!(
            decide_ocr_refresh_reconcile(false, Some("a"), &packs, false),
            OcrRefreshReconcile::None
        );
    }

    #[test]
    fn refresh_disabled_multiple_packs_is_none() {
        let packs = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            decide_ocr_refresh_reconcile(false, None, &packs, false),
            OcrRefreshReconcile::None
        );
    }

    #[test]
    fn refresh_empty_packs_enabled_disables_retaining_missing_model() {
        let packs: Vec<String> = vec![];
        assert_eq!(
            decide_ocr_refresh_reconcile(true, Some("gone"), &packs, false),
            OcrRefreshReconcile::Disable {
                model_id: Some("gone".to_string())
            }
        );
    }

    // --- One-shot capture session ---

    fn test_geometry() -> VirtualScreenGeometry {
        VirtualScreenGeometry {
            origin: (0, 0),
            size: (100, 100),
            monitors: vec![MonitorRect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            }],
        }
    }

    fn test_frame() -> RgbaImage {
        RgbaImage::from_pixel(100, 100, Rgba([255, 0, 0, 255]))
    }

    /// Build overlay-local corners for a guarded `finish_selection_for_session`.
    fn corners(x1: i32, y1: i32, x2: i32, y2: i32) -> SelectionCorners {
        SelectionCorners { x1, y1, x2, y2 }
    }

    fn fake_ocr_result(text: &str) -> OcrResult {
        OcrResult {
            text: text.to_string(),
            lines: Vec::new(),
            total_ms: 12.5,
        }
    }

    /// A service pre-flipped to `Ready` with a live runtime and registered
    /// capture shortcut, the only state a session can begin from.
    fn ready_service() -> OcrService {
        let service = OcrService::with_packs_root(PathBuf::new());
        service.set_status(OcrStatus::Ready);
        *service.registered_hotkey.lock() = Some(dummy_hotkey());
        *service.runtime_active.lock() = true;
        service
    }

    #[tokio::test]
    async fn begin_when_not_ready_is_noop() {
        let service = OcrService::with_packs_root(PathBuf::new());
        assert_eq!(service.status(), OcrStatus::Disabled);

        let captured = Arc::new(std::sync::Mutex::new(false));
        let capture_flag = Arc::clone(&captured);
        let mut shown = false;
        let outcome = service
            .begin_capture_session(
                |_| {},
                move || {
                    *capture_flag.lock().unwrap() = true;
                    Ok((test_geometry(), test_frame()))
                },
                |_geometry| {
                    shown = true;
                    Ok(())
                },
            )
            .await;

        assert!(matches!(outcome, BeginOutcome::Ignored));
        assert!(
            !*captured.lock().unwrap(),
            "capture must not run outside Ready"
        );
        assert!(!shown, "overlay must not be shown outside Ready");
        assert_eq!(service.status(), OcrStatus::Disabled);
    }

    #[tokio::test]
    async fn begin_from_ready_captures_before_show_and_publishes_selecting_area() {
        let service = ready_service();
        let order = Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));
        let capture_order = Arc::clone(&order);
        let show_order = Arc::clone(&order);
        let mut published = Vec::new();

        let outcome = service
            .begin_capture_session(
                |status| published.push(status.clone()),
                move || {
                    capture_order.lock().unwrap().push("capture");
                    Ok((test_geometry(), test_frame()))
                },
                move |_geometry| {
                    show_order.lock().unwrap().push("show");
                    Ok(())
                },
            )
            .await;

        let session_id = match outcome {
            BeginOutcome::Started { session_id } => session_id,
            other => panic!("expected Started, got {other:?}"),
        };
        assert_eq!(service.status(), OcrStatus::SelectingArea);
        assert_eq!(
            service
                .selection_snapshot()
                .await
                .expect("session started")
                .id,
            session_id,
            "Started must report the stored session identity"
        );
        assert_eq!(
            *order.lock().unwrap(),
            vec!["capture", "show"],
            "the frame must be captured before the overlay is revealed"
        );
        assert_eq!(published, vec![OcrStatus::SelectingArea]);
    }

    #[tokio::test]
    async fn begin_capture_failure_keeps_ready_and_shows_no_overlay() {
        let service = ready_service();
        let mut shown = false;

        let outcome = service
            .begin_capture_session(
                |_| {},
                || Err(CaptureError::NoMonitors),
                |_geometry| {
                    shown = true;
                    Ok(())
                },
            )
            .await;

        assert!(matches!(
            outcome,
            BeginOutcome::CaptureFailed(CaptureError::NoMonitors)
        ));
        assert_eq!(service.status(), OcrStatus::Ready);
        assert!(!shown, "overlay must not be shown after a capture failure");
    }

    #[tokio::test]
    async fn begin_while_selecting_area_is_ignored() {
        let service = ready_service();
        let first = service
            .begin_capture_session(|_| {}, || Ok((test_geometry(), test_frame())), |_| Ok(()))
            .await;
        assert!(matches!(first, BeginOutcome::Started { .. }));

        let second_capture = Arc::new(std::sync::Mutex::new(false));
        let capture_flag = Arc::clone(&second_capture);
        let outcome = service
            .begin_capture_session(
                |_| {},
                move || {
                    *capture_flag.lock().unwrap() = true;
                    Ok((test_geometry(), test_frame()))
                },
                |_| Ok(()),
            )
            .await;

        assert!(matches!(outcome, BeginOutcome::Ignored));
        assert!(
            !*second_capture.lock().unwrap(),
            "re-entry must not capture again"
        );
        assert_eq!(service.status(), OcrStatus::SelectingArea);
    }

    #[tokio::test]
    async fn begin_panicking_capture_returns_safe_capture_failed_and_no_overlay() {
        let service = ready_service();

        let mut shown = false;
        let outcome = service
            .begin_capture_session(
                |_| {},
                || panic!("xcap exploded"),
                |_geometry| {
                    shown = true;
                    Ok(())
                },
            )
            .await;

        match outcome {
            BeginOutcome::CaptureFailed(CaptureError::CaptureFailed(reason)) => {
                assert!(
                    !reason.to_lowercase().contains("exploded"),
                    "panic payload must not leak into the failure reason: {reason}"
                );
            }
            other => panic!("expected CaptureFailed, got {other:?}"),
        }
        assert_eq!(service.status(), OcrStatus::Ready);
        assert!(!shown, "overlay must not be shown after a capture panic");
        assert!(
            !service.cancel_session(|_| {}).await,
            "no session may exist after a capture failure"
        );
    }

    #[tokio::test]
    async fn begin_show_failure_clears_session_publishes_ready_and_allows_fresh_start() {
        let service = ready_service();
        let mut published = Vec::new();

        let outcome = service
            .begin_capture_session(
                |status| published.push(status.clone()),
                || Ok((test_geometry(), test_frame())),
                |_geometry| Err("ocr-selection overlay broken".to_string()),
            )
            .await;

        match outcome {
            BeginOutcome::OverlayFailed(reason) => {
                assert_eq!(reason, "ocr-selection overlay broken");
            }
            other => panic!("expected OverlayFailed, got {other:?}"),
        }
        assert_eq!(
            published,
            vec![OcrStatus::SelectingArea, OcrStatus::Ready],
            "show failure must publish SelectingArea then Ready"
        );
        assert_eq!(service.status(), OcrStatus::Ready);
        assert!(
            !service.cancel_session(|_| {}).await,
            "no session may survive a failed overlay show"
        );

        // A fresh session can start afterwards.
        let outcome = service
            .begin_capture_session(|_| {}, || Ok((test_geometry(), test_frame())), |_| Ok(()))
            .await;
        assert!(matches!(outcome, BeginOutcome::Started { .. }));
        assert_eq!(service.status(), OcrStatus::SelectingArea);
    }

    #[tokio::test]
    async fn finish_too_small_keeps_session_and_stays_selecting_area() {
        let service = ready_service();
        service
            .begin_capture_session(|_| {}, || Ok((test_geometry(), test_frame())), |_| Ok(()))
            .await;
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        let mut recognized = false;
        let outcome = service
            .finish_selection(
                0,
                0,
                MIN_SELECTION_SIDE as i32 - 1,
                MIN_SELECTION_SIDE as i32,
                |_| {},
                |_image| {
                    recognized = true;
                    async { Ok(fake_ocr_result("nope")) }
                },
            )
            .await;

        assert_eq!(outcome, FinishOutcome::TooSmall);
        assert!(!recognized, "too-small selection must not run recognition");
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        // The session survived: a valid selection now succeeds on the same
        // saved frame.
        let outcome = service
            .finish_selection(
                0,
                0,
                40,
                40,
                |_| {},
                |_image| async { Ok(fake_ocr_result("ok")) },
            )
            .await;
        assert!(matches!(outcome, FinishOutcome::Recognized(_)));
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn finish_valid_rect_crops_saved_frame_and_returns_ready() {
        let service = ready_service();
        service
            .begin_capture_session(|_| {}, || Ok((test_geometry(), test_frame())), |_| Ok(()))
            .await;

        let mut published = Vec::new();
        let seen = Arc::new(std::sync::Mutex::new(None));
        let seen_clone = seen.clone();
        let recognize = move |image: Arc<RgbImage>| {
            *seen_clone.lock().unwrap() = Some(image.dimensions());
            async { Ok(fake_ocr_result("hello")) }
        };

        let outcome = service
            .finish_selection(
                10,
                20,
                50,
                60,
                |status| published.push(status.clone()),
                recognize,
            )
            .await;

        assert_eq!(outcome, FinishOutcome::Recognized(fake_ocr_result("hello")));
        assert_eq!(
            *seen.lock().unwrap(),
            Some((40, 40)),
            "recognition must run on the crop of the saved frame"
        );
        assert_eq!(published, vec![OcrStatus::Recognizing, OcrStatus::Ready]);
        assert_eq!(service.status(), OcrStatus::Ready);

        // Session was dropped: a cancel afterwards is a no-op.
        assert!(!service.cancel_session(|_| {}).await);
    }

    #[tokio::test]
    async fn finish_translates_overlay_local_rect_by_negative_origin_before_cropping() {
        let service = ready_service();

        // Overlay window sits with its client-area origin on the geometry
        // origin (-100, -50), so an overlay-local corner (lx, ly) denotes the
        // saved-frame pixel at the same (lx, ly) offset once translated.
        let geometry = VirtualScreenGeometry {
            origin: (-100, -50),
            size: (200, 200),
            monitors: vec![MonitorRect {
                x: -100,
                y: -50,
                width: 200,
                height: 200,
            }],
        };
        let mut frame = RgbaImage::from_pixel(200, 200, Rgba([0, 0, 0, 255]));
        frame.put_pixel(12, 22, Rgba([255, 0, 0, 255]));
        frame.put_pixel(38, 58, Rgba([0, 0, 255, 255]));

        service
            .begin_capture_session(|_| {}, move || Ok((geometry, frame)), |_| Ok(()))
            .await;
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        // Overlay-local rectangle (10, 20)-(40, 60). Untranslated it would be
        // clamped against the negative virtual bounds and crop black pixels far
        // from the markers; translated it must crop the exact frame region.
        let seen = Arc::new(std::sync::Mutex::new(None));
        let seen_clone = Arc::clone(&seen);
        let recognize = move |image: Arc<RgbImage>| {
            *seen_clone.lock().unwrap() = Some(image);
            async { Ok(fake_ocr_result("translated")) }
        };

        let outcome = service
            .finish_selection(10, 20, 40, 60, |_| {}, recognize)
            .await;

        assert!(matches!(outcome, FinishOutcome::Recognized(_)));
        let cropped = seen.lock().unwrap().take().expect("recognition ran");
        assert_eq!(cropped.dimensions(), (30, 40));
        assert_eq!(cropped.get_pixel(0, 0), &image::Rgb([0, 0, 0]));
        assert_eq!(cropped.get_pixel(2, 2), &image::Rgb([255, 0, 0]));
        assert_eq!(cropped.get_pixel(28, 38), &image::Rgb([0, 0, 255]));
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn finish_translation_overflow_is_retryable_and_never_recognizes() {
        let service = ready_service();

        // Origin so large that overlay-local corners above x = 1000 overflow
        // i32 on translation. Coordinates the overlay can really report (bounded
        // by the window size) stay far below that, so the retained session still
        // accepts a valid retry.
        let geometry = VirtualScreenGeometry {
            origin: (i32::MAX - 1000, 0),
            size: (200, 200),
            monitors: vec![MonitorRect {
                x: i32::MAX - 1000,
                y: 0,
                width: 200,
                height: 200,
            }],
        };
        let frame = RgbaImage::from_pixel(200, 200, Rgba([255, 0, 0, 255]));

        service
            .begin_capture_session(|_| {}, move || Ok((geometry, frame)), |_| Ok(()))
            .await;
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        let mut published = Vec::new();
        let mut recognized = false;
        let outcome = service
            .finish_selection(
                2000,
                0,
                2100,
                100,
                |status| published.push(status.clone()),
                |_image| {
                    recognized = true;
                    async { Ok(fake_ocr_result("must not run")) }
                },
            )
            .await;

        assert_eq!(outcome, FinishOutcome::TooSmall);
        assert!(
            !recognized,
            "overflowing corners must never reach recognition"
        );
        assert!(published.is_empty(), "no status change on overflow");
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        // The session survived the overflow: a valid selection is recognized.
        let outcome = service
            .finish_selection(
                0,
                0,
                40,
                40,
                |_| {},
                |_image| async { Ok(fake_ocr_result("retried")) },
            )
            .await;
        assert!(matches!(outcome, FinishOutcome::Recognized(_)));
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn finish_recognize_failure_returns_ready_and_failure_outcome() {
        let service = ready_service();
        service
            .begin_capture_session(|_| {}, || Ok((test_geometry(), test_frame())), |_| Ok(()))
            .await;

        let outcome = service
            .finish_selection(
                0,
                0,
                40,
                40,
                |_| {},
                |_image| async { Err("recognizer down".to_string()) },
            )
            .await;

        assert_eq!(
            outcome,
            FinishOutcome::RecognizeFailed("recognizer down".to_string())
        );
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn finish_without_session_is_noop() {
        let service = ready_service();
        let outcome = service
            .finish_selection(
                0,
                0,
                40,
                40,
                |_| {},
                |_image| async { Ok(fake_ocr_result("nope")) },
            )
            .await;

        assert_eq!(outcome, FinishOutcome::NoSession);
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn cancel_drops_session_and_returns_ready() {
        let service = ready_service();
        service
            .begin_capture_session(|_| {}, || Ok((test_geometry(), test_frame())), |_| Ok(()))
            .await;
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        let mut published = Vec::new();
        let cancelled = service
            .cancel_session(|status| published.push(status.clone()))
            .await;

        assert!(cancelled);
        assert_eq!(service.status(), OcrStatus::Ready);
        assert_eq!(published, vec![OcrStatus::Ready]);
    }

    #[tokio::test]
    async fn cancel_without_session_is_noop() {
        let service = ready_service();
        let cancelled = service.cancel_session(|_| {}).await;

        assert!(!cancelled);
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    // --- Stop during an active session ---

    #[tokio::test]
    async fn stop_during_selecting_area_drops_session_and_never_publishes_ready() {
        let service = ready_service();
        service
            .begin_capture_session(|_| {}, || Ok((test_geometry(), test_frame())), |_| Ok(()))
            .await;
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        let mut published = Vec::new();
        service
            .stop(|status| published.push(status.clone()), noop_unregister())
            .await;

        assert_eq!(service.status(), OcrStatus::Disabled);
        assert_eq!(published, vec![OcrStatus::Disabled]);

        // A stale overlay cancel after stop is a no-op: no `Ready` is published.
        let mut published = Vec::new();
        let cancelled = service
            .cancel_session(|status| published.push(status.clone()))
            .await;
        assert!(!cancelled);
        assert!(published.is_empty(), "cancel after stop must not publish");
        assert_eq!(service.status(), OcrStatus::Disabled);

        // A stale overlay submit after stop is a no-op: no `Ready` is published.
        let mut published = Vec::new();
        let outcome = service
            .finish_selection(
                0,
                0,
                40,
                40,
                |status| published.push(status.clone()),
                |_image| async { Ok(fake_ocr_result("stale")) },
            )
            .await;
        assert_eq!(outcome, FinishOutcome::NoSession);
        assert!(published.is_empty(), "finish after stop must not publish");
        assert_eq!(service.status(), OcrStatus::Disabled);
    }

    #[tokio::test]
    async fn stop_resets_session_so_cancel_after_stop_is_noop() {
        let service = ready_service();
        service
            .begin_capture_session(|_| {}, || Ok((test_geometry(), test_frame())), |_| Ok(()))
            .await;
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        service.stop(|_| {}, noop_unregister()).await;
        assert_eq!(service.status(), OcrStatus::Disabled);

        // The session was dropped by stop: cancel is a no-op and the status
        // stays `Disabled` rather than returning to `Ready`.
        let mut published = Vec::new();
        assert!(!service.cancel_session(|s| published.push(s.clone())).await);
        assert!(published.is_empty());
        assert_eq!(service.status(), OcrStatus::Disabled);
    }

    /// Spawn a fully submitted selection whose recognition blocks until the
    /// returned release sender fires.
    ///
    /// Returns the finish task, a signal that the recognition phase is running
    /// (so the transition lock is already released) and the release sender. Every
    /// status published by the finish flow is collected in `published`.
    fn spawn_paused_recognition(
        service: &Arc<OcrService>,
        published: Arc<std::sync::Mutex<Vec<OcrStatus>>>,
    ) -> (
        tokio::task::JoinHandle<()>,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (recognizing_tx, recognizing_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let service = Arc::clone(service);
        let handle = tokio::spawn(async move {
            let id = begin_test_session(&service, test_frame()).await;
            assert!(
                service
                    .present_selection(&id, |_| {}, |_| Ok(()))
                    .await
                    .expect("reveal ran"),
                "the submitted session must be presented"
            );
            service
                .finish_selection_for_session(
                    &id,
                    corners(0, 0, 40, 40),
                    move |status: &OcrStatus| published.lock().unwrap().push(status.clone()),
                    || Ok(()),
                    move |_image| {
                        let _ = recognizing_tx.send(());
                        async move {
                            let _ = release_rx.await;
                            Ok(fake_ocr_result("late"))
                        }
                    },
                )
                .await
                .expect("finish ran");
        });
        (handle, recognizing_rx, release_tx)
    }

    /// Wait (bounded) for a deferred stop to land the terminal `Disabled`.
    async fn await_disabled(service: &OcrService) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while service.status() != OcrStatus::Disabled {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("deferred stop must publish Disabled");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_for_shutdown_returns_quickly_while_recognition_is_in_flight() {
        let service = Arc::new(ready_service());
        let published = Arc::new(std::sync::Mutex::new(Vec::<OcrStatus>::new()));
        let (finish, recognizing, release) =
            spawn_paused_recognition(&service, Arc::clone(&published));

        // The recognition phase is running with the transition lock released.
        recognizing.await.expect("recognition started");

        let started = std::time::Instant::now();
        service
            .clone()
            .stop_for_shutdown(|_| {}, noop_unregister())
            .await;
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "shutdown stop must not block on in-flight recognition"
        );

        await_disabled(&service).await;

        // A result landing after the stop must never resurrect `Ready`.
        let _ = release.send(());
        finish.await.expect("finish task");
        assert_eq!(
            *published.lock().unwrap(),
            vec![OcrStatus::Recognizing],
            "a late recognition must not publish Ready after a shutdown stop"
        );
        assert_eq!(service.status(), OcrStatus::Disabled);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_during_recognition_does_not_wait_for_it_or_publish_ready_afterwards() {
        let service = Arc::new(ready_service());
        let published = Arc::new(std::sync::Mutex::new(Vec::<OcrStatus>::new()));
        let (finish, recognizing, release) =
            spawn_paused_recognition(&service, Arc::clone(&published));
        recognizing.await.expect("recognition started");

        // Recognition no longer holds the transition lock, so a plain stop
        // completes instead of waiting up to the recognition timeout.
        tokio::time::timeout(
            Duration::from_secs(2),
            service.stop(|_| {}, noop_unregister()),
        )
        .await
        .expect("stop must not wait for the in-flight recognition");
        assert_eq!(service.status(), OcrStatus::Disabled);

        let _ = release.send(());
        finish.await.expect("finish task");
        assert_eq!(
            *published.lock().unwrap(),
            vec![OcrStatus::Recognizing],
            "a late recognition must not publish Ready after a stop"
        );
        assert_eq!(service.status(), OcrStatus::Disabled);
    }

    // --- Hotkey rebind during an active session ---

    #[tokio::test]
    async fn reregister_from_selecting_area_swaps_binding_and_keeps_session() {
        let service = ready_service();
        service
            .begin_capture_session(|_| {}, || Ok((test_geometry(), test_frame())), |_| Ok(()))
            .await;
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        let old_key = dummy_hotkey().key;
        let mut new_hotkey = dummy_hotkey();
        new_hotkey.key = "O".to_string();

        let registered = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let unregistered = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let registered_clone = Arc::clone(&registered);
        let unregistered_clone = Arc::clone(&unregistered);

        service
            .reregister_hotkey(
                &new_hotkey,
                |_| {},
                move |hk| {
                    registered_clone.lock().unwrap().push(hk.key.clone());
                    Ok(())
                },
                move |hk| {
                    unregistered_clone.lock().unwrap().push(hk.key.clone());
                    Ok(())
                },
            )
            .await;

        assert_eq!(*registered.lock().unwrap(), vec!["O".to_string()]);
        assert_eq!(*unregistered.lock().unwrap(), vec![old_key]);
        assert_eq!(service.registered_hotkey(), Some(new_hotkey.clone()));
        // The rebind must not disturb the in-flight selection.
        assert_eq!(service.status(), OcrStatus::SelectingArea);
    }

    #[tokio::test]
    async fn reregister_from_recognizing_swaps_binding() {
        let service = ready_service();
        // Simulate the recognition phase directly: the runtime is live and the
        // shortcut is registered, with status `Recognizing`.
        service.set_status(OcrStatus::Recognizing);
        *service.registered_hotkey.lock() = Some(dummy_hotkey());

        let old_key = dummy_hotkey().key;
        let mut new_hotkey = dummy_hotkey();
        new_hotkey.key = "O".to_string();

        let registered = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let unregistered = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let registered_clone = Arc::clone(&registered);
        let unregistered_clone = Arc::clone(&unregistered);

        service
            .reregister_hotkey(
                &new_hotkey,
                |_| {},
                move |hk| {
                    registered_clone.lock().unwrap().push(hk.key.clone());
                    Ok(())
                },
                move |hk| {
                    unregistered_clone.lock().unwrap().push(hk.key.clone());
                    Ok(())
                },
            )
            .await;

        assert_eq!(*registered.lock().unwrap(), vec!["O".to_string()]);
        assert_eq!(*unregistered.lock().unwrap(), vec![old_key]);
        assert_eq!(service.registered_hotkey(), Some(new_hotkey.clone()));
        assert_eq!(service.status(), OcrStatus::Recognizing);
    }

    #[tokio::test]
    async fn reregister_with_empty_runtime_slot_is_noop() {
        let service = OcrService::with_packs_root(PathBuf::new());
        assert_eq!(service.status(), OcrStatus::Disabled);
        assert!(!service.is_runtime_active());

        let mut new_hotkey = dummy_hotkey();
        new_hotkey.key = "O".to_string();

        let mut registered = false;
        service
            .reregister_hotkey(
                &new_hotkey,
                |_| {},
                |_| {
                    registered = true;
                    Ok(())
                },
                noop_unregister(),
            )
            .await;

        assert!(!registered, "no register call with an empty runtime slot");
        assert_eq!(service.status(), OcrStatus::Disabled);
        assert_eq!(service.registered_hotkey(), None);
    }

    // --- Disabling/re-enabling ocr_capture without stopping OCR ---

    #[tokio::test]
    async fn unregister_hotkey_releases_shortcut_and_keeps_ocr_active() {
        let service = ready_service();
        let mut released = Vec::<String>::new();

        service
            .unregister_hotkey(|hotkey| {
                released.push(hotkey.key.clone());
                Ok(())
            })
            .await;

        assert_eq!(released, vec![dummy_hotkey().key]);
        assert_eq!(service.registered_hotkey(), None);
        assert!(
            service.is_runtime_active(),
            "OCR stays enabled after its hotkey is switched off"
        );
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn unregister_hotkey_is_noop_without_registered_shortcut() {
        let service = ready_service();
        service.unregister_hotkey(noop_unregister()).await;

        let mut called = false;
        service
            .unregister_hotkey(|_| {
                called = true;
                Ok(())
            })
            .await;

        assert!(!called);
        assert!(service.is_runtime_active());
    }

    #[tokio::test]
    async fn register_hotkey_registers_again_after_disable() {
        let service = ready_service();
        service.unregister_hotkey(noop_unregister()).await;
        assert_eq!(service.registered_hotkey(), None);

        let mut new_hotkey = dummy_hotkey();
        new_hotkey.key = "O".to_string();
        let mut registered = Vec::<String>::new();

        service
            .register_hotkey(
                &new_hotkey,
                |hotkey| {
                    registered.push(hotkey.key.clone());
                    Ok(())
                },
                noop_unregister(),
            )
            .await
            .unwrap();

        assert_eq!(registered, vec!["O".to_string()]);
        assert_eq!(service.registered_hotkey(), Some(new_hotkey));
        assert!(service.is_runtime_active());
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn register_hotkey_skips_disabled_binding_and_inactive_runtime() {
        let service = ready_service();
        service.unregister_hotkey(noop_unregister()).await;
        let mut disabled = dummy_hotkey();
        disabled.enabled = false;
        let mut called = false;

        service
            .register_hotkey(
                &disabled,
                |_| {
                    called = true;
                    Ok(())
                },
                noop_unregister(),
            )
            .await
            .unwrap();
        assert!(!called, "a disabled binding must not be registered");
        assert_eq!(service.registered_hotkey(), None);

        let inactive = OcrService::with_packs_root(PathBuf::new());
        let mut enabled = dummy_hotkey();
        enabled.key = "O".to_string();
        inactive
            .register_hotkey(
                &enabled,
                |_| {
                    called = true;
                    Ok(())
                },
                noop_unregister(),
            )
            .await
            .unwrap();
        assert!(
            !called,
            "an inactive OCR runtime must not register anything"
        );
        assert_eq!(inactive.registered_hotkey(), None);
    }

    #[tokio::test]
    async fn register_hotkey_failure_keeps_shortcut_released() {
        let service = ready_service();
        service.unregister_hotkey(noop_unregister()).await;
        let mut new_hotkey = dummy_hotkey();
        new_hotkey.key = "O".to_string();

        let error = service
            .register_hotkey(
                &new_hotkey,
                |_| Err("registration failed".to_string()),
                noop_unregister(),
            )
            .await
            .unwrap_err();

        assert_eq!(error, "registration failed");
        assert_eq!(service.registered_hotkey(), None);
        assert!(service.is_runtime_active());
    }

    // --- Presented-overlay session identity and guards ---

    /// Begin a selecting session on `service` and return its identity.
    async fn begin_test_session(service: &OcrService, frame: RgbaImage) -> String {
        let outcome = service
            .begin_capture_session(|_| {}, move || Ok((test_geometry(), frame)), |_| Ok(()))
            .await;
        match outcome {
            // The start must report the identity of the session it stored.
            BeginOutcome::Started { session_id } => {
                assert_eq!(
                    service
                        .selection_snapshot()
                        .await
                        .expect("session started")
                        .id,
                    session_id,
                    "Started must carry the stored session identity"
                );
                session_id
            }
            other => panic!("expected Started, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn selection_snapshot_shares_frame_arc_and_content() {
        let service = ready_service();
        begin_test_session(&service, test_frame()).await;
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        let snap1 = service.selection_snapshot().await.expect("active session");
        let snap2 = service.selection_snapshot().await.expect("active session");

        assert!(!snap1.id.is_empty());
        assert_eq!(snap1.id, snap2.id);
        assert!(
            Arc::ptr_eq(&snap1.frame, &snap2.frame),
            "snapshot must clone the Arc, never the pixels"
        );
        assert_eq!(
            snap1.frame.get_pixel(50, 50),
            &Rgba([255, 0, 0, 255]),
            "snapshot must expose the exact saved frame content"
        );
    }

    #[tokio::test]
    async fn selection_snapshot_is_none_outside_selecting_area() {
        let service = ready_service();
        assert!(service.selection_snapshot().await.is_none());
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn second_session_gets_a_distinct_identity() {
        let service = ready_service();
        let first = begin_test_session(&service, test_frame()).await;
        assert!(
            service.cancel_session(|_| {}).await,
            "first session dropped"
        );
        assert_eq!(service.status(), OcrStatus::Ready);

        let second = begin_test_session(&service, test_frame()).await;
        assert_ne!(
            first, second,
            "each capture session needs a unique identity"
        );
    }

    #[tokio::test]
    async fn present_marks_session_and_allows_reshow() {
        let service = ready_service();
        let id = begin_test_session(&service, test_frame()).await;

        let mut shows = 0;
        let result = service
            .present_selection(
                &id,
                |_| {},
                |_geometry| {
                    shows += 1;
                    Ok(())
                },
            )
            .await;
        assert_eq!(result, Ok(true));
        assert_eq!(shows, 1);
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        // Re-showing an already-presented matching session is permitted
        // (too-small retry after the overlay was hidden).
        let result = service
            .present_selection(
                &id,
                |_| {},
                |_geometry| {
                    shows += 1;
                    Ok(())
                },
            )
            .await;
        assert_eq!(result, Ok(true));
        assert_eq!(shows, 2);
        assert_eq!(service.status(), OcrStatus::SelectingArea);
    }

    #[tokio::test]
    async fn present_without_matching_session_is_noop() {
        let service = ready_service();

        // No session at all: never call the show callback.
        let mut shown = false;
        let result = service
            .present_selection(
                "ghost",
                |_| {},
                |_| {
                    shown = true;
                    Ok(())
                },
            )
            .await;
        assert_eq!(result, Ok(false));
        assert!(!shown, "show must not run without a session");
        assert_eq!(service.status(), OcrStatus::Ready);

        // A stale id for a session that is no longer current.
        let id = begin_test_session(&service, test_frame()).await;
        service.cancel_session(|_| {}).await;
        assert_eq!(service.status(), OcrStatus::Ready);

        let mut shown = false;
        let result = service
            .present_selection(
                &id,
                |_| {},
                |_| {
                    shown = true;
                    Ok(())
                },
            )
            .await;
        assert_eq!(result, Ok(false));
        assert!(!shown, "a stale id must never show");
    }

    #[tokio::test]
    async fn present_show_failure_drops_session_and_publishes_ready() {
        let service = ready_service();
        let id = begin_test_session(&service, test_frame()).await;

        let mut published = Vec::new();
        let result = service
            .present_selection(
                &id,
                |status| published.push(status.clone()),
                |_geometry| Err("selection window missing".to_string()),
            )
            .await;

        assert_eq!(result, Err("selection window missing".to_string()));
        assert_eq!(published, vec![OcrStatus::Ready]);
        assert_eq!(service.status(), OcrStatus::Ready);
        assert!(service.selection_snapshot().await.is_none());
    }

    #[tokio::test]
    async fn stale_events_for_older_session_leave_newer_session_intact() {
        let service = ready_service();
        let old_id = begin_test_session(&service, test_frame()).await;
        service
            .present_selection(&old_id, |_| {}, |_| Ok(()))
            .await
            .unwrap();

        // End the first session and start a newer one behind it.
        assert!(service
            .cancel_selection(&old_id, false, |_| {}, || Ok(()))
            .await
            .unwrap());
        assert_eq!(service.status(), OcrStatus::Ready);
        let new_id = begin_test_session(&service, test_frame()).await;
        assert_ne!(old_id, new_id);
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        // Stale present must not show.
        let mut stale_shown = false;
        let result = service
            .present_selection(
                &old_id,
                |_| {},
                |_| {
                    stale_shown = true;
                    Ok(())
                },
            )
            .await;
        assert_eq!(result, Ok(false));
        assert!(!stale_shown, "stale present must not reveal the overlay");

        // Stale cancel must not hide or publish anything.
        let mut stale_hidden = false;
        let mut published = Vec::new();
        let result = service
            .cancel_selection(
                &old_id,
                false,
                |status| published.push(status.clone()),
                || {
                    stale_hidden = true;
                    Ok(())
                },
            )
            .await;
        assert_eq!(result, Ok(false));
        assert!(!stale_hidden, "stale cancel must not run hide");
        assert!(published.is_empty(), "stale cancel must not publish");

        // Stale submit must not hide or recognize.
        let mut stale_hidden = false;
        let mut stale_recognized = false;
        let result = service
            .finish_selection_for_session(
                &old_id,
                corners(0, 0, 40, 40),
                |_| {},
                || {
                    stale_hidden = true;
                    Ok(())
                },
                |_image| {
                    stale_recognized = true;
                    async { Ok(fake_ocr_result("stale")) }
                },
            )
            .await;
        assert_eq!(result, Ok(FinishOutcome::NoSession));
        assert!(!stale_hidden, "stale submit must not run hide");
        assert!(!stale_recognized, "stale submit must not run recognition");

        // The newer session is untouched and still current.
        assert_eq!(service.status(), OcrStatus::SelectingArea);
        let snapshot = service
            .selection_snapshot()
            .await
            .expect("newer session survives");
        assert_eq!(snapshot.id, new_id);
    }

    #[tokio::test]
    async fn cancelled_pending_preview_cannot_be_presented() {
        let service = ready_service();
        let id = begin_test_session(&service, test_frame()).await;
        // The session exists but its preview is still pending (not presented).
        assert_eq!(
            service
                .cancel_selection(&id, false, |_| {}, || Ok(()))
                .await,
            Ok(true)
        );
        assert_eq!(service.status(), OcrStatus::Ready);

        // A late present for the cancelled pending preview must never reveal.
        let mut shown = false;
        let result = service
            .present_selection(
                &id,
                |_| {},
                |_geometry| {
                    shown = true;
                    Ok(())
                },
            )
            .await;
        assert_eq!(result, Ok(false));
        assert!(!shown, "cancelled pending preview must never show");
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn only_unpresented_cancel_guards_pending_timeout() {
        let service = ready_service();

        // Timeout fires while the preview is still pending (not presented).
        let id = begin_test_session(&service, test_frame()).await;
        let mut hidden = 0;
        assert_eq!(
            service
                .cancel_selection(
                    &id,
                    true,
                    |_| {},
                    || {
                        hidden += 1;
                        Ok(())
                    }
                )
                .await,
            Ok(true)
        );
        assert_eq!(hidden, 1);
        assert_eq!(service.status(), OcrStatus::Ready);

        // The same unpresented-only timeout after ready (session gone) is a
        // no-op that never calls hide.
        let mut hidden = 0;
        assert_eq!(
            service
                .cancel_selection(
                    &id,
                    true,
                    |_| {},
                    || {
                        hidden += 1;
                        Ok(())
                    }
                )
                .await,
            Ok(false)
        );
        assert_eq!(hidden, 0);
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn only_unpresented_cancel_never_hits_a_presented_session() {
        let service = ready_service();
        let id = begin_test_session(&service, test_frame()).await;
        service
            .present_selection(&id, |_| {}, |_| Ok(()))
            .await
            .unwrap();

        let mut hidden = 0;
        let result = service
            .cancel_selection(
                &id,
                true,
                |_| {},
                || {
                    hidden += 1;
                    Ok(())
                },
            )
            .await;
        assert_eq!(
            result,
            Ok(false),
            "an unpresented-only timeout must ignore a presented session"
        );
        assert_eq!(hidden, 0);
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        // A real (presented) cancel still works afterwards.
        assert_eq!(
            service
                .cancel_selection(&id, false, |_| {}, || Ok(()))
                .await,
            Ok(true)
        );
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn cancel_hide_failure_still_drops_session_and_returns_ready() {
        let service = ready_service();
        let id = begin_test_session(&service, test_frame()).await;
        service
            .present_selection(&id, |_| {}, |_| Ok(()))
            .await
            .unwrap();

        let mut published = Vec::new();
        let result = service
            .cancel_selection(
                &id,
                false,
                |status| published.push(status.clone()),
                || Err("window gone".to_string()),
            )
            .await;

        assert_eq!(result, Err("window gone".to_string()));
        assert_eq!(published, vec![OcrStatus::Ready]);
        assert_eq!(service.status(), OcrStatus::Ready);
        assert!(service.selection_snapshot().await.is_none());
    }

    #[tokio::test]
    async fn guarded_submit_requires_current_presented_session() {
        let service = ready_service();

        // No session at all: NoSession without hide or recognition.
        let mut hidden = false;
        let mut recognized = false;
        let result = service
            .finish_selection_for_session(
                "ghost",
                corners(0, 0, 40, 40),
                |_| {},
                || {
                    hidden = true;
                    Ok(())
                },
                |_image| {
                    recognized = true;
                    async { Ok(fake_ocr_result("x")) }
                },
            )
            .await;
        assert_eq!(result, Ok(FinishOutcome::NoSession));
        assert!(!hidden && !recognized);

        // A session that exists but is still unpresented must not submit.
        let id = begin_test_session(&service, test_frame()).await;
        let mut hidden = false;
        let mut recognized = false;
        let result = service
            .finish_selection_for_session(
                &id,
                corners(0, 0, 40, 40),
                |_| {},
                || {
                    hidden = true;
                    Ok(())
                },
                |_image| {
                    recognized = true;
                    async { Ok(fake_ocr_result("x")) }
                },
            )
            .await;
        assert_eq!(result, Ok(FinishOutcome::NoSession));
        assert!(!hidden && !recognized);
        assert_eq!(service.status(), OcrStatus::SelectingArea);
        assert_eq!(
            service
                .selection_snapshot()
                .await
                .expect("session intact")
                .id,
            id
        );

        // A stale id after the session is gone is also NoSession.
        assert!(service
            .cancel_selection(&id, false, |_| {}, || Ok(()))
            .await
            .unwrap());
        let mut hidden = false;
        let result = service
            .finish_selection_for_session(
                &id,
                corners(0, 0, 40, 40),
                |_| {},
                || {
                    hidden = true;
                    Ok(())
                },
                |_image| async { Ok(fake_ocr_result("x")) },
            )
            .await;
        assert_eq!(result, Ok(FinishOutcome::NoSession));
        assert!(!hidden);
        assert_eq!(service.status(), OcrStatus::Ready);
    }

    #[tokio::test]
    async fn guarded_too_small_keeps_session_and_allows_reshow() {
        let service = ready_service();
        let id = begin_test_session(&service, test_frame()).await;
        service
            .present_selection(&id, |_| {}, |_| Ok(()))
            .await
            .unwrap();
        let frame_before = service.selection_snapshot().await.unwrap().frame;

        let mut hidden = 0;
        let mut recognized = false;
        let outcome = service
            .finish_selection_for_session(
                &id,
                corners(
                    0,
                    0,
                    MIN_SELECTION_SIDE as i32 - 1,
                    MIN_SELECTION_SIDE as i32,
                ),
                |_| {},
                || {
                    hidden += 1;
                    Ok(())
                },
                |_image| {
                    recognized = true;
                    async { Ok(fake_ocr_result("nope")) }
                },
            )
            .await;

        assert_eq!(outcome, Ok(FinishOutcome::TooSmall));
        assert_eq!(hidden, 1, "a submitted selection hides the overlay first");
        assert!(!recognized, "too-small selection must not run recognition");
        assert_eq!(service.status(), OcrStatus::SelectingArea);

        // The exact same session (id + frame Arc) survives for the retry.
        let snapshot = service
            .selection_snapshot()
            .await
            .expect("retained session");
        assert_eq!(snapshot.id, id);
        assert!(
            Arc::ptr_eq(&snapshot.frame, &frame_before),
            "TooSmall must keep the saved frame Arc"
        );

        // The retained presented session can be re-shown for the retry.
        let mut shown = false;
        assert_eq!(
            service
                .present_selection(
                    &id,
                    |_| {},
                    |_| {
                        shown = true;
                        Ok(())
                    }
                )
                .await,
            Ok(true)
        );
        assert!(shown);
        assert_eq!(service.status(), OcrStatus::SelectingArea);
    }

    #[tokio::test]
    async fn guarded_finish_hide_failure_abandons_session_and_returns_error() {
        let service = ready_service();
        let id = begin_test_session(&service, test_frame()).await;
        service
            .present_selection(&id, |_| {}, |_| Ok(()))
            .await
            .unwrap();

        let mut published = Vec::new();
        let mut recognized = false;
        let result = service
            .finish_selection_for_session(
                &id,
                corners(0, 0, 40, 40),
                |status| published.push(status.clone()),
                || Err("hide broke".to_string()),
                |_image| {
                    recognized = true;
                    async { Ok(fake_ocr_result("x")) }
                },
            )
            .await;

        assert_eq!(result, Err("hide broke".to_string()));
        assert!(!recognized, "recognition must not run after a failed hide");
        assert_eq!(published, vec![OcrStatus::Ready]);
        assert_eq!(service.status(), OcrStatus::Ready);
        assert!(service.selection_snapshot().await.is_none());
    }

    #[tokio::test]
    async fn guarded_finish_crops_exact_saved_frame() {
        let service = ready_service();
        let mut frame = RgbaImage::from_pixel(100, 100, Rgba([0, 0, 0, 255]));
        frame.put_pixel(10, 20, Rgba([255, 0, 0, 255]));
        frame.put_pixel(49, 59, Rgba([0, 0, 255, 255]));

        let id = begin_test_session(&service, frame).await;
        service
            .present_selection(&id, |_| {}, |_| Ok(()))
            .await
            .unwrap();

        let order = Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));
        let order_hide = Arc::clone(&order);
        let order_recognize = Arc::clone(&order);
        let mut published = Vec::new();
        let seen = Arc::new(std::sync::Mutex::new(None));
        let seen_clone = Arc::clone(&seen);

        let outcome = service
            .finish_selection_for_session(
                &id,
                corners(10, 20, 50, 60),
                |status| published.push(status.clone()),
                move || {
                    order_hide.lock().unwrap().push("hide");
                    Ok(())
                },
                move |image: Arc<RgbImage>| {
                    order_recognize.lock().unwrap().push("recognize");
                    *seen_clone.lock().unwrap() = Some(image);
                    async { Ok(fake_ocr_result("guarded")) }
                },
            )
            .await;

        assert_eq!(
            outcome,
            Ok(FinishOutcome::Recognized(fake_ocr_result("guarded")))
        );
        assert_eq!(published, vec![OcrStatus::Recognizing, OcrStatus::Ready]);
        assert_eq!(service.status(), OcrStatus::Ready);
        let cropped = seen.lock().unwrap().take().expect("recognition ran");
        assert_eq!(cropped.dimensions(), (40, 40));
        assert_eq!(cropped.get_pixel(0, 0), &image::Rgb([255, 0, 0]));
        assert_eq!(cropped.get_pixel(39, 39), &image::Rgb([0, 0, 255]));
        assert_eq!(
            *order.lock().unwrap(),
            vec!["hide", "recognize"],
            "hide must run before recognition"
        );
    }
}
