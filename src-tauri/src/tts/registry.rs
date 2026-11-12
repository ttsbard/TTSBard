use crate::tts::TtsProvider;

#[derive(Clone, Debug)]
pub struct TtsProviderEntry {
    pub id: String,
    pub display_name: String,
    pub provider: TtsProvider,
}

#[derive(Clone, Debug)]
pub struct TtsProviderRegistry {
    entries: Vec<TtsProviderEntry>,
    active_id: Option<String>,
}

impl TtsProviderRegistry {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            active_id: None,
        }
    }

    pub fn add_or_replace(&mut self, entry: TtsProviderEntry) {
        if let Some(pos) = self.entries.iter().position(|e| e.id == entry.id) {
            self.entries[pos] = entry;
        } else {
            self.entries.push(entry);
        }
    }

    pub fn get(&self, id: &str) -> Option<&TtsProviderEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut TtsProviderEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &TtsProviderEntry> {
        self.entries.iter()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn remove(&mut self, id: &str) -> bool {
        if self.active_id.as_deref() == Some(id) {
            self.active_id = None;
        }
        let pos = self.entries.iter().position(|e| e.id == id);
        if let Some(pos) = pos {
            self.entries.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn select(&mut self, id: &str) -> Result<(), String> {
        if self.entries.iter().any(|e| e.id == id) {
            self.active_id = Some(id.to_string());
            Ok(())
        } else {
            Err(format!("no provider with id '{}'", id))
        }
    }

    #[cfg(test)]
    pub fn select_or_first(&mut self, id: &str) {
        if self.entries.iter().any(|e| e.id == id) {
            self.active_id = Some(id.to_string());
        } else if let Some(first) = self.entries.first() {
            self.active_id = Some(first.id.clone());
        }
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active_id.as_deref()
    }

    pub fn active(&self) -> Option<&TtsProviderEntry> {
        self.active_id
            .as_ref()
            .and_then(|id| self.entries.iter().find(|e| e.id == *id))
    }

    #[cfg(test)]
    pub fn active_or_first(&self) -> Option<&TtsProviderEntry> {
        self.active().or_else(|| self.entries.first())
    }

    /// Restore a saved provider ID at startup with safe fallback.
    /// - If `saved_id` exists in the registry → select it.
    /// - If `saved_id` is missing (deleted Piper model, etc.) and nothing is
    ///   active → select the first registered provider.
    /// - If `saved_id` is missing and a built-in provider is already active →
    ///   leave it alone (never auto-select Piper just because it was discovered).
    /// - If `saved_id` is `None` and nothing is active → select the first provider.
    /// - If `saved_id` is `None` and something is already active → leave it.
    pub fn restore_saved_or_first(&mut self, saved_id: Option<&str>) {
        match saved_id {
            Some(id) => {
                if self.entries.iter().any(|e| e.id == id) {
                    self.active_id = Some(id.to_string());
                } else if self.active_id.is_none() {
                    if let Some(first) = self.entries.first() {
                        self.active_id = Some(first.id.clone());
                    }
                }
            }
            None => {
                if self.active_id.is_none() {
                    if let Some(first) = self.entries.first() {
                        self.active_id = Some(first.id.clone());
                    }
                }
            }
        }
    }
}

impl Default for TtsProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::piper::runtime::LocalModelTts;
    use std::sync::Arc;

    fn dummy_provider() -> TtsProvider {
        let tts = LocalModelTts::new("/dummy/model.onnx", "/dummy/model.onnx.json");
        TtsProvider::Piper(Arc::new(tts))
    }

    fn entry(id: &str, name: &str) -> TtsProviderEntry {
        TtsProviderEntry {
            id: id.to_string(),
            display_name: name.to_string(),
            provider: dummy_provider(),
        }
    }

    #[test]
    fn empty_registry() {
        let reg = TtsProviderRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn insert_one_entry() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("piper-en", "English Piper"));
        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());
    }

    #[test]
    fn insert_and_retrieve() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("openai", "OpenAI TTS"));
        let retrieved = reg.get("openai");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().display_name, "OpenAI TTS");
    }

    #[test]
    fn lookup_missing_id() {
        let reg = TtsProviderRegistry::new();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn duplicate_id_replaces() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("piper-en", "English Piper"));
        reg.add_or_replace(entry("piper-en", "English Piper v2"));
        assert_eq!(reg.len(), 1);
        assert_eq!(
            reg.get("piper-en").unwrap().display_name,
            "English Piper v2"
        );
    }

    #[test]
    fn deterministic_order() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("b", "B"));
        reg.add_or_replace(entry("a", "A"));
        reg.add_or_replace(entry("c", "C"));
        let ids: Vec<&str> = reg.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a", "c"]);
    }

    #[test]
    fn remove_entry() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("keep", "Keep"));
        reg.add_or_replace(entry("remove", "Remove"));
        assert!(reg.remove("remove"));
        assert_eq!(reg.len(), 1);
        assert!(reg.get("remove").is_none());
    }

    #[test]
    fn remove_nonexistent() {
        let mut reg = TtsProviderRegistry::new();
        assert!(!reg.remove("nowhere"));
    }

    #[test]
    fn iter_over_multiple() {
        let mut reg = TtsProviderRegistry::new();
        for i in 0..5 {
            reg.add_or_replace(entry(&format!("id-{}", i), &format!("Name {}", i)));
        }
        let count = reg.iter().count();
        assert_eq!(count, 5);
    }

    #[test]
    fn active_id_none_initially() {
        let reg = TtsProviderRegistry::new();
        assert!(reg.active_id().is_none());
        assert!(reg.active().is_none());
    }

    #[test]
    fn select_existing_id() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("a", "Alpha"));
        reg.add_or_replace(entry("b", "Beta"));
        assert!(reg.select("a").is_ok());
        assert_eq!(reg.active_id(), Some("a"));
        assert_eq!(reg.active().unwrap().display_name, "Alpha");
    }

    #[test]
    fn select_missing_id_returns_error() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("a", "Alpha"));
        let err = reg.select("missing").unwrap_err();
        assert!(err.contains("missing"));
        assert!(reg.active_id().is_none());
    }

    #[test]
    fn select_or_first_missing_falls_back() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("a", "Alpha"));
        reg.add_or_replace(entry("b", "Beta"));
        reg.select_or_first("nonexistent");
        assert_eq!(reg.active_id(), Some("a"));
    }

    #[test]
    fn select_or_first_empty_registry() {
        let mut reg = TtsProviderRegistry::new();
        reg.select_or_first("anything");
        assert!(reg.active_id().is_none());
    }

    #[test]
    fn active_or_first_returns_active_when_set() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("a", "Alpha"));
        reg.add_or_replace(entry("b", "Beta"));
        reg.select("b").unwrap();
        let entry = reg.active_or_first().unwrap();
        assert_eq!(entry.id, "b");
    }

    #[test]
    fn active_or_first_falls_back_when_no_active() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("a", "Alpha"));
        reg.add_or_replace(entry("b", "Beta"));
        let entry = reg.active_or_first().unwrap();
        assert_eq!(entry.id, "a");
    }

    #[test]
    fn active_or_first_empty_registry() {
        let reg = TtsProviderRegistry::new();
        assert!(reg.active_or_first().is_none());
    }

    #[test]
    fn replace_active_keeps_selection() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("x", "Original"));
        reg.select("x").unwrap();
        reg.add_or_replace(entry("x", "Updated"));
        assert_eq!(reg.active_id(), Some("x"));
        assert_eq!(reg.active().unwrap().display_name, "Updated");
    }

    #[test]
    fn remove_active_clears_selection() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("x", "Remove Me"));
        reg.select("x").unwrap();
        assert!(reg.remove("x"));
        assert!(reg.active_id().is_none());
        assert!(reg.active().is_none());
    }

    #[test]
    fn remove_nonactive_keeps_selection() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("a", "Keep"));
        reg.add_or_replace(entry("b", "Remove"));
        reg.select("a").unwrap();
        assert!(reg.remove("b"));
        assert_eq!(reg.active_id(), Some("a"));
    }

    #[test]
    fn select_on_empty_registry() {
        let mut reg = TtsProviderRegistry::new();
        assert!(reg.select("anything").is_err());
    }

    #[test]
    fn reselect_different_id() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("a", "Alpha"));
        reg.add_or_replace(entry("b", "Beta"));
        reg.select("a").unwrap();
        reg.select("b").unwrap();
        assert_eq!(reg.active_id(), Some("b"));
    }

    // --- restore_saved_or_first tests ---

    #[test]
    fn restore_saved_existing_id_selects_it() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("saved", "Saved One"));
        reg.add_or_replace(entry("other", "Other"));
        reg.restore_saved_or_first(Some("saved"));
        assert_eq!(reg.active_id(), Some("saved"));
    }

    #[test]
    fn restore_saved_missing_with_active_leaves_it() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("saved", "Saved One"));
        reg.add_or_replace(entry("other", "Other"));
        reg.select("other").unwrap();
        reg.restore_saved_or_first(Some("nonexistent"));
        assert_eq!(reg.active_id(), Some("other"));
    }

    #[test]
    fn restore_saved_missing_without_active_selects_first() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("first", "First One"));
        reg.add_or_replace(entry("second", "Second One"));
        reg.restore_saved_or_first(Some("nonexistent"));
        assert_eq!(reg.active_id(), Some("first"));
    }

    #[test]
    fn restore_no_saved_without_active_selects_first() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("first", "First One"));
        reg.add_or_replace(entry("second", "Second One"));
        reg.restore_saved_or_first(None);
        assert_eq!(reg.active_id(), Some("first"));
    }

    #[test]
    fn restore_no_saved_with_active_leaves_it() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("first", "First One"));
        reg.add_or_replace(entry("second", "Second One"));
        reg.select("second").unwrap();
        reg.restore_saved_or_first(None);
        assert_eq!(reg.active_id(), Some("second"));
    }

    #[test]
    fn restore_saved_empty_registry_noop() {
        let mut reg = TtsProviderRegistry::new();
        reg.restore_saved_or_first(Some("anything"));
        assert!(reg.active_id().is_none());
    }

    #[test]
    fn restore_no_saved_empty_registry_noop() {
        let mut reg = TtsProviderRegistry::new();
        reg.restore_saved_or_first(None);
        assert!(reg.active_id().is_none());
    }

    // --- transactional selection contract tests ---

    #[test]
    fn reconfigure_non_active_preserves_active_id() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("openai", "OpenAI"));
        reg.add_or_replace(entry("silero", "Silero"));
        reg.select("openai").unwrap();
        // re-register silero (simulating key save) must not change active
        reg.add_or_replace(entry("silero", "Silero updated"));
        assert_eq!(reg.active_id(), Some("openai"));
    }

    #[test]
    fn reconfigure_active_entry_preserves_selection() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("openai", "OpenAI"));
        reg.select("openai").unwrap();
        reg.add_or_replace(entry("openai", "OpenAI v2"));
        assert_eq!(reg.active_id(), Some("openai"));
        assert_eq!(reg.active().unwrap().display_name, "OpenAI v2");
    }

    #[test]
    fn select_failure_preserves_previous_active_id() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("a", "Alpha"));
        reg.select("a").unwrap();
        assert_eq!(reg.active_id(), Some("a"));
        let err = reg.select("nonexistent").unwrap_err();
        assert!(err.contains("nonexistent"));
        assert_eq!(reg.active_id(), Some("a"));
    }

    #[test]
    fn select_builtin_then_piper_preserves_correct_ids() {
        let mut reg = TtsProviderRegistry::new();
        reg.add_or_replace(entry("openai", "OpenAI TTS"));
        reg.add_or_replace(entry("local-piper:en-us", "English Piper"));
        reg.select("openai").unwrap();
        assert_eq!(reg.active_id(), Some("openai"));
        reg.select("local-piper:en-us").unwrap();
        assert_eq!(reg.active_id(), Some("local-piper:en-us"));
    }
}
