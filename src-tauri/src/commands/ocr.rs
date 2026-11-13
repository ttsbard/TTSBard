use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ImageEncoder, RgbImage, RgbaImage};

use crate::commands::input_server::accept_external_text;
use crate::commands::speech_queue::SpeechQueueState;
use crate::config::{Hotkey, SettingsManager};
use crate::ocr::capture::{capture_desktop_for_target, CaptureError, VirtualScreenGeometry};
use crate::ocr::monitors::{enumerate_monitors, MonitorInfo};
use crate::ocr::service::{
    decide_ocr_refresh_reconcile, decide_transition, BeginOutcome, FinishOutcome,
    OcrRefreshReconcile, OcrTransition, SelectionCorners,
};
use crate::ocr::settings::OcrSettings;
use crate::ocr::{OcrService, OcrStatus};
use crate::speech_queue::SubmissionSource;
use crate::state::AppState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Event emitted on every real OCR runtime status transition.
pub const OCR_STATUS_CHANGED_EVENT: &str = "ocr-status-changed";

/// Event emitted when a one-shot OCR capture begins but fails before the
/// selection overlay is usable. Payload: `{ reason: string }` (camelCase).
pub const OCR_ONE_SHOT_FAILED_EVENT: &str = "ocr-one-shot-failed";

/// Event emitted when a submitted selection was rejected as too small and the
/// overlay is re-shown at its unchanged geometry. Payload: `{ sessionId }`
/// (camelCase), matching the session that stays active for the retry.
pub const OCR_SELECTION_REJECTED_EVENT: &str = "ocr-selection-rejected";

/// Label of the declarative webview that hosts the OCR selection overlay. The
/// preview/ready handshake and the selection IPCs only accept this caller.
const OCR_SELECTION_WINDOW_LABEL: &str = "ocr-selection";

/// Safe presentation DTO for a discovered OCR pack (no resolved paths).
#[derive(Debug, Clone, Serialize)]
pub struct OcrPackDto {
    pub id: String,
    pub display_name: String,
    pub languages: Vec<String>,
}

/// Serializable payload for [`OCR_ONE_SHOT_FAILED_EVENT`]. Deliberately only a
/// safe reason string: no panic payloads, pixels, paths or model internals.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrOneShotFailedPayload {
    pub reason: String,
}

/// Serializable payload for [`OCR_SELECTION_REJECTED_EVENT`]: the session id
/// that was re-shown for another drag, so a stale TooSmall retry is never
/// mistaken for a live one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrSelectionRejectedPayload {
    pub session_id: String,
}

/// Ephemeral PNG preview of the current OCR selection frame.
///
/// Deliberately only dimensions plus the base64-encoded frozen frame — no
/// path, no URL and no server/protocol registration. The payload is returned
/// only to the `ocr-selection` webview that requested it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewDto {
    pub session_id: String,
    pub width: u32,
    pub height: u32,
    pub png_base64: String,
}

/// Fixed safe user-facing reason for a [`CaptureError`] category. The detailed
/// error text is only ever traced, never serialized to the frontend.
fn capture_failed_reason(error: &CaptureError) -> &'static str {
    match error {
        CaptureError::NoMonitors => "noMonitors",
        CaptureError::MonitorUnavailable => "monitorUnavailable",
        CaptureError::CaptureFailed(_) | CaptureError::SelectionTooSmall => "captureFailed",
    }
}

/// True when recognized OCR text is normalized blank/whitespace-only.
///
/// Such a result carries no usable content, so no incoming item is created —
/// instead a single fixed-safe `emptyResult` failure is surfaced. The text
/// itself is never logged or exposed.
fn is_blank_ocr_text(text: &str) -> bool {
    text.trim().is_empty()
}

fn emit_ocr_status(app_handle: &AppHandle, status: &OcrStatus) {
    let _ = app_handle.emit(OCR_STATUS_CHANGED_EVENT, status.clone());
}

fn emit_ocr_one_shot_failed(app_handle: &AppHandle, reason: String) {
    let _ = app_handle.emit(
        OCR_ONE_SHOT_FAILED_EVENT,
        OcrOneShotFailedPayload { reason },
    );
}

/// Deliver recognized OCR text through the shared external-text intake seam.
///
/// Resolves the managed [`SpeechQueueState`] from the Tauri app handle, then
/// forwards the recognized text to [`accept_external_text`] tagged with the OCR
/// producer source: with `auto_play` off it lands in the Incoming inbox
/// (`input-server-incoming-changed` is emitted), with `auto_play` on it is
/// submitted directly to the speech queue as `source: "ocr"`. Returns a fixed
/// safe reason when the queue state is unavailable or the seam rejects the text
/// (blank, overlong or inbox-full); the recognized text is never logged or
/// exposed.
async fn deliver_recognized_text(
    app_handle: &AppHandle,
    state: &AppState,
    text: String,
) -> Result<(), String> {
    let Some(queue) = app_handle.try_state::<SpeechQueueState>() else {
        tracing::warn!("SpeechQueueState unavailable while delivering OCR text");
        return Err("intakeFailed".to_string());
    };

    if let Err(error) = accept_external_text(
        app_handle,
        state,
        queue.inner(),
        SubmissionSource::Ocr,
        text,
    )
    .await
    {
        // The intake error carries only safe codes/messages, never the text.
        tracing::warn!(error = ?error, "OCR intake rejected recognized text");
        return Err("intakeFailed".to_string());
    }

    Ok(())
}

