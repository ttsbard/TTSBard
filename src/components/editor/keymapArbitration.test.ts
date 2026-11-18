import { describe, it, expect } from 'vitest'
import {
  hotkeyCode,
  matchesEditorHotkey,
  shouldEnterSubmit,
  shouldEscapeSubmit,
} from './keymapArbitration'
import type { HotkeyDto } from '../../types/settings'

function event(init: Partial<KeyboardEvent> & Pick<KeyboardEvent, 'code'>) {
  return {
    code: init.code,
    ctrlKey: init.ctrlKey ?? false,
    shiftKey: init.shiftKey ?? false,
    altKey: init.altKey ?? false,
    metaKey: init.metaKey ?? false,
  }
}

describe('shouldEnterSubmit', () => {
  it('returns true when autocomplete is closed (null status)', () => {
    expect(shouldEnterSubmit(null, -1)).toBe(true)
  })

  it('returns true when autocomplete is closed with nonsensical index', () => {
    expect(shouldEnterSubmit(null, 0)).toBe(true)
  })

  it('returns true when autocomplete is closed with null index', () => {
    expect(shouldEnterSubmit(null, null)).toBe(true)
  })

  it('returns true when autocomplete is active with null index', () => {
    expect(shouldEnterSubmit('active', null)).toBe(true)
  })

  it('returns true when autocomplete is active with no selection', () => {
    expect(shouldEnterSubmit('active', -1)).toBe(true)
  })

  it('returns false when autocomplete is active with a selected option', () => {
    expect(shouldEnterSubmit('active', 0)).toBe(false)
  })

  it('returns false when autocomplete is active with later option selected', () => {
    expect(shouldEnterSubmit('active', 2)).toBe(false)
  })

  it('returns true when autocomplete is pending with no selection', () => {
    expect(shouldEnterSubmit('pending', -1)).toBe(true)
  })

  it('returns true when autocomplete is pending with nonsensical index', () => {
    expect(shouldEnterSubmit('pending', 0)).toBe(true)
  })
})

describe('shouldEscapeSubmit', () => {
  it('returns true when autocomplete is closed', () => {
    expect(shouldEscapeSubmit(null, null)).toBe(true)
  })

  it('returns true when autocomplete is active with null index', () => {
    expect(shouldEscapeSubmit('active', null)).toBe(true)
  })

  it('returns true when autocomplete is active with no selection', () => {
    expect(shouldEscapeSubmit('active', -1)).toBe(true)
  })

  it('returns false when autocomplete is active with a selected option', () => {
    expect(shouldEscapeSubmit('active', 0)).toBe(false)
  })

  it('returns false when autocomplete is active with later option selected', () => {
    expect(shouldEscapeSubmit('active', 2)).toBe(false)
  })

  it('returns true when autocomplete is pending with null index', () => {
    expect(shouldEscapeSubmit('pending', null)).toBe(true)
  })

  it('returns true when autocomplete is pending with no selection', () => {
    expect(shouldEscapeSubmit('pending', -1)).toBe(true)
  })

  it('returns true when autocomplete is pending with nonsensical index', () => {
    expect(shouldEscapeSubmit('pending', 0)).toBe(true)
  })
})

