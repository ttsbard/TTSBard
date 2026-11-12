//! Download and installation of local model packages (ROADMAP-136).
//!
//! The active model catalog is downloaded from the repository into
//! `model-catalog.json` in the program data directory and replaced atomically
//! after validation, so a failed refresh still leaves the previous accepted
//! catalog available. Installs
//! are staged below the effective models root, verified, and only then moved
//! into place. Existing files are backed up until the metadata update succeeds;
//! cancellation or any network/hash/install error therefore leaves the
//! previously installed package untouched.
//!
//! Mutations (install and catalog refresh) are serialized. Cancellation is
//! handled through a per-package token that is checked while streaming and
//! again before the staging area is committed.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use futures::stream::{self, StreamExt};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Tauri event emitted while a package is downloading.
pub const PROGRESS_EVENT: &str = "model-downloads-progress";
const POCKET_MODELS_CHANGED_EVENT: &str = "pocket-models-changed";

/// Default repository catalog URL used when `catalog_source_url` is unset.
pub const DEFAULT_CATALOG_SOURCE_URL: &str =
    "https://raw.githubusercontent.com/ttsbard/TTSBard/master/catalog/model-catalog.json";

const METADATA_FILE_NAME: &str = "model-downloads.json";
const CATALOG_FILE_NAME: &str = "model-catalog.json";
const MODELS_SUBDIR: &str = "models";
const STAGING_SUBDIR: &str = ".model-downloads-staging";
const CATALOG_SCHEMA_VERSION: u32 = 1;
const METADATA_SCHEMA_VERSION: u32 = 1;
const MAX_CATALOG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_URL_LENGTH: usize = 8 * 1024;
const MAX_RELATIVE_PATH_LENGTH: usize = 1024;
const MAX_REDIRECTS: usize = 5;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const DOWNLOAD_WRITE_BUFFER_BYTES: usize = 512 * 1024;
const MAX_CONCURRENT_FILE_DOWNLOADS: usize = 4;
const MAX_CONCURRENT_HTTP_STREAMS: usize = 6;
const MULTIPART_RANGE_THRESHOLD_BYTES: u64 = 16 * 1024 * 1024;
const MULTIPART_RANGE_PARTS: usize = 4;
const CANCELLED_ERROR: &str = "model download cancelled";

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static CONFIG_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Shared v1 catalog / metadata contract
// ---------------------------------------------------------------------------

/// Package group in the catalog. The serialized values are part of the v1
/// contract and are also the first component of every package file path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelCategory {
    Ruaccent,
    Ocr,
    Pocket,
}

impl ModelCategory {
    fn as_str(self) -> &'static str {
        match self {
            ModelCategory::Ruaccent => "ruaccent",
            ModelCategory::Ocr => "ocr",
            ModelCategory::Pocket => "pocket",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    pub published_at: String,
    pub packages: Vec<CatalogPackage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPackage {
    pub id: String,
    pub category: ModelCategory,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub recommended: bool,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    pub files: Vec<CatalogFile>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One package file. Exactly one of `url` and `content` is present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogFile {
    /// Path relative to the effective `<program data>/models` root. It must be
    /// a safe relative Windows path whose first component is the package
    /// category.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Exact UTF-8 content for generated manifests. The checksum and size are
    /// verified against these bytes before anything is written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDownloadsMetadata {
    pub schema_version: u32,
    #[serde(default)]
    pub catalog_source_url: Option<String>,
    // Older releases stored provenance here alongside a bare catalog cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    catalog_download: Option<CatalogDownload>,
    #[serde(default)]
    pub installed: Vec<InstalledPackage>,
}

impl Default for ModelDownloadsMetadata {
    fn default() -> Self {
        Self {
            schema_version: METADATA_SCHEMA_VERSION,
            catalog_source_url: None,
            catalog_download: None,
            installed: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogDownload {
    pub url: String,
    pub downloaded_at: String,
}

#[derive(Serialize, Deserialize)]
struct StoredCatalog {
    source_url: String,
    downloaded_at: String,
    catalog: Catalog,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_sha256: Option<String>,
    pub installed_at: String,
}

/// Payload of `model-downloads-progress`.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressPayload {
    pub package_id: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelDownloadsStatus {
    pub catalog: Option<Catalog>,
    pub catalog_download: Option<CatalogDownload>,
    pub metadata: ModelDownloadsMetadata,
    pub installed_package_ids: Vec<String>,
    pub outdated_package_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallOutcome {
    Installed,
    Cancelled,
}

// ---------------------------------------------------------------------------
// Runtime state and Tauri commands
// ---------------------------------------------------------------------------

/// Serializes mutations and keeps per-package cancellation tokens. The state
/// is shared by all download commands.
#[derive(Clone)]
pub struct ModelDownloadsState {
    mutation: Arc<tokio::sync::Mutex<()>>,
    active: Arc<parking_lot::Mutex<HashMap<String, CancellationToken>>>,
    http_client: Arc<parking_lot::Mutex<Option<reqwest::Client>>>,
}

impl Default for ModelDownloadsState {
    fn default() -> Self {
        Self {
            mutation: Arc::new(tokio::sync::Mutex::new(())),
            active: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            http_client: Arc::new(parking_lot::Mutex::new(None)),
        }
    }
}

impl ModelDownloadsState {
    fn http_client(&self) -> Result<reqwest::Client, String> {
        let mut slot = self.http_client.lock();
        if let Some(client) = slot.as_ref() {
            return Ok(client.clone());
        }
        let client = build_http_client()?;
        *slot = Some(client.clone());
        Ok(client)
    }

    fn register_install(&self, package_id: &str) -> Result<CancellationToken, String> {
        let mut active = self.active.lock();
        if active.contains_key(package_id) {
            return Err(format!(
                "Package '{package_id}' is already being downloaded"
            ));
        }
        let token = CancellationToken::new();
        active.insert(package_id.to_string(), token.clone());
        Ok(token)
    }

    fn cancel_install(&self, package_id: &str) {
        if let Some(token) = self.active.lock().get(package_id).cloned() {
            token.cancel();
        }
    }
}

/// Removes the active-install entry when the command leaves, including on all
/// early errors. Registration itself rejects duplicate IDs.
struct ActiveInstallGuard {
    active: Arc<parking_lot::Mutex<HashMap<String, CancellationToken>>>,
    package_id: String,
}

impl ActiveInstallGuard {
    fn new(
        active: Arc<parking_lot::Mutex<HashMap<String, CancellationToken>>>,
        package_id: String,
    ) -> Self {
        Self { active, package_id }
    }
}

impl Drop for ActiveInstallGuard {
    fn drop(&mut self) {
        self.active.lock().remove(&self.package_id);
    }
}

#[tauri::command]
pub fn model_downloads_status() -> Result<ModelDownloadsStatus, String> {
    let paths = AppPaths::resolve()?;
    build_status(&paths)
}

#[tauri::command]
pub async fn model_downloads_install(
    app: AppHandle,
    state: State<'_, ModelDownloadsState>,
    package_id: String,
) -> Result<InstallOutcome, String> {
    let state = state.inner().clone();
    match run_install(app, state, package_id).await {
        Ok(()) => Ok(InstallOutcome::Installed),
        Err(error) if error == CANCELLED_ERROR => Ok(InstallOutcome::Cancelled),
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub fn model_downloads_cancel(
    state: State<'_, ModelDownloadsState>,
    package_id: String,
) -> Result<(), String> {
    state.cancel_install(&package_id);
    Ok(())
}

#[tauri::command]
pub async fn model_downloads_refresh_catalog(
    state: State<'_, ModelDownloadsState>,
) -> Result<(), String> {
    let client = state.http_client()?;
    let mutation = state.mutation.clone();
    let _guard = mutation.lock().await;
    let paths = AppPaths::resolve()?;
    refresh_catalog_with_client(&paths, &client).await
}

#[tauri::command]
pub async fn model_downloads_open_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let paths = AppPaths::resolve()?;
    let models_root = paths.models_root;
    let display = tokio::task::spawn_blocking(move || -> Result<String, String> {
        std::fs::create_dir_all(&models_root)
            .map_err(|error| format!("Failed to create models folder: {error}"))?;
        let canonical = models_root
            .canonicalize()
            .map_err(|error| format!("Failed to resolve models folder: {error}"))?;
        canonical
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| "Models folder path is not valid UTF-8".to_string())
    })
    .await
    .map_err(|error| format!("Opening the models folder was interrupted: {error}"))??;

    app.opener()
        .open_path(display.clone(), None::<&str>)
        .map_err(|error| format!("Failed to open models folder '{display}': {error}"))
}

async fn run_install(
    app: AppHandle,
    state: ModelDownloadsState,
    package_id: String,
) -> Result<(), String> {
    validate_package_id(&package_id)?;
    let paths = AppPaths::resolve()?;

    // Resolve and validate the package before registering the operation, so an
    // unknown or unavailable package fails immediately and is never fetched.
    let metadata = read_metadata(&paths.metadata_path())?;
    let (Some(catalog), _) = load_catalog(&paths.data_dir, &metadata)? else {
        return Err("Model catalog has not been downloaded yet".to_string());
    };
    let package = find_package(&catalog, &package_id)?;
    ensure_available(package)?;

    let token = state.register_install(&package_id)?;
    let _active = ActiveInstallGuard::new(state.active.clone(), package_id.clone());
    // A queued install must still react to Cancel immediately instead of
    // waiting for an unrelated mutation to finish.
    let _mutation = tokio::select! {
        _ = token.cancelled() => return Err(CANCELLED_ERROR.to_string()),
        guard = state.mutation.lock() => guard,
    };

    if token.is_cancelled() {
        return Err(CANCELLED_ERROR.to_string());
    }

    // Refresh may have finished while this install waited for the mutation
    // lock. Re-read both files so the operation uses one coherent snapshot.
    let metadata = read_metadata(&paths.metadata_path())?;
    let (Some(catalog), _) = load_catalog(&paths.data_dir, &metadata)? else {
        return Err("Model catalog has not been downloaded yet".to_string());
    };
    let package = find_package(&catalog, &package_id)?.clone();
    ensure_available(&package)?;

    let client = state.http_client()?;
    let app_for_progress = app.clone();
    let progress_package_id = package_id.clone();
    let progress_token = token.clone();
    let mut last_emit: Option<Instant> = None;
    let mut progress = move |received_bytes: u64, total_bytes: u64| {
        // Never publish progress after the user asked to cancel; a late event
        // would restore a progress bar the UI has already cleared.
        if progress_token.is_cancelled() {
            return;
        }
        let should_emit = received_bytes >= total_bytes
            || last_emit
                .map(|last| last.elapsed() >= PROGRESS_INTERVAL)
                .unwrap_or(true);
        if !should_emit {
            return;
        }
        last_emit = Some(Instant::now());
        let _ = app_for_progress.emit(
            PROGRESS_EVENT,
            ProgressPayload {
                package_id: progress_package_id.clone(),
                received_bytes,
                total_bytes,
            },
        );
    };

    install_package_core(
        &paths.models_root,
        &paths.metadata_path(),
        &package,
        &client,
        &token,
        &mut progress,
    )
    .await?;
    if package.category == ModelCategory::Pocket {
        app.state::<crate::state::AppState>()
            .register_pocket_providers();
        crate::commands::emit_settings_changed(&app);
        let _ = app.emit(POCKET_MODELS_CHANGED_EVENT, ());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Paths and metadata
// ---------------------------------------------------------------------------

struct AppPaths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    models_root: PathBuf,
}

impl AppPaths {
    fn resolve() -> Result<Self, String> {
        let config_dir = crate::paths::config_root()
            .map_err(|error| format!("Failed to resolve configuration directory: {error}"))?;
        let data_dir = crate::paths::local_root()
            .map_err(|error| format!("Failed to resolve program data directory: {error}"))?;
        let models_root = data_dir.join(MODELS_SUBDIR);
        Ok(Self {
            config_dir,
            data_dir,
            models_root,
        })
    }

    fn metadata_path(&self) -> PathBuf {
        metadata_path_for(&self.config_dir)
    }
}

fn metadata_path_for(config_dir: &Path) -> PathBuf {
    config_dir.join(METADATA_FILE_NAME)
}

fn catalog_path_for(data_dir: &Path) -> PathBuf {
    data_dir.join(CATALOG_FILE_NAME)
}

fn read_metadata(path: &Path) -> Result<ModelDownloadsMetadata, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ModelDownloadsMetadata::default());
        }
        Err(error) => {
            return Err(format!(
                "Failed to read model download metadata '{}': {error}",
                path.display()
            ));
        }
    };
    let metadata: ModelDownloadsMetadata = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "Model download metadata '{}' is malformed: {error}",
            path.display()
        )
    })?;
    validate_metadata(&metadata)?;
    Ok(metadata)
}

fn validate_metadata(metadata: &ModelDownloadsMetadata) -> Result<(), String> {
    if metadata.schema_version != METADATA_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported model download metadata schema version {}",
            metadata.schema_version
        ));
    }
    if let Some(source) = &metadata.catalog_source_url {
        validate_network_url(source)?;
    }
    if let Some(download) = &metadata.catalog_download {
        validate_network_url(&download.url)?;
        validate_utc_timestamp(&download.downloaded_at, "catalog downloaded_at")?;
    }
    let mut ids = HashSet::new();
    for installed in &metadata.installed {
        validate_package_id(&installed.id)?;
        if !ids.insert(installed.id.to_lowercase()) {
            return Err(format!(
                "Model download metadata contains duplicate package '{}'",
                installed.id
            ));
        }
        if installed.version.trim().is_empty() {
            return Err(format!(
                "Model download metadata package '{}' has an empty version",
                installed.id
            ));
        }
        validate_utc_timestamp(&installed.installed_at, "installed_at")?;
    }
    Ok(())
}

