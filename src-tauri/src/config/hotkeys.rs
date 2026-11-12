//! Customizable hotkey configuration
//!
//! This module defines types for storing and managing customizable hotkeys.

use serde::{Deserialize, Serialize};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

/// Hotkey modifier keys
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyModifier {
    Ctrl,
    Shift,
    Alt,
    Super, // Win key
}

/// Backward-compatible default for [`Hotkey::enabled`]: old `settings.json`
/// files without the field keep every hotkey active.
fn default_true() -> bool {
    true
}

/// A single hotkey configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hotkey {
    pub modifiers: Vec<HotkeyModifier>,
    pub key: String,
    /// Per-hotkey switch. `false` keeps the assigned combination stored but
    /// prevents it from being registered or matched.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Editor-scoped hotkeys (never registered as global Windows shortcuts)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EditorHotkeySettings {
    pub edit_word: Hotkey,
    pub submit_continue: Hotkey,
    pub submit_keep_text: Hotkey,
    pub submit_keep_focus: Hotkey,
    pub next_spelling_error: Hotkey,
    pub previous_spelling_error: Hotkey,
    pub next_tab: Hotkey,
    pub previous_tab: Hotkey,
    pub cycle_route: Hotkey,
    pub toggle_typing: Hotkey,
    pub cycle_quick_mode: Hotkey,
    pub toggle_history: Hotkey,
    pub accent_homographs: Hotkey,
    pub approve_next_incoming: Hotkey,
    pub edit_next_incoming: Hotkey,
}

impl Default for EditorHotkeySettings {
    fn default() -> Self {
        Self {
            edit_word: Hotkey::default_edit_word(),
            submit_continue: Hotkey::default_submit_continue(),
            submit_keep_text: Hotkey::default_submit_keep_text(),
            submit_keep_focus: Hotkey::default_submit_keep_focus(),
            next_spelling_error: Hotkey::default_next_spelling_error(),
            previous_spelling_error: Hotkey::default_previous_spelling_error(),
            next_tab: Hotkey::default_next_tab(),
            previous_tab: Hotkey::default_previous_tab(),
            cycle_route: Hotkey::default_cycle_route(),
            toggle_typing: Hotkey::default_toggle_typing(),
            cycle_quick_mode: Hotkey::default_cycle_quick_mode(),
            toggle_history: Hotkey::default_toggle_history(),
            accent_homographs: Hotkey::default_accent_homographs(),
            approve_next_incoming: Hotkey::default_approve_next_incoming(),
            edit_next_incoming: Hotkey::default_edit_next_incoming(),
        }
    }
}

/// Closed list of valid editor action IDs
pub const EDITOR_ACTION_IDS: &[&str] = &[
    "edit_word",
    "submit_continue",
    "submit_keep_text",
    "submit_keep_focus",
    "next_spelling_error",
    "previous_spelling_error",
    "next_tab",
    "previous_tab",
    "cycle_route",
    "toggle_typing",
    "cycle_quick_mode",
    "toggle_history",
    "accent_homographs",
    "approve_next_incoming",
    "edit_next_incoming",
];

impl EditorHotkeySettings {
    pub fn is_valid_action_id(id: &str) -> bool {
        EDITOR_ACTION_IDS.contains(&id)
    }

    /// Human-readable Russian action label used in conflict messages.
    pub fn action_label(action_id: &str) -> &'static str {
        match action_id {
            "edit_word" => "редактирования слова",
            "submit_continue" => "отправки/продолжения",
            "submit_keep_text" => "отправки с сохранением текста",
            "submit_keep_focus" => "отправки с сохранением текста и без смены фокуса",
            "next_spelling_error" => "следующей ошибки",
            "previous_spelling_error" => "предыдущей ошибки",
            "next_tab" => "следующей вкладки",
            "previous_tab" => "предыдущей вкладки",
            "cycle_route" => "переключения маршрута отправки",
            "toggle_typing" => "переключения передачи состояния набора",
            "cycle_quick_mode" => "переключения режима быстрого редактора",
            "toggle_history" => "показа/скрытия истории",
            "accent_homographs" => "расстановки ударений",
            "approve_next_incoming" => "подтверждения отправки входящего",
            "edit_next_incoming" => "редактирования входящего",
            _ => "действия редактора",
        }
    }

    pub fn get_by_id(&self, id: &str) -> Option<&Hotkey> {
        match id {
            "edit_word" => Some(&self.edit_word),
            "submit_continue" => Some(&self.submit_continue),
            "submit_keep_text" => Some(&self.submit_keep_text),
            "submit_keep_focus" => Some(&self.submit_keep_focus),
            "next_spelling_error" => Some(&self.next_spelling_error),
            "previous_spelling_error" => Some(&self.previous_spelling_error),
            "next_tab" => Some(&self.next_tab),
            "previous_tab" => Some(&self.previous_tab),
            "cycle_route" => Some(&self.cycle_route),
            "toggle_typing" => Some(&self.toggle_typing),
            "cycle_quick_mode" => Some(&self.cycle_quick_mode),
            "toggle_history" => Some(&self.toggle_history),
            "accent_homographs" => Some(&self.accent_homographs),
            "approve_next_incoming" => Some(&self.approve_next_incoming),
            "edit_next_incoming" => Some(&self.edit_next_incoming),
            _ => None,
        }
    }

    pub fn get_mut_by_id(&mut self, id: &str) -> Option<&mut Hotkey> {
        match id {
            "edit_word" => Some(&mut self.edit_word),
            "submit_continue" => Some(&mut self.submit_continue),
            "submit_keep_text" => Some(&mut self.submit_keep_text),
            "submit_keep_focus" => Some(&mut self.submit_keep_focus),
            "next_spelling_error" => Some(&mut self.next_spelling_error),
            "previous_spelling_error" => Some(&mut self.previous_spelling_error),
            "next_tab" => Some(&mut self.next_tab),
            "previous_tab" => Some(&mut self.previous_tab),
            "cycle_route" => Some(&mut self.cycle_route),
            "toggle_typing" => Some(&mut self.toggle_typing),
            "cycle_quick_mode" => Some(&mut self.cycle_quick_mode),
            "toggle_history" => Some(&mut self.toggle_history),
            "accent_homographs" => Some(&mut self.accent_homographs),
            "approve_next_incoming" => Some(&mut self.approve_next_incoming),
            "edit_next_incoming" => Some(&mut self.edit_next_incoming),
            _ => None,
        }
    }

    /// Iterate over every editor action with its closed-list id.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &Hotkey)> {
        EDITOR_ACTION_IDS
            .iter()
            .filter_map(|&id| self.get_by_id(id).map(|binding| (id, binding)))
    }

    /// Find a duplicate binding among **enabled** editor actions, returning the
    /// conflicting action name.
    ///
    /// A disabled candidate never conflicts, and a disabled existing binding is
    /// not considered occupied: its combination is kept only for the moment the
    /// user turns it back on, when the enable transition re-checks conflicts.
    pub fn find_duplicate(&self, skip_id: &str, binding: &Hotkey) -> Option<&'static str> {
        if !binding.enabled || binding.key.is_empty() {
            return None;
        }
        for &action_id in EDITOR_ACTION_IDS {
            if action_id == skip_id {
                continue;
            }
            if let Some(existing) = self.get_by_id(action_id) {
                if existing.enabled && existing == binding {
                    return Some(action_id);
                }
            }
        }
        None
    }
}

