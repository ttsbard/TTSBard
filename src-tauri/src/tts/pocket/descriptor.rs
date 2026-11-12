//! Discovery and validation of Pocket TTS model packages.
//!
//! A package is a directory under `models/pocket` with `config.yaml`, weights,
//! a tokenizer and the exported ONNX graphs. The graphs may live in the package
//! root or in an `onnx/` subdirectory. Paths read from `config.yaml` must stay
//! inside the package; absolute paths and `..` traversal are rejected.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use super::tokenizer::PocketTokenizer;
use super::PocketError;

pub const MODELS_SUBDIR: &str = "models/pocket";
pub const ID_PREFIX: &str = "local-pocket";
pub const PACKAGE_FORMAT: &str = "pocket-tts-onnx";
pub const PACKAGE_FORMAT_VERSION: u32 = 1;
pub const DEFAULT_SAMPLE_RATE: u32 = 24_000;
pub const DEFAULT_FRAME_RATE: f32 = 12.5;
pub const DEFAULT_N_BINS: usize = 4000;
pub const DEFAULT_NUM_LAYERS: usize = 6;
pub const DEFAULT_LATENT_DIM: usize = 32;
pub const DEFAULT_TEMPERATURE: f32 = 0.3;
pub const DEFAULT_EOS_THRESHOLD: f32 = -4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PocketTokenizerKind {
    SentencePiece,
    Json,
}

impl PocketTokenizerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PocketTokenizerKind::SentencePiece => "sentencepiece",
            PocketTokenizerKind::Json => "tokenizer_json",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PocketVoiceMode {
    ReferenceWav,
    VoiceState,
}

impl PocketVoiceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PocketVoiceMode::ReferenceWav => "reference_wav",
            PocketVoiceMode::VoiceState => "voice_state",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PocketVoiceDescriptor {
    pub id: String,
    pub display_name: String,
    pub state_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PocketGraphs {
    pub encoder: PathBuf,
    pub conditioner: PathBuf,
    pub main: PathBuf,
    pub flow: PathBuf,
    pub decoder: PathBuf,
    pub quantized: bool,
}

#[derive(Debug, Clone)]
pub struct PocketModelDescriptor {
    /// Stable provider/registry ID (`local-pocket:<package-id>`).
    pub id: String,
    /// Directory name under `models/pocket`.
    pub package_id: String,
    pub display_name: String,
    pub language: String,
    pub format: String,
    pub format_version: u32,
    pub package_root: PathBuf,
    pub config_path: PathBuf,
    pub weights_path: PathBuf,
    pub tokenizer_path: PathBuf,
    pub tokenizer_kind: PocketTokenizerKind,
    pub n_bins: usize,
    pub sample_rate: u32,
    pub frame_rate: f32,
    pub num_layers: usize,
    pub latent_dim: usize,
    pub insert_bos_before_voice: bool,
    pub temperature: f32,
    pub eos_threshold: f32,
    pub voice_modes: Vec<PocketVoiceMode>,
    pub voices: Vec<PocketVoiceDescriptor>,
    pub graphs: PocketGraphs,
    pub(crate) asset_paths: Vec<PathBuf>,
}

impl PocketModelDescriptor {
    /// Check that every asset recorded at discovery is still a file. The
    /// readiness contract reports a package that disappeared after discovery
    /// instead of failing later inside ONNX Runtime.
    pub fn assets_present(&self) -> bool {
        self.asset_paths.iter().all(|path| path.is_file())
    }

    pub fn primary_voice_mode(&self) -> PocketVoiceMode {
        if self.voices.is_empty() {
            PocketVoiceMode::ReferenceWav
        } else {
            PocketVoiceMode::VoiceState
        }
    }

    pub fn voice(&self, id: &str) -> Option<&PocketVoiceDescriptor> {
        self.voices.iter().find(|voice| voice.id == id)
    }
}

/// Discover packages under `root/models/pocket`, creating the directory when
/// the root is managed (writable). Returns only complete, validated packages.
pub fn discover_pocket_models(root: &Path) -> Vec<PocketModelDescriptor> {
    let models_dir = root.join(MODELS_SUBDIR);
    if let Err(error) = std::fs::create_dir_all(&models_dir) {
        tracing::warn!(
            dir = %crate::secret_log::safe_path_for_log(&models_dir),
            error = %error,
            "Failed to create Pocket TTS models directory"
        );
        return Vec::new();
    }
    scan_pocket_models_dir(&models_dir)
}

/// Read-only variant for the process cwd root: never creates directories.
pub fn scan_pocket_models_root(root: &Path) -> Vec<PocketModelDescriptor> {
    scan_pocket_models_dir(&root.join(MODELS_SUBDIR))
}

pub fn scan_pocket_models_dir(models_dir: &Path) -> Vec<PocketModelDescriptor> {
    let entries = match std::fs::read_dir(models_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(
                dir = %crate::secret_log::safe_path_for_log(models_dir),
                error = %error,
                "Failed to read Pocket TTS models directory"
            );
            return Vec::new();
        }
    };

    let mut descriptors = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(package_id) = path.file_name().and_then(|name| name.to_str()) else {
            tracing::warn!(
                path = %crate::secret_log::safe_path_for_log(&path),
                "Skipping Pocket TTS package with non-UTF-8 name"
            );
            continue;
        };
        if package_id.starts_with('.') {
            continue;
        }
        match scan_package(&path, package_id) {
            Ok(descriptor) => descriptors.push(descriptor),
            Err(error) => {
                tracing::warn!(
                    package = %crate::secret_log::safe_path_for_log(&path),
                    error = %error.detail(),
                    "Skipping invalid Pocket TTS package"
                );
            }
        }
    }
    descriptors.sort_by(|left, right| left.id.cmp(&right.id));
    descriptors
}

