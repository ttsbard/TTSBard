//! Intercept Settings: NumPad / F-keys / navigation keys → actions
//!
//! Persisted config stored in %APPDATA%/ttsbard/intercept.json

use crate::config::{
    config_write_lock, replace_file_atomically, EditorHotkeySettings, Hotkey, HotkeySettings,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::info;

pub const INTERCEPT_FILE: &str = "intercept.json";

/// Check-then-write coordination for NumPad code reservations across the two
/// persisted stores: `settings.json` (global/editor hotkeys, written by
/// `commands::window`) and `intercept.json` (sound panel bindings, written by
/// `soundpanel::bindings`). The stores have separate locks, so a conflict
/// check that runs against a snapshot while the other side commits could let
/// two incompatible saves land — a NumPad code reserved by a global hotkey
/// and an intercept binding at once. Every save path that can create such an
/// intersection holds this lock across its whole
/// [load snapshot → conflict check → persist] section, which makes
/// incompatible saves mutually exclusive: the second one to enter always
/// checks against the first one's committed write and is rejected.
///
/// The lock is always the outermost one (the sections take no other lock
/// before it), and no writer path that holds the intercept writer gate or the
/// settings cache lock ever takes it, so there is no lock-order cycle. It is
/// a plain blocking lock on purpose: every critical section is synchronous
/// disk/registry work on user-triggered save paths, never UDP/I/O or audio.
static NUMPAD_COORDINATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquire the shared NumPad check-then-write coordination lock. A section
/// must complete its whole load-check-persist sequence before releasing the
/// guard. A poisoned lock is recovered from: a panicking save path must not
/// wedge every later save.
pub(crate) fn lock_numpad_coordination() -> std::sync::MutexGuard<'static, ()> {
    NUMPAD_COORDINATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InterceptSettings {
    pub enabled: bool,
    pub bindings: Vec<InterceptBinding>,
    #[serde(default)]
    pub allow_any_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptBinding {
    pub key: String,
    pub action: String,
}

pub fn load(appdata_path: &str) -> InterceptSettings {
    let file_path = PathBuf::from(appdata_path).join(INTERCEPT_FILE);
    if !file_path.exists() {
        return InterceptSettings::default();
    }
    match fs::read_to_string(&file_path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(settings) => settings,
            Err(e) => {
                tracing::warn!(error = %e, ?file_path, "Failed to parse intercept.json, using defaults");
                InterceptSettings::default()
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, ?file_path, "Failed to read intercept.json, using defaults");
            InterceptSettings::default()
        }
    }
}

pub fn save(appdata_path: &str, settings: &InterceptSettings) -> Result<(), String> {
    let file_path = PathBuf::from(appdata_path).join(INTERCEPT_FILE);
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize intercept settings: {}", e))?;
    let _guard = config_write_lock().lock();
    replace_file_atomically(&file_path, json.as_bytes())
        .map_err(|e| format!("Failed to write intercept.json: {}", e))?;
    info!(?file_path, "Intercept settings saved");
    Ok(())
}

/// Convert a Windows virtual key code to a canonical key name.
/// Returns None for keys outside the supported range (NumPad + F-keys + navigation keys).
pub fn vk_to_name(vk_code: u32) -> Option<String> {
    match vk_code {
        0x60..=0x69 => Some(format!("NUMPAD{}", vk_code - 0x60)),
        0x6A => Some("NUMPAD_MULTIPLY".to_string()),
        0x6B => Some("NUMPAD_ADD".to_string()),
        0x6D => Some("NUMPAD_SUBTRACT".to_string()),
        0x6E => Some("NUMPAD_DECIMAL".to_string()),
        0x6F => Some("NUMPAD_DIVIDE".to_string()),
        0x70..=0x87 => Some(format!("F{}", vk_code - 0x6F)),
        0x21 => Some("PAGEUP".to_string()),
        0x22 => Some("PAGEDOWN".to_string()),
        0x23 => Some("END".to_string()),
        0x24 => Some("HOME".to_string()),
        0x2D => Some("INSERT".to_string()),
        _ => None,
    }
}

/// Convert a Windows virtual key code to a canonical key name, taking the
/// unrestricted interception mode into account.
///
/// Canonical names (NumPad / F-keys / navigation keys, resolved by
/// [`vk_to_name`]) always win. When `allow_any_key` is true, any other
/// keyboard VK in `0x08..=0xFE` is resolved to `VK_XX` (uppercase two-digit
/// hex). Restricted mode returns `None` for those keys. Values outside the
/// keyboard VK range (including `0` and `255`) are always `None`.
///
/// Sided modifier VKs (`0xA0..=0xA5`) are intentionally not collapsed into
/// generic modifiers: the Windows hook reports them distinctly, so each side
/// keeps its own `VK_XX` name.
pub fn vk_to_name_for_mode(vk_code: u32, allow_any_key: bool) -> Option<String> {
    if let Some(name) = vk_to_name(vk_code) {
        return Some(name);
    }
    if allow_any_key && (0x08..=0xFE).contains(&vk_code) {
        return Some(format!("VK_{:02X}", vk_code));
    }
    None
}

/// Windows VK code behind a canonical NumPad key name, or `None` for any
/// other name.
///
/// This is the inverse of [`vk_to_name`] restricted to the NumPad names and
/// the shared physical-key identifier for the ROADMAP-140 conflict checks
/// between global hotkeys and intercept bindings: the codes are exactly the
/// VKs the `global-hotkey` dependency derives from `Code::Numpad0..9`
/// (0x60..=0x69), `NumpadAdd` (0x6B), `NumpadSubtract` (0x6D),
/// `NumpadDecimal` (0x6E), `NumpadDivide` (0x6F) and `NumpadMultiply`
/// (0x6A), so a saved global NumPad hotkey and an intercept binding fight
/// over the same physical key precisely when this function resolves both
/// names to the same code.
pub(crate) fn numpad_vk_code(key: &str) -> Option<u32> {
    match key.to_uppercase().as_str() {
        "NUMPAD0" => Some(0x60),
        "NUMPAD1" => Some(0x61),
        "NUMPAD2" => Some(0x62),
        "NUMPAD3" => Some(0x63),
        "NUMPAD4" => Some(0x64),
        "NUMPAD5" => Some(0x65),
        "NUMPAD6" => Some(0x66),
        "NUMPAD7" => Some(0x67),
        "NUMPAD8" => Some(0x68),
        "NUMPAD9" => Some(0x69),
        "NUMPAD_MULTIPLY" => Some(0x6A),
        "NUMPAD_ADD" => Some(0x6B),
        "NUMPAD_SUBTRACT" => Some(0x6D),
        "NUMPAD_DECIMAL" => Some(0x6E),
        "NUMPAD_DIVIDE" => Some(0x6F),
        _ => None,
    }
}

/// Russian label for an intercept action id, mirroring the UI catalog
/// (`intercept.actions.*` in `locales/ru.json`). Unknown action ids are
/// shown verbatim so hand-edited `intercept.json` entries stay identifiable.
pub(crate) fn intercept_action_label(action: &str) -> String {
    match action {
        "show_main_window" => "Главное окно",
        "show_soundpanel_window" => "Звуковая панель",
        "show_playback_control_window" => "Управление воспроизведением",
        "playback_pause" => "Пауза / Продолжить",
        "playback_stop" => "Остановить",
        "playback_repeat" => "Повторить",
        "ocr_capture" => "OCR: захват области",
        other => other,
    }
    .to_string()
}

/// Russian nominative label for a global hotkey settings name, used in
/// conflict messages raised from the intercept side. Mirrors the UI catalog
/// (`hotkeys.name.*` in `locales/ru.json`).
pub(crate) fn global_binding_label(name: &str) -> &'static str {
    match name {
        "main_window" => "Основное окно",
        "sound_panel" => "Звуковая панель",
        "playback_pause" => "Пауза воспроизведения",
        "playback_stop" => "Остановка воспроизведения",
        "playback_repeat" => "Повтор воспроизведения",
        "playback_control_window" => "Управление воспроизведением",
        "return_previous_window" => "Возврат в предыдущее окно",
        "toggle_minimal_mode" => "Компактный режим",
        "ocr_capture" => "OCR: захват области",
        _ => "другой глобальный хоткей",
    }
}

