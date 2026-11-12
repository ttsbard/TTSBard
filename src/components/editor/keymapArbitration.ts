import type { HotkeyDto } from '../../types/settings'

export function shouldEnterSubmit(
  completionStatus: string | null,
  selectedIndex: number | null,
): boolean {
  if (completionStatus === 'active' && selectedIndex !== null && selectedIndex >= 0) return false
  return true
}

export function shouldEscapeSubmit(
  completionStatus: string | null,
  selectedIndex: number | null,
): boolean {
  if (completionStatus === null) return true
  if (completionStatus === 'active' && selectedIndex !== null && selectedIndex >= 0) return false
  return true
}

export type HotkeyEventLike = Pick<
  KeyboardEvent,
  'code' | 'ctrlKey' | 'shiftKey' | 'altKey' | 'metaKey'
>

export function hotkeyCode(key: string): string | null {
  const normalized = key.trim().toUpperCase()
  if (!normalized) return null
  if (normalized === 'ENTER') return 'Enter'
  if (normalized === 'TAB') return 'Tab'
  if (normalized === 'SPACE') return 'Space'
  if (/^[A-Z]$/.test(normalized)) return `Key${normalized}`
  if (/^\d$/.test(normalized)) return `Digit${normalized}`
  if (/^F(?:[1-9]|1[0-2])$/.test(normalized)) return normalized
  return null
}

export function matchesEditorHotkey(binding: HotkeyDto, event: HotkeyEventLike): boolean {
  if (binding.enabled === false) return false
  const code = hotkeyCode(binding.key)
  if (code === null || event.code !== code) return false

  const modifiers = new Set(binding.modifiers)
  return (
    event.ctrlKey === modifiers.has('ctrl') &&
    event.shiftKey === modifiers.has('shift') &&
    event.altKey === modifiers.has('alt') &&
    event.metaKey === modifiers.has('super')
  )
}
