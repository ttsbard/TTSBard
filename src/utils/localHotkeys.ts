/**
 * Matching for main-window-local hotkeys (`return_previous_window`,
 * `toggle_minimal_mode`). These bindings are never registered as OS shortcuts;
 * the webview matches raw `keydown` events, so the stored key name must map to
 * the physical DOM `event.code` here. The NumPad half of that mapping lives in
 * `numpadCodeFromKeyName` (`utils/interceptKeys.ts`) — the single shared
 * physical mapping, identical to the backend `parse_key_code` contract.
 *
 * Num Lock contract: the DOM reports `event.code` as the physical key
 * (`Numpad5`) in both Num Lock states, while `event.key` switches between the
 * digit and a navigation name. Matching below uses `event.code` only, so a
 * local NumPad binding fires with Num Lock off too — unlike OS-registered
 * globals, which the system translates to navigation VKs in that state.
 */

import { numpadCodeFromKeyName } from './interceptKeys'

export interface LocalHotkeyBinding {
  modifiers: string[]
  key: string
  enabled?: boolean
}

export interface LocalHotkeyEvent {
  code: string
  ctrlKey: boolean
  shiftKey: boolean
  altKey: boolean
  metaKey: boolean
}

/** Stored main-window-local key name → physical DOM `event.code`. */
export function storedKeyToCode(key: string): string {
  const numpad = numpadCodeFromKeyName(key)
  if (numpad !== null) return numpad
  if (key === 'SPACE') return 'Space'
  if (/^F\d+$/.test(key)) return key
  if (/^[A-Z]$/.test(key)) return `Key${key}`
  if (/^[0-9]$/.test(key)) return `Digit${key}`
  return key
}

export function modifiersMatch(event: LocalHotkeyEvent, hotkeyModifiers: string[]): boolean {
  if (hotkeyModifiers.includes('ctrl') !== event.ctrlKey) return false
  if (hotkeyModifiers.includes('shift') !== event.shiftKey) return false
  if (hotkeyModifiers.includes('alt') !== event.altKey) return false
  if (hotkeyModifiers.includes('super') !== event.metaKey) return false
  // Ensure no extra modifiers are pressed
  const expectedCount = hotkeyModifiers.length
  let actualCount = 0
  if (event.ctrlKey) actualCount++
  if (event.shiftKey) actualCount++
  if (event.altKey) actualCount++
  if (event.metaKey) actualCount++
  return expectedCount === actualCount
}

/**
 * Whether a `keydown` event matches an enabled stored binding: physical code
 * plus the exact modifier set. Disabled or empty bindings never match.
 */
export function matchesLocalHotkey(
  binding: LocalHotkeyBinding | undefined | null,
  event: LocalHotkeyEvent,
): boolean {
  if (!binding || !binding.key || binding.enabled === false) return false
  return event.code === storedKeyToCode(binding.key) && modifiersMatch(event, binding.modifiers)
}