/// Handler for the registered `ocr_capture` shortcut.
///
/// Guards match the existing global handlers: skip while hotkey recording is
/// active and respect the global hotkeys-enabled flag. The one-shot flow runs
/// on the app's Tokio runtime so the shortcut callback never blocks.
pub(crate) fn handle_ocr_capture(app_handle: &AppHandle, app_state: &AppState) {
    if app_state.is_hotkey_recording() {
        return;
    }
    if !app_state.is_hotkey_enabled() {
        return;
    }

    let app_handle = app_handle.clone();
    let app_state = app_state.clone();
    let runtime = app_state.runtime.clone();
    runtime.spawn(async move {
        let service = app_state.ocr.clone();
        // Capture the CURRENT desired target with a fresh enumeration. The
        // target is cloned from the service snapshot, never from the UI monitor
        // list, so a hotkey press always uses the last saved selection.
        let target = service.settings.read().await.capture_target.clone();
        let emit = |status: &OcrStatus| emit_ocr_status(&app_handle, status);
        let capture = move || capture_desktop_for_target(&target);
        // PREPARE only: hide, position, size and exclude the overlay while the
        // begin lock is held. The window is revealed later by `present_selection`
        // through `ocr_selection_ready`, once the frame preview is loaded.
        let prepare_overlay =
            |geometry: &VirtualScreenGeometry| prepare_ocr_selection_overlay(&app_handle, geometry);

        match service
            .begin_capture_session(emit, capture, prepare_overlay)
            .await
        {
            BeginOutcome::Started { session_id } => {
                // The overlay stays hidden until the frontend fetches and
                // decodes the frozen frame. A bounded watchdog cancels a session
                // whose preview is never acknowledged, so a dead overlay cannot
                // leak the SelectingArea state or keep its frame alive. The
                // identity comes straight from the begin outcome, so no second
                // snapshot round trip (and no risk of arming the wrong session)
                // is needed.
                schedule_preview_timeout(app_handle.clone(), service, session_id);
            }
            BeginOutcome::Ignored => {}
            BeginOutcome::CaptureFailed(error) => {
                tracing::warn!(error = %error, "One-shot OCR capture failed");
                emit_ocr_one_shot_failed(&app_handle, capture_failed_reason(&error).to_string());
            }
            BeginOutcome::OverlayFailed(reason) => {
                tracing::warn!(error = %reason, "Failed to prepare ocr-selection overlay");
                emit_ocr_one_shot_failed(&app_handle, "overlayOpenFailed".to_string());
            }
        }
    });
}

/// Bound a prepared-but-unpresented overlay may wait for the frontend to fetch
/// and decode the frozen frame before it is cancelled as a pending preview.
const OCR_SELECTION_PREVIEW_TIMEOUT: Duration = Duration::from_secs(15);

/// Watch a just-prepared capture session that has not been revealed yet.
///
/// If the overlay never acknowledges the preview (no `ocr_selection_ready`)
/// within [`OCR_SELECTION_PREVIEW_TIMEOUT`], cancel that exact session through
/// `cancel_selection(id, only_unpresented: true, ...)` — which ignores an
/// already-presented or superseded session — and surface the fixed-safe
/// `overlayOpenFailed` only when the pending session was actually cancelled.
/// Holds only the session id (never the frame `Arc`) and runs detached on the
/// app runtime, so the hotkey handler never blocks on the wait.
fn schedule_preview_timeout(app_handle: AppHandle, service: Arc<OcrService>, session_id: String) {
    tokio::spawn(async move {
        tokio::time::sleep(OCR_SELECTION_PREVIEW_TIMEOUT).await;
        let emit = |status: &OcrStatus| emit_ocr_status(&app_handle, status);
        let hide = || hide_ocr_selection_overlay(&app_handle);
        match service
            .cancel_selection(&session_id, true, emit, hide)
            .await
        {
            Ok(false) => {}
            Ok(true) | Err(_) => {
                // The pending session was cancelled (an Err only means the
                // post-cancel hide also failed): notify exactly once.
                emit_ocr_one_shot_failed(&app_handle, "overlayOpenFailed".to_string());
            }
        }
    });
}

/// PREPARE the declarative `ocr-selection` overlay over the captured virtual
/// desktop without revealing it: hide any leftover overlay first, then place
/// the window in physical virtual-screen coordinates (`origin` may be negative
/// for monitors left/above the primary) and apply the capture exclusion.
///
/// Runs as the `show_overlay` callback of
/// [`OcrService::begin_capture_session`] while the begin transition lock is
/// held, so geometry is settled before any later ready command can reveal the
/// window. The actual show/focus happens only through
/// [`present_ocr_selection_overlay`], after the overlay has decoded the frozen
/// frame (preview → ready). Every failure is a safe `Err(String)` and leaves
/// the service to drop the session and restore `Ready`.
fn prepare_ocr_selection_overlay(
    app_handle: &AppHandle,
    geometry: &VirtualScreenGeometry,
) -> Result<(), String> {
    let window = app_handle
        .get_webview_window(OCR_SELECTION_WINDOW_LABEL)
        .ok_or_else(|| "ocr-selection window not found".to_string())?;

    // A reveal from a previous TooSmall retry may still be visible; a fresh
    // capture must never appear on top of it, so hide first.
    let _ = window.hide();

    window
        .set_position(tauri::Position::Physical(tauri::PhysicalPosition {
            x: geometry.origin.0,
            y: geometry.origin.1,
        }))
        .map_err(|e| format!("Failed to position ocr-selection overlay: {e}"))?;

    window
        .set_size(tauri::Size::Physical(tauri::PhysicalSize {
            width: geometry.size.0,
            height: geometry.size.1,
        }))
        .map_err(|e| format!("Failed to size ocr-selection overlay: {e}"))?;

    #[cfg(windows)]
    {
        // Capture exclusion is non-fatal: the frame was already saved before
        // the overlay is revealed, so privacy is preserved by capture→reveal
        // ordering. A failed exclusion only degrades a later re-capture and is
        // logged, never surfaced as an overlay-open failure.
        if let Ok(hwnd) = window.hwnd() {
            apply_overlay_capture_exclusion(
                hwnd.0 as isize,
                crate::window::set_window_exclude_from_capture,
            );
        } else {
            tracing::warn!("Failed to get ocr-selection overlay HWND for capture exclusion");
        }
    }

    Ok(())
}

/// Reveal and focus the already-prepared `ocr-selection` overlay window.
///
/// Runs inside the guarded `present_selection` callback (frontend-ready and
/// TooSmall re-show) after the frozen frame has been decoded and rendered, so
/// the window never becomes visible before its pixels are ready. Every failure
/// surfaces only the fixed safe code `overlayOpenFailed` (the detail is
/// traced); the window is hidden best-effort before the error returns.
fn present_ocr_selection_overlay(window: &tauri::WebviewWindow) -> Result<(), String> {
    if let Err(error) = window.show() {
        let _ = window.hide();
        tracing::warn!(error = %error, "Failed to present ocr-selection overlay show");
        return Err("overlayOpenFailed".to_string());
    }

    if let Err(error) = window.set_focus() {
        let _ = window.hide();
        tracing::warn!(error = %error, "Failed to present ocr-selection overlay focus");
        return Err("overlayOpenFailed".to_string());
    }

    Ok(())
}

