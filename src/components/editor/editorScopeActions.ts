import type { EditorHotkeySettingsDto } from '../../types/settings'
import { matchesEditorHotkey, type HotkeyEventLike } from './keymapArbitration'

/**
 * Decision part of the editor-scope hotkey dispatch used by
 * `InputPanel.handleEditorScopeKeydown` (editor tab surface focused, the
 * incoming-tab branch is resolved separately). InputPanel owns the effects:
 * prevent/stop the event and run the action; this module only answers which
 * action an enabled, exact-modifier binding maps the keydown to.
 *
 * The main-window output toggles (ROADMAP-142) are matched at the
 * main-window level now and are not editor-scope actions.
 */
export type EditorScopeAction =
  | 'submit_keep_focus'
  | 'submit_keep_text'
  | 'next_tab'
  | 'previous_tab'
  | 'approve_next_incoming'
  | 'edit_next_incoming'
  | 'accent_homographs'
  | 'cycle_route'
  | 'toggle_typing'
  | 'cycle_quick_mode'
  | 'toggle_history'

/**
 * Resolve an editor-scope keydown to its action. Priority mirrors the
 * historical if-chain in InputPanel: submit bindings first, then tab
 * movement, then the incoming-surface actions, then the toggles.
 */
export function resolveEditorScopeAction(
  bindings: EditorHotkeySettingsDto | undefined,
  event: HotkeyEventLike,
): EditorScopeAction | null {
  if (!bindings) return null
  if (matchesEditorHotkey(bindings.submit_keep_focus, event)) return 'submit_keep_focus'
  if (matchesEditorHotkey(bindings.submit_keep_text, event)) return 'submit_keep_text'
  if (matchesEditorHotkey(bindings.next_tab, event)) return 'next_tab'
  if (matchesEditorHotkey(bindings.previous_tab, event)) return 'previous_tab'
  if (matchesEditorHotkey(bindings.approve_next_incoming, event)) return 'approve_next_incoming'
  if (matchesEditorHotkey(bindings.edit_next_incoming, event)) return 'edit_next_incoming'
  if (matchesEditorHotkey(bindings.accent_homographs, event)) return 'accent_homographs'
  if (matchesEditorHotkey(bindings.cycle_route, event)) return 'cycle_route'
  if (matchesEditorHotkey(bindings.toggle_typing, event)) return 'toggle_typing'
  if (matchesEditorHotkey(bindings.cycle_quick_mode, event)) return 'cycle_quick_mode'
  if (matchesEditorHotkey(bindings.toggle_history, event)) return 'toggle_history'
  return null
}
