/**
 * Pure helpers for recording and displaying intercepted keys.
 *
 * `resolveInterceptKey` turns a DOM key event into the canonical name the
 * backend stores (`NUMPAD1`, `F1`, `PAGEUP`, `HOME`, `INSERT`, or `VK_XX`).
 * It mirrors the backend `vk_to_name_for_mode` contract so frontend and
 * backend always agree on the stored identifier.
 */

export interface InterceptKeyEvent {
  key: string
  code: string
  keyCode: number
  repeat?: boolean
}

const WINDOWS_VK_MIN = 0x08
const WINDOWS_VK_MAX = 0xfe

/**
 * Physical `code` → canonical NumPad name (`NUMPAD0`…`NUMPAD9`, `NUMPAD_ADD`,
 * `NUMPAD_SUBTRACT`, `NUMPAD_MULTIPLY`, `NUMPAD_DIVIDE`, `NUMPAD_DECIMAL`).
 * NumpadEnter and other non-restricted NumPad keys return `null`.
 */
export function numpadKeyNameFromCode(code: string): string | null {
  if (!code.startsWith('Numpad')) return null
  const rest = code.slice('Numpad'.length)
  if (rest === 'Multiply') return 'NUMPAD_MULTIPLY'
  if (rest === 'Add') return 'NUMPAD_ADD'
  if (rest === 'Subtract') return 'NUMPAD_SUBTRACT'
  if (rest === 'Decimal') return 'NUMPAD_DECIMAL'
  if (rest === 'Divide') return 'NUMPAD_DIVIDE'
  if (/^\d$/.test(rest)) return `NUMPAD${rest}`
  return null
}

/**
 * Canonical NumPad name → physical DOM `event.code` (`NUMPAD5` → `Numpad5`,
 * `NUMPAD_ADD` → `NumpadAdd`). The inverse of `numpadKeyNameFromCode` and the
 * single shared physical mapping for every DOM-level consumer of a stored
 * binding (main-window-local matching in `utils/localHotkeys.ts`, editor
 * matching in `editor/keymapArbitration.ts`) — the same code set the backend
 * `parse_key_code` accepts, so stored names stay executable on both sides.
 * `NUMPAD_ENTER` and other names outside the restricted set return `null`,
 * mirroring the backend where they are intentionally absent.
 */
export function numpadCodeFromKeyName(key: string): string | null {
  const normalized = key.trim().toUpperCase()
  if (!normalized.startsWith('NUMPAD')) return null
  // Stored operator names carry a separator (`NUMPAD_ADD`); recorded digit
  // names do not (`NUMPAD5`).
  const rest = normalized.slice('NUMPAD'.length).replace(/^_/, '')
  if (rest === 'ADD') return 'NumpadAdd'
  if (rest === 'SUBTRACT') return 'NumpadSubtract'
  if (rest === 'MULTIPLY') return 'NumpadMultiply'
  if (rest === 'DECIMAL') return 'NumpadDecimal'
  if (rest === 'DIVIDE') return 'NumpadDivide'
  if (/^\d$/.test(rest)) return `Numpad${rest}`
  return null
}

function restrictedNameFromCode(code: string): string | null {
  const numpad = numpadKeyNameFromCode(code)
  if (numpad !== null) return numpad
  // NumpadEnter / NumpadEqual / NumpadComma / NumpadSeparator are not part of
  // the restricted set; they fall through to the unrestricted VK fallback.
  if (code === 'PageUp') return 'PAGEUP'
  if (code === 'PageDown') return 'PAGEDOWN'
  if (code === 'End') return 'END'
  if (code === 'Home') return 'HOME'
  if (code === 'Insert') return 'INSERT'
  if (code.startsWith('F')) {
    const num = Number.parseInt(code.slice(1), 10)
    if (num >= 1 && num <= 24) return code
    return null
  }
  return null
}

function restrictedNameFromVk(vk: number): string | null {
  if (vk >= 0x60 && vk <= 0x69) return `NUMPAD${vk - 0x60}`
  if (vk === 0x6a) return 'NUMPAD_MULTIPLY'
  if (vk === 0x6b) return 'NUMPAD_ADD'
  if (vk === 0x6d) return 'NUMPAD_SUBTRACT'
  if (vk === 0x6e) return 'NUMPAD_DECIMAL'
  if (vk === 0x6f) return 'NUMPAD_DIVIDE'
  if (vk >= 0x70 && vk <= 0x87) return `F${vk - 0x6f}`
  if (vk === 0x21) return 'PAGEUP'
  if (vk === 0x22) return 'PAGEDOWN'
  if (vk === 0x23) return 'END'
  if (vk === 0x24) return 'HOME'
  if (vk === 0x2d) return 'INSERT'
  return null
}