/// Merge validated per-root lists; the earlier root wins on duplicate IDs and
/// the result is sorted for deterministic UI order.
pub fn merge_pocket_roots(per_root: Vec<Vec<PocketModelDescriptor>>) -> Vec<PocketModelDescriptor> {
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();
    for descriptors in per_root {
        for descriptor in descriptors {
            if seen.insert(descriptor.id.clone()) {
                merged.push(descriptor);
            }
        }
    }
    merged.sort_by(|left, right| left.id.cmp(&right.id));
    merged
}

fn scan_package(
    package_root: &Path,
    package_id: &str,
) -> Result<PocketModelDescriptor, PocketError> {
    let package_root = package_root
        .canonicalize()
        .map_err(|error| PocketError::Package(format!("cannot resolve package root: {error}")))?;
    let config_path = package_root.join("config.yaml");
    if !config_path.is_file() {
        return Err(PocketError::Package("config.yaml is missing".to_string()));
    }
    let config_text = std::fs::read_to_string(&config_path)
        .map_err(|error| PocketError::Package(format!("cannot read config.yaml: {error}")))?;
    let config = parse_config(&config_text);

    let weights_value = required(&config, "weights_path")?;
    if weights_value.contains("://") {
        return Err(PocketError::Package(
            "remote weights_path is not supported; package must contain local weights".to_string(),
        ));
    }
    let weights_path = checked_package_file(&package_root, weights_value, "weights_path")?;

    let tokenizer_value = required(&config, "flow_lm.lookup_table.tokenizer_path")?;
    if tokenizer_value.contains("://") {
        return Err(PocketError::Package(
            "remote tokenizer_path is not supported; package must contain the local tokenizer"
                .to_string(),
        ));
    }
    let tokenizer_path = checked_package_file(&package_root, tokenizer_value, "tokenizer_path")?;
    let tokenizer_name = config
        .get("flow_lm.lookup_table.tokenizer")
        .map(String::as_str)
        .unwrap_or("");
    let tokenizer_kind = infer_tokenizer_kind(tokenizer_name, &tokenizer_path)?;

    let n_bins = parse_usize(&config, "flow_lm.lookup_table.n_bins").unwrap_or(DEFAULT_N_BINS);
    PocketTokenizer::validate(&tokenizer_path, tokenizer_kind, n_bins)?;

    let sample_rate = parse_u32(&config, "mimi.sample_rate").unwrap_or(DEFAULT_SAMPLE_RATE);
    if sample_rate != DEFAULT_SAMPLE_RATE {
        return Err(PocketError::Package(format!(
            "mimi.sample_rate {sample_rate} is unsupported; Pocket TTS graphs output 24 kHz"
        )));
    }
    let frame_rate = parse_f32(&config, "mimi.frame_rate").unwrap_or(DEFAULT_FRAME_RATE);
    if !(frame_rate.is_finite() && frame_rate > 0.0) {
        return Err(PocketError::Package(format!(
            "mimi.frame_rate {frame_rate} is invalid"
        )));
    }
    let flow_type = config
        .get("flow_lm.flow.type")
        .or_else(|| config.get("flow_lm.flow_type"))
        .map(String::as_str)
        .unwrap_or("lsd");
    if !flow_type.eq_ignore_ascii_case("lsd") {
        return Err(PocketError::Package(format!(
            "flow type '{flow_type}' is unsupported; this runtime implements LSD"
        )));
    }
    let num_layers =
        parse_usize(&config, "flow_lm.transformer.num_layers").unwrap_or(DEFAULT_NUM_LAYERS);
    if num_layers == 0 || num_layers > 64 {
        return Err(PocketError::Package(format!(
            "flow_lm.transformer.num_layers {num_layers} is invalid"
        )));
    }
    let latent_dim = parse_usize(&config, "mimi.quantizer.dimension").unwrap_or(DEFAULT_LATENT_DIM);
    if latent_dim == 0 || latent_dim > 1024 {
        return Err(PocketError::Package(format!(
            "mimi.quantizer.dimension {latent_dim} is invalid"
        )));
    }
    let insert_bos_before_voice = parse_bool(&config, "flow_lm.insert_bos_before_voice", false);
    let weights_extension = weights_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("safetensors"))
        .unwrap_or(false);
    if !weights_extension {
        return Err(PocketError::Package(
            "weights_path must point at a .safetensors file".to_string(),
        ));
    }
    let weights = super::safetensors::SafeTensorsFile::open(&weights_path)
        .map_err(|error| PocketError::Package(format!("invalid weights file: {error}")))?;
    if insert_bos_before_voice && !weights.contains("flow_lm.bos_before_voice") {
        return Err(PocketError::Package(
            "weights file is missing flow_lm.bos_before_voice".to_string(),
        ));
    }
    drop(weights);
    let temperature = parse_f32(&config, "default_temperature").unwrap_or(DEFAULT_TEMPERATURE);
    let eos_threshold =
        parse_f32(&config, "default_eos_threshold").unwrap_or(DEFAULT_EOS_THRESHOLD);

    let manifest = read_manifest(&package_root)?;
    let language = manifest
        .as_ref()
        .and_then(|manifest| manifest.language.clone())
        .or_else(|| language_from_readme(&package_root))
        .unwrap_or_else(|| match tokenizer_kind {
            PocketTokenizerKind::SentencePiece => "ru".to_string(),
            PocketTokenizerKind::Json => "en".to_string(),
        });
    let display_name = manifest
        .as_ref()
        .and_then(|manifest| manifest.display_name.clone())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| display_name_from_readme(&package_root))
        .unwrap_or_else(|| package_id.to_string());
    let format = manifest
        .as_ref()
        .and_then(|manifest| manifest.format.clone())
        .unwrap_or_else(|| PACKAGE_FORMAT.to_string());
    let format_version = manifest
        .as_ref()
        .and_then(|manifest| manifest.format_version)
        .unwrap_or(PACKAGE_FORMAT_VERSION);

    let graphs = select_graphs(&package_root)?;
    let voices = discover_voices(&package_root, manifest.as_ref())?;
    let mut voice_modes = Vec::new();
    if !voices.is_empty() {
        voice_modes.push(PocketVoiceMode::VoiceState);
    }
    let reference_requested = manifest
        .as_ref()
        .and_then(|manifest| manifest.voice_modes.as_ref())
        .map(|modes| modes.iter().any(|mode| mode == "reference_wav"))
        .unwrap_or(voices.is_empty());
    if reference_requested || voices.is_empty() {
        voice_modes.push(PocketVoiceMode::ReferenceWav);
    }
    let mut asset_paths = vec![
        config_path.clone(),
        weights_path.clone(),
        tokenizer_path.clone(),
        graphs.encoder.clone(),
        graphs.conditioner.clone(),
        graphs.main.clone(),
        graphs.flow.clone(),
        graphs.decoder.clone(),
    ];
    asset_paths.extend(voices.iter().map(|voice| voice.state_path.clone()));

    Ok(PocketModelDescriptor {
        id: format!("{ID_PREFIX}:{package_id}"),
        package_id: package_id.to_string(),
        display_name,
        language,
        format,
        format_version,
        package_root,
        config_path,
        weights_path,
        tokenizer_path,
        tokenizer_kind,
        n_bins,
        sample_rate,
        frame_rate,
        num_layers,
        latent_dim,
        insert_bos_before_voice,
        temperature,
        eos_threshold,
        voice_modes,
        voices,
        graphs,
        asset_paths,
    })
}