fn write_metadata(path: &Path, metadata: &ModelDownloadsMetadata) -> Result<(), String> {
    validate_metadata(metadata)?;
    let mut bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|error| format!("Failed to serialize model download metadata: {error}"))?;
    bytes.push(b'\n');
    write_config_file(path, &bytes)
}

fn write_config_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;

    let parent = path
        .parent()
        .ok_or_else(|| format!("Configuration path '{}' has no parent", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create configuration directory: {error}"))?;
    let _lock = crate::config::config_write_lock()
        .lock()
        .map_err(|_| "Configuration write lock is poisoned".to_string())?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.json");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = CONFIG_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(".{file_name}.{stamp}.{sequence}.tmp"));

    let result = (|| -> Result<(), String> {
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|error| format!("Failed to create temporary file: {error}"))?;
            file.write_all(bytes)
                .map_err(|error| format!("Failed to write temporary file: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("Failed to flush temporary file: {error}"))?;
        }
        // Same-directory rename replaces the previous file on both Windows
        // (MoveFileEx with REPLACE_EXISTING) and Unix. ReplaceFileW is avoided
        // here because it is unavailable in some restricted environments; the
        // metadata/cache files have no ACLs worth preserving.
        std::fs::rename(&temp_path, path)
            .map_err(|error| format!("Failed to replace '{}': {error}", path.display()))
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result.map_err(|error| format!("Failed to write '{}': {error}", path.display()))
}

// ---------------------------------------------------------------------------
// Catalog loading, validation and refresh
// ---------------------------------------------------------------------------

fn load_catalog(
    data_dir: &Path,
    metadata: &ModelDownloadsMetadata,
) -> Result<(Option<Catalog>, Option<CatalogDownload>), String> {
    let catalog_path = catalog_path_for(data_dir);
    match std::fs::read(&catalog_path) {
        Ok(bytes) => match parse_stored_catalog(&bytes, metadata.catalog_download.as_ref()) {
            Ok((catalog, download)) => return Ok((Some(catalog), download)),
            Err(error) => warn!(
                path = %catalog_path.display(),
                error = %error,
                "Ignoring invalid stored model catalog"
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!(
            path = %catalog_path.display(),
            error = %error,
            "Failed to read stored model catalog"
        ),
    }

    Ok((None, None))
}

fn parse_stored_catalog(
    bytes: &[u8],
    legacy_download: Option<&CatalogDownload>,
) -> Result<(Catalog, Option<CatalogDownload>), String> {
    let stored: StoredCatalog = match serde_json::from_slice(bytes) {
        Ok(stored) => stored,
        Err(_) => {
            let catalog = parse_catalog(bytes)?;
            return Ok((catalog, legacy_download.cloned()));
        }
    };
    validate_catalog(&stored.catalog)?;
    validate_network_url(&stored.source_url)?;
    validate_utc_timestamp(&stored.downloaded_at, "catalog download")?;
    Ok((
        stored.catalog,
        Some(CatalogDownload {
            url: stored.source_url,
            downloaded_at: stored.downloaded_at,
        }),
    ))
}

fn package_files_sha256(package: &CatalogPackage) -> String {
    let mut entries: Vec<(&str, String, u64)> = package
        .files
        .iter()
        .map(|file| {
            (
                file.path.as_str(),
                file.sha256.to_ascii_lowercase(),
                file.size_bytes,
            )
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let mut hasher = Sha256::new();
    for (file_path, sha, size) in entries {
        hasher.update(file_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(sha.as_bytes());
        hasher.update(b"\0");
        hasher.update(size.to_le_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

fn effective_catalog_source_url(configured: Option<&str>) -> String {
    configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CATALOG_SOURCE_URL)
        .to_string()
}

fn parse_catalog(bytes: &[u8]) -> Result<Catalog, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("Catalog is not UTF-8: {error}"))?;
    let catalog: Catalog =
        serde_json::from_str(text).map_err(|error| format!("Catalog JSON is invalid: {error}"))?;
    validate_catalog(&catalog)?;
    Ok(catalog)
}

fn validate_catalog(catalog: &Catalog) -> Result<(), String> {
    if catalog.schema_version != CATALOG_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported catalog schema version {}",
            catalog.schema_version
        ));
    }
    NaiveDate::parse_from_str(&catalog.published_at, "%Y-%m-%d")
        .map_err(|_| "Catalog published_at must be a YYYY-MM-DD date".to_string())?;

    let mut package_keys = HashSet::new();
    // Packages may intentionally share files (RUAccent variants reuse the
    // same dictionaries and neural networks), but a shared path must describe
    // identical bytes. Different content at the same path would make installs
    // overwrite each other.
    let mut global_files: HashMap<String, (String, String, u64)> = HashMap::new();
    for package in &catalog.packages {
        validate_package_id(&package.id)?;
        if !package_keys.insert(package.id.to_lowercase()) {
            return Err(format!(
                "Catalog contains duplicate package '{}'",
                package.id
            ));
        }
        if package.name.trim().is_empty() {
            return Err(format!(
                "Catalog package '{}' has an empty name",
                package.id
            ));
        }
        if package.version.trim().is_empty() {
            return Err(format!(
                "Catalog package '{}' has an empty version",
                package.id
            ));
        }
        if let Some(source_url) = &package.source_url {
            validate_network_url(source_url)?;
        }
        // The metadata is optional, but a present value must be displayable:
        // the publication validator and the status parser reject blank values.
        if package
            .license
            .as_deref()
            .is_some_and(|license| license.trim().is_empty())
        {
            return Err(format!(
                "Catalog package '{}' has an empty license",
                package.id
            ));
        }
        // Unavailable entries may intentionally carry no files (no fake
        // assets); available packages must always describe their payload.
        if package.files.is_empty() && package.available {
            return Err(format!("Catalog package '{}' has no files", package.id));
        }

        let category = package.category;
        let category_name = category.as_str();
        let mut file_keys = HashSet::new();
        for file in &package.files {
            let safe = validate_catalog_file_path(category, &file.path)
                .map_err(|error| format!("Package '{}': {error}", package.id))?;
            if !safe
                .components
                .first()
                .is_some_and(|part| part == category_name)
            {
                return Err(format!(
                    "Package '{}' file '{}' must start with '{}'",
                    package.id, file.path, category_name
                ));
            }
            let path_key = safe.collision_key();
            if !file_keys.insert(path_key.clone()) {
                return Err(format!(
                    "Package '{}' contains a case-insensitive duplicate path '{}'",
                    package.id, file.path
                ));
            }
            let previous = global_files.get(&path_key).cloned();
            if let Some((other_id, other_hash, other_size)) = previous {
                if other_hash != file.sha256.to_lowercase() || other_size != file.size_bytes {
                    return Err(format!(
                        "Package '{}' file '{}' conflicts with a file declared by package '{}'",
                        package.id, file.path, other_id
                    ));
                }
            } else {
                global_files.insert(
                    path_key,
                    (
                        package.id.clone(),
                        file.sha256.to_lowercase(),
                        file.size_bytes,
                    ),
                );
            }
            match (&file.url, &file.content) {
                (Some(url), None) => {
                    validate_network_url(url)?;
                    if file.size_bytes == 0 {
                        return Err(format!(
                            "Package '{}' file '{}' has a zero size",
                            package.id, file.path
                        ));
                    }
                }
                (None, Some(content)) => {
                    let content_len = content.len() as u64;
                    if content_len != file.size_bytes {
                        return Err(format!(
                            "Package '{}' generated file '{}' size does not match its content",
                            package.id, file.path
                        ));
                    }
                    if sha256_hex(content.as_bytes()) != file.sha256.to_lowercase() {
                        return Err(format!(
                            "Package '{}' generated file '{}' checksum does not match its content",
                            package.id, file.path
                        ));
                    }
                }
                _ => {
                    return Err(format!(
                        "Package '{}' file '{}' must have exactly one of url or content",
                        package.id, file.path
                    ));
                }
            }
            validate_sha256(&file.sha256).map_err(|error| {
                format!("Package '{}' file '{}': {error}", package.id, file.path)
            })?;
            if file.size_bytes == 0 {
                return Err(format!(
                    "Package '{}' file '{}' has a zero size",
                    package.id, file.path
                ));
            }
        }

        // A path cannot be both a file and a directory.
        for key in &file_keys {
            let parts: Vec<&str> = key.split('/').collect();
            for end in 2..parts.len() {
                let ancestor = parts[..end].join("/");
                if file_keys.contains(&ancestor) {
                    return Err(format!(
                        "Package '{}' contains a file/directory path collision",
                        package.id
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
async fn refresh_catalog(paths: &AppPaths) -> Result<(), String> {
    let client = build_http_client()?;
    refresh_catalog_with_client(paths, &client).await
}

async fn refresh_catalog_with_client(
    paths: &AppPaths,
    client: &reqwest::Client,
) -> Result<(), String> {
    let metadata = read_metadata(&paths.metadata_path())?;
    let source_url = effective_catalog_source_url(metadata.catalog_source_url.as_deref());
    validate_network_url(&source_url)?;

    let bytes = fetch_catalog_bytes(client, &source_url).await?;
    let catalog = parse_catalog(&bytes)?;
    let package_count = catalog.packages.len();
    let stored = StoredCatalog {
        source_url,
        downloaded_at: now_utc(),
        catalog,
    };
    let mut stored_bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|error| format!("Failed to serialize accepted catalog: {error}"))?;
    stored_bytes.push(b'\n');
    write_config_file(&catalog_path_for(&paths.data_dir), &stored_bytes)?;

    info!(packages = package_count, "Accepted refreshed model catalog");
    Ok(())
}

async fn fetch_catalog_bytes(
    client: &reqwest::Client,
    source_url: &str,
) -> Result<Vec<u8>, String> {
    let response = tokio::time::timeout(REQUEST_TIMEOUT, client.get(source_url).send())
        .await
        .map_err(|_| "Catalog request timed out".to_string())?
        .map_err(|error| format!("Catalog request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Catalog request returned HTTP {}",
            response.status()
        ));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_CATALOG_BYTES {
            return Err("Catalog response is too large".to_string());
        }
    }
    if !is_allowed_url(response.url()) {
        return Err("Catalog redirected to a disallowed URL".to_string());
    }

    let mut response = response;
    let mut bytes = Vec::new();
    loop {
        let next = tokio::time::timeout(CHUNK_TIMEOUT, response.chunk())
            .await
            .map_err(|_| "Catalog download timed out".to_string())?;
        let Some(chunk) = next.map_err(|error| format!("Catalog download failed: {error}"))? else {
            break;
        };
        if bytes.len() as u64 + chunk.len() as u64 > MAX_CATALOG_BYTES {
            return Err("Catalog response is too large".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Installed-package detection
// ---------------------------------------------------------------------------

struct InstalledDetection {
    installed: Vec<String>,
    outdated: Vec<String>,
}

fn build_status(paths: &AppPaths) -> Result<ModelDownloadsStatus, String> {
    let mut metadata = read_metadata(&paths.metadata_path())?;
    let (catalog, catalog_download) = load_catalog(&paths.data_dir, &metadata)?;
    metadata.catalog_download = None;
    let search_roots = crate::paths::model_search_root_paths();
    let detection = catalog
        .as_ref()
        .map(|active| detect_installed_packages(active, &search_roots, &metadata.installed))
        .unwrap_or(InstalledDetection {
            installed: Vec::new(),
            outdated: Vec::new(),
        });
    Ok(ModelDownloadsStatus {
        catalog,
        catalog_download,
        metadata,
        installed_package_ids: detection.installed,
        outdated_package_ids: detection.outdated,
    })
}

/// Installed state is derived from the package's declared files, not from the
/// metadata. This lets packages copied in manually (or installed by a previous
/// version) appear as installed without an `installed` record. Files must be
/// present in one single search root; paths from different roots are never
/// mixed.
///
/// When a package was previously installed by the downloader (`recorded_installs`)
/// and the catalog later updates its version or file hashes/sizes while the
/// earlier files remain on disk, it is reported as both installed and outdated
/// so the user is advised to reinstall without losing their working local copy.
fn detect_installed_packages(
    catalog: &Catalog,
    search_roots: &[PathBuf],
    recorded_installs: &[InstalledPackage],
) -> InstalledDetection {
    let mut installed = Vec::new();
    let mut outdated = Vec::new();
    for package in &catalog.packages {
        if package.files.is_empty() {
            // Catalog entries without downloadable files (for example Pocket
            // exports that exist only locally) still correspond to manual
            // packages. Detect the runtime layout by its required marker file
            // so no metadata is needed.
            if search_roots
                .iter()
                .any(|root| manual_package_present(&root.join(MODELS_SUBDIR), package))
            {
                installed.push(package.id.clone());
            }
            continue;
        }
        let recorded = recorded_installs
            .iter()
            .find(|entry| entry.id == package.id);
        let current_digest = package_files_sha256(package);
        let matches_recorded = recorded.is_some_and(|entry| {
            entry.version == package.version
                && entry
                    .files_sha256
                    .as_deref()
                    .is_none_or(|digest| digest == current_digest)
        });
        let files: Option<Vec<_>> = package
            .files
            .iter()
            .map(|file| {
                validate_catalog_file_path(package.category, &file.path)
                    .ok()
                    .map(|path| (path, matches_recorded.then_some(file.size_bytes)))
            })
            .collect();
        let Some(files) = files else {
            continue;
        };
        let files_present = search_roots.iter().any(|root| {
            let models_root = root.join(MODELS_SUBDIR);
            files
                .iter()
                .all(|(relative, size)| package_file_present(&models_root, relative, *size))
        });
        let outdated_present = recorded.is_some()
            && !matches_recorded
            && (files_present
                || search_roots
                    .iter()
                    .any(|root| manual_package_present(&root.join(MODELS_SUBDIR), package)));
        if files_present || outdated_present {
            installed.push(package.id.clone());
            if outdated_present {
                outdated.push(package.id.clone());
            }
        }
    }
    installed.sort();
    outdated.sort();
    InstalledDetection {
        installed,
        outdated,
    }
}

/// Detect a manually installed package that has no catalog file list. The
/// marker is the file each runtime requires before it can discover the pack:
/// `manifest.json` for OCR and `config.yaml` for Pocket. RUAccent packages share
/// a single variant root and are only represented when the catalog lists files.
fn manual_package_present(models_root: &Path, package: &CatalogPackage) -> bool {
    if validate_package_id(&package.id).is_err() {
        return false;
    }
    let marker = match package.category {
        ModelCategory::Ocr => "manifest.json",
        ModelCategory::Pocket => "config.yaml",
        ModelCategory::Ruaccent => return false,
    };

    let category_root = models_root.join(package.category.as_str());
    let Ok(metadata) = std::fs::symlink_metadata(&category_root) else {
        return false;
    };
    if is_reparse_point(&metadata) || !metadata.is_dir() {
        return false;
    }
    let package_root = category_root.join(&package.id);
    let Ok(metadata) = std::fs::symlink_metadata(&package_root) else {
        return false;
    };
    if is_reparse_point(&metadata) || !metadata.is_dir() {
        return false;
    }
    let Ok(metadata) = std::fs::symlink_metadata(package_root.join(marker)) else {
        return false;
    };
    !is_reparse_point(&metadata) && metadata.is_file() && metadata.len() > 0
}

/// Check a manually installed file without following reparse points. A missing
/// file is a normal "not installed" result; an unsafe path is treated as not
/// installed (fail closed).
fn package_file_present(
    models_root: &Path,
    relative: &SafeRelativePath,
    size: Option<u64>,
) -> bool {
    let mut current = models_root.to_path_buf();
    let last = relative.components.len().saturating_sub(1);
    for (index, component) in relative.components.iter().enumerate() {
        current.push(component);
        let metadata = match std::fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(_) => return false,
        };
        if is_reparse_point(&metadata) {
            return false;
        }
        if index < last {
            if !metadata.is_dir() {
                return false;
            }
        } else if !metadata.is_file()
            || !size.map_or(metadata.len() > 0, |size| metadata.len() == size)
        {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Safe relative paths
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct SafeRelativePath {
    components: Vec<String>,
}

impl SafeRelativePath {
    fn as_path(&self) -> PathBuf {
        let mut path = PathBuf::new();
        for component in &self.components {
            path.push(component);
        }
        path
    }

    fn collision_key(&self) -> String {
        self.components.join("/").to_lowercase()
    }
}

fn validate_catalog_file_path(
    category: ModelCategory,
    raw: &str,
) -> Result<SafeRelativePath, String> {
    if raw.is_empty() {
        return Err("file path is empty".to_string());
    }
    if raw.len() > MAX_RELATIVE_PATH_LENGTH {
        return Err(format!("file path '{}' is too long", raw));
    }
    if raw.contains('\\') {
        return Err(format!(
            "file path '{}' must use '/' separators and no backslashes",
            raw
        ));
    }
    if raw.starts_with('/') || raw.ends_with('/') {
        return Err(format!("file path '{}' must be relative", raw));
    }

    let mut components = Vec::new();
    for component in raw.split('/') {
        validate_windows_file_name(component)
            .map_err(|error| format!("file path '{}' is unsafe: {error}", raw))?;
        components.push(component.to_string());
    }
    if components.len() < 2 {
        return Err(format!(
            "file path '{}' must contain a file below its category",
            raw
        ));
    }
    if components.first().map(String::as_str) != Some(category.as_str()) {
        return Err(format!(
            "file path '{}' must start with category '{}'",
            raw,
            category.as_str()
        ));
    }
    Ok(SafeRelativePath { components })
}

fn validate_windows_file_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("path contains an empty component".to_string());
    }
    if name == "." || name == ".." {
        return Err("path traversal components are not allowed".to_string());
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err(format!("'{name}' ends with a space or dot"));
    }
    if name.encode_utf16().count() > 255 {
        return Err(format!("'{name}' is longer than 255 UTF-16 units"));
    }
    for ch in name.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
            return Err(format!("'{name}' contains a character Windows forbids"));
        }
    }
    if is_reserved_windows_name(name) {
        return Err(format!("'{name}' is a reserved Windows device name"));
    }
    Ok(())
}

fn is_reserved_windows_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let bytes = upper.as_bytes();
    bytes.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && (b'1'..=b'9').contains(&bytes[3])
}

fn validate_package_id(package_id: &str) -> Result<(), String> {
    if package_id.is_empty() {
        return Err("Package ID is empty".to_string());
    }
    if package_id.len() > 128 {
        return Err("Package ID is too long".to_string());
    }
    if !package_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!(
            "Package ID '{package_id}' contains unsupported characters"
        ));
    }
    if !package_id
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(format!(
            "Package ID '{package_id}' must start with an ASCII letter or digit"
        ));
    }
    if package_id.ends_with('.') {
        return Err(format!("Package ID '{package_id}' ends with a dot"));
    }
    if is_reserved_windows_name(package_id) {
        return Err(format!("Package ID '{package_id}' is reserved"));
    }
    Ok(())
}

fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Create each missing directory below `models_root`, refusing to traverse an
/// existing reparse point or a non-directory. Newly created directories are
/// recorded so they can be removed if the install rolls back.
fn create_directories_safe(
    models_root: &Path,
    components: &[String],
    created: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let mut current = models_root.to_path_buf();
    for component in components {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_reparse_point(&metadata) {
                    return Err(format!(
                        "Refusing to traverse reparse point '{}'",
                        current.display()
                    ));
                }
                if !metadata.is_dir() {
                    return Err(format!(
                        "Install path '{}' is not a directory",
                        current.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|error| {
                    format!(
                        "Failed to create directory '{}': {error}",
                        current.display()
                    )
                })?;
                created.push(current.clone());
                let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
                    format!(
                        "Failed to verify created directory '{}': {error}",
                        current.display()
                    )
                })?;
                if is_reparse_point(&metadata) || !metadata.is_dir() {
                    return Err(format!(
                        "Created directory '{}' is not a plain directory",
                        current.display()
                    ));
                }
            }
            Err(error) => {
                return Err(format!(
                    "Failed to inspect install path '{}': {error}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

fn ensure_target_file_safe(target: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) => {
            if is_reparse_point(&metadata) {
                return Err(format!(
                    "Refusing to replace reparse point '{}'",
                    target.display()
                ));
            }
            if !metadata.is_file() {
                return Err(format!(
                    "Install target '{}' is not a regular file",
                    target.display()
                ));
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Failed to inspect install target '{}': {error}",
            target.display()
        )),
    }
}

// ---------------------------------------------------------------------------
// Installation transaction
// ---------------------------------------------------------------------------

struct StagingDir {
    path: PathBuf,
    preserve: bool,
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        if !self.preserve && self.path.exists() {
            if let Err(error) = std::fs::remove_dir_all(&self.path) {
                warn!(
                    path = %self.path.display(),
                    error = %error,
                    "Failed to remove model download staging directory"
                );
            }
        }
    }
}

fn create_staging_dir(models_root: &Path, package_id: &str) -> Result<StagingDir, String> {
    std::fs::create_dir_all(models_root)
        .map_err(|error| format!("Failed to create models directory: {error}"))?;
    let metadata = std::fs::symlink_metadata(models_root)
        .map_err(|error| format!("Failed to verify models directory: {error}"))?;
    if !metadata.is_dir() {
        return Err("Models root is not a directory".to_string());
    }

    let staging_root = models_root.join(STAGING_SUBDIR);
    match std::fs::symlink_metadata(&staging_root) {
        Ok(metadata) => {
            if is_reparse_point(&metadata) {
                return Err(format!(
                    "Refusing to use reparse point staging directory '{}'",
                    staging_root.display()
                ));
            }
            if !metadata.is_dir() {
                return Err(format!(
                    "Staging path '{}' is not a directory",
                    staging_root.display()
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&staging_root).map_err(|error| {
                format!(
                    "Failed to create staging directory '{}': {error}",
                    staging_root.display()
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "Failed to inspect staging directory '{}': {error}",
                staging_root.display()
            ));
        }
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = staging_root.join(format!(
        "{}-{}-{}-{}",
        package_id,
        std::process::id(),
        stamp,
        sequence
    ));
    std::fs::create_dir(&path).map_err(|error| {
        format!(
            "Failed to create package staging directory '{}': {error}",
            path.display()
        )
    })?;
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| format!("Failed to verify package staging directory: {error}"))?;
    if is_reparse_point(&metadata) || !metadata.is_dir() {
        return Err("Package staging directory is not a plain directory".to_string());
    }
    Ok(StagingDir {
        path,
        preserve: false,
    })
}

/// Files that have already replaced their targets, with their backups on the
/// same volume. A failed restoration leaves the backup in staging.
#[derive(Default)]
struct InstallTransaction {
    installed: Vec<PathBuf>,
    backups: Vec<(PathBuf, PathBuf)>,
    created_dirs: Vec<PathBuf>,
}

impl InstallTransaction {
    fn abort(&mut self, staging: &mut StagingDir, error: String) -> String {
        match self.rollback() {
            Ok(()) => error,
            Err(rollback_error) => {
                staging.preserve = true;
                warn!(
                    recovery_dir = %staging.path.display(),
                    error = %rollback_error,
                    "Model install rollback was incomplete; recovery files retained"
                );
                format!(
                    "{error}\nRollback failed. Recovery files retained in '{}':\n{rollback_error}",
                    staging.path.display(),
                )
            }
        }
    }

    fn rollback(&mut self) -> Result<(), String> {
        let mut failures = Vec::new();
        for target in self.installed.drain(..).rev() {
            if let Err(error) = std::fs::remove_file(&target) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    failures.push(format!("Failed to remove '{}': {error}", target.display()));
                }
            }
        }
        for (target, backup) in self.backups.drain(..).rev() {
            if let Err(error) = std::fs::rename(&backup, &target) {
                failures.push(format!("Failed to restore '{}': {error}", target.display()));
            }
        }
        for directory in self.created_dirs.drain(..).rev() {
            let _ = std::fs::remove_dir(&directory);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("\n"))
        }
    }
}

fn find_package<'a>(catalog: &'a Catalog, package_id: &str) -> Result<&'a CatalogPackage, String> {
    catalog
        .packages
        .iter()
        .find(|package| package.id == package_id)
        .ok_or_else(|| format!("Unknown model package '{package_id}'"))
}

fn ensure_available(package: &CatalogPackage) -> Result<(), String> {
    if package.available {
        return Ok(());
    }
    let reason = package
        .unavailable_reason
        .as_deref()
        .unwrap_or("no reason was provided");
    Err(format!(
        "Model package '{}' is unavailable: {reason}",
        package.id
    ))
}

// ---------------------------------------------------------------------------
// Install core
// ---------------------------------------------------------------------------

async fn install_package_core<F>(
    models_root: &Path,
    metadata_path: &Path,
    package: &CatalogPackage,
    client: &reqwest::Client,
    token: &CancellationToken,
    progress: &mut F,
) -> Result<(), String>
where
    F: FnMut(u64, u64) + Send,
{
    validate_package_id(&package.id)?;
    ensure_available(package)?;
    if package.files.is_empty() {
        return Err(format!("Model package '{}' has no files", package.id));
    }
    if token.is_cancelled() {
        return Err(CANCELLED_ERROR.to_string());
    }

    // Read before creating any files: malformed metadata must fail closed.
    let mut metadata = read_metadata(metadata_path)?;

    let mut files = Vec::with_capacity(package.files.len());
    let mut total_bytes = 0u64;
    for file in &package.files {
        let relative = validate_catalog_file_path(package.category, &file.path)?;
        total_bytes = total_bytes
            .checked_add(file.size_bytes)
            .ok_or_else(|| "Package size overflows u64".to_string())?;
        files.push((file, relative));
    }
    let progress = SharedProgress::new(total_bytes, progress);
    progress.emit_current();

    let mut staging = create_staging_dir(models_root, &package.id)?;
    let http_semaphore = tokio::sync::Semaphore::new(MAX_CONCURRENT_HTTP_STREAMS);
    let install_token = token.child_token();

    // Schedule largest files first so heavy weights saturate the connection pool
    // early while smaller manifests and configs finish alongside them.
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by(|&a, &b| files[b].0.size_bytes.cmp(&files[a].0.size_bytes));
    let mut next_idx = 0usize;
    let staging_files_root = staging.path.join("files");
    let mut in_flight = stream::FuturesUnordered::new();
    let mut first_error: Option<String> = None;

    loop {
        while first_error.is_none()
            && next_idx < order.len()
            && in_flight.len() < MAX_CONCURRENT_FILE_DOWNLOADS
        {
            let idx = order[next_idx];
            next_idx += 1;
            let (file, relative) = &files[idx];
            let stage_path = staging_files_root.join(relative.as_path());
            let file_token = install_token.clone();
            let http_semaphore = &http_semaphore;
            let progress = &progress;
            in_flight.push(async move {
                let result = stage_package_file(
                    models_root,
                    file,
                    relative,
                    &stage_path,
                    client,
                    &file_token,
                    http_semaphore,
                    progress,
                )
                .await;
                if result.is_err() {
                    file_token.cancel();
                }
                result
            });
        }

        match in_flight.next().await {
            Some(Ok(())) => {}
            Some(Err(err)) => {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
            None => break,
        }
    }

    if let Some(err) = first_error {
        return Err(err);
    }
    progress.emit_current();

    // All files are staged and verified. A late cancellation must not leave a
    // partially replaced package behind.
    if token.is_cancelled() {
        return Err(CANCELLED_ERROR.to_string());
    }

    let mut transaction = InstallTransaction::default();
    let installed_at = now_utc();
    let files_sha256 = Some(package_files_sha256(package));
    if let Some(existing) = metadata
        .installed
        .iter_mut()
        .find(|installed| installed.id == package.id)
    {
        existing.version = package.version.clone();
        existing.files_sha256 = files_sha256;
        existing.installed_at = installed_at;
    } else {
        metadata.installed.push(InstalledPackage {
            id: package.id.clone(),
            version: package.version.clone(),
            files_sha256,
            installed_at,
        });
    }
    let result = commit_staged_files(models_root, &staging.path, &files, &mut transaction, token)
        .and_then(|()| {
            if token.is_cancelled() {
                return Err(CANCELLED_ERROR.to_string());
            }
            write_metadata(metadata_path, &metadata)
        });
    if let Err(error) = result {
        return Err(transaction.abort(&mut staging, error));
    }

    info!(
        package = %package.id,
        version = %package.version,
        files = files.len(),
        "Installed model package"
    );
    Ok(())
}

fn commit_staged_files(
    models_root: &Path,
    staging_root: &Path,
    files: &[(&CatalogFile, SafeRelativePath)],
    transaction: &mut InstallTransaction,
    token: &CancellationToken,
) -> Result<(), String> {
    for (file, relative) in files {
        if token.is_cancelled() {
            return Err(CANCELLED_ERROR.to_string());
        }
        let staged = staging_root.join("files").join(relative.as_path());
        let staged_metadata = std::fs::symlink_metadata(&staged)
            .map_err(|error| format!("Staged file '{}' is missing: {error}", file.path))?;
        if is_reparse_point(&staged_metadata)
            || !staged_metadata.is_file()
            || staged_metadata.len() != file.size_bytes
        {
            return Err(format!("Staged file '{}' failed verification", file.path));
        }

        let parent_components = &relative.components[..relative.components.len() - 1];
        create_directories_safe(
            models_root,
            parent_components,
            &mut transaction.created_dirs,
        )?;

        let target = models_root.join(relative.as_path());
        let exists = ensure_target_file_safe(&target)?;
        if exists {
            let backup = staging_root.join("backup").join(relative.as_path());
            if let Some(parent) = backup.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("Failed to create backup path: {error}"))?;
            }
            std::fs::rename(&target, &backup).map_err(|error| {
                format!(
                    "Failed to back up existing model file '{}': {error}",
                    target.display()
                )
            })?;
            transaction.backups.push((target.clone(), backup));
        }

        std::fs::rename(&staged, &target).map_err(|error| {
            format!(
                "Failed to install model file '{}': {error}",
                target.display()
            )
        })?;
        transaction.installed.push(target);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Streaming downloads
// ---------------------------------------------------------------------------

struct SharedProgress<'a, F>
where
    F: FnMut(u64, u64) + Send,
{
    total_bytes: u64,
    received: AtomicU64,
    state: std::sync::Mutex<(u64, &'a mut F)>,
}

impl<'a, F> SharedProgress<'a, F>
where
    F: FnMut(u64, u64) + Send,
{
    fn new(total_bytes: u64, callback: &'a mut F) -> Self {
        Self {
            total_bytes,
            received: AtomicU64::new(0),
            state: std::sync::Mutex::new((0, callback)),
        }
    }

    fn emit_current(&self) {
        let current = self.received.load(Ordering::Relaxed).min(self.total_bytes);
        if let Ok(mut guard) = self.state.lock() {
            let (emitted, callback) = &mut *guard;
            *emitted = (*emitted).max(current);
            let value = *emitted;
            callback(value, self.total_bytes);
        }
    }

    fn add(&self, delta: u64) {
        if delta == 0 {
            return;
        }
        let next = self
            .received
            .fetch_add(delta, Ordering::Relaxed)
            .saturating_add(delta)
            .min(self.total_bytes);
        if let Ok(mut guard) = self.state.lock() {
            let (emitted, callback) = &mut *guard;
            let monotonic = (*emitted).max(next);
            *emitted = monotonic;
            callback(monotonic, self.total_bytes);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn stage_package_file<F>(
    models_root: &Path,
    file: &CatalogFile,
    relative: &SafeRelativePath,
    stage_path: &Path,
    client: &reqwest::Client,
    token: &CancellationToken,
    http_semaphore: &tokio::sync::Semaphore,
    progress: &SharedProgress<'_, F>,
) -> Result<(), String>
where
    F: FnMut(u64, u64) + Send,
{
    if token.is_cancelled() {
        return Err(CANCELLED_ERROR.to_string());
    }
    if let Some(parent) = stage_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("Failed to create staging path: {error}"))?;
    }

    if let Some(url) = &file.url {
        if try_reuse_existing_file(
            models_root,
            relative,
            stage_path,
            file.size_bytes,
            &file.sha256,
            token,
        )
        .await?
        {
            progress.add(file.size_bytes);
            return Ok(());
        }

        download_file(
            client,
            url,
            stage_path,
            file.size_bytes,
            &file.sha256,
            token,
            http_semaphore,
            MULTIPART_RANGE_THRESHOLD_BYTES,
            &|delta| progress.add(delta),
        )
        .await
        .map_err(|error| {
            if error == CANCELLED_ERROR {
                error
            } else {
                format!("Failed to download '{}': {error}", file.path)
            }
        })?;
        return Ok(());
    }

    if let Some(content) = &file.content {
        if token.is_cancelled() {
            return Err(CANCELLED_ERROR.to_string());
        }
        let bytes = content.as_bytes();
        if bytes.len() as u64 != file.size_bytes || sha256_hex(bytes) != file.sha256.to_lowercase()
        {
            return Err(format!(
                "Generated file '{}' does not match its catalog checksum",
                file.path
            ));
        }
        let staged = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage_path)
            .await
            .map_err(|error| format!("Failed to create '{}': {error}", file.path))?;
        let mut writer = BufWriter::with_capacity(DOWNLOAD_WRITE_BUFFER_BYTES, staged);
        writer
            .write_all(bytes)
            .await
            .map_err(|error| format!("Failed to write '{}': {error}", file.path))?;
        writer
            .flush()
            .await
            .map_err(|error| format!("Failed to flush '{}': {error}", file.path))?;
        writer
            .into_inner()
            .sync_all()
            .await
            .map_err(|error| format!("Failed to flush '{}': {error}", file.path))?;
        progress.add(file.size_bytes);
        return Ok(());
    }

    Err(format!("Package file '{}' has no source", file.path))
}

async fn try_reuse_existing_file(
    models_root: &Path,
    relative: &SafeRelativePath,
    stage_path: &Path,
    expected_size: u64,
    expected_sha256: &str,
    token: &CancellationToken,
) -> Result<bool, String> {
    if !package_file_present(models_root, relative, Some(expected_size)) {
        return Ok(false);
    }

    let existing_path = models_root.join(relative.as_path());
    let stage_path = stage_path.to_path_buf();
    let expected_hash = expected_sha256.to_lowercase();
    let token = token.clone();
    tokio::task::spawn_blocking(move || -> Result<bool, String> {
        if token.is_cancelled() {
            return Err(CANCELLED_ERROR.to_string());
        }
        match std::fs::copy(&existing_path, &stage_path) {
            Ok(copied) if copied == expected_size => {
                // Verify the exact bytes that will be committed. The source can
                // change while another process is updating its local models.
                if verify_file_sha256_sync(&stage_path, expected_size, &expected_hash, &token)? {
                    return Ok(true);
                }
            }
            Ok(_) | Err(_) => {}
        }
        if let Err(error) = std::fs::remove_file(&stage_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(format!(
                    "Failed to remove invalid staged model file: {error}"
                ));
            }
        }
        Ok(false)
    })
    .await
    .map_err(|error| format!("Failed to verify local model file: {error}"))?
}

fn verify_file_sha256_sync(
    path: &Path,
    expected_size: u64,
    expected_hash_lower: &str,
    token: &CancellationToken,
) -> Result<bool, String> {
    let Ok(file) = std::fs::File::open(path) else {
        return Ok(false);
    };
    let Ok(metadata) = file.metadata() else {
        return Ok(false);
    };
    if metadata.len() != expected_size {
        return Ok(false);
    }

    let mut reader = BufReader::with_capacity(DOWNLOAD_WRITE_BUFFER_BYTES, file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if token.is_cancelled() {
            return Err(CANCELLED_ERROR.to_string());
        }
        let read = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return Ok(false),
        };
        hasher.update(&buf[..read]);
    }
    Ok(hex::encode(hasher.finalize()) == expected_hash_lower)
}

async fn verify_staged_file_sha256(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
    token: &CancellationToken,
) -> Result<(), String> {
    let path = path.to_path_buf();
    let expected_hash = expected_sha256.to_lowercase();
    let token = token.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        // Windows FlushFileBuffers (sync_all) requires write access on the handle.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| format!("failed to reopen staged file: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("failed to inspect staged file: {error}"))?;
        if metadata.len() != expected_size {
            return Err(format!(
                "downloaded {} bytes, catalog expects {expected_size}",
                metadata.len()
            ));
        }
        file.sync_all()
            .map_err(|error| format!("failed to flush staged file: {error}"))?;
        let mut reader = BufReader::with_capacity(DOWNLOAD_WRITE_BUFFER_BYTES, file);
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if token.is_cancelled() {
                return Err(CANCELLED_ERROR.to_string());
            }
            let read = reader
                .read(&mut buf)
                .map_err(|error| format!("failed to verify staged file: {error}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
        }
        if hex::encode(hasher.finalize()) != expected_hash {
            return Err("SHA-256 checksum mismatch".to_string());
        }
        Ok(())
    })
    .await
    .map_err(|error| format!("failed to verify staged file: {error}"))?
}

fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("TTSBard/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(8)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS || !is_allowed_url(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|error| format!("Failed to create HTTP client: {error}"))
}

#[allow(clippy::too_many_arguments)]
async fn download_file<F>(
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    expected_size: u64,
    expected_sha256: &str,
    token: &CancellationToken,
    http_semaphore: &tokio::sync::Semaphore,
    multipart_threshold: u64,
    on_bytes: &F,
) -> Result<(), String>
where
    F: Fn(u64) + Send + Sync,
{
    validate_network_url(url)?;
    if token.is_cancelled() {
        return Err(CANCELLED_ERROR.to_string());
    }

    if expected_size >= multipart_threshold && MULTIPART_RANGE_PARTS > 1 {
        download_file_multipart(
            client,
            url,
            destination,
            expected_size,
            expected_sha256,
            token,
            http_semaphore,
            on_bytes,
        )
        .await
    } else {
        download_file_single(
            client,
            url,
            destination,
            expected_size,
            expected_sha256,
            token,
            http_semaphore,
            on_bytes,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn download_file_single<F>(
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    expected_size: u64,
    expected_sha256: &str,
    token: &CancellationToken,
    http_semaphore: &tokio::sync::Semaphore,
    on_bytes: &F,
) -> Result<(), String>
where
    F: Fn(u64) + Send + Sync,
{
    let _permit = tokio::select! {
        _ = token.cancelled() => return Err(CANCELLED_ERROR.to_string()),
        permit = http_semaphore.acquire() => permit.map_err(|_| "HTTP limiter closed".to_string())?,
    };

    let response = tokio::select! {
        _ = token.cancelled() => return Err(CANCELLED_ERROR.to_string()),
        response = tokio::time::timeout(REQUEST_TIMEOUT, client.get(url).send()) => response,
    }
    .map_err(|_| "download request timed out".to_string())?
    .map_err(|error| format!("download request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    if !is_allowed_url(response.url()) {
        return Err("download redirected to a disallowed URL".to_string());
    }
    stream_single_response_to_file(
        response,
        destination,
        expected_size,
        expected_sha256,
        token,
        on_bytes,
    )
    .await
}

async fn stream_response_to_writer<F>(
    mut response: reqwest::Response,
    file: tokio::fs::File,
    expected_len: u64,
    overflow_error: &str,
    token: &CancellationToken,
    mut hasher: Option<&mut Sha256>,
    on_bytes: &F,
) -> Result<(tokio::fs::File, u64), String>
where
    F: Fn(u64) + Send + Sync,
{
    let mut writer = BufWriter::with_capacity(DOWNLOAD_WRITE_BUFFER_BYTES, file);
    let mut received = 0u64;

    loop {
        let next = tokio::select! {
            _ = token.cancelled() => return Err(CANCELLED_ERROR.to_string()),
            next = tokio::time::timeout(CHUNK_TIMEOUT, response.chunk()) => next,
        };
        let chunk = match next {
            Err(_) => return Err("download stalled".to_string()),
            Ok(Err(error)) => return Err(format!("download failed: {error}")),
            Ok(Ok(None)) => break,
            Ok(Ok(Some(chunk))) => chunk,
        };
        if chunk.is_empty() {
            continue;
        }
        let chunk_len = chunk.len() as u64;
        received = received
            .checked_add(chunk_len)
            .ok_or_else(|| "downloaded byte count overflows u64".to_string())?;
        if received > expected_len {
            return Err(overflow_error.to_string());
        }
        if let Some(h) = hasher.as_deref_mut() {
            h.update(&chunk);
        }
        writer
            .write_all(&chunk)
            .await
            .map_err(|error| format!("failed to write staged file: {error}"))?;
        on_bytes(chunk_len);
    }

    writer
        .flush()
        .await
        .map_err(|error| format!("failed to flush staged file: {error}"))?;
    Ok((writer.into_inner(), received))
}

async fn stream_single_response_to_file<F>(
    response: reqwest::Response,
    destination: &Path,
    expected_size: u64,
    expected_sha256: &str,
    token: &CancellationToken,
    on_bytes: &F,
) -> Result<(), String>
where
    F: Fn(u64) + Send + Sync,
{
    if let Some(length) = response.content_length() {
        if length != expected_size {
            return Err(format!(
                "server reported {length} bytes, catalog expects {expected_size}"
            ));
        }
    }

    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .await
        .map_err(|error| format!("failed to create staged file: {error}"))?;
    let mut hasher = Sha256::new();
    let (file, received) = stream_response_to_writer(
        response,
        file,
        expected_size,
        "download exceeded the catalog size",
        token,
        Some(&mut hasher),
        on_bytes,
    )
    .await?;

    file.sync_all()
        .await
        .map_err(|error| format!("failed to flush staged file: {error}"))?;
    drop(file);

    if received != expected_size {
        return Err(format!(
            "downloaded {received} bytes, catalog expects {expected_size}"
        ));
    }
    let actual = hex::encode(hasher.finalize());
    if actual != expected_sha256.to_lowercase() {
        return Err("SHA-256 checksum mismatch".to_string());
    }
    Ok(())
}

fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range_part, total_part) = rest.split_once('/')?;
    let (start_part, end_part) = range_part.split_once('-')?;
    let start = start_part.trim().parse::<u64>().ok()?;
    let end = end_part.trim().parse::<u64>().ok()?;
    let total = total_part.trim().parse::<u64>().ok()?;
    Some((start, end, total))
}

fn validate_partial_response(
    response: &reqwest::Response,
    start: u64,
    end: u64,
    expected_total_size: u64,
) -> Result<(), String> {
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(format!("HTTP {}", response.status()));
    }
    let expected_len = end - start + 1;
    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_range)
        .ok_or_else(|| "server omitted or returned malformed Content-Range".to_string())?;
    if content_range != (start, end, expected_total_size) {
        return Err("server returned an unexpected Content-Range".to_string());
    }
    if let Some(length) = response.content_length() {
        if length != expected_len {
            return Err(format!(
                "server reported {length} bytes for range, catalog expects {expected_len}"
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_file_multipart<F>(
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    expected_size: u64,
    expected_sha256: &str,
    token: &CancellationToken,
    http_semaphore: &tokio::sync::Semaphore,
    on_bytes: &F,
) -> Result<(), String>
where
    F: Fn(u64) + Send + Sync,
{
    let part_count = MULTIPART_RANGE_PARTS.min(expected_size as usize).max(1);
    let part_size = expected_size.div_ceil(part_count as u64);
    let mut ranges = Vec::with_capacity(part_count);
    let mut offset = 0u64;
    while offset < expected_size {
        let end = (offset + part_size - 1).min(expected_size - 1);
        ranges.push((offset, end));
        offset = end + 1;
    }

    let (first_start, first_end) = ranges[0];
    let first_len = first_end - first_start + 1;
    let first_permit = tokio::select! {
        _ = token.cancelled() => return Err(CANCELLED_ERROR.to_string()),
        permit = http_semaphore.acquire() => permit.map_err(|_| "HTTP limiter closed".to_string())?,
    };

    let response = tokio::select! {
        _ = token.cancelled() => return Err(CANCELLED_ERROR.to_string()),
        response = tokio::time::timeout(
            REQUEST_TIMEOUT,
            client
                .get(url)
                .header(reqwest::header::RANGE, format!("bytes={first_start}-{first_end}"))
                .send(),
        ) => response,
    }
    .map_err(|_| "download request timed out".to_string())?
    .map_err(|error| format!("download request failed: {error}"))?;

    if !is_allowed_url(response.url()) {
        return Err("download redirected to a disallowed URL".to_string());
    }

    if response.status() == StatusCode::OK {
        return stream_single_response_to_file(
            response,
            destination,
            expected_size,
            expected_sha256,
            token,
            on_bytes,
        )
        .await;
    }

    if matches!(
        response.status(),
        StatusCode::BAD_REQUEST
            | StatusCode::FORBIDDEN
            | StatusCode::RANGE_NOT_SATISFIABLE
            | StatusCode::NOT_IMPLEMENTED
    ) {
        drop(response);
        drop(first_permit);
        return download_file_single(
            client,
            url,
            destination,
            expected_size,
            expected_sha256,
            token,
            http_semaphore,
            on_bytes,
        )
        .await;
    }

    validate_partial_response(&response, first_start, first_end, expected_size)?;

    let part0_file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .await
        .map_err(|error| format!("failed to create staged file: {error}"))?;
    part0_file
        .set_len(expected_size)
        .await
        .map_err(|error| format!("failed to allocate staged file: {error}"))?;

    // A CDN redirect can be signed for the requested Range. Resolve each
    // range from the original URL so it receives its own valid redirect.
    let file_token = token.child_token();

    let part0_fut = async {
        let _permit = first_permit;
        let res =
            stream_range_response_into_file(response, part0_file, first_len, &file_token, on_bytes)
                .await;
        if res.is_err() {
            file_token.cancel();
        }
        res
    };

    let rest_fut = async {
        let results = stream::iter(ranges.into_iter().skip(1))
            .map(|(start, end)| {
                let file_token = &file_token;
                async move {
                    let res = download_range_part(
                        client,
                        url,
                        destination,
                        start,
                        end,
                        expected_size,
                        file_token,
                        http_semaphore,
                        on_bytes,
                    )
                    .await;
                    if res.is_err() {
                        file_token.cancel();
                    }
                    res
                }
            })
            .buffer_unordered(part_count.saturating_sub(1).max(1))
            .collect::<Vec<Result<(), String>>>()
            .await;
        for result in results {
            result?;
        }
        Ok::<(), String>(())
    };

    let (part0_res, rest_res) = tokio::join!(part0_fut, rest_fut);
    if matches!(&part0_res, Err(error) if error == CANCELLED_ERROR) {
        rest_res?;
        return part0_res;
    }
    part0_res?;
    rest_res?;

    verify_staged_file_sha256(destination, expected_size, expected_sha256, token).await
}

#[allow(clippy::too_many_arguments)]
async fn download_range_part<F>(
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    start: u64,
    end: u64,
    expected_total_size: u64,
    token: &CancellationToken,
    http_semaphore: &tokio::sync::Semaphore,
    on_bytes: &F,
) -> Result<(), String>
where
    F: Fn(u64) + Send + Sync,
{
    let _permit = tokio::select! {
        _ = token.cancelled() => return Err(CANCELLED_ERROR.to_string()),
        permit = http_semaphore.acquire() => permit.map_err(|_| "HTTP limiter closed".to_string())?,
    };
    let expected_len = end - start + 1;
    let response = tokio::select! {
        _ = token.cancelled() => return Err(CANCELLED_ERROR.to_string()),
        response = tokio::time::timeout(
            REQUEST_TIMEOUT,
            client
                .get(url)
                .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                .send(),
        ) => response,
    }
    .map_err(|_| "download request timed out".to_string())?
    .map_err(|error| format!("download request failed: {error}"))?;

    if !is_allowed_url(response.url()) {
        return Err("download redirected to a disallowed URL".to_string());
    }
    validate_partial_response(&response, start, end, expected_total_size)?;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(destination)
        .await
        .map_err(|error| format!("failed to open staged file: {error}"))?;
    file.seek(SeekFrom::Start(start))
        .await
        .map_err(|error| format!("failed to seek staged file: {error}"))?;
    stream_range_response_into_file(response, file, expected_len, token, on_bytes).await
}

async fn stream_range_response_into_file<F>(
    response: reqwest::Response,
    file: tokio::fs::File,
    expected_len: u64,
    token: &CancellationToken,
    on_bytes: &F,
) -> Result<(), String>
where
    F: Fn(u64) + Send + Sync,
{
    let (_file, received) = stream_response_to_writer(
        response,
        file,
        expected_len,
        "range response exceeded expected length",
        token,
        None,
        on_bytes,
    )
    .await?;
    if received != expected_len {
        return Err(format!(
            "downloaded {received} bytes for range, catalog expects {expected_len}"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------------

fn validate_network_url(raw: &str) -> Result<(), String> {
    if raw.is_empty() || raw.len() > MAX_URL_LENGTH {
        return Err("URL is empty or too long".to_string());
    }
    if raw.trim() != raw {
        return Err("URL must not contain surrounding whitespace".to_string());
    }
    let url = Url::parse(raw).map_err(|error| format!("URL is invalid: {error}"))?;
    if !is_allowed_url(&url) {
        return Err(
            "URL must use HTTPS (HTTP is allowed only for loopback test sources)".to_string(),
        );
    }
    Ok(())
}

fn is_allowed_url(url: &Url) -> bool {
    match url.scheme() {
        "https" => url.host_str().is_some_and(|host| !host.is_empty()),
        "http" => url.host_str().is_some_and(is_loopback_host),
        _ => false,
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host == "::1"
        || host == "[::1]"
        || host.starts_with("127.")
}

fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("sha256 must be 64 hexadecimal characters".to_string());
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn validate_utc_timestamp(value: &str, label: &str) -> Result<(), String> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| format!("{label} must be an ISO-8601 UTC timestamp"))?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(format!("{label} must use UTC"));
    }
    Ok(())
}

fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc as StdArc;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ttsbard-model-downloads-test-{}-{}-{}",
                std::process::id(),
                stamp,
                name
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn file_with_content(path: &str, content: &str) -> CatalogFile {
        CatalogFile {
            path: path.to_string(),
            url: None,
            content: Some(content.to_string()),
            sha256: sha256_hex(content.as_bytes()),
            size_bytes: content.len() as u64,
        }
    }

    fn package(id: &str, category: ModelCategory, files: Vec<CatalogFile>) -> CatalogPackage {
        CatalogPackage {
            id: id.to_string(),
            category,
            name: "Test package".to_string(),
            version: "1.0.0".to_string(),
            source_url: None,
            license: Some("MIT".to_string()),
            recommended: false,
            available: true,
            unavailable_reason: None,
            files,
        }
    }

    fn catalog(packages: Vec<CatalogPackage>) -> Catalog {
        Catalog {
            schema_version: CATALOG_SCHEMA_VERSION,
            published_at: "2026-11-11".to_string(),
            packages,
        }
    }

    async fn spawn_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn relative_paths_are_validated_against_windows_rules() {
        assert_eq!(
            validate_catalog_file_path(ModelCategory::Ocr, "ocr/pack/manifest.json")
                .unwrap()
                .as_path(),
            PathBuf::from("ocr").join("pack").join("manifest.json")
        );

        for (category, path) in [
            (ModelCategory::Ocr, "../ocr/file"),
            (ModelCategory::Ocr, "/ocr/file"),
            (ModelCategory::Ocr, "ocr/../../file"),
            (ModelCategory::Ocr, "ocr\\file"),
            (ModelCategory::Ocr, "ocr//file"),
            (ModelCategory::Ocr, "ocr"),
            (ModelCategory::Ocr, "ocr/CON"),
            (ModelCategory::Ocr, "ocr/CON.txt"),
            (ModelCategory::Ocr, "ocr/LPT9.log"),
            (ModelCategory::Ocr, "ocr/file."),
            (ModelCategory::Ocr, "ocr/file "),
            (ModelCategory::Ocr, "ocr/a:b"),
            (ModelCategory::Ocr, "ocr/a?b"),
            (ModelCategory::Ocr, "pocket/file"),
            (ModelCategory::Ruaccent, "ocr/file"),
        ] {
            assert!(
                validate_catalog_file_path(category, path).is_err(),
                "'{path}' should be rejected"
            );
        }
    }

    #[test]
    fn catalog_rejects_duplicate_and_file_directory_collisions() {
        let duplicate = catalog(vec![package(
            "pack",
            ModelCategory::Ocr,
            vec![
                file_with_content("ocr/pack/a.txt", "one"),
                file_with_content("ocr/PACK/a.txt", "two"),
            ],
        )]);
        assert!(validate_catalog(&duplicate).is_err());

        let prefix = catalog(vec![package(
            "pack",
            ModelCategory::Ocr,
            vec![
                file_with_content("ocr/pack/a", "one"),
                file_with_content("ocr/pack/a/b.txt", "two"),
            ],
        )]);
        assert!(validate_catalog(&prefix).is_err());
    }

    #[test]
    fn shared_paths_are_allowed_only_for_identical_content() {
        let same = file_with_content("ruaccent/nn/shared.bin", "same");
        let variants = catalog(vec![
            package("ruaccent-a", ModelCategory::Ruaccent, vec![same.clone()]),
            package("ruaccent-b", ModelCategory::Ruaccent, vec![same]),
        ]);
        validate_catalog(&variants).unwrap();

        let conflicting = catalog(vec![
            package(
                "ocr-a",
                ModelCategory::Ocr,
                vec![file_with_content("ocr/shared.bin", "one")],
            ),
            package(
                "ocr-b",
                ModelCategory::Ocr,
                vec![file_with_content("ocr/shared.bin", "two")],
            ),
        ]);
        assert!(validate_catalog(&conflicting).is_err());
    }

    #[test]
    fn generated_content_must_match_hash_and_size() {
        let mut bad_hash = package(
            "pack",
            ModelCategory::Ocr,
            vec![file_with_content("ocr/pack/manifest.json", "{}")],
        );
        bad_hash.files[0].sha256 = "0".repeat(64);
        assert!(validate_catalog(&catalog(vec![bad_hash])).is_err());

        let mut bad_size = package(
            "pack",
            ModelCategory::Ocr,
            vec![file_with_content("ocr/pack/manifest.json", "{}")],
        );
        bad_size.files[0].size_bytes += 1;
        assert!(validate_catalog(&catalog(vec![bad_size])).is_err());
    }

    #[test]
    fn optional_license_must_not_be_blank() {
        let mut without_license = package(
            "pack",
            ModelCategory::Ocr,
            vec![file_with_content("ocr/pack/manifest.json", "{}")],
        );
        without_license.license = None;
        validate_catalog(&catalog(vec![without_license.clone()])).unwrap();
        let serialized = serde_json::to_value(&without_license).unwrap();
        assert!(serialized.get("license").is_none());

        without_license.license = Some("MIT".to_string());
        let serialized = serde_json::to_value(&without_license).unwrap();
        assert_eq!(serialized["license"], "MIT");

        for license in ["", " ", "\t\n"] {
            let mut invalid = package(
                "pack",
                ModelCategory::Ocr,
                vec![file_with_content("ocr/pack/manifest.json", "{}")],
            );
            invalid.license = Some(license.to_string());
            let error = validate_catalog(&catalog(vec![invalid])).unwrap_err();
            assert!(error.contains("license"), "{error}");
        }
    }

    #[test]
    fn repo_catalog_is_valid() {
        let repo_catalog =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../catalog/model-catalog.json");
        let bytes = std::fs::read(&repo_catalog).expect("repo catalog file missing");
        let parsed = parse_catalog(&bytes).expect("repo catalog is invalid");
        assert_eq!(parsed.schema_version, CATALOG_SCHEMA_VERSION);
        assert!(!parsed.packages.is_empty());
    }

    #[test]
    fn empty_unavailable_package_detects_manual_runtime_directory() {
        let temp = TestDir::new("manual-marker");
        let data_root = temp.path().join("data");
        let package_root = data_root.join("models/pocket/pocket-en");
        std::fs::create_dir_all(&package_root).unwrap();
        std::fs::write(package_root.join("config.yaml"), b"config").unwrap();

        let mut package = package("pocket-en", ModelCategory::Pocket, Vec::new());
        package.available = false;
        package.unavailable_reason = Some("no downloadable export".to_string());
        let catalog = catalog(vec![package]);

        let detection = detect_installed_packages(&catalog, std::slice::from_ref(&data_root), &[]);
        assert_eq!(detection.installed, vec!["pocket-en".to_string()]);
        assert!(detection.outdated.is_empty());
    }

    #[test]
    fn installation_is_detected_from_files_without_metadata() {
        let temp = TestDir::new("manual-detect");
        let data_root = temp.path().join("data");
        let models_root = data_root.join(MODELS_SUBDIR);
        std::fs::create_dir_all(models_root.join("ocr/pack")).unwrap();
        std::fs::write(models_root.join("ocr/pack/manifest.json"), b"{}").unwrap();
        std::fs::write(models_root.join("ocr/pack/model.onnx"), b"model").unwrap();

        let catalog = catalog(vec![package(
            "pack",
            ModelCategory::Ocr,
            vec![
                file_with_content("ocr/pack/manifest.json", "{}"),
                file_with_content("ocr/pack/model.onnx", "model"),
            ],
        )]);

        assert_eq!(
            detect_installed_packages(&catalog, std::slice::from_ref(&data_root), &[]).installed,
            vec!["pack".to_string()]
        );
        let recorded = [InstalledPackage {
            id: "pack".to_string(),
            version: catalog.packages[0].version.clone(),
            files_sha256: None,
            installed_at: now_utc(),
        }];
        std::fs::write(models_root.join("ocr/pack/model.onnx"), b"cut").unwrap();
        assert!(
            detect_installed_packages(&catalog, std::slice::from_ref(&data_root), &recorded)
                .installed
                .is_empty()
        );
        assert_eq!(
            detect_installed_packages(&catalog, &[data_root], &[]).installed,
            ["pack"]
        );
    }

    #[test]
    fn outdated_package_is_flagged_when_version_or_files_hash_changes() {
        let temp = TestDir::new("outdated-detect");
        let data_root = temp.path().join("data");
        let models_root = data_root.join(MODELS_SUBDIR);
        std::fs::create_dir_all(models_root.join("ocr/pack")).unwrap();
        std::fs::write(models_root.join("ocr/pack/manifest.json"), b"{}").unwrap();
        std::fs::write(models_root.join("ocr/pack/model.onnx"), b"v1-bytes").unwrap();

        let v1_pkg = package(
            "pack",
            ModelCategory::Ocr,
            vec![
                file_with_content("ocr/pack/manifest.json", "{}"),
                file_with_content("ocr/pack/model.onnx", "v1-bytes"),
            ],
        );
        let recorded = [InstalledPackage {
            id: "pack".to_string(),
            version: v1_pkg.version.clone(),
            files_sha256: Some(package_files_sha256(&v1_pkg)),
            installed_at: now_utc(),
        }];

        // Same version and same file hash -> installed, not outdated.
        let same_catalog = catalog(vec![v1_pkg]);
        let same_detection =
            detect_installed_packages(&same_catalog, std::slice::from_ref(&data_root), &recorded);
        assert_eq!(same_detection.installed, vec!["pack".to_string()]);
        assert!(same_detection.outdated.is_empty());

        // Same version, updated file hash/content -> installed AND outdated.
        let mut updated_hash_pkg = package(
            "pack",
            ModelCategory::Ocr,
            vec![
                file_with_content("ocr/pack/manifest.json", "{}"),
                file_with_content("ocr/pack/model.onnx", "v2-bytes-longer"),
            ],
        );
        let hash_detection = detect_installed_packages(
            &catalog(vec![updated_hash_pkg.clone()]),
            std::slice::from_ref(&data_root),
            &recorded,
        );
        assert_eq!(hash_detection.installed, vec!["pack".to_string()]);
        assert_eq!(hash_detection.outdated, vec!["pack".to_string()]);

        // Bumped version -> installed AND outdated.
        updated_hash_pkg.version = "2.0.0".to_string();
        let version_detection = detect_installed_packages(
            &catalog(vec![updated_hash_pkg.clone()]),
            std::slice::from_ref(&data_root),
            &recorded,
        );
        assert_eq!(version_detection.installed, vec!["pack".to_string()]);
        assert_eq!(version_detection.outdated, vec!["pack".to_string()]);

        // Removing files from disk clears both installed and outdated.
        std::fs::remove_dir_all(models_root.join("ocr/pack")).unwrap();
        let removed_detection = detect_installed_packages(
            &catalog(vec![updated_hash_pkg]),
            std::slice::from_ref(&data_root),
            &recorded,
        );
        assert!(removed_detection.installed.is_empty());
        assert!(removed_detection.outdated.is_empty());
    }

    #[test]
    fn failed_rollback_leaves_recovery_file_in_staging() {
        let temp = TestDir::new("rollback-recovery");
        let root = temp.path().join("models");
        let mut staging = create_staging_dir(&root, "pack").unwrap();
        let backup = staging.path.join("backup/manifest.json");
        std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
        std::fs::write(&backup, b"previous manifest").unwrap();
        let target = root.join("ocr/manifest.json");
        std::fs::create_dir_all(&target).unwrap();
        let mut transaction = InstallTransaction {
            backups: vec![(target, backup.clone())],
            ..Default::default()
        };

        let error = transaction.abort(&mut staging, "Install failed".to_string());
        drop(staging);
        assert!(error.contains("Recovery files retained"));
        assert_eq!(std::fs::read(backup).unwrap(), b"previous manifest");
    }

    #[tokio::test]
    async fn content_package_installs_and_preserves_unrelated_files() {
        let temp = TestDir::new("install-content");
        let models_root = temp.path().join("data/models");
        let config_dir = temp.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(models_root.join("ocr/keep")).unwrap();
        std::fs::write(models_root.join("ocr/keep/file.txt"), b"keep").unwrap();

        let package = package(
            "pack",
            ModelCategory::Ocr,
            vec![
                file_with_content("ocr/pack/manifest.json", "{}"),
                file_with_content("ocr/pack/model.onnx", "model"),
            ],
        );
        let client = build_http_client().unwrap();
        let token = CancellationToken::new();
        let mut progress = Vec::new();
        install_package_core(
            &models_root,
            &metadata_path_for(&config_dir),
            &package,
            &client,
            &token,
            &mut |received, total| progress.push((received, total)),
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read(models_root.join("ocr/pack/manifest.json")).unwrap(),
            b"{}"
        );
        assert_eq!(
            std::fs::read(models_root.join("ocr/pack/model.onnx")).unwrap(),
            b"model"
        );
        assert_eq!(
            std::fs::read(models_root.join("ocr/keep/file.txt")).unwrap(),
            b"keep"
        );
        assert_eq!(progress.last().copied(), Some((7, 7)));

        let metadata = read_metadata(&metadata_path_for(&config_dir)).unwrap();
        assert_eq!(metadata.installed.len(), 1);
        assert_eq!(metadata.installed[0].id, "pack");
        assert!(validate_utc_timestamp(&metadata.installed[0].installed_at, "test").is_ok());
    }

    #[tokio::test]
    async fn failed_commit_rolls_back_files_already_replaced() {
        let temp = TestDir::new("rollback");
        let models_root = temp.path().join("data/models");
        let config_dir = temp.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        // The second target is a directory, so committing it must fail after
        // the first file has already been moved into place.
        std::fs::create_dir_all(models_root.join("ocr/pack/two.txt")).unwrap();
        std::fs::write(models_root.join("ocr/unrelated.txt"), b"unrelated").unwrap();

        let package = package(
            "pack",
            ModelCategory::Ocr,
            vec![
                file_with_content("ocr/pack/one.txt", "one"),
                file_with_content("ocr/pack/two.txt", "two"),
            ],
        );
        let client = build_http_client().unwrap();
        let token = CancellationToken::new();
        let result = install_package_core(
            &models_root,
            &metadata_path_for(&config_dir),
            &package,
            &client,
            &token,
            &mut |_, _| {},
        )
        .await;

        assert!(result.is_err());
        assert!(!models_root.join("ocr/pack/one.txt").exists());
        assert_eq!(
            std::fs::read(models_root.join("ocr/unrelated.txt")).unwrap(),
            b"unrelated"
        );
        assert!(read_metadata(&metadata_path_for(&config_dir))
            .unwrap()
            .installed
            .is_empty());
    }

    #[tokio::test]
    async fn streamed_download_verifies_checksum_and_reports_progress() {
        async fn hello() -> &'static str {
            "hello"
        }
        let (base, server) = spawn_server(Router::new().route("/ok", get(hello))).await;

        let temp = TestDir::new("download-ok");
        let models_root = temp.path().join("data/models");
        let config_dir = temp.path().join("config");
        let mut package = package("pack", ModelCategory::Ocr, Vec::new());
        package.files.push(CatalogFile {
            path: "ocr/pack/model.onnx".to_string(),
            url: Some(format!("{base}/ok")),
            content: None,
            sha256: sha256_hex(b"hello"),
            size_bytes: 5,
        });
        let mut progress = Vec::new();
        install_package_core(
            &models_root,
            &metadata_path_for(&config_dir),
            &package,
            &build_http_client().unwrap(),
            &CancellationToken::new(),
            &mut |received, total| progress.push((received, total)),
        )
        .await
        .unwrap();
        server.abort();

        assert_eq!(
            std::fs::read(models_root.join("ocr/pack/model.onnx")).unwrap(),
            b"hello"
        );
        assert_eq!(progress.last().copied(), Some((5, 5)));
        assert_eq!(
            read_metadata(&metadata_path_for(&config_dir))
                .unwrap()
                .installed[0]
                .id,
            "pack"
        );
    }

    #[tokio::test]
    async fn checksum_failure_preserves_previous_file() {
        async fn wrong_bytes() -> &'static str {
            "world"
        }
        let (base, server) = spawn_server(Router::new().route("/wrong", get(wrong_bytes))).await;

        let temp = TestDir::new("checksum");
        let models_root = temp.path().join("data/models");
        let config_dir = temp.path().join("config");
        std::fs::create_dir_all(models_root.join("ocr/pack")).unwrap();
        std::fs::write(models_root.join("ocr/pack/model.onnx"), b"previous").unwrap();

        let mut package = package("pack", ModelCategory::Ocr, Vec::new());
        package.files.push(CatalogFile {
            path: "ocr/pack/model.onnx".to_string(),
            url: Some(format!("{base}/wrong")),
            content: None,
            sha256: sha256_hex(b"hello"),
            size_bytes: 5,
        });
        validate_catalog(&catalog(vec![package.clone()])).unwrap();

        let client = build_http_client().unwrap();
        let token = CancellationToken::new();
        let result = install_package_core(
            &models_root,
            &metadata_path_for(&config_dir),
            &package,
            &client,
            &token,
            &mut |_, _| {},
        )
        .await;
        server.abort();

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(models_root.join("ocr/pack/model.onnx")).unwrap(),
            b"previous"
        );
        assert!(read_metadata(&metadata_path_for(&config_dir))
            .unwrap()
            .installed
            .is_empty());
    }

    #[tokio::test]
    async fn cancellation_preserves_previous_files() {
        let temp = TestDir::new("cancel");
        let models_root = temp.path().join("data/models");
        let config_dir = temp.path().join("config");
        std::fs::create_dir_all(models_root.join("ocr/pack")).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(models_root.join("ocr/pack/model.onnx"), b"previous").unwrap();

        let package = package(
            "pack",
            ModelCategory::Ocr,
            vec![file_with_content("ocr/pack/model.onnx", "new")],
        );
        let client = build_http_client().unwrap();
        let token = CancellationToken::new();
        token.cancel();
        let result = install_package_core(
            &models_root,
            &metadata_path_for(&config_dir),
            &package,
            &client,
            &token,
            &mut |_, _| {},
        )
        .await;

        assert_eq!(result.unwrap_err(), CANCELLED_ERROR);
        assert_eq!(
            std::fs::read(models_root.join("ocr/pack/model.onnx")).unwrap(),
            b"previous"
        );
    }

    #[tokio::test]
    async fn cancellation_during_streaming_keeps_the_cancelled_outcome() {
        let (base, server) =
            spawn_server(Router::new().route("/file", get(|| async { "model" }))).await;
        let temp = TestDir::new("cancel-stream");
        let models_root = temp.path().join("models");
        let metadata_path = temp.path().join("config/model-downloads.json");
        let package = package(
            "pack",
            ModelCategory::Ocr,
            vec![CatalogFile {
                path: "ocr/pack/model.onnx".to_string(),
                url: Some(format!("{base}/file")),
                content: None,
                sha256: sha256_hex(b"model"),
                size_bytes: 5,
            }],
        );
        let token = CancellationToken::new();

        let result = install_package_core(
            &models_root,
            &metadata_path,
            &package,
            &build_http_client().unwrap(),
            &token,
            &mut |received, _| {
                if received > 0 {
                    token.cancel();
                }
            },
        )
        .await;
        server.abort();

        assert_eq!(result.unwrap_err(), CANCELLED_ERROR);
        assert!(!models_root.join("ocr/pack/model.onnx").exists());
        assert!(!metadata_path.exists());
    }

    #[tokio::test]
    async fn unavailable_package_is_never_requested() {
        let hits = StdArc::new(AtomicUsize::new(0));
        let handler_hits = hits.clone();
        let router = Router::new().route(
            "/never",
            get(move || {
                let hits = handler_hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    "hello"
                }
            }),
        );
        let (base, server) = spawn_server(router).await;

        let mut package = package("pack", ModelCategory::Ocr, Vec::new());
        package.available = false;
        package.unavailable_reason = Some("source is gone".to_string());
        package.files.push(CatalogFile {
            path: "ocr/pack/model.onnx".to_string(),
            url: Some(format!("{base}/never")),
            content: None,
            sha256: sha256_hex(b"hello"),
            size_bytes: 5,
        });

        let temp = TestDir::new("unavailable");
        let client = build_http_client().unwrap();
        let result = install_package_core(
            &temp.path().join("data/models"),
            &metadata_path_for(&temp.path().join("config")),
            &package,
            &client,
            &CancellationToken::new(),
            &mut |_, _| {},
        )
        .await;
        server.abort();

        assert!(result.unwrap_err().contains("unavailable"));
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn refresh_accepts_valid_catalog_and_uses_it_as_cache() {
        let remote = catalog(vec![package(
            "remote-pack",
            ModelCategory::Ocr,
            vec![file_with_content("ocr/remote-pack/manifest.json", "{}")],
        )]);
        let remote_json = serde_json::to_string(&remote).unwrap();
        let router = Router::new().route(
            "/catalog.json",
            get({
                let body = remote_json.clone();
                move || {
                    let body = body.clone();
                    async move { body }
                }
            }),
        );
        let (base, server) = spawn_server(router).await;

        let temp = TestDir::new("refresh");
        let config_dir = temp.path().join("config");
        let data_dir = temp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        let source = format!("{base}/catalog.json");
        let metadata = ModelDownloadsMetadata {
            catalog_source_url: Some(source.clone()),
            ..Default::default()
        };
        write_metadata(&metadata_path_for(&config_dir), &metadata).unwrap();
        let metadata_before = std::fs::read(metadata_path_for(&config_dir)).unwrap();

        let paths = AppPaths {
            config_dir: config_dir.clone(),
            data_dir: data_dir.clone(),
            models_root: data_dir.join("models"),
        };
        refresh_catalog(&paths).await.unwrap();
        server.abort();

        assert_eq!(
            std::fs::read(metadata_path_for(&config_dir)).unwrap(),
            metadata_before
        );
        let status = build_status(&paths).unwrap();
        let download = status.catalog_download.unwrap();
        assert_eq!(download.url, source);
        assert!(validate_utc_timestamp(&download.downloaded_at, "test").is_ok());
        assert!(catalog_path_for(&data_dir).is_file());
        assert!(!catalog_path_for(&config_dir).exists());

        assert_eq!(status.catalog.unwrap().packages[0].id, "remote-pack");
    }

    #[tokio::test]
    async fn refresh_rejects_malformed_catalog_without_replacing_previous_cache() {
        let router = Router::new().route("/bad.json", get(|| async { "{\"schema_version\":1}" }));
        let (base, server) = spawn_server(router).await;

        let temp = TestDir::new("refresh-bad");
        let config_dir = temp.path().join("config");
        let data_dir = temp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        let source = format!("{base}/bad.json");
        let metadata = ModelDownloadsMetadata {
            catalog_source_url: Some(source),
            ..Default::default()
        };
        write_metadata(&metadata_path_for(&config_dir), &metadata).unwrap();
        let metadata_before = std::fs::read(metadata_path_for(&config_dir)).unwrap();
        let cache_path = catalog_path_for(&data_dir);
        let previous_cache = serde_json::to_vec(&StoredCatalog {
            source_url: "https://example.com/catalog.json".to_string(),
            downloaded_at: "2026-11-11T00:00:00Z".to_string(),
            catalog: catalog(Vec::new()),
        })
        .unwrap();
        write_config_file(&cache_path, &previous_cache).unwrap();

        let paths = AppPaths {
            config_dir: config_dir.clone(),
            data_dir: data_dir.clone(),
            models_root: data_dir.join("models"),
        };
        assert!(refresh_catalog(&paths).await.is_err());
        server.abort();

        assert_eq!(
            std::fs::read(metadata_path_for(&config_dir)).unwrap(),
            metadata_before
        );
        assert_eq!(std::fs::read(cache_path).unwrap(), previous_cache);
        let status = build_status(&paths).unwrap();
        assert_eq!(
            status.catalog_download.unwrap().url,
            "https://example.com/catalog.json"
        );
    }

    #[tokio::test]
    async fn refresh_rejects_blank_license_without_replacing_valid_previous_cache() {
        let mut remote_package = package(
            "remote-pack",
            ModelCategory::Ocr,
            vec![file_with_content("ocr/remote-pack/manifest.json", "{}")],
        );
        remote_package.license = Some("   ".to_string());
        let remote_json = serde_json::to_string(&catalog(vec![remote_package])).unwrap();
        let router = Router::new().route(
            "/blank-license.json",
            get({
                let body = remote_json.clone();
                move || {
                    let body = body.clone();
                    async move { body }
                }
            }),
        );
        let (base, server) = spawn_server(router).await;

        let temp = TestDir::new("refresh-blank-license");
        let config_dir = temp.path().join("config");
        let data_dir = temp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        let source = format!("{base}/blank-license.json");
        let metadata = ModelDownloadsMetadata {
            catalog_source_url: Some(source),
            ..Default::default()
        };
        write_metadata(&metadata_path_for(&config_dir), &metadata).unwrap();
        let metadata_before = std::fs::read(metadata_path_for(&config_dir)).unwrap();
        let cache_path = catalog_path_for(&data_dir);
        let previous_cache = serde_json::to_vec(&StoredCatalog {
            source_url: "https://example.com/catalog.json".to_string(),
            downloaded_at: "2026-11-11T00:00:00Z".to_string(),
            catalog: catalog(vec![package(
                "cached-pack",
                ModelCategory::Ocr,
                vec![file_with_content("ocr/cached-pack/manifest.json", "{}")],
            )]),
        })
        .unwrap();
        write_config_file(&cache_path, &previous_cache).unwrap();

        let paths = AppPaths {
            config_dir: config_dir.clone(),
            data_dir: data_dir.clone(),
            models_root: data_dir.join("models"),
        };
        let error = refresh_catalog(&paths).await.unwrap_err();
        server.abort();

        assert!(error.contains("license"), "{error}");
        assert_eq!(
            std::fs::read(metadata_path_for(&config_dir)).unwrap(),
            metadata_before
        );
        assert_eq!(std::fs::read(cache_path).unwrap(), previous_cache);
        let status = build_status(&paths).unwrap();
        assert_eq!(status.catalog.unwrap().packages[0].id, "cached-pack");
        assert_eq!(
            status.catalog_download.unwrap().url,
            "https://example.com/catalog.json"
        );
    }

    #[tokio::test]
    async fn ruaccent_install_preserves_other_variants_and_unrelated_files() {
        let temp = TestDir::new("ruaccent-coexist");
        let models_root = temp.path().join("data/models");
        let config_dir = temp.path().join("config");
        let other_variant = models_root.join("ruaccent/nn/nn_omograph/other");
        std::fs::create_dir_all(&other_variant).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(other_variant.join("model.onnx"), b"other-model").unwrap();
        std::fs::write(other_variant.join("tokenizer.json"), b"other-tokenizer").unwrap();
        std::fs::write(models_root.join("ruaccent/notes.txt"), b"notes").unwrap();

        let package = package(
            "ruaccent-tiny",
            ModelCategory::Ruaccent,
            vec![
                file_with_content("ruaccent/nn/nn_omograph/tiny/model.onnx", "tiny-model"),
                file_with_content(
                    "ruaccent/nn/nn_omograph/tiny/tokenizer.json",
                    "tiny-tokenizer",
                ),
            ],
        );
        let client = build_http_client().unwrap();
        install_package_core(
            &models_root,
            &metadata_path_for(&config_dir),
            &package,
            &client,
            &CancellationToken::new(),
            &mut |_, _| {},
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read(other_variant.join("model.onnx")).unwrap(),
            b"other-model"
        );
        assert_eq!(
            std::fs::read(other_variant.join("tokenizer.json")).unwrap(),
            b"other-tokenizer"
        );
        assert_eq!(
            std::fs::read(models_root.join("ruaccent/notes.txt")).unwrap(),
            b"notes"
        );
        assert_eq!(
            std::fs::read(models_root.join("ruaccent/nn/nn_omograph/tiny/model.onnx")).unwrap(),
            b"tiny-model"
        );
    }

    #[tokio::test]
    async fn malformed_metadata_blocks_install_without_writing_files() {
        let temp = TestDir::new("malformed-metadata");
        let models_root = temp.path().join("data/models");
        let config_dir = temp.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(models_root.join("ocr/pack")).unwrap();
        std::fs::write(models_root.join("ocr/pack/model.onnx"), b"previous").unwrap();
        let metadata_path = metadata_path_for(&config_dir);
        std::fs::write(&metadata_path, b"{not json").unwrap();

        let package = package(
            "pack",
            ModelCategory::Ocr,
            vec![file_with_content("ocr/pack/model.onnx", "new")],
        );
        let result = install_package_core(
            &models_root,
            &metadata_path,
            &package,
            &build_http_client().unwrap(),
            &CancellationToken::new(),
            &mut |_, _| {},
        )
        .await;

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(models_root.join("ocr/pack/model.onnx")).unwrap(),
            b"previous"
        );
        assert_eq!(std::fs::read(&metadata_path).unwrap(), b"{not json");
    }

    #[test]
    fn missing_or_malformed_catalog_file_returns_empty_catalog() {
        let temp = TestDir::new("catalog-empty-or-broken");
        let config_dir = temp.path().join("config");
        let data_dir = temp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        let paths = AppPaths {
            config_dir: config_dir.clone(),
            data_dir: data_dir.clone(),
            models_root: data_dir.join("models"),
        };

        let fresh = build_status(&paths).unwrap();
        assert!(fresh.catalog.is_none());
        assert!(fresh.catalog_download.is_none());
        assert!(fresh.installed_package_ids.is_empty());
        assert!(fresh.outdated_package_ids.is_empty());

        std::fs::write(catalog_path_for(&data_dir), b"{broken").unwrap();
        let metadata = ModelDownloadsMetadata {
            catalog_download: Some(CatalogDownload {
                url: "https://example.com/model-catalog.json".to_string(),
                downloaded_at: "2026-11-11T00:00:00Z".to_string(),
            }),
            ..Default::default()
        };
        write_metadata(&metadata_path_for(&config_dir), &metadata).unwrap();
        let status = build_status(&paths).unwrap();
        assert!(status.catalog.is_none());
        assert!(status.catalog_download.is_none());
    }

    #[test]
    fn bare_catalog_is_read_with_optional_legacy_provenance() {
        let legacy = CatalogDownload {
            url: "https://example.com/legacy.json".to_string(),
            downloaded_at: "2026-11-11T00:00:00Z".to_string(),
        };
        let old = catalog(Vec::new());
        let bytes = serde_json::to_vec(&old).unwrap();
        let (loaded, provenance) = parse_stored_catalog(&bytes, Some(&legacy)).unwrap();
        assert_eq!(loaded, old);
        assert_eq!(provenance, Some(legacy));

        let (loaded_bare, no_provenance) = parse_stored_catalog(&bytes, None).unwrap();
        assert_eq!(loaded_bare, old);
        assert!(no_provenance.is_none());
    }

    #[test]
    fn default_catalog_source_url_is_used_when_unset() {
        assert_eq!(
            effective_catalog_source_url(None),
            DEFAULT_CATALOG_SOURCE_URL
        );
        assert_eq!(
            effective_catalog_source_url(Some("   ")),
            DEFAULT_CATALOG_SOURCE_URL
        );
        assert_eq!(
            effective_catalog_source_url(Some("https://example.com/custom.json")),
            "https://example.com/custom.json"
        );
    }

    #[tokio::test]
    async fn reuses_verified_local_file_without_downloading() {
        let shared_hits = StdArc::new(AtomicUsize::new(0));
        let wrong_hits = StdArc::new(AtomicUsize::new(0));
        let shared_counter = shared_hits.clone();
        let wrong_counter = wrong_hits.clone();
        let router = Router::new()
            .route(
                "/shared",
                get(move || {
                    let shared_counter = shared_counter.clone();
                    async move {
                        shared_counter.fetch_add(1, Ordering::SeqCst);
                        "shared-ok"
                    }
                }),
            )
            .route(
                "/fresh",
                get(move || {
                    let wrong_counter = wrong_counter.clone();
                    async move {
                        wrong_counter.fetch_add(1, Ordering::SeqCst);
                        "fresh-new"
                    }
                }),
            );
        let (base, server) = spawn_server(router).await;

        let temp = TestDir::new("reuse-local");
        let models_root = temp.path().join("data/models");
        let config_dir = temp.path().join("config");
        std::fs::create_dir_all(models_root.join("ruaccent/dictionary")).unwrap();
        std::fs::write(
            models_root.join("ruaccent/dictionary/shared.bin"),
            b"shared-ok",
        )
        .unwrap();
        std::fs::write(
            models_root.join("ruaccent/dictionary/fresh.bin"),
            b"stale-bad",
        )
        .unwrap();

        let package = package(
            "ruaccent-tiny2",
            ModelCategory::Ruaccent,
            vec![
                CatalogFile {
                    path: "ruaccent/dictionary/shared.bin".to_string(),
                    url: Some(format!("{base}/shared")),
                    content: None,
                    sha256: sha256_hex(b"shared-ok"),
                    size_bytes: 9,
                },
                CatalogFile {
                    path: "ruaccent/dictionary/fresh.bin".to_string(),
                    url: Some(format!("{base}/fresh")),
                    content: None,
                    sha256: sha256_hex(b"fresh-new"),
                    size_bytes: 9,
                },
            ],
        );

        let client = build_http_client().unwrap();
        let mut progress = Vec::new();
        install_package_core(
            &models_root,
            &metadata_path_for(&config_dir),
            &package,
            &client,
            &CancellationToken::new(),
            &mut |received, total| progress.push((received, total)),
        )
        .await
        .unwrap();
        server.abort();

        assert_eq!(shared_hits.load(Ordering::SeqCst), 0);
        assert_eq!(wrong_hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            std::fs::read(models_root.join("ruaccent/dictionary/shared.bin")).unwrap(),
            b"shared-ok"
        );
        assert_eq!(
            std::fs::read(models_root.join("ruaccent/dictionary/fresh.bin")).unwrap(),
            b"fresh-new"
        );
        assert_eq!(progress.last().copied(), Some((18, 18)));
    }

    #[tokio::test]
    async fn multipart_range_download_assembles_and_verifies_sha256() {
        use axum::http::{header, HeaderMap, StatusCode as AxumStatus};

        let payload: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let payload_arc = StdArc::new(payload.clone());
        let range_requests = StdArc::new(AtomicUsize::new(0));
        let counter = range_requests.clone();

        let router = Router::new()
            .route(
                "/big.bin",
                get(|headers: HeaderMap| async move {
                    let range = headers[header::RANGE].to_str().unwrap();
                    axum::response::Redirect::temporary(&format!("/cdn/{}", &range[6..]))
                }),
            )
            .route(
                "/cdn/{signed_range}",
                get(
                    move |axum::extract::Path(signed_range): axum::extract::Path<String>,
                          headers: HeaderMap| {
                        let data = payload_arc.clone();
                        let counter = counter.clone();
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                            let total = data.len();
                            let range_hdr = headers
                                .get(header::RANGE)
                                .and_then(|v| v.to_str().ok())
                                .unwrap();
                            assert_eq!(range_hdr, format!("bytes={signed_range}"));
                            let (start, end) = range_hdr
                                .strip_prefix("bytes=")
                                .and_then(|r| r.split_once('-'))
                                .map(|(s, e)| {
                                    (s.parse::<usize>().unwrap(), e.parse::<usize>().unwrap())
                                })
                                .unwrap();
                            let slice = data[start..=end].to_vec();
                            let mut resp_headers = HeaderMap::new();
                            resp_headers.insert(
                                header::CONTENT_RANGE,
                                format!("bytes {start}-{end}/{total}").parse().unwrap(),
                            );
                            (AxumStatus::PARTIAL_CONTENT, resp_headers, slice)
                        }
                    },
                ),
            );
        let (base, server) = spawn_server(router).await;

        let temp = TestDir::new("multipart-range");
        let dest = temp.path().join("big.bin");
        let semaphore = tokio::sync::Semaphore::new(6);
        let downloaded = AtomicU64::new(0);
        download_file(
            &build_http_client().unwrap(),
            &format!("{base}/big.bin"),
            &dest,
            payload.len() as u64,
            &sha256_hex(&payload),
            &CancellationToken::new(),
            &semaphore,
            16,
            &|delta| {
                downloaded.fetch_add(delta, Ordering::SeqCst);
            },
        )
        .await
        .unwrap();
        server.abort();

        assert_eq!(range_requests.load(Ordering::SeqCst), MULTIPART_RANGE_PARTS);
        assert_eq!(downloaded.load(Ordering::SeqCst), payload.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), payload);
    }

    #[tokio::test]
    async fn multipart_range_falls_back_to_single_stream_on_200_ok() {
        let payload = vec![b'x'; 256];
        let payload_arc = StdArc::new(payload.clone());
        let hits = StdArc::new(AtomicUsize::new(0));
        let counter = hits.clone();

        let router = Router::new().route(
            "/no-range.bin",
            get(move || {
                let data = payload_arc.clone();
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    data.as_ref().clone()
                }
            }),
        );
        let (base, server) = spawn_server(router).await;

        let temp = TestDir::new("range-fallback");
        let dest = temp.path().join("no-range.bin");
        let semaphore = tokio::sync::Semaphore::new(6);
        download_file(
            &build_http_client().unwrap(),
            &format!("{base}/no-range.bin"),
            &dest,
            payload.len() as u64,
            &sha256_hex(&payload),
            &CancellationToken::new(),
            &semaphore,
            16,
            &|_| {},
        )
        .await
        .unwrap();
        server.abort();

        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(&dest).unwrap(), payload);
    }

    #[tokio::test]
    async fn concurrent_package_install_downloads_multiple_files_monotonically() {
        let router = Router::new()
            .route("/f1", get(|| async { "aaaa" }))
            .route("/f2", get(|| async { "bbbbbb" }))
            .route("/f3", get(|| async { "cc" }))
            .route("/f4", get(|| async { "dddddddd" }));
        let (base, server) = spawn_server(router).await;

        let temp = TestDir::new("concurrent-files");
        let models_root = temp.path().join("data/models");
        let config_dir = temp.path().join("config");
        let package = package(
            "multi",
            ModelCategory::Pocket,
            vec![
                CatalogFile {
                    path: "pocket/multi/f1.bin".to_string(),
                    url: Some(format!("{base}/f1")),
                    content: None,
                    sha256: sha256_hex(b"aaaa"),
                    size_bytes: 4,
                },
                CatalogFile {
                    path: "pocket/multi/f2.bin".to_string(),
                    url: Some(format!("{base}/f2")),
                    content: None,
                    sha256: sha256_hex(b"bbbbbb"),
                    size_bytes: 6,
                },
                CatalogFile {
                    path: "pocket/multi/f3.bin".to_string(),
                    url: Some(format!("{base}/f3")),
                    content: None,
                    sha256: sha256_hex(b"cc"),
                    size_bytes: 2,
                },
                CatalogFile {
                    path: "pocket/multi/f4.bin".to_string(),
                    url: Some(format!("{base}/f4")),
                    content: None,
                    sha256: sha256_hex(b"dddddddd"),
                    size_bytes: 8,
                },
            ],
        );

        let mut progress = Vec::new();
        install_package_core(
            &models_root,
            &metadata_path_for(&config_dir),
            &package,
            &build_http_client().unwrap(),
            &CancellationToken::new(),
            &mut |received, total| progress.push((received, total)),
        )
        .await
        .unwrap();
        server.abort();

        assert!(progress.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(progress.last().copied(), Some((20, 20)));
        assert_eq!(
            std::fs::read(models_root.join("pocket/multi/f4.bin")).unwrap(),
            b"dddddddd"
        );
    }
}