/// All configurable hotkeys
///
/// `#[serde(default)]` на каждом поле позволяет читать старые `settings.json`,
/// в которых отсутствуют новые playback-поля: недостающие поля заполняются
/// из `Default`. Без этого добавление нового поля ломает загрузку на старых
/// конфигах (strict serde падает с "missing field").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HotkeySettings {
    pub main_window: Hotkey,
    pub sound_panel: Hotkey,
    pub playback_pause: Hotkey,
    pub playback_stop: Hotkey,
    pub playback_repeat: Hotkey,
    pub playback_control_window: Hotkey,
    pub return_previous_window: Hotkey,
    pub toggle_minimal_mode: Hotkey,
    pub ocr_capture: Hotkey,
    #[serde(default)]
    pub editor: EditorHotkeySettings,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            main_window: Hotkey::default_main_window(),
            sound_panel: Hotkey::default_sound_panel(),
            playback_pause: Hotkey::default_playback_pause(),
            playback_stop: Hotkey::default_playback_stop(),
            playback_repeat: Hotkey::default_playback_repeat(),
            playback_control_window: Hotkey::default_playback_control_window(),
            return_previous_window: Hotkey::default_return_previous_window(),
            toggle_minimal_mode: Hotkey::default_toggle_minimal_mode(),
            ocr_capture: Hotkey::default_ocr_capture(),
            editor: EditorHotkeySettings::default(),
        }
    }
}

impl HotkeySettings {
    /// Names accepted by `set_hotkey` / `reset_hotkey_to_default` and by the
    /// per-hotkey enable command.
    pub const GLOBAL_HOTKEY_NAMES: [&'static str; 9] = [
        "main_window",
        "sound_panel",
        "playback_pause",
        "playback_stop",
        "playback_repeat",
        "playback_control_window",
        "return_previous_window",
        "toggle_minimal_mode",
        "ocr_capture",
    ];

    /// Resolve a global/main-window-local hotkey by its settings name.
    pub fn get_global_by_name(&self, name: &str) -> Option<&Hotkey> {
        match name {
            "main_window" => Some(&self.main_window),
            "sound_panel" => Some(&self.sound_panel),
            "playback_pause" => Some(&self.playback_pause),
            "playback_stop" => Some(&self.playback_stop),
            "playback_repeat" => Some(&self.playback_repeat),
            "playback_control_window" => Some(&self.playback_control_window),
            "return_previous_window" => Some(&self.return_previous_window),
            "toggle_minimal_mode" => Some(&self.toggle_minimal_mode),
            "ocr_capture" => Some(&self.ocr_capture),
            _ => None,
        }
    }

    /// Mutable counterpart of [`Self::get_global_by_name`].
    pub fn get_global_by_name_mut(&mut self, name: &str) -> Option<&mut Hotkey> {
        match name {
            "main_window" => Some(&mut self.main_window),
            "sound_panel" => Some(&mut self.sound_panel),
            "playback_pause" => Some(&mut self.playback_pause),
            "playback_stop" => Some(&mut self.playback_stop),
            "playback_repeat" => Some(&mut self.playback_repeat),
            "playback_control_window" => Some(&mut self.playback_control_window),
            "return_previous_window" => Some(&mut self.return_previous_window),
            "toggle_minimal_mode" => Some(&mut self.toggle_minimal_mode),
            "ocr_capture" => Some(&mut self.ocr_capture),
            _ => None,
        }
    }

