//! Sound Panel Storage
//!
//! Хранение привязок звуковой панели в JSON файле в %APPDATA%.
//! Копирование аудиофайлов в папку soundpanel.
//!
//! NOTE: Appearance settings are now stored in windows.json (via WindowsManager)
//! The old soundpanel_appearance.json file is no longer used.

use crate::config::{config_write_lock, replace_file_atomically, WindowsManager};
use crate::soundpanel::state::{SoundBinding, SoundPanelState, SoundSets};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use uuid::Uuid;

const BINDINGS_FILE: &str = "soundpanel_bindings.json";

/// Настройки внешнего вида звуковой панели (deprecated, use WindowsManager instead)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundPanelAppearance {
    pub opacity: u8,
    pub bg_color: String,
    /// Пропускает ли плавающее окно клики
    #[serde(default = "default_clickthrough")]
    pub clickthrough: bool,
}

fn default_clickthrough() -> bool {
    false
}

impl Default for SoundPanelAppearance {
    fn default() -> Self {
        Self {
            opacity: 90,
            bg_color: "#2a2a2a".to_string(),
            clickthrough: false,
        }
    }
}

/// Загрузить привязки из JSON файла (с миграцией из старого формата)
pub fn load_bindings(state: &SoundPanelState) -> Result<Vec<SoundBinding>, String> {
    let appdata_path = state
        .appdata_path
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .clone();
    let file_path = PathBuf::from(&appdata_path).join(BINDINGS_FILE);

    debug!(?file_path, "Loading bindings");

    if !file_path.exists() {
        debug!("Bindings file does not exist, starting with empty bindings");
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read bindings file: {}", e))?;

    let sets = if let Ok(parsed) = serde_json::from_str::<SoundSets>(&content) {
        info!(
            set_count = parsed.sets.len(),
            "Loaded bindings (new format)"
        );
        parsed
    } else if let Ok(old_bindings) = serde_json::from_str::<Vec<SoundBinding>>(&content) {
        // Миграция из старого формата Vec<SoundBinding> → SoundSets
        warn!(
            count = old_bindings.len(),
            "Migrating bindings from old format (Vec) to SoundSets"
        );
        let id = uuid::Uuid::new_v4().to_string();
        SoundSets {
            active_set_id: id.clone(),
            sets: vec![crate::soundpanel::state::SoundSet {
                id,
                name: "Основной".into(),
                bindings: old_bindings,
            }],
        }
    } else {
        return Err("Failed to parse bindings file: unrecognized format".to_string());
    };

    let bindings = sets
        .find_active()
        .map(|s| s.bindings.clone())
        .unwrap_or_default();

    info!(
        set_count = sets.sets.len(),
        bindings_count = bindings.len(),
        "Loaded bindings"
    );

    state.publish_sets(sets);

    Ok(bindings)
}

/// Сохранить кандидатный снимок наборов для указанного состояния атомарно.
///
/// Persist-first helper для мутаций `SoundSets`: вызывающий код строит снимок,
/// сохраняет его этим вызовом и публикует в runtime только после успеха.
/// Не изменяет runtime-состояние самостоятельно.
pub fn save_sets(state: &SoundPanelState, sets: &SoundSets) -> Result<(), String> {
    let appdata_path = state
        .appdata_path
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .clone();
    persist_sets(&appdata_path, sets)
}

/// Сохранить кандидатный снимок наборов в JSON файл атомарно.
///
/// Использует общий примитив атомарной замены из config. Не изменяет и не
/// публикует runtime-состояние — вызывающий код отвечает за публикацию после
/// успешного сохранения.
pub fn persist_sets(appdata_path: &str, sets: &SoundSets) -> Result<(), String> {
    let file_path = PathBuf::from(appdata_path).join(BINDINGS_FILE);

    info!(set_count = sets.sets.len(), active_set_id = %sets.active_set_id, ?file_path, "Persisting sets");

    let json = serde_json::to_string_pretty(sets)
        .map_err(|e| format!("Failed to serialize sets: {}", e))?;

    let _guard = config_write_lock().lock();
    replace_file_atomically(&file_path, json.as_bytes())
        .map_err(|e| format!("Failed to persist bindings file: {}", e))?;

    info!("Sets persisted successfully");
    Ok(())
}

/// Аудиофайл, скопированный в уникальный staging-файл в папке soundpanel.
///
/// Файл не становится доступным привязкам до вызова [`StagedAudio::commit`];
/// при отказе вызывается [`StagedAudio::cleanup`].
pub struct StagedAudio {
    staging_path: PathBuf,
    final_path: PathBuf,
}

impl StagedAudio {
    /// Финальное имя файла, которое будет записано в кандидатный снимок.
    pub fn final_filename(&self) -> String {
        self.final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string()
    }

    /// Продвинуть staging-файл в финальное имя.
    ///
    /// Должно вызываться только после успешного сохранения кандидатного
    /// снимка. При ошибке staging-файл удаляется, а ошибка возвращается.
    pub fn commit(self) -> Result<(), String> {
        let StagedAudio {
            staging_path,
            final_path,
        } = self;
        if let Err(e) = fs::rename(&staging_path, &final_path) {
            let _ = fs::remove_file(&staging_path);
            return Err(format!("Failed to promote staged audio: {}", e));
        }
        debug!(?final_path, "Promoted staged audio");
        Ok(())
    }

    /// Удалить staging-файл после отказа (best-effort).
    pub fn cleanup(self) {
        let _ = fs::remove_file(&self.staging_path);
    }
}

/// Скопировать аудио в уникальный staging-файл в папке soundpanel.
///
/// Возвращает [`StagedAudio`] с финальным именем для кандидатного снимка.
pub fn stage_sound_file(source_path: &str, appdata_path: &str) -> Result<StagedAudio, String> {
    let soundpanel_dir = PathBuf::from(appdata_path).join("soundpanel");

    if !soundpanel_dir.exists() {
        fs::create_dir_all(&soundpanel_dir)
            .map_err(|e| format!("Failed to create soundpanel directory: {}", e))?;
        debug!(?soundpanel_dir, "Created soundpanel directory");
    }

    let source = PathBuf::from(source_path);
    let filename = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid filename")?;

    let final_path = generate_unique_path(&soundpanel_dir, filename);
    let staging_path = generate_staging_path(&soundpanel_dir, &final_path);

    fs::copy(&source, &staging_path).map_err(|e| format!("Failed to copy sound file: {}", e))?;

    debug!(source_path, ?staging_path, ?final_path, "Staged sound file");

    Ok(StagedAudio {
        staging_path,
        final_path,
    })
}

/// Сгенерировать уникальный путь для staging-файла, сохраняя расширение.
fn generate_staging_path(dir: &Path, final_path: &Path) -> PathBuf {
    let ext = final_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| format!(".{}", s))
        .unwrap_or_default();

    loop {
        let name = format!(".staging.{}{}", Uuid::new_v4(), ext);
        let path = dir.join(&name);
        if !path.exists() {
            return path;
        }
    }
}