/// Apply capture exclusion for the overlay, treating any failure as non-fatal.
///
/// The exclusion is injectable so the non-fatal behavior is unit-testable
/// without a live window.
#[cfg(windows)]
fn apply_overlay_capture_exclusion(
    hwnd: isize,
    exclude: impl FnOnce(isize, bool) -> anyhow::Result<()>,
) {
    if let Err(error) = exclude(hwnd, true) {
        tracing::warn!(error = %error, "Failed to exclude ocr-selection overlay from capture");
    }
}

/// Best-effort hide of the declarative `ocr-selection` overlay window.
///
/// Returns `Err` with a fixed safe code when the window is missing or its hide
/// call fails; the detailed Tauri error is only traced, never serialized to the
/// frontend. A missing window is expected when the overlay already hid itself.
fn hide_ocr_selection_overlay(app_handle: &AppHandle) -> Result<(), String> {
    let Some(window) = app_handle.get_webview_window("ocr-selection") else {
        tracing::warn!("ocr-selection window not found while cancelling selection");
        return Err("overlayHideFailed".to_string());
    };
    if let Err(error) = window.hide() {
        tracing::warn!(error = %error, "Failed to hide ocr-selection overlay");
        return Err("overlayHideFailed".to_string());
    }
    Ok(())
}

/// Handler for the `ocr_selection_cancel` command invoked by the selection
/// overlay on Escape/window blur, carrying the active `session_id`.
///
/// The overlay is only ever hidden and the session dropped after
/// [`OcrService::cancel_selection`] verifies the identity under the transition
/// lock: a stale or missing id is a silent no-op that must not hide a newer
/// overlay or touch its status. A matching session is hidden, dropped and the
/// status returned to `Ready`. This is an ordinary user cancellation, so no
/// `ocr-one-shot-failed` event is emitted; a hide failure after cancellation
/// surfaces only the fixed safe code [`hide_ocr_selection_overlay`] reports.
#[tauri::command]
pub async fn ocr_selection_cancel(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    ensure_ocr_selection_window(&window)?;
    let app_handle = window.app_handle().clone();
    let service = state.ocr.clone();
    let emit = |status: &OcrStatus| emit_ocr_status(&app_handle, status);
    let hide = || hide_ocr_selection_overlay(&app_handle);

    match service
        .cancel_selection(&session_id, false, emit, hide)
        .await
    {
        Ok(_) => Ok(()),
        Err(reason) => Err(reason),
    }
}

/// Require the `ocr-selection` overlay as the caller of the OCR session IPCs.
///
/// The preview/ready handshake and the cancel/submit commands mutate the
/// guarded session and reveal or hide a window, so only the overlay webview
/// may drive them. A foreign caller gets a fixed safe error and no side effect.
fn ensure_ocr_selection_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == OCR_SELECTION_WINDOW_LABEL {
        Ok(())
    } else {
        tracing::warn!(
            label = window.label(),
            "Rejected OCR selection command from a non-overlay window"
        );
        Err("invalidCaller".to_string())
    }
}

/// Fast PNG encode of a frozen RGBA frame to a base64 payload.
///
/// Uses the explicit fast/sub PNG encoder instead of the crate default, whose
/// debug-mode compression is far too slow for a whole virtual desktop. Only the
/// encoder error is ever traced — never pixels, base64 or text.
fn encode_frame_png_base64(frame: &RgbaImage) -> Result<String, String> {
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Sub)
        .write_image(
            frame.as_raw(),
            frame.width(),
            frame.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| {
            tracing::warn!(error = %error, "OCR preview PNG encode failed");
            "previewEncodeFailed".to_string()
        })?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&png))
}

/// Cancel the exact matching selection and surface the fixed-safe one-shot
/// failure only when that session was actually cancelled.
///
/// Used by the preview-encode failure and the explicit overlay `failed`
/// handshake. A stale or already-presented id (per `only_unpresented`) is a
/// silent no-op that must never notify about — or cancel — a newer session; an
/// `Err` from the service means the hide failed AFTER the matching session was
/// already cancelled and dropped, so the failure is still surfaced.
async fn cancel_matching_selection_and_notify(
    app_handle: &AppHandle,
    service: &OcrService,
    session_id: &str,
    only_unpresented: bool,
) -> Result<(), String> {
    let emit = |status: &OcrStatus| emit_ocr_status(app_handle, status);
    let hide = || hide_ocr_selection_overlay(app_handle);
    match service
        .cancel_selection(session_id, only_unpresented, emit, hide)
        .await
    {
        Ok(false) => Ok(()),
        Ok(true) => {
            emit_ocr_one_shot_failed(app_handle, "overlayOpenFailed".to_string());
            Ok(())
        }
        Err(reason) => {
            emit_ocr_one_shot_failed(app_handle, "overlayOpenFailed".to_string());
            Err(reason)
        }
    }
}

