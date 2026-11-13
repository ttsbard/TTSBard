use crate::config::replace_file_atomically;
use crate::preprocessor::{replacements_file, usernames_file};
use crate::state::AppState;
use std::fs;
use tauri::{AppHandle, State};

/// Read one of the preprocessor list files.
///
/// A missing file means "empty list": the UI then shows an empty editor instead
/// of an error.
fn read_list_file(
    path: anyhow::Result<std::path::PathBuf>,
    path_error: &str,
    read_error: &str,
) -> Result<String, String> {
    let path = path.map_err(|e| format!("{}: {}", path_error, e))?;

    if path.exists() {
        fs::read_to_string(&path).map_err(|e| format!("{}: {}", read_error, e))
    } else {
        Ok(String::new())
    }
}

/// Atomically replace one of the preprocessor list files.
///
/// The write goes through the shared config write lock and Windows-safe atomic
/// replace, so a concurrent save cannot truncate the file and a crash mid-write
/// leaves the previous content intact.
fn store_list_file(
    path: anyhow::Result<std::path::PathBuf>,
    path_error: &str,
    write_error: &str,
    content: &str,
) -> Result<(), String> {
    let path = path.map_err(|e| format!("{}: {}", path_error, e))?;

    // Create parent directory if it doesn't exist
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let _write_lock = crate::config::config_write_lock().lock();
    replace_file_atomically(&path, content.as_bytes())
        .map_err(|e| format!("{}: {}", write_error, e))
}

/// Get the current replacements list content
#[tauri::command]
pub async fn get_replacements() -> Result<String, String> {
    tokio::task::spawn_blocking(|| {
        read_list_file(
            replacements_file(),
            "Failed to get replacements file path",
            "Failed to read replacements file",
        )
    })
    .await
    .map_err(|e| format!("Failed to read replacements file: {}", e))?
}

/// Save the replacements list content
#[tauri::command]
pub async fn save_replacements(
    content: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let editor = state.editor.clone();
    tokio::task::spawn_blocking(move || {
        store_list_file(
            replacements_file(),
            "Failed to get replacements file path",
            "Failed to write replacements file",
            &content,
        )?;

        // Reload preprocessor in state
        editor.reload_preprocessor();

        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Failed to write replacements file: {}", e))??;

    super::emit_settings_changed(&app_handle);

    Ok(())
}

/// Get the current usernames list content
#[tauri::command]
pub async fn get_usernames() -> Result<String, String> {
    tokio::task::spawn_blocking(|| {
        read_list_file(
            usernames_file(),
            "Failed to get usernames file path",
            "Failed to read usernames file",
        )
    })
    .await
    .map_err(|e| format!("Failed to read usernames file: {}", e))?
}

/// Save the usernames list content
#[tauri::command]
pub async fn save_usernames(
    content: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let editor = state.editor.clone();
    tokio::task::spawn_blocking(move || {
        store_list_file(
            usernames_file(),
            "Failed to get usernames file path",
            "Failed to write usernames file",
            &content,
        )?;

        // Reload preprocessor in state
        editor.reload_preprocessor();

        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Failed to write usernames file: {}", e))??;

    super::emit_settings_changed(&app_handle);

    Ok(())
}

/// Preview preprocessed text (for UI testing)
///
/// Uses the preprocessor cached in [`crate::editor::EditorService`] instead of
/// re-reading the list files on every preview request.
#[tauri::command]
pub async fn preview_preprocessing(
    text: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let editor = state.editor.clone();
    tokio::task::spawn_blocking(move || {
        let preprocessor = editor
            .get_preprocessor()
            .ok_or_else(|| "Failed to load preprocessor".to_string())?;

        Ok(preprocessor.process(&text))
    })
    .await
    .map_err(|e| format!("Failed to load preprocessor: {}", e))?
}

/// Load preprocessor data for live replacement in UI
/// Returns a struct with replacements and usernames as HashMaps
///
/// Served from the [`crate::editor::EditorService`] cache, like
/// [`preview_preprocessing`], so UI reads do not touch the disk.
#[tauri::command]
pub async fn load_preprocessor_data(
    state: State<'_, AppState>,
) -> Result<PreprocessorData, String> {
    let editor = state.editor.clone();
    tokio::task::spawn_blocking(move || {
        let preprocessor = editor
            .get_preprocessor()
            .ok_or_else(|| "Failed to load preprocessor".to_string())?;

        Ok(PreprocessorData {
            replacements: preprocessor.get_replacements_map().clone(),
            usernames: preprocessor.get_usernames_map().clone(),
        })
    })
    .await
    .map_err(|e| format!("Failed to load preprocessor: {}", e))?
}

/// Struct to hold preprocessor data for UI
#[derive(serde::Serialize)]
pub struct PreprocessorData {
    pub replacements: std::collections::HashMap<String, String>,
    pub usernames: std::collections::HashMap<String, String>,
}