/// Удалить файл звука из папки soundpanel
pub fn delete_sound_file(filename: &str, appdata_path: &str) -> Result<(), String> {
    let soundpanel_dir = PathBuf::from(appdata_path).join("soundpanel");
    let file_path = soundpanel_dir.join(filename);

    if file_path.exists() {
        fs::remove_file(&file_path).map_err(|e| format!("Failed to delete sound file: {}", e))?;
        debug!(?file_path, "Deleted sound file");
    }

    Ok(())
}

/// Сгенерировать уникальный путь для файла
/// Если файл с таким именем существует, добавляет суффикс _1, _2 и т.д.
fn generate_unique_path(dir: &Path, filename: &str) -> PathBuf {
    let mut path = dir.join(filename);
    let mut counter = 1;

    let stem = PathBuf::from(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();

    let ext = PathBuf::from(filename)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{}", s))
        .unwrap_or_default();

    while path.exists() {
        let new_name = format!("{}_{}{}", stem, counter, ext);
        path = dir.join(&new_name);
        counter += 1;
    }

    path
}

/// Загрузить настройки внешнего вида из windows.json
pub fn load_appearance(
    state: &SoundPanelState,
    windows_manager: &WindowsManager,
) -> Result<SoundPanelAppearance, String> {
    debug!("Loading appearance from windows.json");

    let opacity = windows_manager.get_soundpanel_opacity();
    let bg_color = windows_manager.get_soundpanel_bg_color();
    let clickthrough = windows_manager.get_soundpanel_clickthrough();
    let stored_stay_visible = windows_manager.get_soundpanel_stay_visible();
    let hide_on_blur = windows_manager.get_soundpanel_hide_on_blur();
    let stay_visible = normalize_pin_state(stored_stay_visible, hide_on_blur);

    if stored_stay_visible != stay_visible || hide_on_blur == stay_visible {
        windows_manager
            .set_soundpanel_stay_visible(stay_visible)
            .map_err(|e| format!("Failed to migrate SoundPanel pin state: {}", e))?;
    }

    info!(
        opacity,
        bg_color, clickthrough, stay_visible, "Loaded appearance"
    );

    state.set_floating_opacity(opacity);
    state.set_floating_bg_color(bg_color.clone());
    state.set_floating_clickthrough(clickthrough);
    state.set_stay_visible(stay_visible);

    Ok(SoundPanelAppearance {
        opacity,
        bg_color,
        clickthrough,
    })
}

fn normalize_pin_state(stay_visible: bool, hide_on_blur: bool) -> bool {
    stay_visible || !hide_on_blur
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soundpanel::state::SoundSet;

    fn temp_dir(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ttsbard-soundpanel-{}-{}-{}",
            label,
            std::process::id(),
            stamp
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    fn staging_files(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with(".staging."))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn staged_audio_copies_then_promotes_to_final_path() {
        let dir = temp_dir("stage-promote");
        let appdata = dir.to_string_lossy().to_string();
        let source = dir.join("source.mp3");
        fs::write(&source, b"audio-bytes").unwrap();

        let staged = stage_sound_file(&source.to_string_lossy(), &appdata).unwrap();
        let final_name = staged.final_filename();
        assert!(!final_name.is_empty());

        let soundpanel = dir.join("soundpanel");
        assert_eq!(staging_files(&soundpanel).len(), 1);
        assert!(
            !soundpanel.join(&final_name).exists(),
            "final file must not exist before commit"
        );

        staged.commit().unwrap();

        assert!(soundpanel.join(&final_name).is_file());
        assert_eq!(
            fs::read(soundpanel.join(&final_name)).unwrap(),
            b"audio-bytes"
        );
        assert!(
            staging_files(&soundpanel).is_empty(),
            "no staging file should remain after promotion"
        );

        cleanup(&dir);
    }

    #[test]
    fn staged_audio_cleanup_removes_staging_file() {
        let dir = temp_dir("stage-cleanup");
        let appdata = dir.to_string_lossy().to_string();
        let source = dir.join("source.wav");
        fs::write(&source, b"bytes").unwrap();

        let staged = stage_sound_file(&source.to_string_lossy(), &appdata).unwrap();
        let soundpanel = dir.join("soundpanel");
        assert_eq!(staging_files(&soundpanel).len(), 1);

        staged.cleanup();

        assert!(
            staging_files(&soundpanel).is_empty(),
            "no staging file should remain after cleanup"
        );

        cleanup(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn persistence_failure_cleanup_preserves_existing_bindings() {
        let dir = temp_dir("persist-fail");
        let appdata = dir.to_string_lossy().to_string();

        let bindings_path = dir.join(BINDINGS_FILE);
        let original = "{\"active_set_id\":\"set1\",\"sets\":[]}";
        fs::write(&bindings_path, original).unwrap();

        let source = dir.join("source.mp3");
        fs::write(&source, b"bytes").unwrap();
        let staged = stage_sound_file(&source.to_string_lossy(), &appdata).unwrap();
        let soundpanel = dir.join("soundpanel");
        assert_eq!(staging_files(&soundpanel).len(), 1);

        // Hold the target open without FILE_SHARE_DELETE so ReplaceFileW fails
        // while the existing bindings file remains intact on disk.
        let lock = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0x1)
                .open(&bindings_path)
                .unwrap()
        };

        let candidate = SoundSets::default();
        assert!(persist_sets(&appdata, &candidate).is_err());

        drop(lock);
        staged.cleanup();
        assert!(
            staging_files(&soundpanel).is_empty(),
            "no staging file should remain after cleanup"
        );

        assert_eq!(
            fs::read_to_string(&bindings_path).unwrap(),
            original,
            "existing bindings target must remain readable after persistence failure"
        );

        cleanup(&dir);
    }

    #[test]
    fn pin_state_preserves_legacy_hide_on_blur_opt_out() {
        assert!(normalize_pin_state(false, false));
        assert!(normalize_pin_state(true, true));
        assert!(normalize_pin_state(true, false));
        assert!(!normalize_pin_state(false, true));
    }

    #[test]
    fn test_migration_old_vec_to_sound_sets() {
        let old_json = r#"[
            {"key":"A","description":"test a","filename":"a.mp3","original_path":null},
            {"key":"B","description":"test b","filename":"b.mp3","original_path":"D:\\b.mp3"}
        ]"#;

        // Simulate parsing logic
        let content = old_json;
        let sets = if let Ok(parsed) = serde_json::from_str::<SoundSets>(content) {
            parsed
        } else if let Ok(old_bindings) = serde_json::from_str::<Vec<SoundBinding>>(content) {
            let id = uuid::Uuid::new_v4().to_string();
            SoundSets {
                active_set_id: id.clone(),
                sets: vec![SoundSet {
                    id,
                    name: "Основной".into(),
                    bindings: old_bindings,
                }],
            }
        } else {
            SoundSets::default()
        };

        assert_eq!(sets.sets.len(), 1);
        assert_eq!(sets.sets[0].name, "Основной");
        assert_eq!(sets.sets[0].bindings.len(), 2);
        assert_eq!(sets.sets[0].bindings[0].key, 'A');
        assert!(!sets.active_set_id.is_empty());
    }

    #[test]
    fn test_new_format_loads_directly() {
        let new_json = r#"{
            "active_set_id": "set1",
            "sets": [
                {"id": "set1", "name": "Основной", "bindings": []},
                {"id": "set2", "name": "Мемы", "bindings": [
                    {"key":"Z","description":"lol","filename":"lol.mp3","original_path":null}
                ]}
            ]
        }"#;

        let sets: SoundSets = serde_json::from_str(new_json).unwrap();
        assert_eq!(sets.sets.len(), 2);
        assert_eq!(sets.active_set_id, "set1");

        let active = sets.find_active().unwrap();
        assert_eq!(active.id, "set1");
        assert_eq!(active.name, "Основной");
    }
}