/// Handler for the `ocr_selection_preview` command: fetch the frozen frame of
/// the current selection session and return it as an ephemeral PNG preview.
///
/// Invoked by the `ocr-selection` overlay once the SelectingArea status tells
/// it a frame is waiting. The SAME stored `Arc<RgbaImage>` that recognition
/// will later crop is encoded on a blocking worker (no pixels are copied) into
/// a camelCase DTO `{ sessionId, width, height, pngBase64 }`. If the session is
/// superseded while the encoder runs the result is dropped and `Ok(None)` is
/// returned, so a stale frame is never revealed. An encode failure cancels only
/// the matching unpresented session (surfacing `overlayOpenFailed` when it did)
/// and returns a fixed safe error. Nothing is shown, recaptured, cached or
/// served: the overlay reveals through `ocr_selection_ready` only after it has
/// decoded this payload.
#[tauri::command]
pub async fn ocr_selection_preview(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<PreviewDto>, String> {
    ensure_ocr_selection_window(&window)?;
    let app_handle = window.app_handle().clone();
    let service = state.ocr.clone();

    let Some(snapshot) = service.selection_snapshot().await else {
        return Ok(None);
    };
    let session_id = snapshot.id;
    let width = snapshot.frame.width();
    let height = snapshot.frame.height();

    let encode = {
        let frame = Arc::clone(&snapshot.frame);
        move || encode_frame_png_base64(&frame)
    };

    let png_base64 = match tokio::task::spawn_blocking(encode).await {
        Ok(Ok(png)) => png,
        Ok(Err(_)) => {
            // The encoder already traced its detail. Cancel only the matching
            // pending session and notify only if that cancel actually happened.
            let _ = cancel_matching_selection_and_notify(&app_handle, &service, &session_id, true)
                .await;
            return Err("previewEncodeFailed".to_string());
        }
        Err(join_error) => {
            tracing::warn!(error = %join_error, "OCR preview encode worker panicked");
            let _ = cancel_matching_selection_and_notify(&app_handle, &service, &session_id, true)
                .await;
            return Err("previewEncodeFailed".to_string());
        }
    };

    // Revalidate before handing pixels out: a session replaced while encoding
    // must never expose the stale frame as if it were the current one.
    match service.selection_snapshot().await {
        Some(current) if current.id == session_id => Ok(Some(PreviewDto {
            session_id,
            width,
            height,
            png_base64,
        })),
        _ => Ok(None),
    }
}

/// Handler for the `ocr_selection_ready` command: the overlay has decoded and
/// rendered the preview payload, so reveal it now.
///
/// `present_selection` atomically verifies that `session_id` is still the
/// current SelectingArea session under the transition lock (a stale or missing
/// id returns `Ok(false)` and shows/hides nothing), then shows and focuses the
/// overlay through the guarded callback. A failed reveal leaves the matching
/// session dropped — the service restores `Ready` — and surfaces the fixed-safe
/// `overlayOpenFailed`.
#[tauri::command]
pub async fn ocr_selection_ready(
    window: tauri::WebviewWindow,
    app_handle: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<bool, String> {
    ensure_ocr_selection_window(&window)?;
    let service = state.ocr.clone();
    let emit = |status: &OcrStatus| emit_ocr_status(&app_handle, status);
    let show = |_geometry: &VirtualScreenGeometry| present_ocr_selection_overlay(&window);

    match service.present_selection(&session_id, emit, show).await {
        Ok(presented) => Ok(presented),
        Err(reason) => {
            emit_ocr_one_shot_failed(&app_handle, "overlayOpenFailed".to_string());
            Err(reason)
        }
    }
}

/// Handler for the `ocr_selection_failed` command: the overlay could not decode
/// or render the preview, so drop the matching session.
///
/// Cancels exactly the session that failed through `cancel_selection(id,
/// only_unpresented: false)`. A stale id has no effect and never touches a
/// newer session; `overlayOpenFailed` is emitted only when a matching session
/// was actually cancelled (or the post-cancel hide failed).
#[tauri::command]
pub async fn ocr_selection_failed(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    ensure_ocr_selection_window(&window)?;
    let app_handle = window.app_handle().clone();
    cancel_matching_selection_and_notify(&app_handle, &state.ocr, &session_id, false).await
}

/// Handler for the `ocr_selection_submit` command invoked by the selection
/// overlay with the active `session_id` and the physical corners of the dragged
/// rectangle in the overlay's own client-area coordinates.
///
/// [`OcrService::finish_selection_for_session`] verifies identity and
/// presentation under the transition lock first, hides the overlay while still
/// serialized (so its pixels can never enter OCR input), then shifts the
/// corners into virtual-screen coordinates (by the saved session geometry
/// origin) and crops the saved frame for recognition through the stored
/// runtime. A too-small selection keeps the same session and is re-shown
/// through the guarded `present_selection` so a stale retry can never reopen a
/// window over a newer session. The recognized text is forwarded to the shared
/// incoming intake seam; failures surface only as the fixed safe reasons of
/// [`OCR_ONE_SHOT_FAILED_EVENT`].
#[tauri::command]
pub async fn ocr_selection_submit(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
) -> Result<(), String> {
    ensure_ocr_selection_window(&window)?;
    let app_handle = window.app_handle().clone();

    let service = state.ocr.clone();
    let emit = |status: &OcrStatus| emit_ocr_status(&app_handle, status);
    let hide = || hide_ocr_selection_overlay(&app_handle);
    let recognize = {
        let service = service.clone();
        move |image: Arc<RgbImage>| {
            let service = service.clone();
            async move {
                let runtime = service.runtime_slot().lock().await;
                let Some(runtime) = runtime.as_ref() else {
                    tracing::warn!("One-shot OCR runtime unavailable");
                    return Err("runtimeUnavailable".to_string());
                };
                match runtime.recognize(image).await {
                    Ok(result) => Ok(result),
                    Err(error) => {
                        // Only app-owned safe categories are traced; the caller
                        // always sees the fixed safe reason.
                        tracing::warn!(error = ?error, "One-shot OCR recognition failed");
                        Err("recognitionFailed".to_string())
                    }
                }
            }
        }
    };

    let selection = SelectionCorners { x1, y1, x2, y2 };
    let outcome = service
        .finish_selection_for_session(&session_id, selection, emit, hide, recognize)
        .await;

    match outcome {
        Ok(FinishOutcome::TooSmall) => {
            // The session stays active with its same id/frame: re-show the
            // overlay through the guarded present_selection and only then tell
            // it to re-arm, so a stale retry can never reopen anything over a
            // newer session (and never emits stale notifications).
            let emit = |status: &OcrStatus| emit_ocr_status(&app_handle, status);
            let show = |_geometry: &VirtualScreenGeometry| present_ocr_selection_overlay(&window);
            match service.present_selection(&session_id, emit, show).await {
                Ok(true) => {
                    let _ = window.emit(
                        OCR_SELECTION_REJECTED_EVENT,
                        OcrSelectionRejectedPayload {
                            session_id: session_id.clone(),
                        },
                    );
                    Ok(())
                }
                Ok(false) => Ok(()),
                Err(reason) => {
                    // No usable overlay: the session is already dropped by the
                    // service; surface only the fixed safe code.
                    emit_ocr_one_shot_failed(&app_handle, "overlayOpenFailed".to_string());
                    Err(reason)
                }
            }
        }
        Ok(FinishOutcome::Recognized(result)) => {
            // A normalized blank/whitespace-only result is not a real
            // recognition: surface exactly one fixed-safe `emptyResult`
            // failure, create no incoming item/job and return success to the
            // overlay. The text is never logged or exposed.
            if is_blank_ocr_text(&result.text) {
                emit_ocr_one_shot_failed(&app_handle, "emptyResult".to_string());
                return Ok(());
            }

            // Await the shared intake seam before returning. A delivery
            // failure (queue state unavailable, blank/overlong text or inbox
            // full) surfaces only as the fixed safe `intakeFailed` reason.
            match deliver_recognized_text(&app_handle, state.inner(), result.text).await {
                Ok(()) => Ok(()),
                Err(reason) => {
                    emit_ocr_one_shot_failed(&app_handle, reason);
                    Ok(())
                }
            }
        }
        Ok(FinishOutcome::RecognizeFailed(reason)) => {
            // Only the allowlisted `runtimeUnavailable` is preserved; every
            // other closure reason maps to the fixed safe `recognitionFailed`
            // so arbitrary strings are never emitted to the frontend.
            let reason = if reason == "runtimeUnavailable" {
                "runtimeUnavailable"
            } else {
                "recognitionFailed"
            };
            emit_ocr_one_shot_failed(&app_handle, reason.to_string());
            Ok(())
        }
        Ok(FinishOutcome::NoSession) => Ok(()),
        Err(reason) => {
            // finish_selection_for_session errors only when the overlay hide
            // failed after the identity check; the matching session is already
            // dropped and `Ready` restored. Preserve the overlayHideFailed code.
            emit_ocr_one_shot_failed(&app_handle, "overlayHideFailed".to_string());
            Err(reason)
        }
    }
}

fn register_ocr_capture(
    app_handle: &AppHandle,
    app_state: &AppState,
    hotkey: &Hotkey,
) -> Result<(), String> {
    let shortcut = hotkey
        .to_shortcut()
        .map_err(|e| format!("Invalid OCR hotkey: {e}"))?;
    let global_shortcut = app_handle.global_shortcut();

    let handle_clone = app_handle.clone();
    let state_clone = app_state.clone();
    global_shortcut
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            handle_ocr_capture(&handle_clone, &state_clone);
        })
        .map_err(|e| format!("Failed to register OCR hotkey: {e}"))
}

