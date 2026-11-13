//! Graceful application shutdown.
//!
//! [`coordinate_shutdown`] is the single coordination point for every quit path
//! (tray menu, main-window close, `quit_app` command, OS exit request). It
//! flushes the editor tabs, cancels the shared shutdown token, stops the
//! long-lived network services and finally exits the process.
//!
//! The module lives outside `commands` because shutdown is not an IPC concern:
//! `commands::quit_app` stays a thin adapter over this entry point.

use crate::commands::persist_blocking;
use crate::commands::telegram::TelegramState;
use crate::config::WindowsManager;
use crate::state::AppState;
use tauri::{AppHandle, Emitter, EventTarget, Listener, Manager};
use tracing::{error, info, warn};

/// Emitted to the main webview to request a final editor-tab flush before the
/// backend tears down its services on shutdown.
const TABS_FLUSH_REQUEST_EVENT: &str = "tabs-flush-request";
/// Emitted by the frontend to acknowledge the flush request. The payload echoes
/// the request id so stale or unrelated acknowledgements are ignored.
const TABS_FLUSH_ACK_EVENT: &str = "tabs-flush-ack";
/// Upper bound for waiting on the frontend acknowledgement. Shutdown never
/// blocks past this even when the WebView is missing or broken.
const TABS_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
/// Upper bound for stopping the long-lived network services (VTube Studio and
/// Telegram). A stuck disconnect must never keep the process alive, so the wait
/// is bounded instead of sleeping for a fixed grace period.
const SERVICE_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Payload for the flush request sent to the main webview.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TabsFlushRequestPayload {
    request_id: String,
}

/// Payload for the flush acknowledgement emitted by the frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TabsFlushAckPayload {
    request_id: String,
    ok: bool,
}

/// Match a raw acknowledgement payload against the expected request id.
///
/// Returns `Some(ok)` only for a well-formed acknowledgement that echoes the
/// expected id; malformed payloads and stale/unrelated acknowledgements yield
/// `None` so they are ignored.
fn match_tabs_flush_ack(payload: &str, request_id: &str) -> Option<bool> {
    let ack: TabsFlushAckPayload = serde_json::from_str(payload).ok()?;
    (ack.request_id == request_id).then_some(ack.ok)
}

/// The outcome of awaiting a frontend flush acknowledgement.
enum TabFlushAck {
    /// Frontend acknowledged a successful flush.
    Success,
    /// Frontend acknowledged, but reported a save failure.
    Failure,
    /// No acknowledgement within the bounded window.
    Timeout,
    /// The acknowledgement channel closed without a value.
    Closed,
}

/// Await an acknowledgement on the given channel, bounded by `timeout`.
async fn await_tabs_flush_ack(
    rx: tokio::sync::oneshot::Receiver<bool>,
    timeout: std::time::Duration,
) -> TabFlushAck {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(true)) => TabFlushAck::Success,
        Ok(Ok(false)) => TabFlushAck::Failure,
        Ok(Err(_)) => TabFlushAck::Closed,
        Err(_) => TabFlushAck::Timeout,
    }
}