/// Find a stored intercept binding that occupies the same NumPad code as the
/// global `hotkey` candidate, naming the conflicting binding.
///
/// ROADMAP-140 conflict rule (global-editor side): a saved intercept binding
/// reserves its NumPad code against a global assignment **regardless of the
/// global candidate's modifiers, of its `enabled` switch and of the
/// intercept mode switch** — an OS-registered global shortcut swallows the
/// key before the low-level hook can see it, and the verdict must not depend
/// on the editing order. Callers therefore must run this check *before*
/// their enabled guard, so saving a disabled global candidate over a saved
/// intercept binding is rejected too. Only NumPad codes are checked: the
/// F-key and navigation overlaps predate ROADMAP-140 and stay out of its
/// scope. Keys that do not resolve to a NumPad code (the top digit row,
/// F-keys, hand-edited `VK_XX` names, an empty key) never conflict here.
pub(crate) fn find_numpad_binding_conflict<'a>(
    settings: &'a InterceptSettings,
    hotkey: &Hotkey,
) -> Option<&'a InterceptBinding> {
    let vk = numpad_vk_code(&hotkey.key)?;
    settings
        .bindings
        .iter()
        .find(|binding| numpad_vk_code(&binding.key) == Some(vk))
}

/// Find the global hotkey settings name that occupies the same NumPad code
/// as the intercept key candidate, returning the conflicting settings name.
///
/// ROADMAP-140 conflict rule (intercept-editor side): every stored global
/// binding — including `ocr_capture`, **including a switched-off one** —
/// reserves its NumPad code against a new intercept binding, regardless of
/// the stored modifiers. Stored keys that do not resolve to a NumPad code
/// (the top digit row, F-keys, editor-only names) never conflict here.
pub(crate) fn find_numpad_global_conflict(
    hotkeys: &HotkeySettings,
    key: &str,
) -> Option<&'static str> {
    let vk = numpad_vk_code(key)?;
    HotkeySettings::GLOBAL_HOTKEY_NAMES
        .iter()
        .copied()
        .find(|&name| {
            hotkeys
                .get_global_by_name(name)
                .and_then(|binding| numpad_vk_code(&binding.key))
                == Some(vk)
        })
}