fn unregister_ocr_capture(app_handle: &AppHandle, hotkey: &Hotkey) -> Result<(), String> {
    let shortcut = hotkey
        .to_shortcut()
        .map_err(|e| format!("Invalid OCR hotkey: {e}"))?;
    let global_shortcut = app_handle.global_shortcut();
    if global_shortcut.is_registered(shortcut) {
        global_shortcut
            .unregister(shortcut)
            .map_err(|e| format!("Failed to unregister OCR hotkey: {e}"))?;
    }
    Ok(())
}

/// Whether a failed OCR start requires an untick persist, and the model id to
/// keep. Returns `Some(model_id)` when the feature is still enabled (the
/// checkbox must be cleared), `None` when it is already disabled.
fn failed_start_untick(enabled: bool, model_id: Option<String>) -> Option<Option<String>> {
    enabled.then_some(model_id)
}

/// Untick the OCR checkbox after a failed start and persist the setting.
///
/// The runtime start already published `Error`; this helper makes the persisted
/// `enabled` flag agree with that truth. `model_id` is kept as-is (a dangling id
/// is allowed per spec) so returning the files restores the feature on re-enable.
/// A persist failure is only warned about: the settings stay truthful (enabled,
/// error visible via status) until the next successful save.
async fn disable_ocr_after_failed_start(app_handle: &AppHandle, state: &AppState) {
    let service = &state.ocr;
    let (enabled, model_id) = {
        let settings = service.settings.read().await;
        (settings.enabled, settings.model_id.clone())
    };

    let Some(model_id) = failed_start_untick(enabled, model_id) else {
        return;
    };

    let Some(settings_manager) = app_handle.try_state::<SettingsManager>() else {
        tracing::warn!("SettingsManager unavailable while disabling OCR after failed start");
        return;
    };

    match super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_ocr_section(false, model_id)
    })
    .await
    {
        Ok(()) => {
            service.settings.write().await.enabled = false;
            super::emit_settings_changed(app_handle);
        }
        Err(error) => {
            tracing::warn!(error = %error, "Failed to persist OCR disable after failed start");
        }
    }
}

/// Start the OCR runtime against the current hotkey binding (shared seam used
/// by both the save command and startup boot-apply).
pub(crate) async fn start_ocr_runtime(app_handle: &AppHandle, state: &AppState, hotkey: &Hotkey) {
    // A disabled capture hotkey must never be registered; the caller either
    // skips OCR entirely (boot) or keeps the runtime untouched (settings save).
    if !hotkey.enabled {
        tracing::info!("OCR runtime start skipped: ocr_capture hotkey is disabled");
        return;
    }
    let service = &state.ocr;
    let emit = |status: &OcrStatus| emit_ocr_status(app_handle, status);
    let register = |hotkey: &Hotkey| register_ocr_capture(app_handle, state, hotkey);
    service.start(hotkey, emit, register).await;

    // A failed start must clear the persisted enable flag so the checkbox stays
    // truthful ("enabled" only while the feature actually works).
    if matches!(service.status(), OcrStatus::Error { .. }) {
        disable_ocr_after_failed_start(app_handle, state).await;
    }
}

/// Stop the OCR runtime, releasing the shortcut (shared seam used by both the
/// save command and app shutdown).
pub(crate) async fn stop_ocr_runtime(app_handle: &AppHandle, state: &AppState) {
    let service = &state.ocr;
    // Best-effort hide of the selection overlay: a stop during an active
    // `SelectingArea`/`Recognizing` session must not leave the overlay behind.
    let _ = hide_ocr_selection_overlay(app_handle);
    let emit = |status: &OcrStatus| emit_ocr_status(app_handle, status);
    let unregister = |hotkey: &Hotkey| unregister_ocr_capture(app_handle, hotkey);
    service.stop(emit, unregister).await;
    // A ready command that raced the stop may have revealed the overlay after
    // the early hide above but before the stop drained the session. Once the
    // stop completed there is no session left to present, so hide again: a
    // late-ready window must never stay visible.
    let _ = hide_ocr_selection_overlay(app_handle);
}

