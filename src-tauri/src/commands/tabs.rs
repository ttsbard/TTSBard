use crate::tabs::{TabManager, TabsData};
use std::sync::Arc;
use tauri::State;

pub struct TabsState(pub Arc<TabManager>);

const TABS_PERSIST_FAILED: &str = "tabs.persist_failed";

#[tauri::command]
pub fn get_tabs(state: State<'_, TabsState>) -> TabsData {
    state.0.load_all()
}

/// Persisting tabs does a synchronous atomic file replace, so it runs on the
/// blocking pool and never on the async executor.
#[tauri::command]
pub async fn save_tabs(state: State<'_, TabsState>, data: TabsData) -> Result<(), String> {
    let manager = state.0.clone();
    tokio::task::spawn_blocking(move || manager.save_all(data))
        .await
        .map_err(|e| {
            tracing::error!("Failed to persist tabs: {:#}", e);
            TABS_PERSIST_FAILED.to_string()
        })?
        .map_err(|e| {
            tracing::error!("Failed to persist tabs: {:#}", e);
            TABS_PERSIST_FAILED.to_string()
        })
}