fn infer_tokenizer_kind(
    tokenizer_name: &str,
    path: &Path,
) -> Result<PocketTokenizerKind, PocketError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    if tokenizer_name.eq_ignore_ascii_case("sentencepiece")
        || extension.eq_ignore_ascii_case("model")
    {
        if !extension.eq_ignore_ascii_case("model") {
            return Err(PocketError::Package(
                "SentencePiece tokenizer must have a .model extension".to_string(),
            ));
        }
        return Ok(PocketTokenizerKind::SentencePiece);
    }
    if tokenizer_name.eq_ignore_ascii_case("tokenizers")
        || tokenizer_name.eq_ignore_ascii_case("json")
        || extension.eq_ignore_ascii_case("json")
    {
        if !extension.eq_ignore_ascii_case("json") {
            return Err(PocketError::Package(
                "tokenizer.json config must point at a .json file".to_string(),
            ));
        }
        return Ok(PocketTokenizerKind::Json);
    }
    Err(PocketError::Package(format!(
        "unsupported tokenizer kind '{tokenizer_name}' for '{}'",
        path.display()
    )))
}

fn select_graphs(package_root: &Path) -> Result<PocketGraphs, PocketError> {
    let bases = [package_root.to_path_buf(), package_root.join("onnx")];
    let mut fp32 = None;
    let mut int8 = None;
    for base in &bases {
        let main = base.join("flow_lm_main.onnx");
        let flow = base.join("flow_lm_flow.onnx");
        let decoder = base.join("mimi_decoder.onnx");
        if fp32.is_none() && main.is_file() && flow.is_file() && decoder.is_file() {
            fp32 = Some((main, flow, decoder, base.clone()));
        }
        let main_i8 = base.join("flow_lm_main_int8.onnx");
        let flow_i8 = base.join("flow_lm_flow_int8.onnx");
        let decoder_i8 = base.join("mimi_decoder_int8.onnx");
        if int8.is_none() && main_i8.is_file() && flow_i8.is_file() && decoder_i8.is_file() {
            int8 = Some((main_i8, flow_i8, decoder_i8, base.clone()));
        }
    }

    let (main, flow, decoder, base, quantized) = if let Some((main, flow, decoder, base)) = int8 {
        (main, flow, decoder, base, true)
    } else if let Some((main, flow, decoder, base)) = fp32 {
        (main, flow, decoder, base, false)
    } else {
        return Err(PocketError::Package(
            "ONNX graphs flow_lm_main/flow_lm_flow/mimi_decoder are missing".to_string(),
        ));
    };

    let encoder = find_aux_graph(package_root, &base, "mimi_encoder.onnx")?;
    let conditioner = find_aux_graph(package_root, &base, "text_conditioner.onnx")?;
    for path in [&main, &flow, &decoder, &encoder, &conditioner] {
        let size = std::fs::metadata(path)
            .map_err(|error| PocketError::Package(format!("cannot stat graph: {error}")))?
            .len();
        if size == 0 {
            return Err(PocketError::Package(format!(
                "graph '{}' is empty",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("?")
            )));
        }
    }
    Ok(PocketGraphs {
        encoder,
        conditioner,
        main,
        flow,
        decoder,
        quantized,
    })
}