/// Stop the OCR runtime for app shutdown without blocking on an in-flight
/// recognition.
///
/// Clones the service into `stop_for_shutdown`, which defers the real stop to a
/// background task when a recognition holds the transition lock; app exit is
/// never delayed by OCR, and the process exit reclaims the runtime thread.
pub(crate) async fn stop_ocr_runtime_for_shutdown(app_handle: &AppHandle, state: &AppState) {
    let service = state.ocr.clone();
    let _ = hide_ocr_selection_overlay(app_handle);

    let emit_handle = app_handle.clone();
    let emit = move |status: &OcrStatus| emit_ocr_status(&emit_handle, status);
    let unregister_handle = app_handle.clone();
    let unregister = move |hotkey: &Hotkey| unregister_ocr_capture(&unregister_handle, hotkey);

    service.stop_for_shutdown(emit, unregister).await;
}

/// Whether a save that turns OCR on must be rejected because the capture
/// hotkey is switched off. Only an actual `false -> true` enable attempt is
/// blocked; OCR already enabled with a disabled hotkey may still save unrelated
/// settings (the Hotkeys panel warned that capture is unavailable).
fn ocr_enable_blocked(requested_enabled: bool, was_enabled: bool, hotkey_enabled: bool) -> bool {
    requested_enabled && !was_enabled && !hotkey_enabled
}

/// Re-register the OCR capture shortcut for a changed binding while `Ready`.
///
/// Shared rebind seam used by the hotkey save/reset commands. The OCR service
/// stays the single owner of the shortcut (register-new-first, release-old-
/// after, rollback on failure).
pub(crate) async fn rebind_ocr_capture_hotkey(
    app_handle: &AppHandle,
    state: &AppState,
    hotkey: &Hotkey,
) {
    let service = &state.ocr;
    let emit = |status: &OcrStatus| emit_ocr_status(app_handle, status);
    let register = |hotkey: &Hotkey| register_ocr_capture(app_handle, state, hotkey);
    let unregister = |hotkey: &Hotkey| unregister_ocr_capture(app_handle, hotkey);
    service
        .reregister_hotkey(hotkey, emit, register, unregister)
        .await;
}

/// Release the OCR capture shortcut while keeping the OCR runtime enabled.
///
/// Used by the per-hotkey enable command when `ocr_capture` is switched off.
pub(crate) async fn unregister_ocr_capture_while_running(app_handle: &AppHandle, state: &AppState) {
    let service = &state.ocr;
    let unregister = |hotkey: &Hotkey| unregister_ocr_capture(app_handle, hotkey);
    service.unregister_hotkey(unregister).await;
}

/// Register the OCR capture shortcut again while the OCR runtime is enabled.
///
/// Used by the per-hotkey enable command when `ocr_capture` is switched back
/// on. Registration failures are returned so the command can keep the switch
/// off instead of persisting an enabled hotkey that never registers.
pub(crate) async fn register_ocr_capture_while_running(
    app_handle: &AppHandle,
    state: &AppState,
    hotkey: &Hotkey,
) -> Result<(), String> {
    let service = &state.ocr;
    let register = |hotkey: &Hotkey| register_ocr_capture(app_handle, state, hotkey);
    let unregister = |hotkey: &Hotkey| unregister_ocr_capture(app_handle, hotkey);
    service.register_hotkey(hotkey, register, unregister).await
}

#[tauri::command]
pub fn get_ocr_settings(
    settings_manager: State<'_, SettingsManager>,
) -> Result<OcrSettings, String> {
    settings_manager
        .load()
        .map(|settings| settings.ocr)
        .map_err(|e| format!("Failed to load settings: {e}"))
}

#[tauri::command]
pub async fn save_ocr_settings(
    settings: OcrSettings,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let settings_manager = app_handle
        .try_state::<SettingsManager>()
        .ok_or_else(|| "SettingsManager not available".to_string())?;

    let service = state.ocr.clone();

    // Serialise the whole read-modify-write with `refresh_ocr_packs`: both
    // commands load the persisted OCR section, persist a reconcile result and
    // drive a lifecycle transition, and an interleaved pair could persist a
    // decision made from a snapshot the other command already replaced. The
    // guard also makes the `OcrTransition::Restart` stop+start below atomic.
    let _settings_guard = service.settings_write_lock().lock().await;

    let old_settings = service.settings.read().await.clone();
    let old_hotkey = service.registered_hotkey();

    // Read the current capture binding fresh from disk: the shared
    // `settings_cache` is only refreshed by unrelated save paths and would
    // serve a stale (default) binding here.
    let new_hotkey = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {e}"))?
        .hotkeys
        .ocr_capture;

    // Turning OCR on while its capture hotkey is switched off would enable a
    // feature that can never be triggered. Refuse before persisting so the
    // checkbox stays unticked and OcrPanel shows the error.
    if ocr_enable_blocked(settings.enabled, old_settings.enabled, new_hotkey.enabled) {
        return Err(
            "Нельзя включить OCR: горячая клавиша «OCR: захват области» выключена".to_string(),
        );
    }

    let settings_for_persist = settings.clone();
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.set_ocr_settings(settings_for_persist)
    })
    .await?;

    {
        let mut snapshot = service.settings.write().await;
        snapshot.enabled = settings.enabled;
        snapshot.model_id = settings.model_id.clone();
        snapshot.capture_target = settings.capture_target.clone();
    }

    // With the capture hotkey switched off OCR may stay enabled, but no
    // start/restart may silently register that disabled combination. Such
    // saves only update the persisted desired state; re-enabling the hotkey
    // later brings the shortcut (and a missing runtime) back.
    let decision = if settings.enabled && !new_hotkey.enabled {
        OcrTransition::Noop
    } else {
        decide_transition(
            &old_settings,
            &settings,
            old_hotkey.as_ref(),
            Some(&new_hotkey),
            &service.status(),
        )
    };

    match decision {
        OcrTransition::Noop => {}
        OcrTransition::Start => {
            start_ocr_runtime(&app_handle, state.inner(), &new_hotkey).await;
        }
        OcrTransition::Stop => {
            stop_ocr_runtime(&app_handle, state.inner()).await;
        }
        OcrTransition::Restart => {
            // Both halves run under the settings-write guard, so no other
            // settings-writing command can observe or act on the intermediate
            // `Disabled` state.
            stop_ocr_runtime(&app_handle, state.inner()).await;
            start_ocr_runtime(&app_handle, state.inner(), &new_hotkey).await;
        }
        OcrTransition::Retry => {
            start_ocr_runtime(&app_handle, state.inner(), &new_hotkey).await;
        }
        OcrTransition::ReregisterHotkey => {
            let service = &state.ocr;
            let emit = |status: &OcrStatus| emit_ocr_status(&app_handle, status);
            let register =
                |hotkey: &Hotkey| register_ocr_capture(&app_handle, state.inner(), hotkey);
            let unregister = |hotkey: &Hotkey| unregister_ocr_capture(&app_handle, hotkey);
            service
                .reregister_hotkey(&new_hotkey, emit, register, unregister)
                .await;
        }
    }

    super::emit_settings_changed(&app_handle);

    Ok(())
}