    /// Find another **enabled** global/main-window-local hotkey that already
    /// uses `candidate`'s combination.
    ///
    /// Disabled hotkeys are skipped on both sides: a disabled candidate is not
    /// occupied and a disabled stored binding is not occupied either.
    pub fn find_global_conflict(
        &self,
        skip_name: &str,
        candidate: &Hotkey,
    ) -> Option<&'static str> {
        if !candidate.enabled || candidate.key.is_empty() {
            return None;
        }
        for &name in &Self::GLOBAL_HOTKEY_NAMES {
            if name == skip_name {
                continue;
            }
            if let Some(binding) = self.get_global_by_name(name) {
                if binding.enabled && binding == candidate {
                    return Some(name);
                }
            }
        }
        None
    }

    /// First enabled editor action that already uses `candidate`'s combination.
    pub fn find_editor_conflict(&self, candidate: &Hotkey) -> Option<&'static str> {
        if !candidate.enabled || candidate.key.is_empty() {
            return None;
        }
        self.editor
            .iter()
            .find(|(_, binding)| binding.enabled && *binding == candidate)
            .map(|(id, _)| id)
    }
}

impl Hotkey {
    /// Create a hotkey with Ctrl+Shift+F3 (main window default)
    pub fn default_main_window() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F3".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+Shift+F2 (sound panel default)
    pub fn default_sound_panel() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F2".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+Shift+F4 (playback pause/resume default)
    pub fn default_playback_pause() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F4".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+Shift+F5 (playback stop default)
    pub fn default_playback_stop() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F5".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+Shift+F6 (playback repeat default)
    pub fn default_playback_repeat() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F6".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+F (return to previous window default)
    pub fn default_return_previous_window() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "F".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+Shift+F7 (playback control window default)
    pub fn default_playback_control_window() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F7".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+M (minimal/normal mode toggle default)
    pub fn default_toggle_minimal_mode() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "M".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+Shift+P (one-shot screen OCR capture default)
    pub fn default_ocr_capture() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "P".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+E (edit word default)
    pub fn default_edit_word() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "E".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+Enter (submit/continue default)
    pub fn default_submit_continue() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "Enter".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Alt+Enter (submit and keep text default)
    pub fn default_submit_keep_text() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Alt],
            key: "Enter".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+Alt+Enter (submit, keep text and focus default)
    pub fn default_submit_keep_focus() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Alt],
            key: "Enter".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with F7 (next spelling error default)
    pub fn default_next_spelling_error() -> Self {
        Self {
            modifiers: vec![],
            key: "F7".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Shift+F7 (previous spelling error default)
    pub fn default_previous_spelling_error() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Shift],
            key: "F7".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+Tab (next tab default)
    pub fn default_next_tab() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "Tab".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+Shift+Tab (previous tab default)
    pub fn default_previous_tab() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "Tab".to_string(),
            enabled: true,
        }
    }

    /// Create the default Ctrl+R route-cycle binding.
    ///
    /// The combination stays assigned for discovery and reset, but the action
    /// ships switched off: changing the send route is easy to trigger by
    /// accident, so it must be enabled deliberately.
    pub fn default_cycle_route() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "R".to_string(),
            enabled: false,
        }
    }

    /// Create the default Ctrl+T typing-status binding.
    ///
    /// The combination stays assigned for discovery and reset, but the action
    /// ships switched off: sharing typing state is opt-in.
    pub fn default_toggle_typing() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "T".to_string(),
            enabled: false,
        }
    }

    /// Create the default Ctrl+W quick-editor-mode binding.
    ///
    /// The combination stays assigned for discovery and reset, but the action
    /// ships switched off: changing the window/focus behaviour is opt-in.
    pub fn default_cycle_quick_mode() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "W".to_string(),
            enabled: false,
        }
    }

    /// Create a hotkey with Ctrl+H (toggle history default)
    pub fn default_toggle_history() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "H".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+U (accent homographs default)
    pub fn default_accent_homographs() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "U".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+K (approve next incoming default)
    pub fn default_approve_next_incoming() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "K".to_string(),
            enabled: true,
        }
    }

    /// Create a hotkey with Ctrl+I (edit next incoming default)
    pub fn default_edit_next_incoming() -> Self {
        Self {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "I".to_string(),
            enabled: true,
        }
    }

    /// Returns true if the key is empty (disabled binding)
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.key.is_empty()
    }

    /// Check if this enabled binding conflicts with any enabled global hotkey.
    ///
    /// Disabled bindings on either side are ignored: their stored combination
    /// is not reserved while the switch is off.
    pub fn conflicts_with_global(&self, global: &HotkeySettings) -> bool {
        if !self.enabled || self.key.is_empty() {
            return false;
        }
        global.find_global_conflict("", self).is_some()
    }

    /// Convert to tauri_plugin_global_shortcut::Shortcut
    pub fn to_shortcut(&self) -> Result<Shortcut, String> {
        let mut modifiers = Modifiers::empty();
        for m in &self.modifiers {
            modifiers |= match m {
                HotkeyModifier::Ctrl => Modifiers::CONTROL,
                HotkeyModifier::Shift => Modifiers::SHIFT,
                HotkeyModifier::Alt => Modifiers::ALT,
                HotkeyModifier::Super => Modifiers::SUPER,
            };
        }
        let code = parse_key_code(&self.key)?;
        Ok(Shortcut::new(
            if modifiers.is_empty() {
                None
            } else {
                Some(modifiers)
            },
            code,
        ))
    }

    /// Format hotkey for display (e.g., "Ctrl+Shift+F3")
    pub fn format_display(&self) -> String {
        let mods: Vec<&str> = self
            .modifiers
            .iter()
            .map(|m| match m {
                HotkeyModifier::Ctrl => "Ctrl",
                HotkeyModifier::Shift => "Shift",
                HotkeyModifier::Alt => "Alt",
                HotkeyModifier::Super => "Win",
            })
            .collect();
        if mods.is_empty() {
            self.key.clone()
        } else {
            format!("{}+{}", mods.join("+"), self.key)
        }
    }
}