fn find_aux_graph(
    package_root: &Path,
    graph_base: &Path,
    name: &str,
) -> Result<PathBuf, PocketError> {
    for base in [graph_base, package_root, &package_root.join("onnx")] {
        let path = base.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(PocketError::Package(format!("{name} is missing")))
}

#[derive(Debug, serde::Deserialize)]
struct PocketManifest {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    format_version: Option<u32>,
    #[serde(default)]
    voice_modes: Option<Vec<String>>,
    #[serde(default)]
    voices: Option<Vec<ManifestVoice>>,
}

#[derive(Debug, serde::Deserialize)]
struct ManifestVoice {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    path: String,
}

fn read_manifest(package_root: &Path) -> Result<Option<PocketManifest>, PocketError> {
    let path = package_root.join("pocket.json");
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| PocketError::Package(format!("cannot read pocket.json: {error}")))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| PocketError::Package(format!("invalid pocket.json: {error}")))
}

fn discover_voices(
    package_root: &Path,
    manifest: Option<&PocketManifest>,
) -> Result<Vec<PocketVoiceDescriptor>, PocketError> {
    if let Some(manifest) = manifest {
        if let Some(voices) = &manifest.voices {
            let mut result = Vec::new();
            for voice in voices {
                if voice.id.trim().is_empty() {
                    return Err(PocketError::Package(
                        "pocket.json voice id must not be empty".to_string(),
                    ));
                }
                let state_path =
                    checked_package_file(package_root, &voice.path, "voice state path")?;
                validate_voice_file(&state_path)?;
                result.push(PocketVoiceDescriptor {
                    id: voice.id.clone(),
                    display_name: voice
                        .display_name
                        .clone()
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or_else(|| voice.id.clone()),
                    state_path,
                });
            }
            return Ok(result);
        }
    }

    let embeddings = package_root.join("embeddings");
    let entries = match std::fs::read_dir(&embeddings) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(PocketError::Package(format!(
                "cannot read embeddings directory: {error}"
            )));
        }
    };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file()
            || path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| !extension.eq_ignore_ascii_case("safetensors"))
                .unwrap_or(true)
        {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let state_path = checked_package_file(
            package_root,
            path.strip_prefix(package_root)
                .map_err(|_| PocketError::Package("voice path escaped package".to_string()))?
                .to_str()
                .ok_or_else(|| PocketError::Package("voice path is not UTF-8".to_string()))?,
            "voice state path",
        )?;
        validate_voice_file(&state_path)?;
        result.push(PocketVoiceDescriptor {
            id: stem.to_string(),
            display_name: capitalize_voice_name(stem),
            state_path,
        });
    }
    result.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(result)
}

