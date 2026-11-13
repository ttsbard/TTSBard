use crate::history::{HistoryEntry, HistoryManager, PhraseEntry, PhraseSuggestion};
use std::sync::Arc;
use tauri::State;

pub struct HistoryState(pub Arc<HistoryManager>);

const HISTORY_PERSIST_FAILED: &str = "history.persist_failed";

#[tauri::command]
pub fn get_history_suggestions(
    query: String,
    limit: Option<usize>,
    history_state: State<'_, HistoryState>,
) -> Result<Vec<HistoryEntry>, String> {
    let limit = limit.unwrap_or(10);
    let manager = &history_state.0;
    Ok(manager.suggest(&query, limit))
}

#[tauri::command]
pub fn get_phrase_completion(
    context: String,
    limit: Option<usize>,
    history_state: State<'_, HistoryState>,
) -> Result<Vec<PhraseSuggestion>, String> {
    let limit = limit.unwrap_or(5);
    let manager = &history_state.0;
    Ok(manager.suggest_phrase(&context, limit))
}

/// History mutations mutate in-memory state and then flush JSON to disk, so
/// every write runs on the blocking pool instead of the async executor.
#[tauri::command]
pub async fn record_history(
    text: String,
    history_state: State<'_, HistoryState>,
) -> Result<(), String> {
    let manager = history_state.0.clone();
    tokio::task::spawn_blocking(move || manager.record_text(&text))
        .await
        .map_err(|e| {
            tracing::error!("Failed to persist input history: {:#}", e);
            HISTORY_PERSIST_FAILED.to_string()
        })?
        .map_err(|e| {
            tracing::error!("Failed to persist input history: {:#}", e);
            HISTORY_PERSIST_FAILED.to_string()
        })
}

#[tauri::command]
pub async fn clear_history(history_state: State<'_, HistoryState>) -> Result<(), String> {
    let manager = history_state.0.clone();
    tokio::task::spawn_blocking(move || manager.clear())
        .await
        .map_err(|e| {
            tracing::error!("Failed to persist input history clear: {:#}", e);
            HISTORY_PERSIST_FAILED.to_string()
        })?
        .map_err(|e| {
            tracing::error!("Failed to persist input history clear: {:#}", e);
            HISTORY_PERSIST_FAILED.to_string()
        })
}

#[tauri::command]
pub fn get_phrase_history(
    filter: Option<String>,
    limit: Option<usize>,
    history_state: State<'_, HistoryState>,
) -> Result<Vec<PhraseEntry>, String> {
    let limit = limit.unwrap_or(100);
    let manager = &history_state.0;
    Ok(manager.get_phrases(filter.as_deref(), limit))
}

#[tauri::command]
pub async fn delete_phrase_history(
    id: String,
    history_state: State<'_, HistoryState>,
) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("Phrase id cannot be empty".to_string());
    }
    let manager = history_state.0.clone();
    tokio::task::spawn_blocking(move || manager.delete_phrase(&id))
        .await
        .map_err(|e| {
            tracing::error!("Failed to persist phrase deletion: {:#}", e);
            HISTORY_PERSIST_FAILED.to_string()
        })?
        .map_err(|e| {
            tracing::error!("Failed to persist phrase deletion: {:#}", e);
            HISTORY_PERSIST_FAILED.to_string()
        })
}

#[tauri::command]
pub async fn clear_phrase_history(history_state: State<'_, HistoryState>) -> Result<(), String> {
    let manager = history_state.0.clone();
    tokio::task::spawn_blocking(move || manager.clear_phrases())
        .await
        .map_err(|e| {
            tracing::error!("Failed to persist phrase clear: {:#}", e);
            HISTORY_PERSIST_FAILED.to_string()
        })?
        .map_err(|e| {
            tracing::error!("Failed to persist phrase clear: {:#}", e);
            HISTORY_PERSIST_FAILED.to_string()
        })
}

#[tauri::command]
pub fn replay_phrase_from_cache(
    phrase_id: String,
    history_state: State<'_, HistoryState>,
    state: State<'_, crate::state::AppState>,
) -> Result<(), String> {
    let entry = history_state
        .0
        .get_phrase_by_id(&phrase_id)
        .ok_or_else(|| "CacheMiss".to_string())?;

    if entry.cache_key.is_empty() {
        return Err("CacheMiss".to_string());
    }

    let pcm = crate::history::read_audio_cache(&entry.cache_key).map_err(|e| {
        if e.to_string().contains("CacheMiss") {
            "CacheMiss".to_string()
        } else {
            e.to_string()
        }
    })?;

    let pb = state.playback_manager.lock();
    let pb = pb
        .as_ref()
        .ok_or_else(|| "Playback manager not initialized".to_string())?;

    // Получаем текущие настройки из кэша
    let settings = state.settings_cache.read().clone();

    // Вычисляем live-настройки вывода
    let (speaker_config, mic_config) = crate::commands::tts_pipeline::compute_output_configs(
        &settings.audio,
        &settings.audio_effects,
    );

    let replay_id = format!("hist_{}", entry.cache_key);
    let enqueued = pb.enqueue_with_outputs(
        replay_id.clone(),
        entry.insert_text.clone(),
        pcm,
        speaker_config,
        mic_config,
    );

    if !enqueued {
        return Err("Playback queue full".to_string());
    }

    Ok(())
}