/// Ask the main webview to flush its editor tabs and wait for its
/// acknowledgement before tearing services down.
///
/// Best-effort: without a main window the flush is skipped, and a missing or
/// broken frontend is bounded by [`TABS_FLUSH_TIMEOUT`]. The acknowledgement
/// listener is registered before the request is emitted and removed afterwards
/// so it never outlives this single shutdown request.
async fn request_tab_flush(app_handle: &AppHandle) {
    let Some(main_window) = app_handle.get_webview_window("main") else {
        info!("request_tab_flush: no main window, skipping flush");
        return;
    };

    let request_id = uuid::Uuid::new_v4().to_string();

    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    let expected = request_id.clone();
    // Scope the acknowledgement listener to the main webview window so unrelated
    // webviews can never satisfy this shutdown request.
    let listener_id = main_window.listen(TABS_FLUSH_ACK_EVENT, move |event| {
        if let Some(ok) = match_tabs_flush_ack(event.payload(), &expected) {
            let mut guard = match tx.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(tx) = guard.take() {
                let _ = tx.send(ok);
            }
        }
    });

    // `WebviewWindow::emit` broadcasts globally; target the main window explicitly
    // so only its frontend subscription receives the flush request.
    if let Err(error) = app_handle.emit_to(
        EventTarget::webview_window("main"),
        TABS_FLUSH_REQUEST_EVENT,
        TabsFlushRequestPayload { request_id },
    ) {
        warn!("request_tab_flush: failed to emit flush request: {error}");
        main_window.unlisten(listener_id);
        return;
    }

    let outcome = await_tabs_flush_ack(rx, TABS_FLUSH_TIMEOUT).await;
    main_window.unlisten(listener_id);

    match outcome {
        TabFlushAck::Success => info!("request_tab_flush: frontend acknowledged successful flush"),
        TabFlushAck::Failure => warn!("request_tab_flush: frontend reported a flush failure"),
        TabFlushAck::Timeout => {
            warn!("request_tab_flush: timeout waiting for flush acknowledgement")
        }
        TabFlushAck::Closed => {
            warn!("request_tab_flush: acknowledgement channel closed unexpectedly")
        }
    }
}

/// Stop the long-lived network services that must not be left to process exit.
///
/// VTube Studio and Telegram own sockets and remote sessions that are torn down
/// explicitly. Both teardowns run concurrently and the whole phase is bounded by
/// [`SERVICE_STOP_TIMEOUT`], so a stuck disconnect cannot delay application exit
/// indefinitely (this replaces the former blind grace-period sleep).
async fn stop_network_services(app_handle: &AppHandle, state: &AppState) {
    let vtube_studio = state.vtube_studio.clone();
    let telegram_state = app_handle.try_state::<TelegramState>();

    let stop = async {
        let vtube = async {
            vtube_studio.disconnect().await;
            info!("coordinate_shutdown: VTube Studio disconnected");
        };
        let telegram = async {
            let Some(telegram_state) = telegram_state.as_ref() else {
                warn!("coordinate_shutdown: TelegramState not available, skipping disconnect");
                return;
            };
            let Some(client) = telegram_state.clear_client().await else {
                return;
            };
            match client.disconnect().await {
                Ok(_) => info!("coordinate_shutdown: Telegram client disconnected"),
                Err(error) => warn!("coordinate_shutdown: Telegram disconnect failed: {error}"),
            }
        };
        tokio::join!(vtube, telegram);
    };

    match tokio::time::timeout(SERVICE_STOP_TIMEOUT, stop).await {
        Ok(()) => info!("coordinate_shutdown: network services stopped"),
        Err(_) => warn!(
            timeout = ?SERVICE_STOP_TIMEOUT,
            "coordinate_shutdown: network services did not stop in time, exiting anyway"
        ),
    }
}

