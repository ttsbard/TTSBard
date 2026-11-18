import { describe, it, expect } from 'vitest'
import numpadFixtureJson from '../../scripts/contract-fixtures/numpad/numpad-keys.json'
import {
  resolveInterceptKey,
  formatInterceptKey,
  numpadKeyNameFromCode,
  numpadCodeFromKeyName,
  formatNumpadKeyLabel,
  isNumpadDigitKey,
  type InterceptKeyEvent,
} from './interceptKeys'

// Shared NumPad parity table (scripts/contract-fixtures/numpad/): the same
// rows the Rust config::hotkeys and soundpanel::intercept tests assert on.
// Static import (resolveJsonModule + native Vite JSON support) instead of
// node:fs — a clean install has no @types/node, so node:* imports fail
// vue-tsc during `npm run build`.
const numpadFixture = numpadFixtureJson as {
  keys: { domCode: string; canonicalName: string; windowsVk: number }[]
  excluded: { domCode: string; canonicalName: string; windowsVk: number }[]
  digitRow: { domCode: string; canonicalName: string; windowsVk: number }
}

function ev(partial: Partial<InterceptKeyEvent>): InterceptKeyEvent {
  return { key: '', code: '', keyCode: 0, ...partial }
}

describe('resolveInterceptKey — restricted mode', () => {
  it('rejects ordinary letters', () => {
    expect(resolveInterceptKey(ev({ key: 'a', code: 'KeyA', keyCode: 65 }), false)).toBeNull()
    expect(resolveInterceptKey(ev({ key: 'ф', code: 'KeyA', keyCode: 65 }), false)).toBeNull()
  })

  it('rejects Delete and other non-restricted keys', () => {
    expect(resolveInterceptKey(ev({ key: 'Delete', code: 'Delete', keyCode: 46 }), false)).toBeNull()
    expect(resolveInterceptKey(ev({ key: ' ', code: 'Space', keyCode: 32 }), false)).toBeNull()
    expect(resolveInterceptKey(ev({ key: 'Shift', code: 'ShiftLeft', keyCode: 16 }), false)).toBeNull()
  })

  it('accepts Home and Insert', () => {
    expect(resolveInterceptKey(ev({ key: 'Home', code: 'Home', keyCode: 36 }), false)).toBe('HOME')
    expect(resolveInterceptKey(ev({ key: 'Insert', code: 'Insert', keyCode: 45 }), false)).toBe('INSERT')
  })

  it('preserves legacy NumPad names including with NumLock-off navigation keys', () => {
    expect(resolveInterceptKey(ev({ key: '1', code: 'Numpad1', keyCode: 97 }), false)).toBe('NUMPAD1')
    // NumLock off: the same key reports a navigation `key` but still maps by code.
    expect(resolveInterceptKey(ev({ key: 'End', code: 'Numpad1', keyCode: 35 }), false)).toBe('NUMPAD1')
    expect(resolveInterceptKey(ev({ key: '*', code: 'NumpadMultiply', keyCode: 106 }), false)).toBe('NUMPAD_MULTIPLY')
    expect(resolveInterceptKey(ev({ key: '+', code: 'NumpadAdd', keyCode: 107 }), false)).toBe('NUMPAD_ADD')
    expect(resolveInterceptKey(ev({ key: '/', code: 'NumpadDivide', keyCode: 111 }), false)).toBe('NUMPAD_DIVIDE')
  })

  it('preserves F-keys and navigation keys', () => {
    expect(resolveInterceptKey(ev({ key: 'F5', code: 'F5', keyCode: 116 }), false)).toBe('F5')
    expect(resolveInterceptKey(ev({ key: 'F24', code: 'F24', keyCode: 135 }), false)).toBe('F24')
    expect(resolveInterceptKey(ev({ key: 'PageUp', code: 'PageUp', keyCode: 33 }), false)).toBe('PAGEUP')
    expect(resolveInterceptKey(ev({ key: 'End', code: 'End', keyCode: 35 }), false)).toBe('END')
    expect(resolveInterceptKey(ev({ key: 'PageDown', code: 'PageDown', keyCode: 34 }), false)).toBe('PAGEDOWN')
  })
})