/// Find the editor action id that occupies the same NumPad code as the
/// intercept key candidate, returning the conflicting action id.
///
/// ROADMAP-140 conflict rule (intercept-editor side): every stored editor
/// binding — **including a switched-off one** — reserves its NumPad code
/// against a new intercept binding, regardless of the stored modifiers.
/// This mirrors the global rule above and keeps the verdict independent of
/// the editing order: a conflicting combination stays blocked no matter
/// which side saved first. Stored keys that do not resolve to a NumPad code
/// (the top digit row, F-keys, navigation keys, an empty key) never
/// conflict here.
pub(crate) fn find_numpad_editor_conflict(
    hotkeys: &HotkeySettings,
    key: &str,
) -> Option<&'static str> {
    let vk = numpad_vk_code(key)?;
    hotkeys
        .editor
        .iter()
        .find(|(_, binding)| numpad_vk_code(&binding.key) == Some(vk))
        .map(|(action_id, _)| action_id)
}

/// User-facing rejection message for a global candidate that lands on a
/// saved intercept binding: names the key, the section and the action.
pub(crate) fn numpad_intercept_conflict_message(
    hotkey: &Hotkey,
    binding: &InterceptBinding,
) -> String {
    format!(
        "Клавиша {} уже назначена в разделе «Перехват клавиш» (действие: {})",
        hotkey.format_display(),
        intercept_action_label(&binding.action),
    )
}

/// User-facing rejection message for an intercept candidate that lands on a
/// saved global binding: names the key, the section and the action.
pub(crate) fn numpad_global_conflict_message(key: &str, name: &str) -> String {
    format!(
        "Клавиша {} уже назначена в разделе «Глобальные» (действие: {})",
        key,
        global_binding_label(name),
    )
}