#[tauri::command]
pub fn get_ocr_status(state: State<'_, AppState>) -> OcrStatus {
    state.ocr.status()
}

#[tauri::command]
pub fn list_ocr_packs(state: State<'_, AppState>) -> Vec<OcrPackDto> {
    state
        .ocr
        .discover_packs()
        .into_iter()
        .map(|pack| OcrPackDto {
            id: pack.id,
            display_name: pack.display_name,
            languages: pack.languages,
        })
        .collect()
}

/// Fixed safe user-facing reason for a monitor inventory failure. Distinct from
/// [`capture_failed_reason`]: inventory is a user-facing list load, so a
/// genuinely empty inventory keeps the `noMonitors` meaning while every other
/// enumeration failure surfaces as `monitorListFailed` (never a raw OS error).
fn monitor_list_failed_reason(error: &CaptureError) -> &'static str {
    match error {
        CaptureError::NoMonitors => "noMonitors",
        _ => "monitorListFailed",
    }
}

/// Enumerate the currently attached displays for the OCR capture-target picker.
///
/// Runs on the blocking pool (enumeration touches Windows display APIs and the
/// xcap monitor list) and returns the safe [`MonitorInfo`] inventory DTO
/// (`devicePath`, `label`, `sourceName`, `isPrimary`, `geometry`). Every failure
/// surfaces a fixed safe reason — `noMonitors` for a genuinely empty inventory
/// and `monitorListFailed` for any enumeration error — so the frontend never
/// receives OS-level detail.
#[tauri::command]
pub async fn list_ocr_monitors() -> Result<Vec<MonitorInfo>, String> {
    let inventory = tokio::task::spawn_blocking(enumerate_monitors).await;

    match inventory {
        Ok(Ok(monitors)) => Ok(monitors),
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "OCR monitor inventory failed");
            Err(monitor_list_failed_reason(&error).to_string())
        }
        Err(join_error) => {
            tracing::warn!(error = %join_error, "OCR monitor inventory worker panicked");
            Err("monitorListFailed".to_string())
        }
    }
}

/// Re-scan the OCR packs roots and reconcile the persisted selection.
///
/// Uses the same merged discovery as the runtime (`discover_packs`), so the
/// reconcile decisions and the returned list always match what a start would
/// load, including models that only exist in the cwd root. The reconcile
/// follows the shared checkbox rule: when the selected model is
/// gone and not held in memory the feature is disabled with a save (auto-
/// selecting the single remaining model); a dangling selection with exactly one
/// pack auto-selects it without enabling; a model still held in memory is left
/// untouched. Returns the fresh pack list.
#[tauri::command]
pub async fn refresh_ocr_packs(
    state: State<'_, AppState>,
    settings_manager: State<'_, SettingsManager>,
    app_handle: AppHandle,
) -> Result<Vec<OcrPackDto>, String> {
    // Serialise with `save_ocr_settings`: the reconcile decision below is made
    // from the persisted snapshot and then persisted back, so a concurrent save
    // must not slip a different enabled/model pair in between.
    let service = state.ocr.clone();
    let _settings_guard = service.settings_write_lock().lock().await;

    let packs = service.discover_packs_blocking().await;
    let persisted = settings_manager
        .load()
        .map_err(|e| format!("Failed to load settings: {e}"))?
        .ocr;

    let pack_ids: Vec<String> = packs.iter().map(|p| p.id.clone()).collect();

    let holds_in_memory = {
        let snapshot = state.ocr.settings.read().await;
        state.ocr.is_runtime_active() && persisted.model_id.is_some() && *snapshot == persisted
    };

    let decision = decide_ocr_refresh_reconcile(
        persisted.enabled,
        persisted.model_id.as_deref(),
        &pack_ids,
        holds_in_memory,
    );

    match decision {
        OcrRefreshReconcile::None => {}
        OcrRefreshReconcile::Disable { model_id } => {
            let model_id_for_persist = model_id.clone();
            super::persist_blocking(settings_manager.inner(), move |mgr| {
                mgr.set_ocr_section(false, model_id_for_persist)
            })
            .await?;
            {
                let mut snapshot = state.ocr.settings.write().await;
                snapshot.enabled = false;
                snapshot.model_id = model_id;
            }
            // Stop unconditionally: `stop_ocr_runtime` serializes on the
            // service transition lock, so a stop issued while a concurrent
            // startup is still `Starting` waits for that start and tears down
            // whatever it installed. Gating on `is_runtime_active()` would
            // skip the stop while a startup is mid-flight (no shortcut
            // registered yet) and let that start publish `Ready` after
            // `enabled` was already persisted false.
            stop_ocr_runtime(&app_handle, state.inner()).await;
            super::emit_settings_changed(&app_handle);
        }
        OcrRefreshReconcile::AutoSelect(id) => {
            let id_for_persist = id.clone();
            super::persist_blocking(settings_manager.inner(), move |mgr| {
                mgr.set_ocr_section(false, Some(id_for_persist))
            })
            .await?;
            {
                let mut snapshot = state.ocr.settings.write().await;
                snapshot.enabled = false;
                snapshot.model_id = Some(id);
            }
            super::emit_settings_changed(&app_handle);
        }
    }

    Ok(packs
        .into_iter()
        .map(|pack| OcrPackDto {
            id: pack.id,
            display_name: pack.display_name,
            languages: pack.languages,
        })
        .collect())
}