describe('resolveInterceptKey — unrestricted mode', () => {
  it('maps letters independently of the active key text/layout', () => {
    expect(resolveInterceptKey(ev({ key: 'a', code: 'KeyA', keyCode: 65 }), true)).toBe('VK_41')
    expect(resolveInterceptKey(ev({ key: 'ф', code: 'KeyA', keyCode: 65 }), true)).toBe('VK_41')
  })

  it('maps OEM keys across key text/layout', () => {
    expect(resolveInterceptKey(ev({ key: '=', code: 'Equal', keyCode: 0xbb }), true)).toBe('VK_BB')
    expect(resolveInterceptKey(ev({ key: ';', code: 'Semicolon', keyCode: 0xba }), true)).toBe('VK_BA')
  })

  it('maps sided modifiers via physical code instead of generic keyCode', () => {
    expect(resolveInterceptKey(ev({ key: 'Shift', code: 'ShiftLeft', keyCode: 16 }), true)).toBe('VK_A0')
    expect(resolveInterceptKey(ev({ key: 'Shift', code: 'ShiftRight', keyCode: 16 }), true)).toBe('VK_A1')
    expect(resolveInterceptKey(ev({ key: 'Control', code: 'ControlLeft', keyCode: 17 }), true)).toBe('VK_A2')
    expect(resolveInterceptKey(ev({ key: 'Control', code: 'ControlRight', keyCode: 17 }), true)).toBe('VK_A3')
    expect(resolveInterceptKey(ev({ key: 'Alt', code: 'AltLeft', keyCode: 18 }), true)).toBe('VK_A4')
    expect(resolveInterceptKey(ev({ key: 'Alt', code: 'AltRight', keyCode: 18 }), true)).toBe('VK_A5')
    expect(resolveInterceptKey(ev({ key: 'Meta', code: 'MetaLeft', keyCode: 91 }), true)).toBe('VK_5B')
    expect(resolveInterceptKey(ev({ key: 'Meta', code: 'MetaRight', keyCode: 91 }), true)).toBe('VK_5C')
  })

  it('allows binding Escape in unrestricted mode', () => {
    expect(resolveInterceptKey(ev({ key: 'Escape', code: 'Escape', keyCode: 27 }), true)).toBe('VK_1B')
  })

  it('maps NumpadEnter through the VK fallback without inventing an identifier', () => {
    expect(resolveInterceptKey(ev({ key: 'Enter', code: 'NumpadEnter', keyCode: 13 }), true)).toBe('VK_0D')
  })

  it('maps top-row digits, media and browser keys', () => {
    expect(resolveInterceptKey(ev({ key: '1', code: 'Digit1', keyCode: 49 }), true)).toBe('VK_31')
    expect(resolveInterceptKey(ev({ key: 'MediaPlayPause', code: 'MediaPlayPause', keyCode: 0xb3 }), true)).toBe('VK_B3')
    expect(resolveInterceptKey(ev({ key: 'BrowserBack', code: 'BrowserBack', keyCode: 0xa6 }), true)).toBe('VK_A6')
  })

  it('reuses restricted canonical names when the VK represents a restricted key', () => {
    // keyCode still identifies Home even though the code is not the DOM name.
    expect(resolveInterceptKey(ev({ key: 'Home', code: 'Home', keyCode: 0x24 }), true)).toBe('HOME')
    expect(resolveInterceptKey(ev({ key: 'Insert', code: 'Insert', keyCode: 0x2d }), true)).toBe('INSERT')
  })

  it('falls back to physical code when keyCode is zero', () => {
    expect(resolveInterceptKey(ev({ key: 'a', code: 'KeyA', keyCode: 0 }), true)).toBe('VK_41')
    expect(resolveInterceptKey(ev({ key: 'Delete', code: 'Delete', keyCode: 0 }), true)).toBe('VK_2E')
    expect(resolveInterceptKey(ev({ key: 'F5', code: 'F5', keyCode: 0 }), true)).toBe('F5')
  })

  it('returns null for unmappable, zero and out-of-range VKs', () => {
    expect(resolveInterceptKey(ev({ key: '', code: 'NumpadEqual', keyCode: 0 }), true)).toBeNull()
    expect(resolveInterceptKey(ev({ key: '', code: '', keyCode: 0 }), true)).toBeNull()
    expect(resolveInterceptKey(ev({ key: '', code: 'UnknownCode', keyCode: 0x07 }), true)).toBeNull()
    expect(resolveInterceptKey(ev({ key: '', code: 'UnknownCode', keyCode: 0xff }), true)).toBeNull()
    expect(resolveInterceptKey(ev({ key: '', code: 'UnknownCode', keyCode: 0x100 }), true)).toBeNull()
  })
})