/**
 * Windows VKs for the sided modifier keys. The DOM collapses Shift/Ctrl/Alt to
 * generic keyCode 16/17/18 and Meta to 91, so the physical `code` is the only
 * reliable source for left/right variants.
 */
const SIDED_MODIFIER_VK: Record<string, number> = {
  ShiftLeft: 0xa0,
  ShiftRight: 0xa1,
  ControlLeft: 0xa2,
  ControlRight: 0xa3,
  AltLeft: 0xa4,
  AltRight: 0xa5,
  MetaLeft: 0x5b,
  MetaRight: 0x5c,
}

/**
 * Physical `code` → Windows VK fallback for keys whose native keyCode is zero.
 * Prefer the nonzero native keyCode (see `resolveUnrestrictedVk`) so OEM keys
 * keep the VK of the active layout; this map is only a last resort.
 */
const CODE_TO_VK: Record<string, number> = {
  Escape: 0x1b,
  Tab: 0x09,
  CapsLock: 0x14,
  Space: 0x20,
  Enter: 0x0d,
  Backspace: 0x08,
  Delete: 0x2e,
  Insert: 0x2d,
  Home: 0x24,
  End: 0x23,
  PageUp: 0x21,
  PageDown: 0x22,
  ArrowLeft: 0x25,
  ArrowUp: 0x26,
  ArrowRight: 0x27,
  ArrowDown: 0x28,
  PrintScreen: 0x2c,
  ScrollLock: 0x91,
  Pause: 0x13,
  NumLock: 0x90,
  ContextMenu: 0x5d,
  Semicolon: 0xba,
  Equal: 0xbb,
  Comma: 0xbc,
  Minus: 0xbd,
  Period: 0xbe,
  Slash: 0xbf,
  Backquote: 0xc0,
  BracketLeft: 0xdb,
  Backslash: 0xdc,
  BracketRight: 0xdd,
  Quote: 0xde,
  IntlBackslash: 0xe2,
  MediaTrackNext: 0xb0,
  MediaTrackPrevious: 0xb1,
  MediaStop: 0xb2,
  MediaPlayPause: 0xb3,
  VolumeMute: 0xad,
  VolumeDown: 0xae,
  VolumeUp: 0xaf,
  AudioVolumeMute: 0xad,
  AudioVolumeDown: 0xae,
  AudioVolumeUp: 0xaf,
  LaunchMail: 0xb4,
  LaunchMediaSelect: 0xb5,
  MediaSelect: 0xb5,
  LaunchApp1: 0xb6,
  LaunchApp2: 0xb7,
  BrowserBack: 0xa6,
  BrowserForward: 0xa7,
  BrowserRefresh: 0xa8,
  BrowserStop: 0xa9,
  BrowserSearch: 0xaa,
  BrowserFavorites: 0xab,
  BrowserHome: 0xac,
}

for (let i = 0; i < 26; i += 1) CODE_TO_VK[`Key${String.fromCharCode(0x41 + i)}`] = 0x41 + i
for (let i = 0; i < 10; i += 1) CODE_TO_VK[`Digit${i}`] = 0x30 + i
for (let i = 1; i <= 24; i += 1) CODE_TO_VK[`F${i}`] = 0x6f + i

function formatVk(vk: number): string {
  return `VK_${vk.toString(16).toUpperCase().padStart(2, '0')}`
}

function resolveUnrestrictedVk(event: InterceptKeyEvent): number | null {
  const sided = SIDED_MODIFIER_VK[event.code]
  if (sided !== undefined) return sided

  if (
    Number.isInteger(event.keyCode)
    && event.keyCode >= WINDOWS_VK_MIN
    && event.keyCode <= WINDOWS_VK_MAX
  ) {
    return event.keyCode
  }

  const fallback = CODE_TO_VK[event.code]
  if (fallback !== undefined) return fallback

  return null
}

/**
 * Resolve a key event to the canonical stored name.
 *
 * Restricted names (NumPad / F-keys / navigation keys including Home and
 * Insert) always win and are accepted in both modes. In restricted mode any
 * other key resolves to `null`. In unrestricted mode the key resolves to a
 * `VK_XX` name, reusing a restricted canonical name whenever the Windows VK
 * represents a restricted key so frontend and backend agree.
 */
