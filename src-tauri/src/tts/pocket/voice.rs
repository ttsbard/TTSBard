//! Voice selection, readiness and the small Pocket TTS preference file.
//!
//! Reference WAV paths and ready voice IDs belong to the provider, not to the
//! global TTS settings: they are persisted in `pocket-voices.json` under the
//! application config root and keyed by the stable package ID. A queue snapshot
//! clones the selection at submit time, so changing the card afterwards cannot
//! change an already submitted request.

use std::collections::BTreeMap;
use std::path::PathBuf;

use parking_lot::Mutex;

use super::audio::validate_reference_wav;
use super::descriptor::PocketModelDescriptor;
use super::{PocketError, PocketSelectionDto, PocketVoiceInput, PocketVoiceMode};

pub const VOICE_STORE_FILE: &str = "pocket-voices.json";
const VOICE_STORE_VERSION: u32 = 1;

/// Serializes every read-modify-atomic-replace transaction of
/// `pocket-voices.json`, including the registry reconciliation performed by a
/// package refresh. Without this lock two writers can load the same snapshot
/// and whichever saves last silently drops the other model's preference.
static VOICE_STORE_LOCK: Mutex<()> = Mutex::new(());

/// UI-facing readiness of a model + selected voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PocketReadiness {
    Ready,
    NeedsReference,
    ReferenceMissing,
    ReferenceInvalid,
    VoiceMissing,
    PackageInvalid,
}

impl PocketReadiness {
    pub fn code(self) -> Option<&'static str> {
        match self {
            PocketReadiness::Ready => None,
            PocketReadiness::NeedsReference => Some("pocket.reference_required"),
            PocketReadiness::ReferenceMissing => Some("pocket.reference_missing"),
            PocketReadiness::ReferenceInvalid => Some("pocket.reference_invalid"),
            PocketReadiness::VoiceMissing => Some("pocket.voice_missing"),
            PocketReadiness::PackageInvalid => Some("pocket.package_invalid"),
        }
    }
}

/// Selection captured by a provider instance and therefore by a queue snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PocketVoiceSelection {
    ReferenceWav(PathBuf),
    VoiceState(String),
}

impl PocketVoiceSelection {
    pub fn identity(&self, _descriptor: &PocketModelDescriptor) -> String {
        match self {
            PocketVoiceSelection::ReferenceWav(path) => {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("reference.wav");
                // Include size and modification time so replacing the WAV
                // invalidates the audio cache even when the file name is kept.
                let metadata = std::fs::metadata(path).ok();
                let length = metadata.as_ref().map(|meta| meta.len()).unwrap_or(0);
                let modified = metadata
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0);
                format!("reference:{name}:{length}:{modified}")
            }
            PocketVoiceSelection::VoiceState(voice_id) => format!("voice:{voice_id}"),
        }
    }

    pub fn dto(&self, descriptor: &PocketModelDescriptor) -> PocketSelectionDto {
        match self {
            PocketVoiceSelection::ReferenceWav(path) => PocketSelectionDto::ReferenceWav {
                path: path.to_string_lossy().to_string(),
                file_name: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("reference.wav")
                    .to_string(),
            },
            PocketVoiceSelection::VoiceState(voice_id) => PocketSelectionDto::VoiceState {
                voice_id: voice_id.clone(),
                display_name: descriptor
                    .voice(voice_id)
                    .map(|voice| voice.display_name.clone())
                    .unwrap_or_else(|| voice_id.clone()),
            },
        }
    }

    pub fn from_input(input: &PocketVoiceInput) -> Self {
        input.to_selection()
    }

    fn to_stored(&self) -> StoredVoice {
        match self {
            PocketVoiceSelection::ReferenceWav(path) => StoredVoice::ReferenceWav {
                path: path.to_string_lossy().to_string(),
            },
            PocketVoiceSelection::VoiceState(voice_id) => StoredVoice::VoiceState {
                voice_id: voice_id.clone(),
            },
        }
    }

    fn from_stored(stored: &StoredVoice) -> Self {
        match stored {
            StoredVoice::ReferenceWav { path } => {
                PocketVoiceSelection::ReferenceWav(PathBuf::from(path))
            }
            StoredVoice::VoiceState { voice_id } => {
                PocketVoiceSelection::VoiceState(voice_id.clone())
            }
        }
    }
}

/// Default selection for a package: its first ready voice, if it has any.
/// Russian reference-only sets intentionally return `None`.
pub fn default_selection(descriptor: &PocketModelDescriptor) -> Option<PocketVoiceSelection> {
    descriptor
        .voices
        .first()
        .map(|voice| PocketVoiceSelection::VoiceState(voice.id.clone()))
}

