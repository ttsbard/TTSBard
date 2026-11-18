// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { resolveEditorScopeAction } from './editorScopeActions'
import type { EditorHotkeySettingsDto, HotkeyDto } from '../../types/settings'

const key = (modifiers: string[], k: string, enabled = true): HotkeyDto =>
  ({ modifiers, key: k, enabled }) as HotkeyDto

function bindings(overrides: Partial<EditorHotkeySettingsDto> = {}): EditorHotkeySettingsDto {
  // The cast keeps storage-compatibility fields (hotkeys.editor.toggle_*)
  // out of the dispatch tests' type surface.
  return ({
    edit_word: key([], ''),
    submit_continue: key([], ''),
    submit_keep_text: key([], ''),
    submit_keep_focus: key([], ''),
    next_spelling_error: key([], ''),
    previous_spelling_error: key([], ''),
    next_tab: key(['ctrl'], 'T'),
    previous_tab: key(['ctrl', 'shift'], 'T'),
    cycle_route: key(['ctrl'], 'R'),
    toggle_typing: key(['ctrl'], 'Y'),
    cycle_quick_mode: key([], ''),
    toggle_history: key(['ctrl'], 'H'),
    accent_homographs: key(['ctrl'], 'U'),
    approve_next_incoming: key(['ctrl'], 'K'),
    edit_next_incoming: key(['ctrl'], 'I'),
    ...overrides,
  }) as EditorHotkeySettingsDto
}

const ev = (code: string, mods: Partial<KeyboardEvent> = {}) =>
  ({
    code,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
    ...mods,
  }) as KeyboardEvent

describe('editor scope action resolution (real InputPanel dispatch path)', () => {
  it('does not steal existing actions: the route cycle wins over typing toggle keys', () => {
    const b = bindings({
      cycle_route: key(['ctrl'], 'M'),
      toggle_typing: key(['ctrl'], 'M'),
    })
    expect(resolveEditorScopeAction(b, ev('KeyM', { ctrlKey: true }))).toBe('cycle_route')
  })

  it('keeps the historical action priority: submit first, tabs before toggles', () => {
    const b = bindings({
      submit_keep_focus: key([], 'Enter'),
      next_tab: key(['ctrl'], 'M'),
      toggle_virtual_mic: key(['ctrl'], 'M'),
    })
    expect(resolveEditorScopeAction(b, ev('Enter'))).toBe('submit_keep_focus')
    expect(resolveEditorScopeAction(b, ev('KeyM', { ctrlKey: true }))).toBe('next_tab')
  })

  it('returns null without bindings', () => {
    expect(resolveEditorScopeAction(undefined, ev('KeyM', { ctrlKey: true }))).toBeNull()
  })
})