/// User-facing rejection message for an intercept candidate that lands on a
/// saved editor binding: names the key, the section and the action.
pub(crate) fn numpad_editor_conflict_message(key: &str, action_id: &str) -> String {
    format!(
        "Клавиша {} уже назначена в разделе «Редактор» (действие: {})",
        key,
        EditorHotkeySettings::action_label(action_id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;

    fn tempdir() -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "ttsbard_intercept_test_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ========= NumPad check-then-write coordination (ROADMAP-140 race) =========

    use std::time::Duration;

    /// The shared coordination lock is mutually exclusive: a second
    /// acquisition cannot complete while the first guard is held, and
    /// proceeds as soon as it is released. This is the primitive the
    /// settings.json and intercept.json save paths rely on to make
    /// incompatible NumPad saves mutually exclusive.
    #[test]
    fn numpad_coordination_lock_is_mutually_exclusive() {
        let guard = lock_numpad_coordination();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let competitor = std::thread::spawn(move || {
            let _second = lock_numpad_coordination();
            let _ = entered_tx.send(());
            let _ = release_rx.recv();
        });

        assert!(
            entered_rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "the second acquisition must block while the lock is held"
        );

        drop(guard);
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the second acquisition proceeds after the release");
        release_tx.send(()).expect("the release is sent");
        competitor.join().expect("the competitor thread finishes");
    }

    /// A panicking save path must not wedge every later save: the wrapper
    /// recovers from the poisoned lock instead of propagating the poison.
    #[test]
    fn numpad_coordination_recovers_from_a_poisoned_lock() {
        let poisoned = std::panic::catch_unwind(|| {
            let _guard = lock_numpad_coordination();
            panic!("simulated save-path panic while holding the coordination lock");
        });
        assert!(poisoned.is_err(), "the simulated panic is contained");

        let _guard = lock_numpad_coordination();
        drop(_guard);
    }

    // ── vk_to_name ──────────────────────────────────────────────────────

    #[test]
    fn vk_to_name_numpad_digits() {
        for i in 0u32..=9 {
            assert_eq!(vk_to_name(0x60 + i), Some(format!("NUMPAD{}", i)));
        }
    }

    #[test]
    fn vk_to_name_numpad_operators() {
        assert_eq!(vk_to_name(0x6A), Some("NUMPAD_MULTIPLY".into()));
        assert_eq!(vk_to_name(0x6B), Some("NUMPAD_ADD".into()));
        assert_eq!(vk_to_name(0x6D), Some("NUMPAD_SUBTRACT".into()));
        assert_eq!(vk_to_name(0x6E), Some("NUMPAD_DECIMAL".into()));
        assert_eq!(vk_to_name(0x6F), Some("NUMPAD_DIVIDE".into()));
    }

    #[test]
    fn vk_to_name_f_keys() {
        for i in 1u32..=24 {
            assert_eq!(vk_to_name(0x6F + i), Some(format!("F{}", i)));
        }
    }

    #[test]
    fn vk_to_name_outside_range() {
        // Just below NUMPAD0
        assert_eq!(vk_to_name(0x5F), None);
        // Just above F24
        assert_eq!(vk_to_name(0x88), None);
        // Unrelated keys
        assert_eq!(vk_to_name(0x41), None); // A
        assert_eq!(vk_to_name(0x0D), None); // Enter
        assert_eq!(vk_to_name(0x20), None); // Space
        assert_eq!(vk_to_name(0x1B), None); // Escape
        assert_eq!(vk_to_name(0x00), None);
        assert_eq!(vk_to_name(0xFF), None);
        // Gap between NUMPAD_ADD and NUMPAD_SUBTRACT
        assert_eq!(vk_to_name(0x6C), None);
    }

    #[test]
    fn vk_to_name_maps_home_and_insert() {
        assert_eq!(vk_to_name(0x24), Some("HOME".into()));
        assert_eq!(vk_to_name(0x2D), Some("INSERT".into()));
    }

    // ── vk_to_name_for_mode ─────────────────────────────────────────────

    #[test]
    fn vk_to_name_for_mode_prefers_canonical_names() {
        // Canonical names win in both restricted and unrestricted mode.
        assert_eq!(vk_to_name_for_mode(0x24, false), Some("HOME".into()));
        assert_eq!(vk_to_name_for_mode(0x24, true), Some("HOME".into()));
        assert_eq!(vk_to_name_for_mode(0x2D, false), Some("INSERT".into()));
        assert_eq!(vk_to_name_for_mode(0x2D, true), Some("INSERT".into()));
        assert_eq!(vk_to_name_for_mode(0x61, true), Some("NUMPAD1".into()));
        assert_eq!(vk_to_name_for_mode(0x70, true), Some("F1".into()));
    }

    #[test]
    fn vk_to_name_for_mode_restricted_returns_none_for_other_keys() {
        assert_eq!(vk_to_name_for_mode(0x41, false), None); // A
        assert_eq!(vk_to_name_for_mode(0x2E, false), None); // Delete
        assert_eq!(vk_to_name_for_mode(0x20, false), None); // Space
        assert_eq!(vk_to_name_for_mode(0xA0, false), None); // LShift
        assert_eq!(vk_to_name_for_mode(0xBB, false), None); // OEM +
        assert_eq!(vk_to_name_for_mode(0xB0, false), None); // media Next
    }

    #[test]
    fn vk_to_name_for_mode_unrestricted_resolves_other_keys_as_vk_hex() {
        assert_eq!(vk_to_name_for_mode(0x41, true), Some("VK_41".into())); // A
        assert_eq!(vk_to_name_for_mode(0x2E, true), Some("VK_2E".into())); // Delete
        assert_eq!(vk_to_name_for_mode(0x20, true), Some("VK_20".into())); // Space
        assert_eq!(vk_to_name_for_mode(0xA0, true), Some("VK_A0".into())); // LShift
        assert_eq!(vk_to_name_for_mode(0xA1, true), Some("VK_A1".into())); // RShift
        assert_eq!(vk_to_name_for_mode(0xBB, true), Some("VK_BB".into())); // OEM +
        assert_eq!(vk_to_name_for_mode(0xB0, true), Some("VK_B0".into())); // media Next
    }

    #[test]
    fn vk_to_name_for_mode_does_not_collapse_sided_modifiers() {
        assert_eq!(vk_to_name_for_mode(0xA0, true), Some("VK_A0".into()));
        assert_eq!(vk_to_name_for_mode(0xA1, true), Some("VK_A1".into()));
        assert_eq!(vk_to_name_for_mode(0xA2, true), Some("VK_A2".into()));
        assert_eq!(vk_to_name_for_mode(0xA3, true), Some("VK_A3".into()));
        assert_eq!(vk_to_name_for_mode(0xA4, true), Some("VK_A4".into()));
        assert_eq!(vk_to_name_for_mode(0xA5, true), Some("VK_A5".into()));
    }

    #[test]
    fn vk_to_name_for_mode_rejects_out_of_range_and_boundaries() {
        assert_eq!(vk_to_name_for_mode(0x00, true), None);
        assert_eq!(vk_to_name_for_mode(0x07, true), None); // below 0x08
        assert_eq!(vk_to_name_for_mode(0xFF, true), None); // 255
        assert_eq!(vk_to_name_for_mode(0x100, true), None); // out of range
        assert_eq!(vk_to_name_for_mode(0x08, true), Some("VK_08".into())); // backspace
        assert_eq!(vk_to_name_for_mode(0xFE, true), Some("VK_FE".into()));
    }

    // ── load / save ─────────────────────────────────────────────────────

    #[test]
    fn save_creates_and_replaces_pretty_json() {
        let dir = tempdir();
        let path = dir.join(INTERCEPT_FILE);
        let mut settings = InterceptSettings::default();
        for enabled in [false, true] {
            settings.enabled = enabled;
            save(dir.to_str().unwrap(), &settings).unwrap();
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                serde_json::to_string_pretty(&settings).unwrap()
            );
            assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn save_failure_preserves_previous_file() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempdir();
        let path = dir.join(INTERCEPT_FILE);
        let previous = b"{\"enabled\":false,\"bindings\":[]}";
        fs::write(&path, previous).unwrap();
        // Allow reads and writes, but deny deletion/replacement of the destination.
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0x00000001 | 0x00000002)
            .open(&path)
            .unwrap();
        let settings = InterceptSettings {
            enabled: true,
            ..InterceptSettings::default()
        };

        let error = save(dir.to_str().unwrap(), &settings).unwrap_err();
        assert!(error.starts_with("Failed to write intercept.json: "));
        assert_eq!(fs::read(&path).unwrap(), previous);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

        drop(handle);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let dir = tempdir();
        let settings = load(dir.to_str().unwrap());
        assert!(!settings.enabled);
        assert!(settings.bindings.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempdir();
        let original = InterceptSettings {
            enabled: true,
            bindings: vec![
                InterceptBinding {
                    key: "NUMPAD1".into(),
                    action: "play_sound".into(),
                },
                InterceptBinding {
                    key: "F5".into(),
                    action: "mute_mic".into(),
                },
                InterceptBinding {
                    key: "PAGEUP".into(),
                    action: "playback_pause".into(),
                },
                InterceptBinding {
                    key: "END".into(),
                    action: "playback_stop".into(),
                },
                InterceptBinding {
                    key: "PAGEDOWN".into(),
                    action: "playback_repeat".into(),
                },
            ],
            allow_any_key: true,
        };
        save(dir.to_str().unwrap(), &original).unwrap();
        let loaded = load(dir.to_str().unwrap());
        assert!(loaded.enabled);
        assert!(loaded.allow_any_key);
        assert_eq!(loaded.bindings.len(), 5);
        assert_eq!(loaded.bindings[0].key, "NUMPAD1");
        assert_eq!(loaded.bindings[0].action, "play_sound");
        assert_eq!(loaded.bindings[1].key, "F5");
        assert_eq!(loaded.bindings[1].action, "mute_mic");
        assert_eq!(loaded.bindings[2].key, "PAGEUP");
        assert_eq!(loaded.bindings[2].action, "playback_pause");
        assert_eq!(loaded.bindings[3].key, "END");
        assert_eq!(loaded.bindings[3].action, "playback_stop");
        assert_eq!(loaded.bindings[4].key, "PAGEDOWN");
        assert_eq!(loaded.bindings[4].action, "playback_repeat");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_json_returns_defaults() {
        let dir = tempdir();
        let file_path = dir.join(INTERCEPT_FILE);
        fs::write(&file_path, b"not valid json {{{").unwrap();
        let settings = load(dir.to_str().unwrap());
        assert!(!settings.enabled);
        assert!(settings.bindings.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_bindings_roundtrip() {
        let dir = tempdir();
        let original = InterceptSettings {
            enabled: false,
            bindings: vec![],
            allow_any_key: false,
        };
        save(dir.to_str().unwrap(), &original).unwrap();
        let loaded = load(dir.to_str().unwrap());
        assert!(!loaded.enabled);
        assert!(loaded.bindings.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_json_without_allow_any_key_loads_with_false_and_bindings_intact() {
        let dir = tempdir();
        let file_path = dir.join(INTERCEPT_FILE);
        fs::write(
            &file_path,
            r#"{"enabled":true,"bindings":[{"key":"NUMPAD1","action":"play_sound"},{"key":"HOME","action":"playback_stop"}]}"#,
        )
        .unwrap();
        let settings = load(dir.to_str().unwrap());
        assert!(settings.enabled);
        assert!(!settings.allow_any_key);
        assert_eq!(settings.bindings.len(), 2);
        assert_eq!(settings.bindings[0].key, "NUMPAD1");
        assert_eq!(settings.bindings[0].action, "play_sound");
        assert_eq!(settings.bindings[1].key, "HOME");
        assert_eq!(settings.bindings[1].action, "playback_stop");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn allow_any_key_true_roundtrips() {
        let dir = tempdir();
        let original = InterceptSettings {
            enabled: true,
            bindings: vec![],
            allow_any_key: true,
        };
        save(dir.to_str().unwrap(), &original).unwrap();
        let loaded = load(dir.to_str().unwrap());
        assert!(loaded.allow_any_key);
        let _ = fs::remove_dir_all(&dir);
    }

    // ── NumPad ↔ global hotkey conflicts (ROADMAP-140) ─────────────────

    use crate::config::HotkeyModifier;

    fn global_hotkey(modifiers: Vec<HotkeyModifier>, key: &str) -> Hotkey {
        Hotkey {
            modifiers,
            key: key.to_string(),
            enabled: true,
        }
    }

    fn intercept_with_binding(key: &str, action: &str) -> InterceptSettings {
        InterceptSettings {
            enabled: true,
            bindings: vec![InterceptBinding {
                key: key.to_string(),
                action: action.to_string(),
            }],
            allow_any_key: false,
        }
    }

    /// Shared parity fixture for the supported NumPad keys
    /// (scripts/contract-fixtures/numpad/numpad-keys.json): the same table
    /// the frontend (`interceptKeys.test.ts`) and the `config::hotkeys`
    /// shortcut parsing assert on. The fixture's browser-side `domCode`
    /// column is not parsed here: the Rust side matches on Windows VKs and
    /// canonical names only, and serde skips the extra JSON field.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct NumpadFixtureRow {
        canonical_name: String,
        windows_vk: u32,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct NumpadFixture {
        keys: Vec<NumpadFixtureRow>,
        excluded: Vec<NumpadFixtureRow>,
        digit_row: NumpadFixtureRow,
    }

    fn numpad_fixture() -> NumpadFixture {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../scripts/contract-fixtures/numpad/numpad-keys.json"
        );
        let raw = std::fs::read_to_string(path)
            .expect("numpad parity fixture must exist next to the crate");
        serde_json::from_str(&raw).expect("numpad parity fixture must parse")
    }

    /// `numpad_vk_code` is the exact inverse of `vk_to_name` on the supported
    /// NumPad set, driven by the shared parity fixture: every fixture row's
    /// canonical name resolves to exactly the row's Windows VK (the VKs the
    /// `global-hotkey` dependency derives from `Code::Numpad*`), and the
    /// reverse mapping returns the name. Excluded rows — NumPad Enter with
    /// the shared Enter VK 0x0D — resolve in neither direction, and the top
    /// digit row (VK 0x35) is never a NumPad code.
    #[test]
    fn numpad_vk_code_is_inverse_of_vk_to_name() {
        let fixture = numpad_fixture();
        for row in &fixture.keys {
            assert_eq!(
                vk_to_name(row.windows_vk),
                Some(row.canonical_name.clone()),
                "VK 0x{:02X} must map to the fixture's canonical name",
                row.windows_vk
            );
            assert_eq!(
                numpad_vk_code(&row.canonical_name),
                Some(row.windows_vk),
                "{} must map to the fixture's Windows VK",
                row.canonical_name
            );
        }
        for row in &fixture.excluded {
            assert_eq!(
                numpad_vk_code(&row.canonical_name),
                None,
                "{} stays outside the NumPad codes",
                row.canonical_name
            );
            assert_eq!(
                vk_to_name(row.windows_vk),
                None,
                "the shared Enter VK stays outside the restricted names"
            );
        }
        // The top digit row and other non-NumPad names never resolve.
        assert_eq!(numpad_vk_code(&fixture.digit_row.canonical_name), None);
        assert_eq!(vk_to_name(fixture.digit_row.windows_vk), None);
        assert_eq!(numpad_vk_code("F5"), None);
        assert_eq!(numpad_vk_code("HOME"), None);
        assert_eq!(numpad_vk_code("VK_60"), None);
        assert_eq!(numpad_vk_code(""), None);
    }

    /// A global candidate on the same NumPad code conflicts with a saved
    /// intercept binding regardless of the global modifiers.
    #[test]
    fn binding_conflict_matches_same_code_regardless_of_modifiers() {
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");
        for modifiers in [
            vec![],
            vec![HotkeyModifier::Ctrl],
            vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            vec![HotkeyModifier::Super],
        ] {
            let candidate = global_hotkey(modifiers, "NUMPAD1");
            let binding = find_numpad_binding_conflict(&intercept, &candidate)
                .expect("same NumPad code must conflict");
            assert_eq!(binding.key, "NUMPAD1");
            assert_eq!(binding.action, "show_main_window");
        }
    }

    /// The saved intercept binding reserves its code even while the intercept
    /// mode is switched off.
    #[test]
    fn binding_conflict_ignores_intercept_mode_switch() {
        let mut intercept = intercept_with_binding("NUMPAD5", "playback_stop");
        intercept.enabled = false;
        let candidate = global_hotkey(vec![HotkeyModifier::Ctrl], "NUMPAD5");
        assert!(find_numpad_binding_conflict(&intercept, &candidate).is_some());
    }

    /// Unrelated assignments never conflict: other NumPad keys, the top
    /// digit row, F-keys and keys that do not resolve to a NumPad code.
    #[test]
    fn binding_conflict_ignores_unrelated_assignments() {
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");
        // A different NumPad key is free.
        assert!(
            find_numpad_binding_conflict(&intercept, &global_hotkey(vec![], "NUMPAD2")).is_none()
        );
        // The top digit row stays a separate code, exactly as it was before
        // ROADMAP-140.
        assert!(find_numpad_binding_conflict(&intercept, &global_hotkey(vec![], "1")).is_none());
        // F-key overlaps predate ROADMAP-140 and stay out of its scope.
        let f_keys = intercept_with_binding("F5", "playback_stop");
        assert!(find_numpad_binding_conflict(&f_keys, &global_hotkey(vec![], "F5")).is_none());
        // Hand-edited non-canonical binding keys do not resolve and are
        // skipped instead of blocking unrelated saves.
        let raw_vk = intercept_with_binding("VK_60", "show_main_window");
        assert!(find_numpad_binding_conflict(&raw_vk, &global_hotkey(vec![], "NUMPAD0")).is_none());
    }

    /// The intercept-side rule: a new binding on a stored global NumPad code
    /// conflicts regardless of the stored modifiers — and regardless of the
    /// stored global's switch, so a disabled assignment still reserves its
    /// code until the user fixes it.
    #[test]
    fn global_conflict_matches_same_code_regardless_of_modifiers_and_switch() {
        for stored_enabled in [true, false] {
            let mut hotkeys = HotkeySettings::default();
            let mut stored =
                global_hotkey(vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift], "NUMPAD7");
            stored.enabled = stored_enabled;
            hotkeys.sound_panel = stored;

            for candidate_modifiers in [vec![], vec![HotkeyModifier::Alt]] {
                let candidate = global_hotkey(candidate_modifiers, "NUMPAD7");
                assert_eq!(
                    find_numpad_global_conflict(&hotkeys, &candidate.key),
                    Some("sound_panel"),
                    "stored enabled={stored_enabled} must reserve its code"
                );
            }
        }
    }

    /// Unrelated global assignments never block the intercept editor: the
    /// top digit row, non-NumPad candidates and free codes.
    #[test]
    fn global_conflict_ignores_unrelated_assignments() {
        let hotkeys = HotkeySettings {
            sound_panel: global_hotkey(vec![], "NUMPAD1"),
            ..Default::default()
        };
        // Top digit row is a different code.
        assert_eq!(find_numpad_global_conflict(&hotkeys, "NUMPAD2"), None);
        // Non-NumPad candidates (even a stored matching F-key) stay out of
        // the ROADMAP-140 scope.
        let f_keys = HotkeySettings {
            playback_stop: global_hotkey(vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift], "F5"),
            ..Default::default()
        };
        assert_eq!(find_numpad_global_conflict(&f_keys, "F5"), None);
        assert_eq!(find_numpad_global_conflict(&f_keys, "NUMPAD1"), None);
    }

    /// Every global section participates, including the OCR assignment.
    #[test]
    fn global_conflict_covers_all_global_names_including_ocr() {
        let hotkeys = HotkeySettings {
            ocr_capture: global_hotkey(vec![HotkeyModifier::Ctrl], "NUMPAD_DECIMAL"),
            ..Default::default()
        };
        assert_eq!(
            find_numpad_global_conflict(&hotkeys, "NUMPAD_DECIMAL"),
            Some("ocr_capture")
        );

        let hotkeys = HotkeySettings {
            return_previous_window: global_hotkey(vec![], "NUMPAD_ADD"),
            ..Default::default()
        };
        assert_eq!(
            find_numpad_global_conflict(&hotkeys, "NUMPAD_ADD"),
            Some("return_previous_window")
        );
    }

    /// Rejection messages name the key, the section and the action.
    #[test]
    fn conflict_messages_name_key_section_and_action() {
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");
        let candidate = global_hotkey(vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift], "NUMPAD1");
        let binding = find_numpad_binding_conflict(&intercept, &candidate).unwrap();
        let message = numpad_intercept_conflict_message(&candidate, binding);
        assert!(
            message.contains("Ctrl+Shift+NUMPAD1"),
            "unexpected: {message}"
        );
        assert!(message.contains("Перехват клавиш"), "unexpected: {message}");
        assert!(message.contains("Главное окно"), "unexpected: {message}");

        let hotkeys = HotkeySettings {
            sound_panel: global_hotkey(vec![HotkeyModifier::Alt], "NUMPAD2"),
            ..Default::default()
        };
        let name = find_numpad_global_conflict(&hotkeys, "NUMPAD2").unwrap();
        let message = numpad_global_conflict_message("NUMPAD2", name);
        assert!(message.contains("NUMPAD2"), "unexpected: {message}");
        assert!(message.contains("Глобальные"), "unexpected: {message}");
        assert!(message.contains("Звуковая панель"), "unexpected: {message}");
    }

    /// Editing order must not matter (review fix): a stored disabled global
    /// NumPad still blocks an intercept binding on the same code, and a
    /// stored intercept binding blocks even a disabled global candidate —
    /// the global validator runs `find_numpad_binding_conflict` before its
    /// enabled guard.
    #[test]
    fn numpad_conflict_is_rejected_in_both_editing_orders() {
        let mut hotkeys = HotkeySettings::default();
        let mut stored = global_hotkey(vec![HotkeyModifier::Ctrl], "NUMPAD1");
        stored.enabled = false;
        hotkeys.sound_panel = stored;

        // Order A: global stored (switched off) first, intercept binding
        // saved second.
        assert_eq!(
            find_numpad_global_conflict(&hotkeys, "NUMPAD1"),
            Some("sound_panel")
        );

        // Order B: intercept binding exists, disabled global candidate saved
        // second.
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");
        let mut candidate = global_hotkey(vec![], "NUMPAD1");
        candidate.enabled = false;
        assert!(find_numpad_binding_conflict(&intercept, &candidate).is_some());
    }

    /// The intercept-side rule against editor actions: a new binding on a
    /// stored editor NumPad code conflicts regardless of the stored
    /// modifiers — and regardless of the stored editor's switch, so a
    /// disabled editor assignment still reserves its code until the user
    /// fixes it.
    #[test]
    fn editor_conflict_matches_same_code_regardless_of_modifiers_and_switch() {
        for stored_enabled in [true, false] {
            let mut hotkeys = HotkeySettings::default();
            let mut stored =
                global_hotkey(vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift], "NUMPAD7");
            stored.enabled = stored_enabled;
            hotkeys.editor.edit_word = stored;

            assert_eq!(
                find_numpad_editor_conflict(&hotkeys, "NUMPAD7"),
                Some("edit_word"),
                "stored enabled={stored_enabled} must reserve its code"
            );
        }
    }

    /// Unrelated editor assignments never block the intercept editor: other
    /// NumPad codes, the top digit row, F-keys and a cleared binding.
    #[test]
    fn editor_conflict_ignores_unrelated_assignments() {
        let mut hotkeys = HotkeySettings::default();
        hotkeys.editor.edit_word = global_hotkey(vec![], "NUMPAD1");
        // A different NumPad key is free.
        assert_eq!(find_numpad_editor_conflict(&hotkeys, "NUMPAD2"), None);
        // The top digit row stays a separate code on both sides.
        let mut digit_row = HotkeySettings::default();
        digit_row.editor.next_tab = global_hotkey(vec![], "1");
        assert_eq!(find_numpad_editor_conflict(&digit_row, "1"), None);
        assert_eq!(find_numpad_editor_conflict(&digit_row, "NUMPAD1"), None);
        // Non-NumPad candidates stay out of the ROADMAP-140 scope.
        let mut f_keys = HotkeySettings::default();
        f_keys.editor.accent_homographs =
            global_hotkey(vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift], "F5");
        assert_eq!(find_numpad_editor_conflict(&f_keys, "F5"), None);
        // A cleared binding has no code to intersect.
        let mut cleared = HotkeySettings::default();
        cleared.editor.toggle_typing = global_hotkey(vec![], "");
        assert_eq!(find_numpad_editor_conflict(&cleared, "NUMPAD5"), None);
    }

    /// Editing order must not matter between the editor and the intercept
    /// sections: a stored (even disabled) editor NumPad binding blocks an
    /// intercept binding on the same code, and a stored intercept binding
    /// blocks even a disabled editor candidate.
    #[test]
    fn editor_intercept_conflict_is_rejected_in_both_editing_orders() {
        // Order A: editor binding stored (switched off) first, intercept
        // binding saved second.
        let mut hotkeys = HotkeySettings::default();
        let mut stored = global_hotkey(vec![HotkeyModifier::Ctrl], "NUMPAD1");
        stored.enabled = false;
        hotkeys.editor.edit_word = stored;
        assert_eq!(
            find_numpad_editor_conflict(&hotkeys, "NUMPAD1"),
            Some("edit_word")
        );

        // Order B: intercept binding exists, disabled editor candidate saved
        // second.
        let intercept = intercept_with_binding("NUMPAD1", "show_main_window");
        let mut candidate = global_hotkey(vec![], "NUMPAD1");
        candidate.enabled = false;
        assert!(find_numpad_binding_conflict(&intercept, &candidate).is_some());
    }

    /// The intercept→editor rejection message names the key, the section
    /// and the editor action.
    #[test]
    fn numpad_editor_conflict_message_names_key_section_and_action() {
        let message = numpad_editor_conflict_message("NUMPAD1", "edit_word");
        assert!(message.contains("NUMPAD1"), "unexpected: {message}");
        assert!(message.contains("Редактор"), "unexpected: {message}");
        assert!(
            message.contains("редактирования слова"),
            "unexpected: {message}"
        );
    }

    /// Unknown intercept action ids are shown verbatim, known ones map to
    /// the UI catalog labels.
    #[test]
    fn intercept_action_labels_follow_ui_catalog() {
        assert_eq!(intercept_action_label("show_main_window"), "Главное окно");
        assert_eq!(intercept_action_label("ocr_capture"), "OCR: захват области");
        assert_eq!(intercept_action_label("play_sound"), "play_sound");
    }
}