/// Validate a selection and return the UI readiness verdict.
pub fn evaluate_readiness(
    descriptor: &PocketModelDescriptor,
    selection: Option<&PocketVoiceSelection>,
) -> PocketReadiness {
    evaluate_readiness_with_duration(descriptor, selection).0
}

pub(super) fn evaluate_readiness_with_duration(
    descriptor: &PocketModelDescriptor,
    selection: Option<&PocketVoiceSelection>,
) -> (PocketReadiness, Option<f32>) {
    if !descriptor.assets_present() {
        return (PocketReadiness::PackageInvalid, None);
    }
    let readiness = match selection {
        None => {
            if descriptor
                .voice_modes
                .contains(&PocketVoiceMode::ReferenceWav)
                && descriptor.voices.is_empty()
            {
                PocketReadiness::NeedsReference
            } else {
                PocketReadiness::VoiceMissing
            }
        }
        Some(PocketVoiceSelection::ReferenceWav(path)) => {
            if !descriptor
                .voice_modes
                .contains(&PocketVoiceMode::ReferenceWav)
            {
                return (PocketReadiness::ReferenceInvalid, None);
            }
            match validate_reference_wav(path) {
                Ok(info) => return (PocketReadiness::Ready, Some(info.duration_seconds)),
                Err(PocketError::ReferenceMissing) => PocketReadiness::ReferenceMissing,
                Err(_) => PocketReadiness::ReferenceInvalid,
            }
        }
        Some(PocketVoiceSelection::VoiceState(voice_id)) => match descriptor.voice(voice_id) {
            Some(voice) if voice.state_path.is_file() => PocketReadiness::Ready,
            _ => PocketReadiness::VoiceMissing,
        },
    };
    (readiness, None)
}