/// Parse a key string into a tauri_plugin_global_shortcut::Code
fn parse_key_code(key: &str) -> Result<Code, String> {
    match key.to_uppercase().as_str() {
        // F1-F12
        "F1" => Ok(Code::F1),
        "F2" => Ok(Code::F2),
        "F3" => Ok(Code::F3),
        "F4" => Ok(Code::F4),
        "F5" => Ok(Code::F5),
        "F6" => Ok(Code::F6),
        "F7" => Ok(Code::F7),
        "F8" => Ok(Code::F8),
        "F9" => Ok(Code::F9),
        "F10" => Ok(Code::F10),
        "F11" => Ok(Code::F11),
        "F12" => Ok(Code::F12),
        // A-Z
        "A" => Ok(Code::KeyA),
        "B" => Ok(Code::KeyB),
        "C" => Ok(Code::KeyC),
        "D" => Ok(Code::KeyD),
        "E" => Ok(Code::KeyE),
        "F" => Ok(Code::KeyF),
        "G" => Ok(Code::KeyG),
        "H" => Ok(Code::KeyH),
        "I" => Ok(Code::KeyI),
        "J" => Ok(Code::KeyJ),
        "K" => Ok(Code::KeyK),
        "L" => Ok(Code::KeyL),
        "M" => Ok(Code::KeyM),
        "N" => Ok(Code::KeyN),
        "O" => Ok(Code::KeyO),
        "P" => Ok(Code::KeyP),
        "Q" => Ok(Code::KeyQ),
        "R" => Ok(Code::KeyR),
        "S" => Ok(Code::KeyS),
        "T" => Ok(Code::KeyT),
        "U" => Ok(Code::KeyU),
        "V" => Ok(Code::KeyV),
        "W" => Ok(Code::KeyW),
        "X" => Ok(Code::KeyX),
        "Y" => Ok(Code::KeyY),
        "Z" => Ok(Code::KeyZ),
        // 0-9
        "0" => Ok(Code::Digit0),
        "1" => Ok(Code::Digit1),
        "2" => Ok(Code::Digit2),
        "3" => Ok(Code::Digit3),
        "4" => Ok(Code::Digit4),
        "5" => Ok(Code::Digit5),
        "6" => Ok(Code::Digit6),
        "7" => Ok(Code::Digit7),
        "8" => Ok(Code::Digit8),
        "9" => Ok(Code::Digit9),
        // Space
        "SPACE" => Ok(Code::Space),
        // Enter / Tab
        "ENTER" => Ok(Code::Enter),
        "TAB" => Ok(Code::Tab),
        _ => Err(format!("Invalid key: {}", key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hotkey_to_shortcut() {
        let hotkey = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F3".to_string(),
            enabled: true,
        };
        let shortcut = hotkey.to_shortcut().unwrap();
        assert!(shortcut.to_string().contains("F3"));
    }

    #[test]
    fn test_parse_key_code() {
        assert!(matches!(parse_key_code("F2"), Ok(Code::F2)));
        assert!(matches!(parse_key_code("A"), Ok(Code::KeyA)));
        assert!(matches!(parse_key_code("0"), Ok(Code::Digit0)));
        assert!(matches!(parse_key_code("Space"), Ok(Code::Space)));
        assert!(matches!(parse_key_code("Enter"), Ok(Code::Enter)));
        assert!(matches!(parse_key_code("Tab"), Ok(Code::Tab)));
        assert!(parse_key_code("INVALID").is_err());
    }

    #[test]
    fn test_format_display() {
        let hotkey = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F3".to_string(),
            enabled: true,
        };
        assert_eq!(hotkey.format_display(), "Ctrl+Shift+F3");

        let hotkey_no_mods = Hotkey {
            modifiers: vec![],
            key: "A".to_string(),
            enabled: true,
        };
        assert_eq!(hotkey_no_mods.format_display(), "A");
    }

    #[test]
    fn test_default_hotkeys() {
        let main = Hotkey::default_main_window();
        assert_eq!(main.key, "F3");
        assert_eq!(main.modifiers.len(), 2);

        let sound = Hotkey::default_sound_panel();
        assert_eq!(sound.key, "F2");
        assert_eq!(sound.modifiers.len(), 2);
    }

    // ==================== Editor hotkey default tests ====================

    #[test]
    fn test_default_edit_word() {
        let hk = Hotkey::default_edit_word();
        assert_eq!(hk.key, "E");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hk.format_display(), "Ctrl+E");
    }

    #[test]
    fn test_default_submit_continue() {
        let hk = Hotkey::default_submit_continue();
        assert_eq!(hk.key, "Enter");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
    }

    #[test]
    fn test_default_submit_keep_text() {
        let hk = Hotkey::default_submit_keep_text();
        assert_eq!(hk.key, "Enter");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Alt);
        assert_eq!(hk.format_display(), "Alt+Enter");
    }

    #[test]
    fn test_default_submit_keep_focus() {
        let hk = Hotkey::default_submit_keep_focus();
        assert_eq!(hk.key, "Enter");
        assert_eq!(hk.modifiers.len(), 2);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hk.modifiers[1], HotkeyModifier::Alt);
        assert_eq!(hk.format_display(), "Ctrl+Alt+Enter");
    }

    #[test]
    fn test_default_next_spelling_error() {
        let hk = Hotkey::default_next_spelling_error();
        assert_eq!(hk.key, "F7");
        assert!(hk.modifiers.is_empty());
    }

    #[test]
    fn test_default_previous_spelling_error() {
        let hk = Hotkey::default_previous_spelling_error();
        assert_eq!(hk.key, "F7");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Shift);
    }

    #[test]
    fn test_default_next_tab() {
        let hk = Hotkey::default_next_tab();
        assert_eq!(hk.key, "Tab");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
    }

    #[test]
    fn test_default_previous_tab() {
        let hk = Hotkey::default_previous_tab();
        assert_eq!(hk.key, "Tab");
        assert_eq!(hk.modifiers.len(), 2);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hk.modifiers[1], HotkeyModifier::Shift);
    }

    #[test]
    fn test_default_cycle_route() {
        let hk = Hotkey::default_cycle_route();
        assert_eq!(hk.key, "R");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hk.format_display(), "Ctrl+R");
        assert!(!hk.enabled, "Ctrl+R must ship switched off");
    }

    #[test]
    fn test_default_toggle_typing() {
        let hk = Hotkey::default_toggle_typing();
        assert_eq!(hk.key, "T");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hk.format_display(), "Ctrl+T");
        assert!(!hk.enabled, "Ctrl+T must ship switched off");
    }

    #[test]
    fn test_default_cycle_quick_mode() {
        let hk = Hotkey::default_cycle_quick_mode();
        assert_eq!(hk.key, "W");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hk.format_display(), "Ctrl+W");
        assert!(!hk.enabled, "Ctrl+W must ship switched off");
    }

    #[test]
    fn test_default_toggle_history() {
        let hk = Hotkey::default_toggle_history();
        assert_eq!(hk.key, "H");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hk.format_display(), "Ctrl+H");
    }

    #[test]
    fn test_default_accent_homographs() {
        let hk = Hotkey::default_accent_homographs();
        assert_eq!(hk.key, "U");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hk.format_display(), "Ctrl+U");
    }

    #[test]
    fn test_default_approve_next_incoming() {
        let hk = Hotkey::default_approve_next_incoming();
        assert_eq!(hk.key, "K");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hk.format_display(), "Ctrl+K");
    }

    #[test]
    fn test_default_edit_next_incoming() {
        let hk = Hotkey::default_edit_next_incoming();
        assert_eq!(hk.key, "I");
        assert_eq!(hk.modifiers.len(), 1);
        assert_eq!(hk.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hk.format_display(), "Ctrl+I");
    }

    #[test]
    fn test_hotkey_is_empty() {
        let empty = Hotkey {
            modifiers: vec![],
            key: String::new(),
            enabled: true,
        };
        assert!(empty.is_empty());
        let not_empty = Hotkey {
            modifiers: vec![],
            key: "F7".to_string(),
            enabled: true,
        };
        assert!(!not_empty.is_empty());
    }

    // ==================== EditorHotkeySettings tests ====================

    #[test]
    fn editor_hotkey_defaults_are_set() {
        let s = EditorHotkeySettings::default();
        assert_eq!(s.edit_word.key, "E");
        assert_eq!(s.submit_continue.key, "Enter");
        assert_eq!(s.submit_keep_text.key, "Enter");
        assert_eq!(s.submit_keep_focus.key, "Enter");
        assert_eq!(s.next_spelling_error.key, "F7");
        assert_eq!(s.previous_spelling_error.key, "F7");
        assert_eq!(s.next_tab.key, "Tab");
        assert_eq!(s.previous_tab.key, "Tab");
        assert_eq!(s.cycle_route.key, "R");
        assert_eq!(s.toggle_typing.key, "T");
        assert_eq!(s.cycle_quick_mode.key, "W");
        assert_eq!(s.toggle_history.key, "H");
        assert_eq!(s.accent_homographs.key, "U");
        assert_eq!(s.approve_next_incoming.key, "K");
        assert_eq!(s.edit_next_incoming.key, "I");
        // These three defaults keep their combination but ship switched off.
        assert!(!s.cycle_route.enabled);
        assert!(!s.toggle_typing.enabled);
        assert!(!s.cycle_quick_mode.enabled);
        assert!(s.toggle_history.enabled);
        assert!(s.accent_homographs.enabled);
    }

    #[test]
    fn is_valid_action_id_accepts_all_editor_ids() {
        for &id in EDITOR_ACTION_IDS {
            assert!(
                EditorHotkeySettings::is_valid_action_id(id),
                "{} should be valid",
                id
            );
        }
        assert!(!EditorHotkeySettings::is_valid_action_id("bogus"));
        assert!(!EditorHotkeySettings::is_valid_action_id("main_window"));
    }

    #[test]
    fn get_by_id_returns_correct_field() {
        let s = EditorHotkeySettings::default();
        assert_eq!(s.get_by_id("edit_word").unwrap().key, "E");
        assert_eq!(s.get_by_id("submit_continue").unwrap().key, "Enter");
        assert_eq!(s.get_by_id("submit_keep_text").unwrap().key, "Enter");
        assert_eq!(s.get_by_id("submit_keep_focus").unwrap().key, "Enter");
        assert_eq!(s.get_by_id("cycle_route").unwrap().key, "R");
        assert_eq!(s.get_by_id("toggle_typing").unwrap().key, "T");
        assert_eq!(s.get_by_id("cycle_quick_mode").unwrap().key, "W");
        assert_eq!(s.get_by_id("toggle_history").unwrap().key, "H");
        assert_eq!(s.get_by_id("accent_homographs").unwrap().key, "U");
        assert!(s.get_by_id("bogus").is_none());
    }

    #[test]
    fn get_mut_by_id_modifies_field() {
        let mut s = EditorHotkeySettings::default();
        {
            let field = s.get_mut_by_id("edit_word").unwrap();
            field.key = "F9".to_string();
        }
        assert_eq!(s.edit_word.key, "F9");

        {
            let field = s.get_mut_by_id("accent_homographs").unwrap();
            field.key = "F10".to_string();
        }
        assert_eq!(s.accent_homographs.key, "F10");
    }

    #[test]
    fn find_duplicate_detects_same_binding() {
        // Make next_tab identical to edit_word
        let s = EditorHotkeySettings {
            edit_word: Hotkey {
                modifiers: vec![HotkeyModifier::Ctrl],
                key: "E".to_string(),
                enabled: true,
            },
            next_tab: Hotkey {
                modifiers: vec![HotkeyModifier::Ctrl],
                key: "E".to_string(),
                enabled: true,
            },
            ..Default::default()
        };
        let conflict = s.find_duplicate("edit_word", &s.edit_word.clone());
        assert!(conflict.is_some());
    }

    #[test]
    fn find_duplicate_skips_self() {
        let s = EditorHotkeySettings::default();
        let binding = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "E".to_string(),
            enabled: true,
        };
        let conflict = s.find_duplicate("edit_word", &binding);
        assert!(conflict.is_none());
    }

    #[test]
    fn find_duplicate_detects_new_action_bindings() {
        let s = EditorHotkeySettings {
            cycle_route: Hotkey {
                modifiers: vec![HotkeyModifier::Ctrl],
                key: "R".to_string(),
                enabled: true,
            },
            toggle_typing: Hotkey {
                modifiers: vec![HotkeyModifier::Ctrl],
                key: "R".to_string(),
                enabled: true,
            },
            ..Default::default()
        };
        let conflict = s.find_duplicate("cycle_route", &s.cycle_route.clone());
        assert_eq!(conflict, Some("toggle_typing"));
    }

    #[test]
    fn find_duplicate_empty_binding_is_ignored() {
        let s = EditorHotkeySettings::default();
        let empty = Hotkey {
            modifiers: vec![],
            key: String::new(),
            enabled: true,
        };
        assert!(s.find_duplicate("edit_word", &empty).is_none());
    }

    #[test]
    fn find_duplicate_detects_accent_homographs_conflict() {
        let s = EditorHotkeySettings {
            accent_homographs: Hotkey {
                modifiers: vec![HotkeyModifier::Ctrl],
                key: "U".to_string(),
                enabled: true,
            },
            toggle_history: Hotkey {
                modifiers: vec![HotkeyModifier::Ctrl],
                key: "U".to_string(),
                enabled: true,
            },
            ..Default::default()
        };
        let conflict = s.find_duplicate("accent_homographs", &s.accent_homographs.clone());
        assert_eq!(conflict, Some("toggle_history"));
    }

    #[test]
    fn conflicts_with_global_detects_overlap() {
        let editor_binding = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F3".to_string(),
            enabled: true,
        };
        let global = HotkeySettings::default();
        assert!(editor_binding.conflicts_with_global(&global));
    }

    #[test]
    fn conflicts_with_global_empty_binding_is_ok() {
        let empty = Hotkey {
            modifiers: vec![],
            key: String::new(),
            enabled: true,
        };
        let global = HotkeySettings::default();
        assert!(!empty.conflicts_with_global(&global));
    }

    #[test]
    fn conflicts_with_global_unique_binding_is_ok() {
        let unique = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "F12".to_string(),
            enabled: true,
        };
        let global = HotkeySettings::default();
        assert!(!unique.conflicts_with_global(&global));
    }

    /// Регрессия на "Failed to parse settings: missing field `playback_pause`":
    /// старый settings.json (до plan 74) содержит только main_window и sound_panel.
    /// Десериализация должна проходить, а недостающие playback-поля — заполняться
    /// дефолтом (F4/F5/F6), не ломая загрузку приложения.
    #[test]
    fn test_hotkey_settings_backwards_compatible_without_playback_fields() {
        let old_json = r#"{
            "main_window": { "modifiers": ["ctrl", "shift"], "key": "F3" },
            "sound_panel": { "modifiers": ["ctrl", "alt"], "key": "P" }
        }"#;
        let settings: HotkeySettings = serde_json::from_str(old_json).unwrap();
        assert_eq!(settings.main_window.key, "F3");
        assert_eq!(settings.sound_panel.key, "P");
        assert_eq!(settings.playback_pause.key, "F4");
        assert_eq!(settings.playback_stop.key, "F5");
        assert_eq!(settings.playback_repeat.key, "F6");
        assert_eq!(settings.return_previous_window.key, "F");
        assert_eq!(settings.return_previous_window.modifiers.len(), 1);
        assert_eq!(
            settings.return_previous_window.modifiers[0],
            HotkeyModifier::Ctrl
        );
    }

    /// Старый settings.json без "editor" группы: editor поля должны заполняться defaults.
    #[test]
    fn test_hotkey_settings_backwards_compatible_without_editor_field() {
        let old_json = r#"{
            "main_window": { "modifiers": ["ctrl", "shift"], "key": "F3" },
            "sound_panel": { "modifiers": ["ctrl", "alt"], "key": "P" }
        }"#;
        let settings: HotkeySettings = serde_json::from_str(old_json).unwrap();
        assert_eq!(settings.editor.edit_word.key, "E");
        assert_eq!(settings.editor.submit_continue.key, "Enter");
        assert_eq!(settings.editor.submit_keep_text.key, "Enter");
        assert_eq!(settings.editor.submit_keep_focus.key, "Enter");
        assert_eq!(settings.editor.next_spelling_error.key, "F7");
        assert_eq!(settings.editor.previous_spelling_error.key, "F7");
        assert_eq!(settings.editor.next_tab.key, "Tab");
        assert_eq!(settings.editor.previous_tab.key, "Tab");
        assert_eq!(settings.editor.cycle_route.key, "R");
        assert_eq!(settings.editor.toggle_typing.key, "T");
        assert_eq!(settings.editor.cycle_quick_mode.key, "W");
        assert_eq!(settings.editor.toggle_history.key, "H");
        assert_eq!(settings.editor.accent_homographs.key, "U");
    }

    #[test]
    fn test_default_return_previous_window() {
        let hotkey = Hotkey::default_return_previous_window();
        assert_eq!(hotkey.key, "F");
        assert_eq!(hotkey.modifiers.len(), 1);
        assert_eq!(hotkey.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hotkey.format_display(), "Ctrl+F");
    }

    /// Round-trip: HotkeySettings with editor group must serialize + deserialize.
    #[test]
    fn hotkey_settings_with_editor_round_trip() {
        let original = HotkeySettings {
            editor: EditorHotkeySettings {
                edit_word: Hotkey {
                    modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Alt],
                    key: "W".to_string(),
                    enabled: true,
                },
                ..EditorHotkeySettings::default()
            },
            ..HotkeySettings::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: HotkeySettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.editor.edit_word.key, "W");
        assert_eq!(back.editor.edit_word.modifiers.len(), 2);
        assert_eq!(back.editor.submit_continue.key, "Enter");
    }

    #[test]
    fn test_default_ocr_capture() {
        let hotkey = Hotkey::default_ocr_capture();
        assert_eq!(hotkey.key, "P");
        assert_eq!(hotkey.modifiers.len(), 2);
        assert_eq!(hotkey.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(hotkey.modifiers[1], HotkeyModifier::Shift);
        assert_eq!(hotkey.format_display(), "Ctrl+Shift+P");
    }

    /// Backward-compat: an old settings.json without `ocr_capture` must
    /// deserialize with the Ctrl+Shift+P default.
    #[test]
    fn test_hotkey_settings_backwards_compatible_without_ocr_capture_field() {
        let old_json = r#"{
            "main_window": { "modifiers": ["ctrl", "shift"], "key": "F3" },
            "sound_panel": { "modifiers": ["ctrl", "alt"], "key": "P" }
        }"#;
        let settings: HotkeySettings = serde_json::from_str(old_json).unwrap();
        assert_eq!(settings.ocr_capture.key, "P");
        assert_eq!(settings.ocr_capture.modifiers.len(), 2);
        assert_eq!(settings.ocr_capture.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(settings.ocr_capture.modifiers[1], HotkeyModifier::Shift);
    }

    /// An editor hotkey equal to the new Ctrl+Shift+P default must conflict.
    #[test]
    fn conflicts_with_global_detects_ocr_capture_overlap() {
        let editor_binding = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "P".to_string(),
            enabled: true,
        };
        let global = HotkeySettings::default();
        assert!(editor_binding.conflicts_with_global(&global));
    }

    /// Round-trip: a custom ocr_capture binding survives serialization.
    #[test]
    fn hotkey_settings_ocr_capture_round_trip() {
        let original = HotkeySettings {
            ocr_capture: Hotkey {
                modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Alt],
                key: "O".to_string(),
                enabled: true,
            },
            ..HotkeySettings::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: HotkeySettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ocr_capture.key, "O");
        assert_eq!(back.ocr_capture.modifiers.len(), 2);
        assert_eq!(back.ocr_capture.modifiers[0], HotkeyModifier::Ctrl);
        assert_eq!(back.ocr_capture.modifiers[1], HotkeyModifier::Alt);
    }

    // ==================== Per-hotkey enabled switch ====================

    /// Old settings.json without `enabled` must load every present hotkey
    /// object as enabled, including editor bindings that now ship disabled by
    /// default. Missing editor fields still follow the new product defaults.
    #[test]
    fn legacy_hotkeys_without_enabled_default_to_true() {
        let old_json = r#"{
            "main_window": { "modifiers": ["ctrl", "shift"], "key": "F3" },
            "sound_panel": { "modifiers": ["ctrl", "alt"], "key": "P" },
            "editor": {
                "edit_word": { "modifiers": ["ctrl"], "key": "E" }
            }
        }"#;
        let settings: HotkeySettings = serde_json::from_str(old_json).unwrap();
        assert!(settings.main_window.enabled);
        assert!(settings.sound_panel.enabled);
        assert!(settings.editor.edit_word.enabled);
        // Fields absent from the old editor object use the current defaults,
        // three of which are assigned but switched off.
        assert!(!settings.editor.cycle_route.enabled);
        assert!(!settings.editor.toggle_typing.enabled);
        assert!(!settings.editor.cycle_quick_mode.enabled);
        assert!(settings.ocr_capture.enabled);
    }

    /// A pre-`enabled` settings.json that already stores the Ctrl+R, Ctrl+T or
    /// Ctrl+W binding objects must keep them active: the switch only becomes
    /// opt-in for fields that are absent from the legacy payload.
    #[test]
    fn legacy_editor_binding_objects_without_enabled_stay_enabled() {
        let old_json = r#"{
            "editor": {
                "cycle_route": { "modifiers": ["ctrl"], "key": "R" },
                "toggle_typing": { "modifiers": ["ctrl"], "key": "T" },
                "cycle_quick_mode": { "modifiers": ["ctrl"], "key": "W" }
            }
        }"#;
        let settings: HotkeySettings = serde_json::from_str(old_json).unwrap();
        assert!(settings.editor.cycle_route.enabled);
        assert!(settings.editor.toggle_typing.enabled);
        assert!(settings.editor.cycle_quick_mode.enabled);
        assert_eq!(settings.editor.cycle_route.format_display(), "Ctrl+R");
        assert_eq!(settings.editor.toggle_typing.format_display(), "Ctrl+T");
        assert_eq!(settings.editor.cycle_quick_mode.format_display(), "Ctrl+W");
    }

    /// The fresh defaults keep the combinations assigned while switched off.
    #[test]
    fn editor_hotkey_defaults_keep_combinations_but_ship_switched_off() {
        let settings = EditorHotkeySettings::default();
        for id in ["cycle_route", "toggle_typing", "cycle_quick_mode"] {
            let binding = settings.get_by_id(id).expect("default binding");
            assert!(!binding.key.is_empty(), "{id} must keep its combination");
            assert!(!binding.enabled, "{id} must ship switched off");
        }
    }

    #[test]
    fn disabled_hotkey_round_trips_and_is_not_equal_to_enabled_one() {
        let disabled = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "E".to_string(),
            enabled: false,
        };
        let json = serde_json::to_string(&disabled).unwrap();
        assert!(json.contains("\"enabled\":false"));
        let back: Hotkey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, disabled);

        let mut enabled = disabled.clone();
        enabled.enabled = true;
        assert_ne!(back, enabled);
    }

    #[test]
    fn find_duplicate_ignores_disabled_existing_binding() {
        let s = EditorHotkeySettings {
            edit_word: Hotkey {
                modifiers: vec![HotkeyModifier::Ctrl],
                key: "E".to_string(),
                enabled: false,
            },
            ..Default::default()
        };
        let candidate = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "E".to_string(),
            enabled: true,
        };
        assert!(s.find_duplicate("next_tab", &candidate).is_none());
    }

    #[test]
    fn find_duplicate_ignores_disabled_candidate() {
        let s = EditorHotkeySettings::default();
        let candidate = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl],
            key: "Enter".to_string(),
            enabled: false,
        };
        // submit_continue already uses Ctrl+Enter, but a disabled candidate is
        // allowed to keep the same combination while switched off.
        assert!(s.find_duplicate("edit_word", &candidate).is_none());
    }

    #[test]
    fn conflicts_with_global_ignores_disabled_global() {
        let mut global = HotkeySettings::default();
        global.main_window.enabled = false;
        let candidate = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F3".to_string(),
            enabled: true,
        };
        assert!(!candidate.conflicts_with_global(&global));
        assert_eq!(global.find_global_conflict("", &candidate), None);
    }

    #[test]
    fn conflicts_with_global_ignores_disabled_candidate() {
        let global = HotkeySettings::default();
        let mut candidate = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F3".to_string(),
            enabled: false,
        };
        assert!(!candidate.conflicts_with_global(&global));
        candidate.enabled = true;
        assert!(candidate.conflicts_with_global(&global));
    }

    #[test]
    fn find_global_conflict_reports_other_enabled_name() {
        let global = HotkeySettings::default();
        let candidate = Hotkey {
            modifiers: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            key: "F2".to_string(),
            enabled: true,
        };
        assert_eq!(
            global.find_global_conflict("main_window", &candidate),
            Some("sound_panel")
        );
        assert_eq!(global.find_global_conflict("sound_panel", &candidate), None);
    }

    #[test]
    fn find_editor_conflict_ignores_disabled_editor_binding() {
        let mut settings = HotkeySettings::default();
        settings.editor.edit_word.enabled = false;
        let candidate = settings.editor.edit_word.clone();
        let mut enabled_candidate = candidate.clone();
        enabled_candidate.enabled = true;
        assert_eq!(settings.find_editor_conflict(&candidate), None);
        // With the stored editor binding disabled, the same combination is free.
        assert_eq!(settings.find_editor_conflict(&enabled_candidate), None);
    }
}
