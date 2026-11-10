import { describe, it, expect, vi, beforeEach } from 'vitest'

const { mockInvoke, mockEditorSettings } = vi.hoisted(() => {
  let _val: unknown = undefined
  return {
    mockInvoke: vi.fn(),
    mockEditorSettings: {
      get value() { return _val },
      set value(v) { _val = v },
    },
  }
})

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mockInvoke,
}))

vi.mock('./useAppSettings', () => ({
  useEditorSettings: () => mockEditorSettings,
}))

import { useSpellcheck } from './useSpellcheck'
import type { AppSettingsDto } from '../types/settings'

function createSettings(overrides?: Partial<AppSettingsDto['editor']>): AppSettingsDto['editor'] {
  return {
    quick: 'disabled',
    ai: false,
    ai_completion: false,
    autocomplete_enabled: true,
    number_conversion_enabled: true,
    spellcheck_enabled: false,
    spellcheck_source: 'online',
    editor_height: 200,
    typing_idle_timeout_ms: 800,
    typing_enabled: true,
    default_route: 'everywhere',
    keep_text_after_send: false,
    font_family: 'default',
    font_size_px: 16,
    homograph_accentor: { enabled: false, accentor_pack_id: null, load_on_start: false },
    ...overrides,
  }
}

describe('useSpellcheck', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockEditorSettings.value = undefined
  })

  it('returns off when editor settings are undefined', () => {
    const { source, enabled } = useSpellcheck()
    expect(source.value).toBe('off')
    expect(enabled.value).toBe(false)
  })

  it('returns off when spellcheck is disabled', () => {
    mockEditorSettings.value = createSettings({ spellcheck_enabled: false })
    const { source, enabled } = useSpellcheck()
    expect(source.value).toBe('off')
    expect(enabled.value).toBe(false)
  })

  it('returns online when enabled and source is online', () => {
    mockEditorSettings.value = createSettings({ spellcheck_enabled: true, spellcheck_source: 'online' })
    const { source, enabled } = useSpellcheck()
    expect(source.value).toBe('online')
    expect(enabled.value).toBe(true)
  })

  it('returns offline when enabled and source is offline', () => {
    mockEditorSettings.value = createSettings({ spellcheck_enabled: true, spellcheck_source: 'offline' })
    const { source, enabled } = useSpellcheck()
    expect(source.value).toBe('offline')
    expect(enabled.value).toBe(true)
  })

  it('returns empty array without invoking when source is off', async () => {
    mockEditorSettings.value = createSettings({ spellcheck_enabled: false })
    const { checkWords } = useSpellcheck()
    const result = await checkWords(['hello'])
    expect(result).toEqual([])
    expect(mockInvoke).not.toHaveBeenCalled()
  })

  it('returns empty array without invoking when word list is empty', async () => {
    mockEditorSettings.value = createSettings({ spellcheck_enabled: true })
    const { checkWords } = useSpellcheck()
    const result = await checkWords([])
    expect(result).toEqual([])
    expect(mockInvoke).not.toHaveBeenCalled()
  })

  it('invokes spellcheck for online source', async () => {
    mockEditorSettings.value = createSettings({ spellcheck_enabled: true, spellcheck_source: 'online' })
    mockInvoke.mockResolvedValue([{ word: 'rust', correct: true, suggestions: [] }])
    const { checkWords } = useSpellcheck()
    const result = await checkWords(['rust'])
    expect(mockInvoke).toHaveBeenCalledTimes(1)
    expect(mockInvoke).toHaveBeenCalledWith('spellcheck', { words: ['rust'] })
    expect(result).toEqual([{ word: 'rust', correct: true, suggestions: [] }])
  })

  it('invokes spellcheck for offline source', async () => {
    mockEditorSettings.value = createSettings({ spellcheck_enabled: true, spellcheck_source: 'offline' })
    mockInvoke.mockResolvedValue([{ word: 'helo', correct: false, suggestions: ['hello'] }])
    const { checkWords } = useSpellcheck()
    const result = await checkWords(['helo'])
    expect(mockInvoke).toHaveBeenCalledTimes(1)
    expect(mockInvoke).toHaveBeenCalledWith('spellcheck', { words: ['helo'] })
    expect(result).toEqual([{ word: 'helo', correct: false, suggestions: ['hello'] }])
  })

  it('computes correct source based on settings', () => {
    mockEditorSettings.value = createSettings({ spellcheck_enabled: true, spellcheck_source: 'online' })
    const first = useSpellcheck()
    expect(first.source.value).toBe('online')
    expect(first.enabled.value).toBe(true)

    mockEditorSettings.value = createSettings({ spellcheck_enabled: false })
    const second = useSpellcheck()
    expect(second.source.value).toBe('off')
    expect(second.enabled.value).toBe(false)
  })

  describe('available state', () => {
    it('starts as available', () => {
      mockEditorSettings.value = createSettings({ spellcheck_enabled: true, spellcheck_source: 'offline' })
      const { available } = useSpellcheck()
      expect(available.value).toBe(true)
    })

    it('sets available to false when invoke fails', async () => {
      mockEditorSettings.value = createSettings({ spellcheck_enabled: true, spellcheck_source: 'offline' })
      mockInvoke.mockRejectedValue(new Error('dictionary unavailable'))
      const { checkWords, available } = useSpellcheck()
      await checkWords(['test'])
      expect(available.value).toBe(false)
    })

    it('returns empty array when invoke fails', async () => {
      mockEditorSettings.value = createSettings({ spellcheck_enabled: true, spellcheck_source: 'offline' })
      mockInvoke.mockRejectedValue(new Error('dictionary unavailable'))
      const { checkWords } = useSpellcheck()
      const result = await checkWords(['test'])
      expect(result).toEqual([])
    })

    it('sets available back to true on next successful check', async () => {
      mockEditorSettings.value = createSettings({ spellcheck_enabled: true, spellcheck_source: 'offline' })
      const { checkWords, available } = useSpellcheck()

      mockInvoke.mockRejectedValueOnce(new Error('dictionary unavailable'))
      await checkWords(['test'])
      expect(available.value).toBe(false)

      mockInvoke.mockResolvedValueOnce([{ word: 'test', correct: true, suggestions: [] }])
      await checkWords(['test'])
      expect(available.value).toBe(true)
    })

    it('does not call invoke when disabled even when unavailable', async () => {
      mockEditorSettings.value = createSettings({ spellcheck_enabled: false })
      const { checkWords, available } = useSpellcheck()
      available.value = false
      const result = await checkWords(['test'])
      expect(result).toEqual([])
      expect(mockInvoke).not.toHaveBeenCalled()
    })
  })
})
