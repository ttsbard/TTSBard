use crate::spellcheck::{SpellResult, SpellcheckManager};
use std::sync::Arc;
use tauri::State;

pub struct SpellcheckState(pub Arc<SpellcheckManager>);

/// Spellchecking walks a Hunspell dictionary and can be slow for long texts,
/// so the work runs on the blocking pool instead of the async executor.
#[tauri::command]
pub async fn spellcheck(
    words: Vec<String>,
    state: State<'_, SpellcheckState>,
) -> Result<Vec<SpellResult>, String> {
    let manager = state.0.clone();
    if !manager.is_available() {
        return Err("dictionary unavailable".into());
    }
    tokio::task::spawn_blocking(move || manager.check_words(&words))
        .await
        .map_err(|e| format!("spellcheck task failed: {}", e))
}