/// Coordinate a single graceful shutdown across all quit entry points.
///
/// Persists the main window position, stops the keyboard hook, flushes the editor
/// tabs, cancels the shutdown token, stops the network services with bounded
/// waits, notifies the WebView and emits `app-exit` before exiting. Only the
/// first call wins; subsequent calls are ignored via `begin_shutdown`.
pub(crate) async fn coordinate_shutdown(app_handle: AppHandle) {
    let Some(state) = app_handle.try_state::<AppState>() else {
        error!("coordinate_shutdown: AppState not available, exiting immediately");
        app_handle.exit(0);
        return;
    };

    if !state.begin_shutdown() {
        info!("Shutdown already in progress - ignoring duplicate request");
        return;
    }

    if let Some(windows_manager) = app_handle.try_state::<WindowsManager>() {
        if let Some(main_window) = app_handle.get_webview_window("main") {
            if let Ok(pos) = main_window.outer_position() {
                let x = pos.x;
                let y = pos.y;
                info!(x, y, "Saving main window position");
                let wm = windows_manager.inner();
                let _ =
                    persist_blocking(wm, move |mgr| mgr.set_main_position(Some(x), Some(y))).await;
            }
        }
    }

    {
        let mut hook_guard = state.soundpanel_hook.lock();
        if let Some(ref mut hook_manager) = *hook_guard {
            hook_manager.stop();
        }
        *hook_guard = None;
    }

    // Flush editor tabs in the main webview before cancelling services so the
    // latest tab state is persisted for the quit_app/ExitRequested path. The
    // window close path already awaits its own flush via onCloseRequested.
    request_tab_flush(&app_handle).await;

    state.shutdown.cancel();
    info!("Shutdown token cancelled — all servers notified");

    // Stop the OCR runtime without waiting for an in-flight recognition: the
    // shutdown stop defers the real stop to a background task when the
    // transition lock is held, so app exit is never delayed by OCR.
    crate::commands::ocr::stop_ocr_runtime_for_shutdown(&app_handle, state.inner()).await;

    // Wait for the registered network service teardowns with a bounded timeout
    // instead of a blind grace-period sleep.
    stop_network_services(&app_handle, state.inner()).await;

    state.webview.send_event(crate::events::AppEvent::Quit);

    let _ = app_handle.emit("app-exit", ());

    app_handle.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── tabs flush acknowledgement matching ──

    #[test]
    fn tabs_flush_ack_accepts_matching_id() {
        assert_eq!(
            match_tabs_flush_ack(r#"{"request_id":"abc","ok":true}"#, "abc"),
            Some(true)
        );
        assert_eq!(
            match_tabs_flush_ack(r#"{"request_id":"abc","ok":false}"#, "abc"),
            Some(false)
        );
    }

    #[test]
    fn tabs_flush_ack_ignores_mismatched_id() {
        assert_eq!(
            match_tabs_flush_ack(r#"{"request_id":"other","ok":true}"#, "abc"),
            None
        );
    }

    #[test]
    fn tabs_flush_ack_ignores_malformed_payload() {
        assert_eq!(match_tabs_flush_ack("not json", "abc"), None);
        assert_eq!(match_tabs_flush_ack(r#"{"ok":true}"#, "abc"), None);
        assert_eq!(
            match_tabs_flush_ack(r#"{"request_id":123,"ok":true}"#, "abc"),
            None
        );
        assert_eq!(match_tabs_flush_ack(r#"{"request_id":"abc"}"#, "abc"), None);
    }

    // ── tabs flush bounded wait ──

    #[tokio::test]
    async fn tabs_flush_await_resolves_success_and_failure() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        tx.send(true).unwrap();
        assert!(matches!(
            await_tabs_flush_ack(rx, std::time::Duration::from_secs(1)).await,
            TabFlushAck::Success
        ));

        let (tx, rx) = tokio::sync::oneshot::channel();
        tx.send(false).unwrap();
        assert!(matches!(
            await_tabs_flush_ack(rx, std::time::Duration::from_secs(1)).await,
            TabFlushAck::Failure
        ));
    }

    #[tokio::test]
    async fn tabs_flush_await_times_out_when_sender_stays_silent() {
        // Keep the sender alive so the channel is not closed; the bounded wait
        // must elapse rather than resolving early.
        let (_tx, rx) = tokio::sync::oneshot::channel();
        assert!(matches!(
            await_tabs_flush_ack(rx, std::time::Duration::from_millis(10)).await,
            TabFlushAck::Timeout
        ));
    }

    #[tokio::test]
    async fn tabs_flush_await_detects_closed_channel() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        drop(tx);
        assert!(matches!(
            await_tabs_flush_ack(rx, std::time::Duration::from_secs(1)).await,
            TabFlushAck::Closed
        ));
    }
}