describe('editor hotkey matching', () => {
  const ctrlE: HotkeyDto = { modifiers: ['ctrl'], key: 'E' }

  it('uses physical code, independent of keyboard layout', () => {
    expect(matchesEditorHotkey(ctrlE, event({ code: 'KeyE', ctrlKey: true }))).toBe(true)
  })

  it('requires an exact modifier set', () => {
    expect(matchesEditorHotkey(ctrlE, event({ code: 'KeyE' }))).toBe(false)
    expect(matchesEditorHotkey(ctrlE, event({ code: 'KeyE', ctrlKey: true, shiftKey: true }))).toBe(false)
  })

  it('distinguishes F7 from Shift+F7', () => {
    expect(matchesEditorHotkey({ modifiers: [], key: 'F7' }, event({ code: 'F7' }))).toBe(true)
    expect(matchesEditorHotkey({ modifiers: [], key: 'F7' }, event({ code: 'F7', shiftKey: true }))).toBe(false)
    expect(matchesEditorHotkey({ modifiers: ['shift'], key: 'F7' }, event({ code: 'F7', shiftKey: true }))).toBe(true)
  })

  it('does not match disabled bindings', () => {
    expect(matchesEditorHotkey({ modifiers: [], key: '' }, event({ code: 'KeyE' }))).toBe(false)
  })

  it('does not match when the binding is switched off', () => {
    expect(matchesEditorHotkey({ modifiers: ['ctrl'], key: 'E', enabled: false }, event({ code: 'KeyE', ctrlKey: true }))).toBe(false)
    expect(matchesEditorHotkey({ modifiers: ['ctrl'], key: 'E', enabled: true }, event({ code: 'KeyE', ctrlKey: true }))).toBe(true)
    expect(matchesEditorHotkey({ modifiers: ['ctrl'], key: 'E' }, event({ code: 'KeyE', ctrlKey: true }))).toBe(true)
  })

  it('matches Ctrl+R for cycle_route default', () => {
    const ctrlR: HotkeyDto = { modifiers: ['ctrl'], key: 'R' }
    expect(matchesEditorHotkey(ctrlR, event({ code: 'KeyR', ctrlKey: true }))).toBe(true)
    expect(matchesEditorHotkey(ctrlR, event({ code: 'KeyR' }))).toBe(false)
    expect(matchesEditorHotkey(ctrlR, event({ code: 'KeyR', ctrlKey: true, shiftKey: true }))).toBe(false)
    expect(matchesEditorHotkey(ctrlR, event({ code: 'KeyR', ctrlKey: true, altKey: true }))).toBe(false)
  })

  it('matches Ctrl+U for accent_homographs default', () => {
    const ctrlU: HotkeyDto = { modifiers: ['ctrl'], key: 'U' }
    expect(matchesEditorHotkey(ctrlU, event({ code: 'KeyU', ctrlKey: true }))).toBe(true)
    expect(matchesEditorHotkey(ctrlU, event({ code: 'KeyU' }))).toBe(false)
    expect(matchesEditorHotkey(ctrlU, event({ code: 'KeyU', ctrlKey: true, shiftKey: true }))).toBe(false)
  })

  it('maps Enter and Tab to their physical codes', () => {
    expect(hotkeyCode('ENTER')).toBe('Enter')
    expect(hotkeyCode('TAB')).toBe('Tab')
  })

  it('maps stored NumPad names to their physical codes', () => {
    expect(hotkeyCode('NUMPAD5')).toBe('Numpad5')
    expect(hotkeyCode('NUMPAD0')).toBe('Numpad0')
    expect(hotkeyCode('NUMPAD_ADD')).toBe('NumpadAdd')
    expect(hotkeyCode('NUMPAD_SUBTRACT')).toBe('NumpadSubtract')
    expect(hotkeyCode('NUMPAD_MULTIPLY')).toBe('NumpadMultiply')
    expect(hotkeyCode('NUMPAD_DIVIDE')).toBe('NumpadDivide')
    expect(hotkeyCode('NUMPAD_DECIMAL')).toBe('NumpadDecimal')
    // Case-insensitive like the backend parse.
    expect(hotkeyCode('numpad5')).toBe('Numpad5')
    // NumPad Enter stays outside the restricted set.
    expect(hotkeyCode('NUMPAD_ENTER')).toBeNull()
  })

  it('matches NumPad bindings by physical code with the exact modifier set', () => {
    const numpad5: HotkeyDto = { modifiers: [], key: 'NUMPAD5' }
    expect(matchesEditorHotkey(numpad5, event({ code: 'Numpad5' }))).toBe(true)
    // The top digit row is a different physical key and never matches NumPad.
    expect(matchesEditorHotkey(numpad5, event({ code: 'Digit5' }))).toBe(false)
    expect(matchesEditorHotkey(numpad5, event({ code: 'Numpad5', ctrlKey: true }))).toBe(false)
    expect(matchesEditorHotkey(numpad5, event({ code: 'Numpad6' }))).toBe(false)

    // Num-Lock-off: the same physical key reports a navigation `key` text while
    // `code` stays physical; matching below only reads the code, so this fires.
    expect(matchesEditorHotkey(
      { modifiers: [], key: 'NUMPAD4' },
      { code: 'Numpad4', ctrlKey: false, shiftKey: false, altKey: false, metaKey: false },
    )).toBe(true)

    const ctrlNumpadAdd: HotkeyDto = { modifiers: ['ctrl'], key: 'NUMPAD_ADD' }
    expect(matchesEditorHotkey(ctrlNumpadAdd, event({ code: 'NumpadAdd', ctrlKey: true }))).toBe(true)
    expect(matchesEditorHotkey(ctrlNumpadAdd, event({ code: 'NumpadAdd' }))).toBe(false)
    // NumPad Enter cannot be recorded and cannot match either.
    expect(matchesEditorHotkey({ modifiers: [], key: 'NUMPAD_ENTER' }, event({ code: 'NumpadEnter' }))).toBe(false)
  })

  it('prioritizes submit bindings by modifier set (keep_focus > keep_text > submit_continue)', () => {
    const submitContinue: HotkeyDto = { modifiers: ['ctrl'], key: 'Enter' }
    const submitKeepText: HotkeyDto = { modifiers: ['alt'], key: 'Enter' }
    const submitKeepFocus: HotkeyDto = { modifiers: ['ctrl', 'alt'], key: 'Enter' }

    const ctrlEnter = event({ code: 'Enter', ctrlKey: true })
    const altEnter = event({ code: 'Enter', altKey: true })
    const ctrlAltEnter = event({ code: 'Enter', ctrlKey: true, altKey: true })
    const plainEnter = event({ code: 'Enter' })

    // Ctrl+Enter → submit_continue only.
    expect(matchesEditorHotkey(submitContinue, ctrlEnter)).toBe(true)
    expect(matchesEditorHotkey(submitKeepText, ctrlEnter)).toBe(false)
    expect(matchesEditorHotkey(submitKeepFocus, ctrlEnter)).toBe(false)

    // Alt+Enter → submit_keep_text only.
    expect(matchesEditorHotkey(submitKeepText, altEnter)).toBe(true)
    expect(matchesEditorHotkey(submitContinue, altEnter)).toBe(false)
    expect(matchesEditorHotkey(submitKeepFocus, altEnter)).toBe(false)

    // Ctrl+Alt+Enter → submit_keep_focus only.
    expect(matchesEditorHotkey(submitKeepFocus, ctrlAltEnter)).toBe(true)
    expect(matchesEditorHotkey(submitKeepText, ctrlAltEnter)).toBe(false)
    expect(matchesEditorHotkey(submitContinue, ctrlAltEnter)).toBe(false)

    // Plain Enter matches none of the modifier bindings.
    expect(matchesEditorHotkey(submitContinue, plainEnter)).toBe(false)
    expect(matchesEditorHotkey(submitKeepText, plainEnter)).toBe(false)
    expect(matchesEditorHotkey(submitKeepFocus, plainEnter)).toBe(false)
  })
})