describe('formatInterceptKey', () => {
  it('preserves readable restricted names', () => {
    expect(formatInterceptKey('NUMPAD1')).toBe('NUMPAD1')
    expect(formatInterceptKey('F5')).toBe('F5')
    expect(formatInterceptKey('HOME')).toBe('HOME')
    expect(formatInterceptKey('INSERT')).toBe('INSERT')
  })

  it('resolves known VK codes to readable labels', () => {
    expect(formatInterceptKey('VK_41')).toBe('A')
    expect(formatInterceptKey('VK_20')).toBe('Space')
    expect(formatInterceptKey('VK_2E')).toBe('Delete')
    expect(formatInterceptKey('VK_A0')).toBe('Shift (Left)')
    expect(formatInterceptKey('VK_5B')).toBe('Win (Left)')
  })

  it('displays the raw code for unknown VK values', () => {
    expect(formatInterceptKey('VK_BA')).toBe('VK_BA')
    expect(formatInterceptKey('VK_FE')).toBe('VK_FE')
  })
})

describe('numpadKeyNameFromCode', () => {
  it('maps numpad digits by physical code', () => {
    expect(numpadKeyNameFromCode('Numpad0')).toBe('NUMPAD0')
    expect(numpadKeyNameFromCode('Numpad5')).toBe('NUMPAD5')
    expect(numpadKeyNameFromCode('Numpad9')).toBe('NUMPAD9')
  })

  it('maps numpad operator and decimal keys', () => {
    expect(numpadKeyNameFromCode('NumpadAdd')).toBe('NUMPAD_ADD')
    expect(numpadKeyNameFromCode('NumpadSubtract')).toBe('NUMPAD_SUBTRACT')
    expect(numpadKeyNameFromCode('NumpadMultiply')).toBe('NUMPAD_MULTIPLY')
    expect(numpadKeyNameFromCode('NumpadDivide')).toBe('NUMPAD_DIVIDE')
    expect(numpadKeyNameFromCode('NumpadDecimal')).toBe('NUMPAD_DECIMAL')
  })

  it('rejects NumpadEnter and non-numpad codes', () => {
    expect(numpadKeyNameFromCode('NumpadEnter')).toBeNull()
    expect(numpadKeyNameFromCode('NumpadEqual')).toBeNull()
    expect(numpadKeyNameFromCode('NumpadComma')).toBeNull()
    expect(numpadKeyNameFromCode('Digit5')).toBeNull()
    expect(numpadKeyNameFromCode('KeyA')).toBeNull()
    expect(numpadKeyNameFromCode('')).toBeNull()
  })
})