/// Open the managed OCR packs root in the OS file manager.
///
/// The target is the managed state's `packs_root()` — the effective data root
/// («Данные программы»), deliberately not the higher-priority cwd discovery
/// root — so the frontend can never supply an arbitrary path. The root
/// directory is created when missing, canonicalized to a real platform path,
/// then revealed with the platform file manager (Explorer on Windows),
/// mirroring the existing `open_template_folder` command. Model-pack contents
/// are never scanned, downloaded, deleted or modified. Every failure is a
/// safe `Err(String)`.
#[tauri::command]
pub async fn open_ocr_packs_folder(state: State<'_, AppState>) -> Result<(), String> {
    let root = state.ocr.packs_root().to_path_buf();

    // Directory creation, canonicalization and the file-manager spawn are
    // blocking file-system/process work: run them on the blocking pool so a slow
    // or hung source (e.g. an unavailable network drive) never occupies a Tokio
    // worker.
    match tokio::task::spawn_blocking(move || reveal_ocr_packs_root(&root)).await {
        Ok(result) => result,
        Err(join_error) => {
            tracing::warn!(error = %join_error, "OCR packs folder worker panicked");
            Err("openFolderFailed".to_string())
        }
    }
}

/// Blocking body of [`open_ocr_packs_folder`]: create the managed packs root when
/// missing, canonicalize it and reveal it in the platform file manager.
fn reveal_ocr_packs_root(root: &Path) -> Result<(), String> {
    // Create the root directory first, before canonicalize.
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;

    let root = root
        .canonicalize()
        .map_err(|e| format!("Invalid OCR packs root: {e}"))?;

    let path = root.to_str().ok_or("Invalid path")?;

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args([path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args([path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .args([path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_blank_ocr_text;

    #[test]
    fn blank_and_whitespace_only_text_is_detected() {
        assert!(is_blank_ocr_text(""));
        assert!(is_blank_ocr_text(" "));
        assert!(is_blank_ocr_text("\t\n\u{00A0}"));
    }

    #[test]
    fn non_blank_text_is_not_detected() {
        assert!(!is_blank_ocr_text("привет"));
        assert!(!is_blank_ocr_text("  hello world  "));
        assert!(!is_blank_ocr_text("\n\nпервая строка\n\n"));
    }

    #[test]
    fn capture_failure_reason_maps_every_category() {
        use super::{capture_failed_reason, CaptureError};

        assert_eq!(
            capture_failed_reason(&CaptureError::NoMonitors),
            "noMonitors"
        );
        assert_eq!(
            capture_failed_reason(&CaptureError::MonitorUnavailable),
            "monitorUnavailable"
        );
        assert_eq!(
            capture_failed_reason(&CaptureError::CaptureFailed("boom".to_string())),
            "captureFailed"
        );
        assert_eq!(
            capture_failed_reason(&CaptureError::SelectionTooSmall),
            "captureFailed"
        );
    }

    #[test]
    fn preview_png_round_trips_dimensions_and_color() {
        use base64::Engine;

        let mut frame = image::RgbaImage::from_pixel(4, 3, image::Rgba([12, 34, 56, 255]));
        frame.put_pixel(3, 2, image::Rgba([200, 60, 5, 255]));

        let encoded = super::encode_frame_png_base64(&frame).expect("fast PNG encode");
        let png = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("valid base64");
        let decoded = image::load_from_memory(&png).expect("valid PNG").to_rgba8();

        assert_eq!(decoded.dimensions(), (4, 3), "dimensions must round-trip");
        assert_eq!(decoded.get_pixel(0, 0), &image::Rgba([12, 34, 56, 255]));
        assert_eq!(decoded.get_pixel(3, 2), &image::Rgba([200, 60, 5, 255]));
    }

    #[cfg(windows)]
    #[test]
    fn capture_exclusion_failure_is_non_fatal() {
        let mut called = false;
        super::apply_overlay_capture_exclusion(123, |hwnd, exclude| {
            called = true;
            assert_eq!(hwnd, 123);
            assert!(exclude);
            Err(anyhow::anyhow!("affinity denied"))
        });
        assert!(called, "exclude closure must be invoked");
    }

    #[test]
    fn failed_start_untick_only_when_enabled_and_keeps_model_id() {
        assert_eq!(
            super::failed_start_untick(true, Some("a".to_string())),
            Some(Some("a".to_string()))
        );
        assert_eq!(super::failed_start_untick(true, None), Some(None));
        assert_eq!(
            super::failed_start_untick(false, Some("a".to_string())),
            None
        );
    }

    #[test]
    fn failed_start_disable_persists_enabled_false_with_model_id_kept() {
        use super::SettingsManager;

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-ocr-untick-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = SettingsManager::with_config_dir(dir.clone()).unwrap();

        manager
            .set_ocr_section(true, Some("com.example.ocr".to_string()))
            .unwrap();

        // The failed-start untick persists enabled=false and keeps the model id.
        manager
            .set_ocr_section(false, Some("com.example.ocr".to_string()))
            .unwrap();

        let after = manager.load().unwrap();
        assert!(!after.ocr.enabled);
        assert_eq!(after.ocr.model_id.as_deref(), Some("com.example.ocr"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enable_is_blocked_only_on_false_to_true_with_disabled_hotkey() {
        assert!(super::ocr_enable_blocked(true, false, false));
        // OCR already enabled with the hotkey off: unrelated saves still work.
        assert!(!super::ocr_enable_blocked(true, true, false));
        // Hotkey enabled: the transition is allowed.
        assert!(!super::ocr_enable_blocked(true, false, true));
        // Turning OCR off is always allowed.
        assert!(!super::ocr_enable_blocked(false, true, false));
        assert!(!super::ocr_enable_blocked(false, false, false));
    }
}
