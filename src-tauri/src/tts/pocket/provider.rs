//! TTS provider wrapper for Pocket TTS.
//!
//! A provider instance owns an immutable voice selection. Queue submission
//! clones the provider, so changing the card after sending a request cannot
//! alter the model or voice of that request. ONNX sessions are shared between
//! clones through a bounded process-wide cache; every synthesis resets its own
//! KV/Mimi state and a failure leaves the shared runtime usable for the next
//! task.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use parking_lot::Mutex;
use tracing::warn;

use super::descriptor::PocketModelDescriptor;
use super::runtime::PocketRuntime;
use super::voice::{evaluate_readiness, PocketReadiness, PocketVoiceSelection};
use super::{PocketError, PocketSelectionDto};
use crate::tts::engine::TtsEngine;

/// Keep at most this many idle model runtimes loaded. A runtime in use by a
/// synthesis thread is never evicted; queued provider snapshots keep their own
/// slot reference and lazily reload if their idle runtime was evicted.
const MAX_CACHED_RUNTIMES: usize = 2;

/// Identity of the model assets a runtime was loaded from. File length and
/// modification time make an in-place graph/weights/tokenizer replacement
/// produce a new cache slot instead of reusing stale sessions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileStamp {
    path: PathBuf,
    length: u64,
    modified_nanos: u128,
}