describe('numpadCodeFromKeyName', () => {
  it('maps stored numpad names to their physical DOM codes', () => {
    expect(numpadCodeFromKeyName('NUMPAD0')).toBe('Numpad0')
    expect(numpadCodeFromKeyName('NUMPAD5')).toBe('Numpad5')
    expect(numpadCodeFromKeyName('NUMPAD9')).toBe('Numpad9')
    expect(numpadCodeFromKeyName('NUMPAD_ADD')).toBe('NumpadAdd')
    expect(numpadCodeFromKeyName('NUMPAD_SUBTRACT')).toBe('NumpadSubtract')
    expect(numpadCodeFromKeyName('NUMPAD_MULTIPLY')).toBe('NumpadMultiply')
    expect(numpadCodeFromKeyName('NUMPAD_DECIMAL')).toBe('NumpadDecimal')
    expect(numpadCodeFromKeyName('NUMPAD_DIVIDE')).toBe('NumpadDivide')
  })

  it('is case-insensitive and trims like the backend parse', () => {
    expect(numpadCodeFromKeyName('numpad5')).toBe('Numpad5')
    expect(numpadCodeFromKeyName(' NumpadAdd ')).toBe('NumpadAdd')
  })

  it('leaves NumpadEnter, the digit row and non-numpad names unmapped', () => {
    expect(numpadCodeFromKeyName('NUMPAD_ENTER')).toBeNull()
    expect(numpadCodeFromKeyName('NUMPADEQUAL')).toBeNull()
    expect(numpadCodeFromKeyName('5')).toBeNull()
    expect(numpadCodeFromKeyName('F5')).toBeNull()
    expect(numpadCodeFromKeyName('SPACE')).toBeNull()
    expect(numpadCodeFromKeyName('')).toBeNull()
  })

  it('round-trips with numpadKeyNameFromCode over the whole restricted set', () => {
    for (const { domCode } of numpadFixture.keys) {
      const name = numpadKeyNameFromCode(domCode)
      expect(name, `numpadKeyNameFromCode(${domCode})`).not.toBeNull()
      expect(numpadCodeFromKeyName(name as string)).toBe(domCode)
    }
  })

  // ==================== shared fixture parity (008-numpad-parity) ====================

  describe('shared NumPad fixture parity', () => {
    it('maps every fixture row: DOM code to canonical name (recording)', () => {
      for (const { domCode, canonicalName } of numpadFixture.keys) {
        expect(numpadKeyNameFromCode(domCode), domCode).toBe(canonicalName)
      }
    })

    it('maps every fixture row: canonical name back to the DOM code', () => {
      for (const { domCode, canonicalName } of numpadFixture.keys) {
        expect(numpadCodeFromKeyName(canonicalName), canonicalName).toBe(domCode)
      }
    })

    it('resolves every fixture Windows VK to the canonical name', () => {
      // The VK path is what an actual hook event takes when the DOM code is
      // unavailable: the unrestricted fallback reuses restricted canonical
      // names by VK (see restrictedNameFromVk).
      for (const { canonicalName, windowsVk } of numpadFixture.keys) {
        expect(
          resolveInterceptKey(ev({ code: 'UnrelatedCode', keyCode: windowsVk }), true),
          `VK 0x${windowsVk.toString(16)}`,
        ).toBe(canonicalName)
      }
    })

    it('keeps NumpadEnter out of the supported set', () => {
      for (const { domCode, canonicalName, windowsVk } of numpadFixture.excluded) {
        expect(numpadKeyNameFromCode(domCode), domCode).toBeNull()
        expect(numpadCodeFromKeyName(canonicalName), canonicalName).toBeNull()
        expect(formatNumpadKeyLabel(canonicalName), canonicalName).toBeNull()
        // Restricted mode rejects the key; unrestricted mode falls through to
        // the shared Enter VK (0x0D) instead of inventing a NumPad name.
        expect(
          resolveInterceptKey(ev({ key: 'Enter', code: domCode, keyCode: windowsVk }), false),
          domCode,
        ).toBeNull()
        expect(
          resolveInterceptKey(ev({ key: 'Enter', code: domCode, keyCode: windowsVk }), true),
          domCode,
        ).toBe(`VK_${windowsVk.toString(16).toUpperCase().padStart(2, '0')}`)
      }
    })

    it('keeps the top digit row a different physical key from the NumPad', () => {
      const { domCode, canonicalName, windowsVk } = numpadFixture.digitRow
      // The digit row has no NumPad name in either direction.
      expect(numpadKeyNameFromCode(domCode)).toBeNull()
      expect(numpadCodeFromKeyName(canonicalName)).toBeNull()
      // Its unrestricted VK (0x35) is distinct from the fixture's NumPad VKs.
      expect(
        resolveInterceptKey(ev({ key: canonicalName, code: domCode, keyCode: windowsVk }), true),
      ).toBe(`VK_${windowsVk.toString(16).toUpperCase().padStart(2, '0')}`)
      const numpadVks = new Set(numpadFixture.keys.map(({ windowsVk: vk }) => vk))
      expect(numpadVks.has(windowsVk)).toBe(false)
    })
  })
})

describe('formatNumpadKeyLabel', () => {
  it('labels digits and operators readably', () => {
    expect(formatNumpadKeyLabel('NUMPAD0')).toBe('Num 0')
    expect(formatNumpadKeyLabel('NUMPAD9')).toBe('Num 9')
    expect(formatNumpadKeyLabel('NUMPAD_ADD')).toBe('Num +')
    expect(formatNumpadKeyLabel('NUMPAD_SUBTRACT')).toBe('Num -')
    expect(formatNumpadKeyLabel('NUMPAD_MULTIPLY')).toBe('Num *')
    expect(formatNumpadKeyLabel('NUMPAD_DIVIDE')).toBe('Num /')
    expect(formatNumpadKeyLabel('NUMPAD_DECIMAL')).toBe('Num .')
  })

  it('leaves non-numpad keys to the caller', () => {
    expect(formatNumpadKeyLabel('M')).toBeNull()
    expect(formatNumpadKeyLabel('F5')).toBeNull()
    expect(formatNumpadKeyLabel('SPACE')).toBeNull()
    expect(formatNumpadKeyLabel('NUMPAD_ENTER')).toBeNull()
  })
})

describe('isNumpadDigitKey', () => {
  it('accepts only numpad digits, not operators', () => {
    expect(isNumpadDigitKey('NUMPAD5')).toBe(true)
    expect(isNumpadDigitKey('NUMPAD0')).toBe(true)
    expect(isNumpadDigitKey('NUMPAD_ADD')).toBe(false)
    expect(isNumpadDigitKey('NUMPAD_DECIMAL')).toBe(false)
    expect(isNumpadDigitKey('5')).toBe(false)
  })
})
