use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Runtime capability of a discovered RUAccent pack.
///
/// `NativeTiny` identifies a compatible RUAccent model directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeCapability {
    NativeTiny,
}

/// Descriptor for a discovered local RUAccent pack.
///
/// Produced purely from filesystem/metadata discovery: no ONNX session is
/// created and no model files are loaded. The descriptor carries just enough
/// stable identity and path information to build a runtime later.
#[derive(Debug, Clone)]
pub struct RuAccentPackDescriptor {
    pub id: String,
    pub display_name: String,
    pub runtime_version: String,
    pub pack_root: PathBuf,
    pub runtime_capability: RuntimeCapability,
    /// Upstream `nn/nn_omograph` model selected for the native runtime.
    pub omograph_model_id: String,
}

pub(crate) const PRIMARY_MODELS_SUBDIR: &str = "models/ruaccent";
const UPSTREAM_COMMON_FILES: &[&str] = &[
    "dictionary/omographs.json.gz",
    "dictionary/yo_words.json.gz",
    "dictionary/yo_homographs.json.gz",
    "nn/nn_accent/model.onnx",
    "nn/nn_accent/config.json",
    "nn/nn_accent/vocab.txt",
    "nn/nn_stress_usage_predictor/model.onnx",
    "nn/nn_stress_usage_predictor/config.json",
    "nn/nn_stress_usage_predictor/tokenizer.json",
    "nn/nn_yo_homograph_resolver/model.onnx",
    "nn/nn_yo_homograph_resolver/config.json",
    "nn/nn_yo_homograph_resolver/tokenizer.json",
];

/// Discover valid local RUAccent packs across the given search roots.
///
/// Each search root is a config/resource root. The only supported layout is the
/// upstream `models/ruaccent` root: shared files live directly under it, and
/// each variant lives in `nn/nn_omograph/<model-id>`.
pub fn discover_ruaccent_packs(search_roots: &[PathBuf]) -> Vec<RuAccentPackDescriptor> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut results: Vec<RuAccentPackDescriptor> = Vec::new();

    for root in search_roots {
        let root = std::path::absolute(root).unwrap_or_else(|_| root.clone());
        let models_root = root.join(PRIMARY_MODELS_SUBDIR);
        for descriptor in read_upstream_models(&models_root) {
            if seen.insert(descriptor.id.clone()) {
                results.push(descriptor);
            }
        }
    }
    results.sort_by(|a, b| a.id.cmp(&b.id));
    results
}

/// Read the shared files and complete omograph variants of an upstream root.
///
/// Returns one descriptor per complete `nn/nn_omograph/<model-id>` directory
/// (a directory containing both `model.onnx` and `tokenizer.json`). An
/// absent root is normal (e.g. the resource dir) and stays silent; an EXISTING
/// but incomplete root logs the failed check so a transient boot-time
/// filesystem failure is diagnosable instead of reading as "no models".
fn read_upstream_models(pack_root: &Path) -> Vec<RuAccentPackDescriptor> {
    if !pack_root.is_dir() {
        return Vec::new();
    }
    let has_accents = pack_root.join("dictionary/accents.fst").is_file()
        || pack_root.join("dictionary/accents.json.gz").is_file();
    if !has_accents {
        tracing::warn!(
            root = %crate::secret_log::safe_path_for_log(pack_root),
            missing = "dictionary/accents.fst or dictionary/accents.json.gz",
            "RUAccent upstream root incomplete; ignoring"
        );
        return Vec::new();
    }
    if let Some(missing) = UPSTREAM_COMMON_FILES
        .iter()
        .find(|path| !pack_root.join(path).is_file())
    {
        tracing::warn!(
            root = %crate::secret_log::safe_path_for_log(pack_root),
            missing = %missing,
            "RUAccent upstream root incomplete; ignoring"
        );
        return Vec::new();
    }
    let entries = match std::fs::read_dir(pack_root.join("nn/nn_omograph")) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(
                root = %crate::secret_log::safe_path_for_log(pack_root),
                error = %error,
                "RUAccent omograph directory unreadable; ignoring"
            );
            return Vec::new();
        }
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .filter_map(|dir| {
            let model_id = dir.file_name()?.to_str()?.to_string();
            (dir.join("model.onnx").is_file() && dir.join("tokenizer.json").is_file()).then(|| {
                RuAccentPackDescriptor {
                    id: format!("ruaccent.upstream.{model_id}"),
                    display_name: format!("RUAccent {model_id}"),
                    runtime_version: "upstream".to_string(),
                    pack_root: pack_root.to_path_buf(),
                    runtime_capability: RuntimeCapability::NativeTiny,
                    omograph_model_id: model_id,
                }
            })
        })
        .collect()
}