fn validate_voice_file(path: &Path) -> Result<(), PocketError> {
    let mut file = super::safetensors::SafeTensorsFile::open(path)
        .map_err(|error| PocketError::Package(format!("invalid voice state: {error}")))?;
    // The graph has six transformer layers in the supported packages. Validate
    // the keys for the layers that are present instead of hardcoding six, so a
    // future compatible set is not rejected by discovery.
    let mut layers = std::collections::BTreeSet::new();
    for key in file.keys() {
        if let Some(rest) = key.strip_prefix("transformer.layers.") {
            if let Some((layer, _)) = rest.split_once('.') {
                if let Ok(layer) = layer.parse::<usize>() {
                    layers.insert(layer);
                }
            }
        }
    }
    if layers.is_empty() {
        return Err(PocketError::Package(
            "voice state has no transformer layer caches".to_string(),
        ));
    }
    let layers_count = *layers.iter().next_back().unwrap_or(&0) + 1;
    file.validate_pocket_voice(layers_count)
        .map_err(|error| PocketError::Package(format!("invalid voice state: {error}")))
}

fn checked_package_file(
    package_root: &Path,
    relative: &str,
    field: &str,
) -> Result<PathBuf, PocketError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(PocketError::Package(format!(
            "{field} must be a relative path inside the package"
        )));
    }
    let full = package_root.join(relative_path);
    if !full.is_file() {
        return Err(PocketError::Package(format!("{field} does not exist")));
    }
    let canonical = full.canonicalize().map_err(|error| {
        PocketError::Package(format!(
            "cannot resolve {field} '{}': {error}",
            full.display()
        ))
    })?;
    let root = package_root
        .canonicalize()
        .map_err(|error| PocketError::Package(format!("cannot resolve package root: {error}")))?;
    let root_text = root.to_string_lossy().to_lowercase();
    let canonical_text = canonical.to_string_lossy().to_lowercase();
    let separator = std::path::MAIN_SEPARATOR;
    if canonical_text != root_text
        && !canonical_text.starts_with(&format!("{root_text}{separator}"))
        && !canonical_text.starts_with(&format!("{root_text}\\"))
        && !canonical_text.starts_with(&format!("{root_text}/"))
    {
        return Err(PocketError::Package(format!(
            "{field} resolves outside the package"
        )));
    }
    Ok(full)
}