/// Normalize a requested selection against the package capabilities.
pub fn validate_selection(
    descriptor: &PocketModelDescriptor,
    input: &PocketVoiceInput,
) -> Result<PocketVoiceSelection, PocketError> {
    let selection = PocketVoiceSelection::from_input(input);
    match input {
        PocketVoiceInput::ReferenceWav { .. } => {
            if !descriptor
                .voice_modes
                .contains(&PocketVoiceMode::ReferenceWav)
            {
                return Err(PocketError::ReferenceInvalid(
                    "this model does not accept a reference WAV".to_string(),
                ));
            }
            let PocketVoiceSelection::ReferenceWav(path) = &selection else {
                unreachable!();
            };
            validate_reference_wav(path)?;
            Ok(selection)
        }
        PocketVoiceInput::VoiceState { voice_id } => {
            let voice = descriptor
                .voice(voice_id)
                .ok_or_else(|| PocketError::VoiceMissing(voice_id.clone()))?;
            if !voice.state_path.is_file() {
                return Err(PocketError::VoiceMissing(voice_id.clone()));
            }
            Ok(selection)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredVoice {
    ReferenceWav { path: String },
    VoiceState { voice_id: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct VoiceStoreFile {
    #[serde(default = "default_store_version")]
    version: u32,
    #[serde(default)]
    models: BTreeMap<String, StoredVoice>,
}

fn default_store_version() -> u32 {
    VOICE_STORE_VERSION
}

impl Default for VoiceStoreFile {
    fn default() -> Self {
        Self {
            version: VOICE_STORE_VERSION,
            models: BTreeMap::new(),
        }
    }
}

/// In-memory preference store backed by `pocket-voices.json`.
#[derive(Debug, Clone)]
pub struct PocketVoiceStore {
    path: PathBuf,
    file: VoiceStoreFile,
}

impl PocketVoiceStore {
    pub fn default_path() -> Option<PathBuf> {
        crate::paths::config_root()
            .ok()
            .map(|root| root.join(VOICE_STORE_FILE))
    }

    pub fn load(path: PathBuf) -> Self {
        let file = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<VoiceStoreFile>(&text) {
                Ok(file) if file.version <= VOICE_STORE_VERSION => file,
                Ok(file) => {
                    tracing::warn!(
                        version = file.version,
                        "Unsupported pocket-voices.json version; ignoring it"
                    );
                    VoiceStoreFile::default()
                }
                Err(error) => {
                    tracing::warn!(
                        path = %crate::secret_log::safe_path_for_log(&path),
                        error = %error,
                        "Ignoring corrupted pocket-voices.json"
                    );
                    VoiceStoreFile::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => VoiceStoreFile::default(),
            Err(error) => {
                tracing::warn!(
                    path = %crate::secret_log::safe_path_for_log(&path),
                    error = %error,
                    "Failed to read pocket-voices.json"
                );
                VoiceStoreFile::default()
            }
        };
        Self { path, file }
    }

    pub fn selection_for(&self, package_id: &str) -> Option<PocketVoiceSelection> {
        self.file
            .models
            .get(package_id)
            .map(PocketVoiceSelection::from_stored)
    }

    pub fn set(
        &mut self,
        package_id: &str,
        selection: &PocketVoiceSelection,
    ) -> Result<(), PocketError> {
        let previous = self
            .file
            .models
            .insert(package_id.to_string(), selection.to_stored());
        if let Err(error) = self.save() {
            match previous {
                Some(previous) => {
                    self.file.models.insert(package_id.to_string(), previous);
                }
                None => {
                    self.file.models.remove(package_id);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    pub fn clear(&mut self, package_id: &str) -> Result<(), PocketError> {
        let Some(previous) = self.file.models.remove(package_id) else {
            return Ok(());
        };
        if let Err(error) = self.save() {
            self.file.models.insert(package_id.to_string(), previous);
            return Err(error);
        }
        Ok(())
    }

    fn save(&self) -> Result<(), PocketError> {
        let Some(parent) = self.path.parent() else {
            return Err(PocketError::Runtime(
                "pocket-voices.json has no parent directory".to_string(),
            ));
        };
        std::fs::create_dir_all(parent).map_err(|error| {
            PocketError::Runtime(format!("failed to create config directory: {error}"))
        })?;
        let text = serde_json::to_string_pretty(&self.file).map_err(|error| {
            PocketError::Runtime(format!("failed to serialize voices: {error}"))
        })?;
        let _guard = crate::config::config_write_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crate::config::replace_file_atomically(&self.path, text.as_bytes()).map_err(|error| {
            tracing::warn!(
                path = %crate::secret_log::safe_path_for_log(&self.path),
                error = %error,
                "Atomic Pocket voice save failed; keeping the previous file"
            );
            PocketError::Runtime(format!("failed to persist pocket-voices.json: {error}"))
        })
    }
}

/// Run a read-modify-atomic-replace transaction against the default
/// `pocket-voices.json`.
///
/// The whole transaction, including any registry reconciliation that depends
/// on the loaded snapshot, runs under [`VOICE_STORE_LOCK`]. That keeps a
/// concurrent refresh from writing an older selection back into the registry
/// after a voice change has already been persisted.
pub fn with_voice_store<T>(action: impl FnOnce(&mut PocketVoiceStore) -> T) -> T {
    let path = match PocketVoiceStore::default_path() {
        Some(path) => path,
        None => PathBuf::from(VOICE_STORE_FILE),
    };
    with_voice_store_at(path, action)
}

/// Path-explicit form of [`with_voice_store`], used by tests and by callers
/// that need to operate on a specific preference file.
pub fn with_voice_store_at<T>(path: PathBuf, action: impl FnOnce(&mut PocketVoiceStore) -> T) -> T {
    let _guard = VOICE_STORE_LOCK.lock();
    let mut store = PocketVoiceStore::load(path);
    action(&mut store)
}

/// Convenience used by the provider registry when loading a package.
pub fn resolved_selection(
    descriptor: &PocketModelDescriptor,
    store: &PocketVoiceStore,
) -> Option<PocketVoiceSelection> {
    store
        .selection_for(&descriptor.package_id)
        .or_else(|| default_selection(descriptor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::pocket::descriptor::{
        PocketGraphs, PocketTokenizerKind, PocketVoiceDescriptor,
    };
    use std::path::Path;
    use std::sync::{Arc, Barrier};

    fn descriptor(root: &Path, with_voice: bool) -> PocketModelDescriptor {
        let voice_path = root.join("embeddings/alba.safetensors");
        PocketModelDescriptor {
            id: "local-pocket:test".to_string(),
            package_id: "test".to_string(),
            display_name: "Test".to_string(),
            language: "en".to_string(),
            format: "pocket-tts-onnx".to_string(),
            format_version: 1,
            package_root: root.to_path_buf(),
            config_path: root.join("config.yaml"),
            weights_path: root.join("model.safetensors"),
            tokenizer_path: root.join("tokenizer.json"),
            tokenizer_kind: PocketTokenizerKind::Json,
            n_bins: 10,
            sample_rate: 24000,
            frame_rate: 12.5,
            num_layers: 1,
            latent_dim: 32,
            insert_bos_before_voice: true,
            temperature: 0.3,
            eos_threshold: -4.0,
            voice_modes: if with_voice {
                vec![PocketVoiceMode::VoiceState]
            } else {
                vec![PocketVoiceMode::ReferenceWav]
            },
            voices: if with_voice {
                vec![PocketVoiceDescriptor {
                    id: "alba".to_string(),
                    display_name: "Alba".to_string(),
                    state_path: voice_path,
                }]
            } else {
                Vec::new()
            },
            graphs: PocketGraphs {
                encoder: root.join("mimi_encoder.onnx"),
                conditioner: root.join("text_conditioner.onnx"),
                main: root.join("flow_lm_main.onnx"),
                flow: root.join("flow_lm_flow.onnx"),
                decoder: root.join("mimi_decoder.onnx"),
                quantized: false,
            },
            asset_paths: Vec::new(),
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ttsbard-pocket-voice-{}-{}-{name}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn readiness_distinguishes_missing_and_invalid_reference() {
        let root = std::env::temp_dir();
        let descriptor = descriptor(&root, false);
        let missing = root.join("does-not-exist.wav");
        assert_eq!(
            evaluate_readiness(
                &descriptor,
                Some(&PocketVoiceSelection::ReferenceWav(missing))
            ),
            PocketReadiness::ReferenceMissing
        );

        let invalid = temp_path("invalid").with_extension("wav");
        std::fs::write(&invalid, b"not wav").unwrap();
        assert_eq!(
            evaluate_readiness(
                &descriptor,
                Some(&PocketVoiceSelection::ReferenceWav(invalid.clone()))
            ),
            PocketReadiness::ReferenceInvalid
        );
        std::fs::remove_file(invalid).ok();
    }

    #[test]
    fn readiness_needs_reference_when_none_selected() {
        let root = std::env::temp_dir();
        let descriptor = descriptor(&root, false);
        assert_eq!(
            evaluate_readiness(&descriptor, None),
            PocketReadiness::NeedsReference
        );
    }

    #[test]
    fn store_round_trips_selection_and_survives_missing_file() {
        let path = temp_path("store");
        {
            let mut store = PocketVoiceStore::load(path.clone());
            store
                .set(
                    "shtorm",
                    &PocketVoiceSelection::ReferenceWav(PathBuf::from("C:/voice.wav")),
                )
                .unwrap();
            store
                .set(
                    "kyutai",
                    &PocketVoiceSelection::VoiceState("alba".to_string()),
                )
                .unwrap();
        }
        let store = PocketVoiceStore::load(path.clone());
        assert_eq!(
            store.selection_for("shtorm"),
            Some(PocketVoiceSelection::ReferenceWav(PathBuf::from(
                "C:/voice.wav"
            )))
        );
        assert_eq!(
            store.selection_for("kyutai"),
            Some(PocketVoiceSelection::VoiceState("alba".to_string()))
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn corrupted_store_is_ignored() {
        let path = temp_path("corrupt");
        std::fs::write(&path, b"{not json").unwrap();
        let store = PocketVoiceStore::load(path.clone());
        assert!(store.selection_for("anything").is_none());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn concurrent_store_transactions_do_not_lose_updates() {
        let path = temp_path("concurrent");
        let workers = 8usize;
        let barrier = Arc::new(Barrier::new(workers + 1));
        let mut handles = Vec::new();
        for index in 0..workers {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                with_voice_store_at(path, |store| {
                    store
                        .set(
                            &format!("package-{index}"),
                            &PocketVoiceSelection::VoiceState(format!("voice-{index}")),
                        )
                        .unwrap();
                });
            }));
        }
        barrier.wait();
        for handle in handles {
            handle.join().unwrap();
        }

        let store = PocketVoiceStore::load(path.clone());
        for index in 0..workers {
            assert_eq!(
                store.selection_for(&format!("package-{index}")),
                Some(PocketVoiceSelection::VoiceState(format!("voice-{index}")))
            );
        }
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn failed_atomic_save_rolls_back_the_in_memory_selection() {
        let path = temp_path("save-failure");
        let mut store = PocketVoiceStore::load(path.clone());
        let first = PocketVoiceSelection::ReferenceWav(PathBuf::from("C:/first.wav"));
        let second = PocketVoiceSelection::ReferenceWav(PathBuf::from("C:/second.wav"));
        store.set("first", &first).unwrap();

        // Make the destination a directory so the atomic replacement fails
        // before touching it. The preference visible through the store must
        // stay unchanged instead of retaining the unsaved value.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(store.set("second", &second).is_err());
        assert_eq!(store.selection_for("first"), Some(first));
        assert!(store.selection_for("second").is_none());

        std::fs::remove_dir(&path).ok();
    }
}