export function resolveInterceptKey(
  event: InterceptKeyEvent,
  allowAnyKey: boolean,
): string | null {
  const restricted = restrictedNameFromCode(event.code)
  if (restricted !== null) return restricted

  if (!allowAnyKey) return null

  const vk = resolveUnrestrictedVk(event)
  if (vk === null) return null

  const canonical = restrictedNameFromVk(vk)
  if (canonical !== null) return canonical

  return formatVk(vk)
}

const NUMPAD_KEY_LABELS: Record<string, string> = {
  NUMPAD0: 'Num 0',
  NUMPAD1: 'Num 1',
  NUMPAD2: 'Num 2',
  NUMPAD3: 'Num 3',
  NUMPAD4: 'Num 4',
  NUMPAD5: 'Num 5',
  NUMPAD6: 'Num 6',
  NUMPAD7: 'Num 7',
  NUMPAD8: 'Num 8',
  NUMPAD9: 'Num 9',
  NUMPAD_ADD: 'Num +',
  NUMPAD_SUBTRACT: 'Num -',
  NUMPAD_MULTIPLY: 'Num *',
  NUMPAD_DIVIDE: 'Num /',
  NUMPAD_DECIMAL: 'Num .',
}

/**
 * Readable label for a stored NumPad key name (`NUMPAD5` → `Num 5`).
 * Non-NumPad names return `null` so callers keep their own display.
 */
export function formatNumpadKeyLabel(key: string): string | null {
  return NUMPAD_KEY_LABELS[key.toUpperCase()] ?? null
}

/**
 * Whether a stored key name is a NumPad digit (`NUMPAD0`…`NUMPAD9`).
 * Only digits depend on the Num Lock state; the operator keys do not.
 */
export function isNumpadDigitKey(key: string): boolean {
  return /^NUMPAD\d$/.test(key.toUpperCase())
}

const VK_LABELS: Record<number, string> = {
  0x08: 'Backspace',
  0x09: 'Tab',
  0x0d: 'Enter',
  0x13: 'Pause',
  0x14: 'Caps Lock',
  0x1b: 'Escape',
  0x20: 'Space',
  0x21: 'Page Up',
  0x22: 'Page Down',
  0x23: 'End',
  0x24: 'Home',
  0x25: '←',
  0x26: '↑',
  0x27: '→',
  0x28: '↓',
  0x2c: 'Print Screen',
  0x2d: 'Insert',
  0x2e: 'Delete',
  0x5b: 'Win (Left)',
  0x5c: 'Win (Right)',
  0x5d: 'Menu',
  0x90: 'Num Lock',
  0x91: 'Scroll Lock',
  0xa0: 'Shift (Left)',
  0xa1: 'Shift (Right)',
  0xa2: 'Ctrl (Left)',
  0xa3: 'Ctrl (Right)',
  0xa4: 'Alt (Left)',
  0xa5: 'Alt (Right)',
  0xa6: 'Back',
  0xa7: 'Forward',
  0xa8: 'Refresh',
  0xa9: 'Stop',
  0xaa: 'Search',
  0xab: 'Favorites',
  0xac: 'Home',
  0xad: 'Mute',
  0xae: 'Volume Down',
  0xaf: 'Volume Up',
  0xb0: 'Next Track',
  0xb1: 'Previous Track',
  0xb2: 'Media Stop',
  0xb3: 'Play/Pause',
  0xb4: 'Mail',
  0xb5: 'Media Select',
  0xb6: 'App 1',
  0xb7: 'App 2',
}

for (let i = 0; i < 26; i += 1) VK_LABELS[0x41 + i] = String.fromCharCode(0x41 + i)
for (let i = 0; i < 10; i += 1) VK_LABELS[0x30 + i] = String(i)

/**
 * Readable label for a stored binding key.
 *
 * Restricted names (`NUMPAD1`, `F5`, `PAGEUP`, `HOME`, `INSERT`, ...) are
 * returned unchanged to preserve the readable names of existing bindings.
 * `VK_XX` codes resolve to a readable label where known and otherwise keep the
 * raw `VK_XX` code.
 */
export function formatInterceptKey(key: string): string {
  if (!key.startsWith('VK_')) return key
  const hex = key.slice('VK_'.length)
  if (!/^[0-9A-Fa-f]{2}$/.test(hex)) return key
  const vk = Number.parseInt(hex, 16)
  return VK_LABELS[vk] ?? key
}