fn required<'a>(values: &'a HashMap<String, String>, key: &str) -> Result<&'a str, PocketError> {
    values
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| PocketError::Package(format!("config.yaml is missing {key}")))
}

fn parse_usize(values: &HashMap<String, String>, key: &str) -> Option<usize> {
    values.get(key)?.trim().parse().ok()
}

fn parse_u32(values: &HashMap<String, String>, key: &str) -> Option<u32> {
    values.get(key)?.trim().parse().ok()
}

fn parse_f32(values: &HashMap<String, String>, key: &str) -> Option<f32> {
    values.get(key)?.trim().parse().ok()
}

fn parse_bool(values: &HashMap<String, String>, key: &str, default: bool) -> bool {
    match values.get(key).map(|value| value.trim()) {
        Some("true" | "True" | "TRUE" | "yes" | "on") => true,
        Some("false" | "False" | "FALSE" | "no" | "off") => false,
        _ => default,
    }
}

/// Small YAML subset parser sufficient for Pocket TTS configs.
///
/// It records dotted scalar paths (`flow_lm.lookup_table.n_bins`), ignores list
/// items, comments and block scalars. The real configs are simple key/value
/// trees; adding a full YAML dependency for this is unnecessary.
fn parse_config(text: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    for raw_line in text.lines() {
        let line = strip_yaml_comment(raw_line);
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('-') || trimmed == "---" {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        while stack
            .last()
            .map(|(parent_indent, _)| *parent_indent >= indent)
            .unwrap_or(false)
        {
            stack.pop();
        }
        if value.trim().is_empty() {
            stack.push((indent, key.to_string()));
            continue;
        }
        let mut full = String::new();
        for (_, parent) in &stack {
            if !full.is_empty() {
                full.push('.');
            }
            full.push_str(parent);
        }
        if !full.is_empty() {
            full.push('.');
        }
        full.push_str(key);
        values.insert(full, unquote_yaml(value.trim()));
    }
    values
}

fn strip_yaml_comment(line: &str) -> &str {
    let mut quote = None;
    for (index, ch) in line.char_indices() {
        match ch {
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            ch if Some(ch) == quote => quote = None,
            '#' if quote.is_none()
                && (index == 0
                    || line[..index].ends_with(' ')
                    || line[..index].ends_with('\t')) =>
            {
                return &line[..index];
            }
            _ => {}
        }
    }
    line
}

fn unquote_yaml(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        let last = value.as_bytes()[value.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

fn language_from_readme(package_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(package_root.join("README.md")).ok()?;
    let mut in_frontmatter = false;
    let mut lines = text.lines();
    let mut language_key = false;
    for line in lines.by_ref() {
        let trimmed = line.trim();
        if trimmed == "---" {
            if in_frontmatter {
                break;
            }
            in_frontmatter = true;
            continue;
        }
        if !in_frontmatter {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("language:") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.trim_matches(['"', '\'']).to_string());
            }
            language_key = true;
            continue;
        }
        if language_key {
            if let Some(value) = trimmed.strip_prefix('-') {
                let value = value.trim().trim_matches(['"', '\'']);
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn display_name_from_readme(package_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(package_root.join("README.md")).ok()?;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            let title = title.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

fn capitalize_voice_name(stem: &str) -> String {
    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => stem.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ttsbard-pocket-scan-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_minimal_package(
        root: &Path,
        package: &str,
        tokenizer_name: &str,
        tokenizer_path: &str,
    ) {
        let dir = root.join(MODELS_SUBDIR).join(package);
        std::fs::create_dir_all(&dir).unwrap();
        let config = format!(
            concat!(
                "weights_path: model.safetensors\n",
                "default_temperature: 0.3\n",
                "flow_lm:\n",
                "  insert_bos_before_voice: true\n",
                "  transformer:\n",
                "    num_layers: 1\n",
                "  lookup_table:\n",
                "    n_bins: 3\n",
                "    tokenizer: {}\n",
                "    tokenizer_path: {}\n",
                "mimi:\n",
                "  sample_rate: 24000\n",
                "  frame_rate: 12.5\n",
                "  quantizer:\n",
                "    dimension: 32\n",
            ),
            tokenizer_name, tokenizer_path
        );
        std::fs::write(dir.join("config.yaml"), config).unwrap();
        write_bos_weights(&dir.join("model.safetensors"));
        std::fs::write(dir.join("mimi_encoder.onnx"), b"not-empty-but-not-onnx").unwrap();
        std::fs::write(dir.join("text_conditioner.onnx"), b"x").unwrap();
        std::fs::write(dir.join("flow_lm_main.onnx"), b"x").unwrap();
        std::fs::write(dir.join("flow_lm_flow.onnx"), b"x").unwrap();
        std::fs::write(dir.join("mimi_decoder.onnx"), b"x").unwrap();
        if tokenizer_path.ends_with(".model") {
            write_sentencepiece_tokenizer(&dir.join(tokenizer_path));
        } else {
            let tokenizer = serde_json::json!({
                "version": "1.0",
                "truncation": null,
                "padding": null,
                "added_tokens": [],
                "normalizer": null,
                "pre_tokenizer": null,
                "post_processor": null,
                "decoder": null,
                "model": {
                    "type": "WordLevel",
                    "vocab": {"[UNK]": 0, "a": 1, "b": 2},
                    "unk_token": "[UNK]"
                }
            });
            std::fs::write(dir.join(tokenizer_path), tokenizer.to_string()).unwrap();
        }
    }

    fn write_bos_weights(path: &Path) {
        let header = serde_json::json!({
            "flow_lm.bos_before_voice": {
                "dtype": "F32",
                "shape": [1, 1, 1],
                "data_offsets": [0, 4]
            }
        })
        .to_string();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(header.len() as u64).to_le_bytes());
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(&0.5f32.to_le_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    fn write_sentencepiece_tokenizer(path: &Path) {
        // ModelProto { pieces: SentencePiece { piece: "<unk>" },
        //              pieces: SentencePiece { piece: "a" },
        //              pieces: SentencePiece { piece: "b" } }
        let mut pieces = Vec::new();
        for piece in ["<unk>", "a", "b"] {
            let mut message = Vec::new();
            message.push(0x0A);
            message.push(piece.len() as u8);
            message.extend_from_slice(piece.as_bytes());
            let mut field = vec![0x0A, message.len() as u8];
            field.extend_from_slice(&message);
            pieces.extend_from_slice(&field);
        }
        std::fs::write(path, pieces).unwrap();
    }

    #[test]
    fn discovers_sentencepiece_package_and_uses_ru_language() {
        let root = temp_root("sp");
        write_minimal_package(&root, "shtorm-test", "sentencepiece", "tokenizer.model");
        std::fs::write(
            root.join(MODELS_SUBDIR)
                .join("shtorm-test")
                .join("README.md"),
            "---\nlanguage:\n- ru\n---\n# Shtorm Test\n",
        )
        .unwrap();
        let models = discover_pocket_models(&root);
        assert_eq!(models.len(), 1);
        let model = &models[0];
        assert_eq!(model.id, "local-pocket:shtorm-test");
        assert_eq!(model.language, "ru");
        assert_eq!(model.tokenizer_kind, PocketTokenizerKind::SentencePiece);
        assert!(model.voice_modes.contains(&PocketVoiceMode::ReferenceWav));
        assert_eq!(model.display_name, "Shtorm Test");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn discovers_json_package_and_voice_state() {
        let root = temp_root("json");
        write_minimal_package(&root, "kyutai-test", "tokenizers", "tokenizer.json");
        let dir = root.join(MODELS_SUBDIR).join("kyutai-test");
        std::fs::create_dir_all(dir.join("embeddings")).unwrap();
        write_voice_state(&dir.join("embeddings/alba.safetensors"));
        let models = discover_pocket_models(&root);
        assert_eq!(models.len(), 1);
        let model = &models[0];
        assert_eq!(model.language, "en");
        assert_eq!(model.tokenizer_kind, PocketTokenizerKind::Json);
        assert_eq!(model.voices.len(), 1);
        assert_eq!(model.voices[0].id, "alba");
        assert_eq!(model.primary_voice_mode(), PocketVoiceMode::VoiceState);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rejects_tokenizer_path_traversal() {
        let root = temp_root("traversal");
        let dir = root.join(MODELS_SUBDIR).join("bad");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.yaml"),
            concat!(
                "weights_path: model.safetensors\n",
                "flow_lm:\n",
                "  lookup_table:\n",
                "    n_bins: 3\n",
                "    tokenizer: sentencepiece\n",
                "    tokenizer_path: ../tokenizer.model\n",
                "mimi:\n",
                "  sample_rate: 24000\n",
            ),
        )
        .unwrap();
        std::fs::write(dir.join("model.safetensors"), b"x").unwrap();
        assert!(discover_pocket_models(&root).is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rejects_incomplete_graph_set() {
        let root = temp_root("incomplete");
        write_minimal_package(&root, "partial", "sentencepiece", "tokenizer.model");
        std::fs::remove_file(
            root.join(MODELS_SUBDIR)
                .join("partial")
                .join("mimi_decoder.onnx"),
        )
        .unwrap();
        assert!(discover_pocket_models(&root).is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn prefers_int8_graphs_when_complete() {
        let root = temp_root("int8");
        write_minimal_package(&root, "q", "sentencepiece", "tokenizer.model");
        let dir = root.join(MODELS_SUBDIR).join("q");
        for name in [
            "flow_lm_main_int8.onnx",
            "flow_lm_flow_int8.onnx",
            "mimi_decoder_int8.onnx",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let models = discover_pocket_models(&root);
        assert_eq!(models.len(), 1);
        assert!(models[0].graphs.quantized);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn read_only_scan_does_not_create_models_directory() {
        let root = temp_root("readonly");
        let models = scan_pocket_models_root(&root);
        assert!(models.is_empty());
        assert!(!root.join("models").exists());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn merge_keeps_earlier_root_duplicate() {
        let first = temp_root("merge-a");
        let second = temp_root("merge-b");
        write_minimal_package(&first, "same", "sentencepiece", "tokenizer.model");
        write_minimal_package(&second, "same", "sentencepiece", "tokenizer.model");
        let a = discover_pocket_models(&first);
        let b = discover_pocket_models(&second);
        let merged = merge_pocket_roots(vec![a, b]);
        assert_eq!(merged.len(), 1);
        assert!(
            merged[0].package_root.to_string_lossy().contains("merge-a"),
            "earlier root must win: {:?}",
            merged[0].package_root
        );
        std::fs::remove_dir_all(first).ok();
        std::fs::remove_dir_all(second).ok();
    }

    fn write_voice_state(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut data = Vec::new();
        let header = serde_json::json!({
            "transformer.layers.0.self_attn/cache": {
                "dtype": "F32",
                "shape": [2, 1, 1, 1, 1],
                "data_offsets": [0, 8]
            },
            "transformer.layers.0.self_attn/offset": {
                "dtype": "I64",
                "shape": [1],
                "data_offsets": [8, 16]
            }
        })
        .to_string();
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&2.0f32.to_le_bytes());
        data.extend_from_slice(&1i64.to_le_bytes());
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(&(header.len() as u64).to_le_bytes())
            .unwrap();
        file.write_all(header.as_bytes()).unwrap();
        file.write_all(&data).unwrap();
    }
}