impl FileStamp {
    fn read(path: &Path) -> Self {
        let metadata = std::fs::metadata(path).ok();
        let length = metadata.as_ref().map(|meta| meta.len()).unwrap_or(0);
        let modified_nanos = metadata
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        Self {
            path: path.to_path_buf(),
            length,
            modified_nanos,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PocketRuntimeKey {
    id: String,
    language: String,
    tokenizer_kind: &'static str,
    n_bins: usize,
    sample_rate: u32,
    frame_rate_bits: u32,
    num_layers: usize,
    latent_dim: usize,
    insert_bos_before_voice: bool,
    temperature_bits: u32,
    eos_threshold_bits: u32,
    graphs: [FileStamp; 5],
    weights: FileStamp,
    tokenizer: FileStamp,
    voices: Vec<(String, FileStamp)>,
}

impl PocketRuntimeKey {
    fn new(descriptor: &PocketModelDescriptor) -> Self {
        Self {
            id: descriptor.id.clone(),
            language: descriptor.language.clone(),
            tokenizer_kind: descriptor.tokenizer_kind.as_str(),
            n_bins: descriptor.n_bins,
            sample_rate: descriptor.sample_rate,
            frame_rate_bits: descriptor.frame_rate.to_bits(),
            num_layers: descriptor.num_layers,
            latent_dim: descriptor.latent_dim,
            insert_bos_before_voice: descriptor.insert_bos_before_voice,
            temperature_bits: descriptor.temperature.to_bits(),
            eos_threshold_bits: descriptor.eos_threshold.to_bits(),
            graphs: [
                FileStamp::read(&descriptor.graphs.encoder),
                FileStamp::read(&descriptor.graphs.conditioner),
                FileStamp::read(&descriptor.graphs.main),
                FileStamp::read(&descriptor.graphs.flow),
                FileStamp::read(&descriptor.graphs.decoder),
            ],
            weights: FileStamp::read(&descriptor.weights_path),
            tokenizer: FileStamp::read(&descriptor.tokenizer_path),
            voices: descriptor
                .voices
                .iter()
                .map(|voice| (voice.id.clone(), FileStamp::read(&voice.state_path)))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotLoadState {
    Empty,
    Loaded,
    Busy,
}

struct RuntimeSlot<T> {
    key: PocketRuntimeKey,
    runtime: Mutex<Option<T>>,
}

impl<T> RuntimeSlot<T> {
    fn load_state(&self) -> SlotLoadState {
        match self.runtime.try_lock() {
            None => SlotLoadState::Busy,
            Some(guard) if guard.is_some() => SlotLoadState::Loaded,
            Some(_) => SlotLoadState::Empty,
        }
    }

    fn take_if_idle(&self) -> Option<T> {
        self.runtime.try_lock()?.take()
    }
}

/// Process-wide LRU-ish cache of loaded model runtimes. Slots are retained
/// while any provider/queue snapshot holds them, but their ONNX sessions are
/// dropped once the slot is idle and the cache is over its loaded-runtime
/// bound. In-flight slots are skipped and survive until the synthesis ends.
struct RuntimeCache<T> {
    max_loaded: usize,
    slots: Mutex<Vec<Arc<RuntimeSlot<T>>>>,
}

impl<T> RuntimeCache<T> {
    fn new(max_loaded: usize) -> Self {
        Self {
            max_loaded: max_loaded.max(1),
            slots: Mutex::new(Vec::new()),
        }
    }

    fn acquire(&self, key: PocketRuntimeKey) -> Arc<RuntimeSlot<T>> {
        let (slot, removed, evicted) = {
            let mut slots = self.slots.lock();
            let slot = match slots.iter().position(|slot| slot.key == key) {
                Some(index) => {
                    let slot = slots.remove(index);
                    slots.push(Arc::clone(&slot));
                    slot
                }
                None => {
                    let slot = Arc::new(RuntimeSlot {
                        key,
                        runtime: Mutex::new(None),
                    });
                    slots.push(Arc::clone(&slot));
                    slot
                }
            };
            let (removed, evicted) = self.collect_trimmed(&mut slots);
            (slot, removed, evicted)
        };
        drop(removed);
        drop(evicted);
        slot
    }

    fn touch(&self, slot: &Arc<RuntimeSlot<T>>) {
        let (removed, evicted) = {
            let mut slots = self.slots.lock();
            match slots
                .iter()
                .position(|candidate| Arc::ptr_eq(candidate, slot))
            {
                Some(index) => {
                    let slot = slots.remove(index);
                    slots.push(slot);
                }
                None => slots.push(Arc::clone(slot)),
            }
            self.collect_trimmed(&mut slots)
        };
        drop(removed);
        drop(evicted);
    }

    fn trim(&self) {
        let (removed, evicted) = {
            let mut slots = self.slots.lock();
            self.collect_trimmed(&mut slots)
        };
        drop(removed);
        drop(evicted);
    }

    fn collect_trimmed(
        &self,
        slots: &mut Vec<Arc<RuntimeSlot<T>>>,
    ) -> (Vec<Arc<RuntimeSlot<T>>>, Vec<T>) {
        let mut removed = Vec::new();
        let mut evicted = Vec::new();

        // The cache owns no provider lifetime: once nobody else references a
        // slot, remove it (and any sessions it still holds).
        let mut index = 0;
        while index < slots.len() {
            if Arc::strong_count(&slots[index]) == 1 {
                removed.push(slots.remove(index));
            } else {
                index += 1;
            }
        }

        // Evict only idle sessions, oldest (front of the LRU list) first.
        loop {
            let loaded = slots
                .iter()
                .filter(|slot| slot.load_state() != SlotLoadState::Empty)
                .count();
            if loaded <= self.max_loaded {
                break;
            }
            let mut evicted_one = false;
            for slot in slots.iter() {
                if let Some(runtime) = slot.take_if_idle() {
                    evicted.push(runtime);
                    evicted_one = true;
                    break;
                }
            }
            if !evicted_one {
                break;
            }
        }

        (removed, evicted)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.slots.lock().len()
    }
}

type PocketShared = RuntimeSlot<PocketRuntime>;

static RUNTIME_CACHE: OnceLock<RuntimeCache<PocketRuntime>> = OnceLock::new();

fn runtime_cache() -> &'static RuntimeCache<PocketRuntime> {
    RUNTIME_CACHE.get_or_init(|| RuntimeCache::new(MAX_CACHED_RUNTIMES))
}

/// One Pocket TTS model registered in the provider registry.
#[derive(Clone)]
pub struct PocketTts {
    descriptor: Arc<PocketModelDescriptor>,
    selection: Option<PocketVoiceSelection>,
    shared: Arc<PocketShared>,
}

impl std::fmt::Debug for PocketTts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PocketTts")
            .field("id", &self.descriptor.id)
            .field("selection", &self.selection)
            .field("loaded", &self.is_loaded())
            .finish()
    }
}

impl PocketTts {
    pub fn new(descriptor: PocketModelDescriptor, selection: Option<PocketVoiceSelection>) -> Self {
        let shared = runtime_cache().acquire(PocketRuntimeKey::new(&descriptor));
        Self {
            descriptor: Arc::new(descriptor),
            selection,
            shared,
        }
    }

    /// Replace the voice selection while keeping the loaded sessions. The
    /// returned instance is what the registry stores; queue snapshots created
    /// before this call retain the previous selection.
    pub fn with_voice(&self, selection: Option<PocketVoiceSelection>) -> Self {
        Self {
            descriptor: Arc::clone(&self.descriptor),
            selection,
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn descriptor(&self) -> &PocketModelDescriptor {
        &self.descriptor
    }

    pub fn language(&self) -> &str {
        &self.descriptor.language
    }

    pub fn selection(&self) -> Option<&PocketVoiceSelection> {
        self.selection.as_ref()
    }

    pub fn selection_dto(&self) -> Option<PocketSelectionDto> {
        self.selection
            .as_ref()
            .map(|selection| selection.dto(&self.descriptor))
    }

    pub fn readiness(&self) -> PocketReadiness {
        evaluate_readiness(&self.descriptor, self.selection.as_ref())
    }

    pub fn readiness_code(&self) -> Option<&'static str> {
        self.readiness().code()
    }

    pub fn status_string(&self) -> &'static str {
        self.readiness().as_status_str()
    }

    pub fn voice_identity(&self) -> String {
        match &self.selection {
            Some(selection) => selection.identity(&self.descriptor),
            None => "none".to_string(),
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.shared.runtime.lock().is_some()
    }

    /// Validate the selected voice, then load the ONNX sessions.
    pub fn prepare(&self) -> Result<(), String> {
        let readiness = self.readiness();
        if readiness != PocketReadiness::Ready {
            let error = readiness_to_error(readiness);
            warn!(
                package = %self.descriptor.package_id,
                status = %readiness.as_status_str(),
                "Pocket TTS is not ready"
            );
            return Err(error.message());
        }
        let cache = runtime_cache();
        cache.touch(&self.shared);
        let cancel = AtomicBool::new(false);
        let mut guard = self.shared.runtime.lock();
        if guard.is_none() {
            match PocketRuntime::load(&self.descriptor, &cancel) {
                Ok(runtime) => *guard = Some(runtime),
                Err(error) => {
                    warn!(
                        package = %self.descriptor.package_id,
                        error = %error.detail(),
                        "Pocket TTS prepare failed"
                    );
                    drop(guard);
                    cache.trim();
                    return Err(error.message());
                }
            }
        }
        drop(guard);
        cache.trim();
        Ok(())
    }

    /// Synthesis entry point used by the queue. The provided flag is checked by
    /// the worker between generation frames; when the future is dropped the
    /// caller also sets it via [`CancelOnDrop`].
    pub async fn synthesize_cancellable(
        &self,
        text: &str,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<u8>, String> {
        let readiness = self.readiness();
        if readiness != PocketReadiness::Ready {
            return Err(readiness_to_error(readiness).message());
        }
        let selection = self.selection.clone();
        let descriptor = Arc::clone(&self.descriptor);
        let shared = Arc::clone(&self.shared);
        let text = text.to_string();
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let spawn = std::thread::Builder::new()
            .name(format!("pocket-tts-{}", self.descriptor.package_id))
            .spawn(move || {
                let cache = runtime_cache();
                cache.touch(&shared);
                let result = (|| -> Result<Vec<u8>, PocketError> {
                    let mut guard = shared.runtime.lock();
                    if guard.is_none() {
                        *guard = Some(PocketRuntime::load(&descriptor, &worker_cancel)?);
                    }
                    if worker_cancel.load(Ordering::Relaxed) {
                        return Err(PocketError::Cancelled);
                    }
                    let runtime = guard.as_mut().ok_or_else(|| {
                        PocketError::Runtime("runtime disappeared after loading".to_string())
                    })?;
                    let output = runtime.synthesize(&text, selection.as_ref(), &worker_cancel)?;
                    Ok(output.wav)
                })();
                cache.trim();
                let _ = sender.send(result);
            });
        if let Err(error) = spawn {
            return Err(format!("failed to start Pocket TTS worker: {error}"));
        }
        match receiver.await {
            Ok(Ok(wav)) => Ok(wav),
            Ok(Err(error)) => Err(error.message()),
            Err(_) => Err("Pocket TTS worker stopped unexpectedly".to_string()),
        }
    }
}

fn readiness_to_error(readiness: PocketReadiness) -> PocketError {
    match readiness {
        PocketReadiness::Ready => PocketError::Generation("unexpected not-ready state".to_string()),
        PocketReadiness::NeedsReference => PocketError::ReferenceRequired,
        PocketReadiness::ReferenceMissing => PocketError::ReferenceMissing,
        PocketReadiness::ReferenceInvalid => {
            PocketError::ReferenceInvalid("invalid reference".to_string())
        }
        PocketReadiness::VoiceMissing => PocketError::VoiceMissing("selected voice".to_string()),
        PocketReadiness::PackageInvalid => {
            PocketError::Package("package assets are missing".to_string())
        }
    }
}

/// Dropping the synthesis future sets the worker's cancellation flag. The
/// worker observes it at the next generation-frame boundary and releases the
/// shared runtime lock, so the next queued task is not blocked by a cancelled
/// one.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[async_trait]
impl TtsEngine for PocketTts {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let guard = CancelOnDrop(Arc::clone(&cancel));
        let result = self.synthesize_cancellable(text, cancel).await;
        drop(guard);
        result
    }
}

impl PocketReadiness {
    pub fn as_status_str(self) -> &'static str {
        match self {
            PocketReadiness::Ready => "ready",
            PocketReadiness::NeedsReference => "needs_reference",
            PocketReadiness::ReferenceMissing => "reference_missing",
            PocketReadiness::ReferenceInvalid => "reference_invalid",
            PocketReadiness::VoiceMissing => "voice_missing",
            PocketReadiness::PackageInvalid => "package_invalid",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::pocket::descriptor::{PocketGraphs, PocketVoiceDescriptor, PocketVoiceMode};
    use crate::tts::pocket::PocketTokenizerKind;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;

    fn descriptor() -> PocketModelDescriptor {
        let root = std::env::temp_dir();
        PocketModelDescriptor {
            id: "local-pocket:test".to_string(),
            package_id: "test".to_string(),
            display_name: "Test".to_string(),
            language: "ru".to_string(),
            format: "pocket-tts-onnx".to_string(),
            format_version: 1,
            package_root: root.clone(),
            config_path: root.join("config.yaml"),
            weights_path: root.join("model.safetensors"),
            tokenizer_path: root.join("tokenizer.model"),
            tokenizer_kind: PocketTokenizerKind::SentencePiece,
            n_bins: 3,
            sample_rate: 24000,
            frame_rate: 12.5,
            num_layers: 1,
            latent_dim: 32,
            insert_bos_before_voice: true,
            temperature: 0.3,
            eos_threshold: -4.0,
            voice_modes: vec![PocketVoiceMode::ReferenceWav],
            voices: Vec::new(),
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

    #[test]
    fn provider_clone_keeps_selection_snapshot() {
        let first_path = PathBuf::from("C:/voices/first.wav");
        let second_path = PathBuf::from("C:/voices/second.wav");
        let provider = PocketTts::new(
            descriptor(),
            Some(PocketVoiceSelection::ReferenceWav(first_path.clone())),
        );
        let snapshot = provider.clone();
        let updated = provider.with_voice(Some(PocketVoiceSelection::ReferenceWav(second_path)));
        assert_eq!(
            snapshot.selection(),
            Some(&PocketVoiceSelection::ReferenceWav(first_path))
        );
        assert!(matches!(
            updated.selection(),
            Some(PocketVoiceSelection::ReferenceWav(path)) if path.ends_with("second.wav")
        ));
    }

    #[test]
    fn readiness_is_exposed_as_stable_status_string() {
        assert_eq!(
            PocketReadiness::NeedsReference.as_status_str(),
            "needs_reference"
        );
        assert_eq!(
            PocketReadiness::ReferenceMissing.as_status_str(),
            "reference_missing"
        );
        assert_eq!(
            PocketReadiness::ReferenceInvalid.as_status_str(),
            "reference_invalid"
        );
        assert_eq!(PocketReadiness::Ready.as_status_str(), "ready");
    }

    #[test]
    fn voice_modes_and_voices_are_preserved_in_descriptor() {
        let mut descriptor = descriptor();
        descriptor.voice_modes = vec![PocketVoiceMode::VoiceState];
        descriptor.voices = vec![PocketVoiceDescriptor {
            id: "alba".to_string(),
            display_name: "Alba".to_string(),
            state_path: std::env::temp_dir().join("alba.safetensors"),
        }];
        let provider = PocketTts::new(descriptor, None);
        assert_eq!(provider.language(), "ru");
        assert!(provider.readiness_code().is_some());
    }

    #[tokio::test]
    async fn cancelled_synthesis_does_not_poison_provider() {
        let provider = PocketTts::new(descriptor(), None);
        let cancel = Arc::new(AtomicBool::new(true));
        let result = provider.synthesize_cancellable("привет", cancel).await;
        assert!(result.is_err());
        // The provider is still constructible and has no poisoned lock.
        assert!(!provider.is_loaded());
    }

    #[derive(Debug)]
    struct TestRuntime(Arc<AtomicUsize>);

    impl Drop for TestRuntime {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn cache_key(name: &str) -> PocketRuntimeKey {
        let mut descriptor = descriptor();
        descriptor.id = format!("local-pocket:{name}");
        PocketRuntimeKey::new(&descriptor)
    }

    #[test]
    fn provider_instances_share_one_runtime_slot_per_model() {
        let first = PocketTts::new(descriptor(), None);
        let second = PocketTts::new(descriptor(), None);
        assert!(Arc::ptr_eq(&first.shared, &second.shared));

        let updated = first.with_voice(None);
        assert!(Arc::ptr_eq(&first.shared, &updated.shared));
    }

    #[test]
    fn runtime_cache_bounds_idle_sessions_and_reuses_slots() {
        let cache = RuntimeCache::new(2);
        let drops = Arc::new(AtomicUsize::new(0));
        let key_a = cache_key("cache-a");
        let a = cache.acquire(key_a.clone());
        *a.runtime.lock() = Some(TestRuntime(Arc::clone(&drops)));
        let b = cache.acquire(cache_key("cache-b"));
        *b.runtime.lock() = Some(TestRuntime(Arc::clone(&drops)));
        let c = cache.acquire(cache_key("cache-c"));
        *c.runtime.lock() = Some(TestRuntime(Arc::clone(&drops)));

        cache.trim();

        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(a.runtime.lock().is_none());
        assert!(b.runtime.lock().is_some());
        assert!(c.runtime.lock().is_some());

        let a_again = cache.acquire(key_a);
        assert!(Arc::ptr_eq(&a, &a_again));
    }

    #[test]
    fn runtime_cache_never_evicts_an_in_flight_session() {
        let cache = RuntimeCache::new(2);
        let drops = Arc::new(AtomicUsize::new(0));
        let a = cache.acquire(cache_key("busy-a"));
        *a.runtime.lock() = Some(TestRuntime(Arc::clone(&drops)));
        let b = cache.acquire(cache_key("busy-b"));
        *b.runtime.lock() = Some(TestRuntime(Arc::clone(&drops)));

        let a_guard = a.runtime.lock();
        let c = cache.acquire(cache_key("busy-c"));
        *c.runtime.lock() = Some(TestRuntime(Arc::clone(&drops)));
        cache.trim();

        assert!(
            a.runtime.try_lock().is_none(),
            "in-flight slot stays locked"
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(b.runtime.lock().is_none());
        assert!(c.runtime.lock().is_some());

        drop(a_guard);
        cache.trim();
        assert!(a.runtime.lock().is_some());
    }

    #[test]
    fn evicted_idle_runtime_keeps_queued_snapshot_slot_usable() {
        let cache = RuntimeCache::new(1);
        let drops = Arc::new(AtomicUsize::new(0));
        let key_a = cache_key("snapshot-a");
        let a = cache.acquire(key_a.clone());
        *a.runtime.lock() = Some(TestRuntime(Arc::clone(&drops)));
        let snapshot = Arc::clone(&a);

        let b = cache.acquire(cache_key("snapshot-b"));
        *b.runtime.lock() = Some(TestRuntime(Arc::clone(&drops)));
        cache.trim();

        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(snapshot.runtime.lock().is_none());
        let reloaded = cache.acquire(key_a);
        assert!(Arc::ptr_eq(&snapshot, &reloaded));
        *reloaded.runtime.lock() = Some(TestRuntime(Arc::clone(&drops)));
        assert!(snapshot.runtime.lock().is_some());
    }

    #[test]
    fn runtime_cache_drops_slots_after_all_references_are_gone() {
        let cache = RuntimeCache::new(2);
        let drops = Arc::new(AtomicUsize::new(0));
        {
            let only = cache.acquire(cache_key("orphan"));
            *only.runtime.lock() = Some(TestRuntime(Arc::clone(&drops)));
        }
        assert_eq!(cache.len(), 1);

        cache.trim();

        assert_eq!(cache.len(), 0);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
