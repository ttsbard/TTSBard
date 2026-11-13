use parking_lot::RwLock;
use std::collections::HashMap;

const RU_AFF: &str = include_str!("../resources/dict/ru.aff");
const RU_DIC: &str = include_str!("../resources/dict/ru.dic");

/// Upper bound for the per-word result cache.
///
/// The cache is an optimization only: an evicted word is simply checked again
/// on the next request, so the batch that reaches the limit drops the whole
/// cache instead of letting memory grow with every unique word the user types.
const MAX_CACHE_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpellResult {
    pub word: String,
    pub correct: bool,
    pub suggestions: Vec<String>,
}

pub struct SpellcheckManager {
    dict: RwLock<Option<spellbook::Dictionary>>,
    cache: RwLock<HashMap<String, SpellResult>>,
}

impl SpellcheckManager {
    pub fn new() -> Self {
        Self::from_dict_str(RU_AFF, RU_DIC)
    }

    fn from_dict_str(aff: &str, dic: &str) -> Self {
        let dict = spellbook::Dictionary::new(aff, dic)
            .map_err(|e| anyhow::anyhow!("parse hunspell dict: {e:?}"));
        if let Err(e) = &dict {
            eprintln!("[spellcheck] dictionary load failed: {e:?} (spellcheck disabled)");
        }
        Self {
            dict: RwLock::new(dict.ok()),
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn is_available(&self) -> bool {
        self.dict.read().is_some()
    }

    pub fn check_words(&self, words: &[String]) -> Vec<SpellResult> {
        let mut results = Vec::with_capacity(words.len());
        let mut to_check: Vec<usize> = Vec::new();

        {
            let cache = self.cache.read();
            for (i, w) in words.iter().enumerate() {
                if let Some(r) = cache.get(w) {
                    results.push(r.clone());
                } else {
                    to_check.push(i);
                }
            }
        }

        if to_check.is_empty() {
            return results;
        }

        let mut new_results: Vec<(usize, SpellResult)> = Vec::with_capacity(to_check.len());
        {
            let dict_guard = self.dict.read();
            if let Some(dict) = dict_guard.as_ref() {
                for &idx in &to_check {
                    let w = &words[idx];
                    let correct = dict.check(w);
                    let mut suggestions = Vec::new();
                    if !correct {
                        dict.suggest(w, &mut suggestions);
                    }
                    new_results.push((
                        idx,
                        SpellResult {
                            word: w.clone(),
                            correct,
                            suggestions,
                        },
                    ));
                }
            } else {
                return Vec::new();
            }
        }

        // Phase 3: write new results to cache (dict read-lock released)
        {
            let mut cache = self.cache.write();
            // Bound the cache growth: once it is full, the incoming batch
            // evicts it wholesale. Results stay correct — every evicted word is
            // re-checked on demand.
            if cache.len() >= MAX_CACHE_ENTRIES {
                cache.clear();
            }
            for (idx, r) in &new_results {
                let w = &words[*idx];
                cache.entry(w.clone()).or_insert_with(|| r.clone());
            }
        }

        // Phase 4: merge cached (Phase 1) + new (Phase 2) in original word order
        let mut final_results: Vec<SpellResult> = Vec::with_capacity(words.len());
        let mut ni = 0;
        let mut ri = 0;
        for i in 0..words.len() {
            if ni < to_check.len() && to_check[ni] == i {
                final_results.push(new_results[ni].1.clone());
                ni += 1;
            } else {
                final_results.push(results[ri].clone());
                ri += 1;
            }
        }

        final_results
    }
}

impl Default for SpellcheckManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid_dictionary_manager() -> SpellcheckManager {
        SpellcheckManager::from_dict_str("not a hunspell affix file", "not a hunspell dictionary")
    }

    #[test]
    fn is_available_false_when_dict_invalid() {
        let mgr = invalid_dictionary_manager();
        assert!(!mgr.is_available());
    }

    #[test]
    fn is_available_true_when_dict_loaded() {
        let mgr = SpellcheckManager::new();
        assert!(mgr.is_available());
    }

    #[test]
    fn check_words_returns_empty_when_dict_unavailable() {
        let mgr = invalid_dictionary_manager();
        let results = mgr.check_words(&["тест".into(), "слово".into()]);
        assert!(results.is_empty());
    }

    #[test]
    fn check_words_no_false_correct_when_dict_unavailable() {
        let mgr = invalid_dictionary_manager();
        let words: Vec<String> = vec!["любой".into(), "текст".into()];
        let results = mgr.check_words(&words);
        assert!(
            results.is_empty(),
            "must not return correct=true for all words"
        );
    }

    #[test]
    fn check_words_with_embedded_dict_marks_misspelling() {
        let mgr = SpellcheckManager::new();
        assert!(mgr.is_available());
        let words: Vec<String> = vec!["здрвствуйте".into()];
        let results = mgr.check_words(&words);
        assert_eq!(results.len(), 1);
        assert!(!results[0].correct);
    }

    #[test]
    fn check_words_with_embedded_dict_suggests_corrections() {
        let mgr = SpellcheckManager::new();
        assert!(mgr.is_available());
        let words: Vec<String> = vec!["здрвствуйте".into()];
        let results = mgr.check_words(&words);
        assert_eq!(results.len(), 1);
        assert!(
            !results[0].suggestions.is_empty(),
            "misspelled Russian word must produce suggestions"
        );
    }

    #[test]
    fn check_words_with_embedded_dict_accepts_correct_word() {
        let mgr = SpellcheckManager::new();
        assert!(mgr.is_available());
        let words: Vec<String> = vec!["здравствуйте".into()];
        let results = mgr.check_words(&words);
        assert_eq!(results.len(), 1);
        assert!(results[0].correct);
    }

    #[test]
    fn check_words_caches_results() {
        let mgr = SpellcheckManager::new();
        assert!(mgr.is_available());
        let words: Vec<String> = vec!["здравствуйте".into()];
        let _ = mgr.check_words(&words);
        let second = mgr.check_words(&words);
        assert_eq!(second.len(), 1);
        assert!(second[0].correct);
    }

    #[test]
    fn check_words_no_cache_when_dict_unavailable() {
        let mgr = invalid_dictionary_manager();
        assert!(!mgr.is_available());
        let words: Vec<String> = vec!["any".into(), "text".into()];
        let _ = mgr.check_words(&words);
        let second = mgr.check_words(&words);
        assert!(second.is_empty());
    }

    /// A full cache must be dropped instead of growing without limit, and the
    /// evicted results must still be produced correctly on demand.
    #[test]
    fn cache_is_cleared_when_entry_limit_is_reached() {
        let mgr = SpellcheckManager::new();
        assert!(mgr.is_available());

        {
            let mut cache = mgr.cache.write();
            for i in 0..MAX_CACHE_ENTRIES {
                let word = format!("слово{}", i);
                cache.insert(
                    word.clone(),
                    SpellResult {
                        word,
                        correct: true,
                        suggestions: Vec::new(),
                    },
                );
            }
        }
        assert_eq!(mgr.cache.read().len(), MAX_CACHE_ENTRIES);

        let results = mgr.check_words(&["здравствуйте".into()]);
        assert_eq!(results.len(), 1);
        assert!(results[0].correct);
        assert_eq!(
            mgr.cache.read().len(),
            1,
            "a full cache must be cleared before accepting new entries"
        );
    }

    /// Below the limit the cache keeps accumulating instead of being cleared.
    #[test]
    fn cache_keeps_entries_below_the_limit() {
        let mgr = SpellcheckManager::new();
        assert!(mgr.is_available());

        let words: Vec<String> = vec!["здравствуйте".into(), "привет".into()];
        let _ = mgr.check_words(&words);
        let _ = mgr.check_words(&words);

        assert_eq!(mgr.cache.read().len(), words.len());
    }
}