/// Shared test scaffolding for building the upstream RUAccent layout in temp
/// directories.
///
/// Used by unit tests across `packs`, `runtime`, `state` and `tts_pipeline`.
#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    pub(crate) fn unique_test_root(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ttsbard-ruaccent-test-{}-{}-{}",
            std::process::id(),
            unique,
            name
        ))
    }

    pub(crate) fn upstream_pack_root(root: &Path) -> PathBuf {
        root.join(PRIMARY_MODELS_SUBDIR)
    }

    /// Create a complete upstream pack with all shared files and one or more
    /// complete omograph variants. Returns the pack root (`models/ruaccent`).
    pub(crate) fn write_upstream_pack(root: &Path, model_ids: &[&str]) -> PathBuf {
        let pack = upstream_pack_root(root);
        let accents = pack.join("dictionary/accents.json.gz");
        std::fs::create_dir_all(accents.parent().unwrap()).unwrap();
        std::fs::write(accents, b"data").unwrap();
        for path in UPSTREAM_COMMON_FILES {
            let file = pack.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"data").unwrap();
        }
        for model_id in model_ids {
            write_omograph_variant(&pack, model_id);
        }
        pack
    }

    /// Create shared files but drop one common file so the root is incomplete.
    pub(crate) fn write_incomplete_root(root: &Path, model_ids: &[&str]) -> PathBuf {
        let pack = write_upstream_pack(root, model_ids);
        std::fs::remove_file(pack.join(UPSTREAM_COMMON_FILES[0])).unwrap();
        pack
    }

    /// Create shared files plus a variant missing `tokenizer.json`, so the
    /// variant is incomplete and never discovered.
    pub(crate) fn write_incomplete_variant(root: &Path, model_id: &str) -> PathBuf {
        let pack = write_upstream_pack(root, &[]);
        let dir = pack.join("nn/nn_omograph").join(model_id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.onnx"), b"data").unwrap();
        pack
    }

    fn write_omograph_variant(pack: &Path, model_id: &str) {
        let dir = pack.join("nn/nn_omograph").join(model_id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.onnx"), b"data").unwrap();
        std::fs::write(dir.join("tokenizer.json"), b"data").unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_util as tu;
    use super::*;

    #[test]
    fn complete_upstream_root_is_discovered() {
        let root = tu::unique_test_root("complete");
        let pack = tu::write_upstream_pack(&root, &["ruaccent-v1"]);

        let descriptors = discover_ruaccent_packs(std::slice::from_ref(&root));
        assert_eq!(descriptors.len(), 1);
        let d = &descriptors[0];
        assert_eq!(d.id, "ruaccent.upstream.ruaccent-v1");
        assert_eq!(d.display_name, "RUAccent ruaccent-v1");
        assert_eq!(d.runtime_version, "upstream");
        assert_eq!(d.pack_root, pack);
        assert_eq!(d.omograph_model_id, "ruaccent-v1");
        assert_eq!(d.runtime_capability, RuntimeCapability::NativeTiny);

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn fst_accent_storage_upstream_root_is_discovered() {
        let root = tu::unique_test_root("fst-accents");
        let pack = tu::write_upstream_pack(&root, &["tiny"]);
        std::fs::remove_file(pack.join("dictionary/accents.json.gz")).unwrap();
        std::fs::write(pack.join("dictionary/accents.fst"), b"fst-data").unwrap();

        let descriptors = discover_ruaccent_packs(std::slice::from_ref(&root));
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id, "ruaccent.upstream.tiny");

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn two_model_variants_are_discovered() {
        let root = tu::unique_test_root("two-variants");
        let pack = tu::write_upstream_pack(&root, &["variant-b", "variant-a"]);

        let descriptors = discover_ruaccent_packs(std::slice::from_ref(&root));
        let ids: Vec<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["ruaccent.upstream.variant-a", "ruaccent.upstream.variant-b"]
        );
        assert!(descriptors.iter().all(|d| d.pack_root == pack));
        assert_eq!(descriptors[0].omograph_model_id, "variant-a");
        assert_eq!(descriptors[1].omograph_model_id, "variant-b");

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn incomplete_root_is_ignored() {
        let root = tu::unique_test_root("incomplete-root");
        tu::write_incomplete_root(&root, &["variant"]);

        assert!(discover_ruaccent_packs(std::slice::from_ref(&root)).is_empty());

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn incomplete_variant_is_ignored() {
        let root = tu::unique_test_root("incomplete-variant");
        tu::write_upstream_pack(&root, &["complete-variant"]);
        tu::write_incomplete_variant(&root, "broken-variant");

        let descriptors = discover_ruaccent_packs(std::slice::from_ref(&root));
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id, "ruaccent.upstream.complete-variant");

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn missing_root_returns_empty() {
        let root = tu::unique_test_root("missing");
        assert!(!root.exists());

        let descriptors = discover_ruaccent_packs(std::slice::from_ref(&root));
        assert!(descriptors.is_empty());
    }

    #[test]
    fn relative_root_yields_absolute_pack_root() {
        let cwd = std::env::current_dir().unwrap();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let abs_root = cwd.join(format!(
            "ttsbard-ruaccent-relative-test-{}-{}",
            std::process::id(),
            unique
        ));
        tu::write_upstream_pack(&abs_root, &["variant"]);

        let relative_root = abs_root.strip_prefix(&cwd).unwrap().to_path_buf();
        assert!(relative_root.is_relative());

        let descriptors = discover_ruaccent_packs(&[relative_root]);

        std::fs::remove_dir_all(&abs_root).ok();

        assert_eq!(descriptors.len(), 1);
        assert!(descriptors[0].pack_root.is_absolute());
    }

    #[test]
    fn first_root_wins_duplicate_model_id() {
        let root1 = tu::unique_test_root("first");
        let root2 = tu::unique_test_root("second");

        let pack1 = tu::write_upstream_pack(&root1, &["dup"]);
        let pack2 = tu::write_upstream_pack(&root2, &["dup"]);

        let descriptors = discover_ruaccent_packs(&[root1.clone(), root2.clone()]);
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].pack_root, pack1);
        assert_ne!(descriptors[0].pack_root, pack2);

        std::fs::remove_dir_all(root1).ok();
        std::fs::remove_dir_all(root2).ok();
    }

    /// Different valid IDs from both search roots stay available together
    /// (cwd → data root, ROADMAP-117).
    #[test]
    fn both_roots_ids_are_discovered_together() {
        let root1 = tu::unique_test_root("cwd-root");
        let root2 = tu::unique_test_root("data-root");

        let pack1 = tu::write_upstream_pack(&root1, &["cwd-variant"]);
        let pack2 = tu::write_upstream_pack(&root2, &["data-variant"]);

        let descriptors = discover_ruaccent_packs(&[root1.clone(), root2.clone()]);
        assert_eq!(descriptors.len(), 2);
        assert_eq!(
            descriptors[0].pack_root, pack1,
            "sorted by ID: cwd-variant first"
        );
        assert_eq!(descriptors[0].omograph_model_id, "cwd-variant");
        assert_eq!(descriptors[1].pack_root, pack2);
        assert_eq!(descriptors[1].omograph_model_id, "data-variant");

        std::fs::remove_dir_all(root1).ok();
        std::fs::remove_dir_all(root2).ok();
    }

    /// A broken same-ID candidate in the earlier root does not block the
    /// valid variant in the later root: dedup happens after validation.
    #[test]
    fn broken_earlier_root_allows_later_root_fallback() {
        let root1 = tu::unique_test_root("broken-cwd");
        let root2 = tu::unique_test_root("valid-data");

        tu::write_incomplete_root(&root1, &["dup"]);
        let pack2 = tu::write_upstream_pack(&root2, &["dup"]);

        let descriptors = discover_ruaccent_packs(&[root1.clone(), root2.clone()]);
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].pack_root, pack2);
        assert_ne!(descriptors[0].pack_root, root1.join("models/ruaccent"));

        std::fs::remove_dir_all(root1).ok();
        std::fs::remove_dir_all(root2).ok();
    }

    /// A missing earlier root (e.g. no models in the cwd) must not break
    /// discovery of the later root; the missing directory is not created.
    #[test]
    fn missing_earlier_root_still_scans_later_root() {
        let root1 = tu::unique_test_root("missing-cwd");
        assert!(!root1.exists());
        let root2 = tu::unique_test_root("data-root");

        let pack2 = tu::write_upstream_pack(&root2, &["variant"]);

        let descriptors = discover_ruaccent_packs(&[root1.clone(), root2.clone()]);
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].pack_root, pack2);
        assert!(!root1.exists(), "missing root must not be created");

        std::fs::remove_dir_all(root1).ok();
        std::fs::remove_dir_all(root2).ok();
    }
}
