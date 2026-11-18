import { describe, it, expect } from 'vitest'
import {
  storedKeyToCode,
  modifiersMatch,
  matchesLocalHotkey,
  type LocalHotkeyEvent,
} from './localHotkeys'

function ev(partial: Partial<LocalHotkeyEvent> & Pick<LocalHotkeyEvent, 'code'>): LocalHotkeyEvent {
  return {
    code: partial.code,
    ctrlKey: partial.ctrlKey ?? false,
    shiftKey: partial.shiftKey ?? false,
    altKey: partial.altKey ?? false,
    metaKey: partial.metaKey ?? false,
  }
}

describe('storedKeyToCode', () => {
  it('maps NumPad names through the shared physical mapping', () => {
    expect(storedKeyToCode('NUMPAD5')).toBe('Numpad5')
    expect(storedKeyToCode('NUMPAD_ADD')).toBe('NumpadAdd')
    expect(storedKeyToCode('NUMPAD_DECIMAL')).toBe('NumpadDecimal')
    // Outside the restricted set the name stays unusable rather than invented.
    expect(storedKeyToCode('NUMPAD_ENTER')).toBe('NUMPAD_ENTER')
  })

  it('preserves the legacy non-NumPad mappings', () => {
    expect(storedKeyToCode('SPACE')).toBe('Space')
    expect(storedKeyToCode('F3')).toBe('F3')
    expect(storedKeyToCode('M')).toBe('KeyM')
    expect(storedKeyToCode('5')).toBe('Digit5')
    expect(storedKeyToCode('Enter')).toBe('Enter')
    expect(storedKeyToCode('Tab')).toBe('Tab')
  })
})

describe('modifiersMatch', () => {
  it('requires the exact modifier set', () => {
    expect(modifiersMatch(ev({ code: 'Numpad5', ctrlKey: true }), ['ctrl'])).toBe(true)
    expect(modifiersMatch(ev({ code: 'Numpad5' }), ['ctrl'])).toBe(false)
    expect(modifiersMatch(ev({ code: 'Numpad5', ctrlKey: true, shiftKey: true }), ['ctrl'])).toBe(false)
    expect(modifiersMatch(ev({ code: 'Numpad5', ctrlKey: true, altKey: true }), ['ctrl'])).toBe(false)
    expect(modifiersMatch(ev({ code: 'Numpad5', metaKey: true }), ['super'])).toBe(true)
  })
})

describe('matchesLocalHotkey', () => {
  it('matches a bare NumPad binding by physical code', () => {
    const binding = { modifiers: [], key: 'NUMPAD5' }
    expect(matchesLocalHotkey(binding, ev({ code: 'Numpad5' }))).toBe(true)
    expect(matchesLocalHotkey(binding, ev({ code: 'Digit5' }))).toBe(false)
    expect(matchesLocalHotkey(binding, ev({ code: 'Numpad6' }))).toBe(false)
  })

  it('ignores the key text, so a Num-Lock-off event still fires the binding', () => {
    // With Num Lock off the same physical key reports a navigation `key`
    // ('ArrowLeft' for Numpad4) while `code` stays 'Numpad4'; the event below
    // carries that navigation text and must still match.
    const binding = { modifiers: [], key: 'NUMPAD4' }
    const numLockOffEvent = {
      code: 'Numpad4',
      key: 'ArrowLeft',
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    }
    expect(matchesLocalHotkey(binding, numLockOffEvent)).toBe(true)
  })

  it('matches NumPad operators with modifiers', () => {
    const binding = { modifiers: ['ctrl'], key: 'NUMPAD_ADD' }
    expect(matchesLocalHotkey(binding, ev({ code: 'NumpadAdd', ctrlKey: true }))).toBe(true)
    expect(matchesLocalHotkey(binding, ev({ code: 'NumpadAdd' }))).toBe(false)
    expect(matchesLocalHotkey(binding, ev({ code: 'NumpadAdd', ctrlKey: true, altKey: true }))).toBe(false)
  })

  it('keeps the top digit row separate from NumPad digits', () => {
    const digitRow = { modifiers: [], key: '5' }
    const numpad = { modifiers: [], key: 'NUMPAD5' }
    expect(matchesLocalHotkey(digitRow, ev({ code: 'Digit5' }))).toBe(true)
    expect(matchesLocalHotkey(digitRow, ev({ code: 'Numpad5' }))).toBe(false)
    expect(matchesLocalHotkey(numpad, ev({ code: 'Numpad5' }))).toBe(true)
    expect(matchesLocalHotkey(numpad, ev({ code: 'Digit5' }))).toBe(false)
  })

  it('never matches disabled, empty or missing bindings', () => {
    expect(matchesLocalHotkey({ modifiers: [], key: 'NUMPAD5', enabled: false }, ev({ code: 'Numpad5' }))).toBe(false)
    expect(matchesLocalHotkey({ modifiers: [], key: '' }, ev({ code: 'Numpad5' }))).toBe(false)
    expect(matchesLocalHotkey(undefined, ev({ code: 'Numpad5' }))).toBe(false)
    // A missing `enabled` comes from a legacy payload and means enabled.
    expect(matchesLocalHotkey({ modifiers: [], key: 'NUMPAD5' }, ev({ code: 'Numpad5' }))).toBe(true)
  })

  it('does not match unrelated events even when the stored name has no code mapping', () => {
    const binding = { modifiers: ['ctrl'], key: 'NUMPAD_ENTER' }
    expect(matchesLocalHotkey(binding, ev({ code: 'Enter', ctrlKey: true }))).toBe(false)
    expect(matchesLocalHotkey(binding, ev({ code: 'NumpadEnter', ctrlKey: true }))).toBe(false)
  })
})
