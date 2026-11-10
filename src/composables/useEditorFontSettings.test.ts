import { describe, it, expect, vi, beforeEach, beforeAll } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'

const { mockInvoke } = vi.hoisted(() => ({ mockInvoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mockInvoke,
}))

import { useEditorFontSettings } from './useEditorFontSettings'
import type { EditorSettingsDto } from '../types/settings'
import { i18n } from '../i18n'
import ruCatalog from '../../locales/ru.json'

beforeAll(() => {
  i18n.global.setLocaleMessage('ru', (ruCatalog as { messages: Record<string, string> }).messages)
  ;(i18n.global.locale as unknown as { value: string }).value = 'ru'
})

function createSettings(overrides?: Partial<EditorSettingsDto>): EditorSettingsDto {
  return {
    quick: 'disabled',
    ai: false,
    ai_completion: false,
    autocomplete_enabled: true,
    number_conversion_enabled: true,
    spellcheck_enabled: true,
    spellcheck_source: 'offline',
    editor_height: 340,
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

/** Calls the composable made to one IPC command, ignoring the catalog fetch. */
function invokeCalls(command: string) {
  return mockInvoke.mock.calls.filter(([name]) => name === command)
}

describe('useEditorFontSettings', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    // The composable fetches the font catalog on creation; give every test a
    // sane default so unmocked calls never return undefined.
    mockInvoke.mockImplementation((command: unknown) =>
      Promise.resolve(command === 'get_system_font_families' ? [] : undefined),
    )
  })

  it('adopts initial font family and size from settings', () => {
    const source = ref<EditorSettingsDto | undefined>(
      createSettings({ font_family: 'georgia', font_size_px: 20 }),
    )
    const c = useEditorFontSettings(source)
    expect(c.family.value).toBe('georgia')
    expect(c.sizeInput.value).toBe(20)
  })

  it('falls back to defaults while settings are still loading', () => {
    const source = ref<EditorSettingsDto | undefined>(undefined)
    const c = useEditorFontSettings(source)
    expect(c.family.value).toBe('default')
    expect(c.sizeInput.value).toBe(16)
  })

  it('adopts settings once they load after mount', async () => {
    const source = ref<EditorSettingsDto | undefined>(undefined)
    const c = useEditorFontSettings(source)
    source.value = createSettings({ font_family: 'consolas', font_size_px: 24 })
    await nextTick()
    expect(c.family.value).toBe('consolas')
    expect(c.sizeInput.value).toBe(24)
  })

  it('saves a family change through IPC and updates the confirmed value', async () => {
    mockInvoke.mockResolvedValue('arial')
    const c = useEditorFontSettings(ref(createSettings()))
    await c.onFamilyChange('arial')
    expect(invokeCalls('set_editor_font_family')).toEqual([
      ['set_editor_font_family', { family: 'arial' }],
    ])
    expect(c.family.value).toBe('arial')
    expect(c.saving.value).toBe(false)
    expect(c.saveError.value).toBeNull()
  })

  it('saves an arbitrary catalog family through IPC with a CSS fallback', async () => {
    mockInvoke.mockResolvedValue('PT Sans')
    const c = useEditorFontSettings(ref(createSettings()))
    await c.onFamilyChange('PT Sans')
    expect(invokeCalls('set_editor_font_family')).toEqual([
      ['set_editor_font_family', { family: 'PT Sans' }],
    ])
    expect(c.family.value).toBe('PT Sans')
    expect(c.previewFontFamily.value).toBe("'PT Sans', var(--font-mono)")
  })

  it('saves a size change through IPC with a sizePx payload', async () => {
    mockInvoke.mockResolvedValue(96)
    const c = useEditorFontSettings(ref(createSettings()))
    await c.onSizeChange(96)
    expect(invokeCalls('set_editor_font_size')).toEqual([['set_editor_font_size', { sizePx: 96 }]])
    expect(c.sizeInput.value).toBe(96)
  })

  it('accepts a numeric string size and normalizes it', async () => {
    mockInvoke.mockResolvedValue(96)
    const c = useEditorFontSettings(ref(createSettings()))
    await c.onSizeChange('96')
    expect(mockInvoke).toHaveBeenCalledWith('set_editor_font_size', { sizePx: 96 })
    expect(c.sizeInput.value).toBe(96)
  })

  describe('invalid size input never invokes IPC', () => {
    it.each([
      ['empty', ''],
      ['non-numeric', 'abc'],
      ['below range', 11],
      ['above range', 97],
      ['fraction number', 16.5],
      ['fraction string', '16.5'],
      ['null', null],
    ])('%s', async (_name, raw) => {
      const source = ref(createSettings({ font_size_px: 16 }))
      const c = useEditorFontSettings(source)
      await c.onSizeChange(raw)
      expect(invokeCalls('set_editor_font_size')).toHaveLength(0)
      expect(c.sizeInput.value).toBe(16)
      expect(c.saveError.value).not.toBeNull()
      expect(c.saving.value).toBe(false)
    })
  })

  it('restores the confirmed value and shows a message on family save error', async () => {
    mockInvoke.mockRejectedValue(new Error('backend busy'))
    const source = ref(createSettings({ font_family: 'default' }))
    const c = useEditorFontSettings(source)
    await c.onFamilyChange('arial')
    expect(c.family.value).toBe('default')
    expect(c.saveError.value).toContain('backend busy')
    expect(c.saving.value).toBe(false)
  })

  it('restores the confirmed value and shows a message on size save error', async () => {
    mockInvoke.mockRejectedValue(new Error('backend busy'))
    const source = ref(createSettings({ font_size_px: 16 }))
    const c = useEditorFontSettings(source)
    await c.onSizeChange(28)
    expect(c.sizeInput.value).toBe(16)
    expect(c.saveError.value).toContain('backend busy')
    expect(c.saving.value).toBe(false)
  })

  it('allows a retry as the next edit after an error', async () => {
    const c = useEditorFontSettings(ref(createSettings()))
    mockInvoke.mockRejectedValueOnce(new Error('first fails'))
    mockInvoke.mockResolvedValueOnce('arial')
    await c.onFamilyChange('arial')
    expect(c.family.value).toBe('default')

    await c.onFamilyChange('arial')
    expect(c.family.value).toBe('arial')
    expect(invokeCalls('set_editor_font_family')).toHaveLength(2)
  })

  it('guards against rapid changes while a command is pending', async () => {
    let resolveFamily: ((value: string) => void) | undefined
    mockInvoke.mockImplementation(
      () =>
        new Promise<string>((resolve) => {
          resolveFamily = resolve
        }),
    )

    const c = useEditorFontSettings(ref(createSettings()))
    const first = c.onFamilyChange('arial')
    expect(c.saving.value).toBe(true)

    await c.onFamilyChange('georgia')
    await c.onSizeChange(96)

    expect(invokeCalls('set_editor_font_family')).toHaveLength(1)
    expect(invokeCalls('set_editor_font_size')).toHaveLength(0)
    expect(c.saving.value).toBe(true)

    resolveFamily?.('arial')
    await first

    expect(c.saving.value).toBe(false)
    expect(c.family.value).toBe('arial')
    expect(c.sizeInput.value).toBe(16)
  })

  it('ignores a stale settings event that would undo a newer successful edit', async () => {
    mockInvoke.mockResolvedValue('arial')
    const source = ref<EditorSettingsDto | undefined>(
      createSettings({ font_family: 'default', font_size_px: 16 }),
    )
    const c = useEditorFontSettings(source)

    await c.onFamilyChange('arial')
    expect(c.family.value).toBe('arial')

    source.value = createSettings({ font_family: 'default', font_size_px: 16 })
    await nextTick()

    expect(c.family.value).toBe('arial')
  })

  it('ignores a stale size event that would undo a newer successful edit', async () => {
    mockInvoke.mockResolvedValue(24)
    const source = ref<EditorSettingsDto | undefined>(
      createSettings({ font_family: 'default', font_size_px: 16 }),
    )
    const c = useEditorFontSettings(source)

    await c.onSizeChange(24)
    expect(c.sizeInput.value).toBe(24)

    source.value = createSettings({ font_family: 'default', font_size_px: 16 })
    await nextTick()

    expect(c.sizeInput.value).toBe(24)
  })

  it('still adopts unrelated field changes before any local confirmation', async () => {
    mockInvoke.mockResolvedValue('arial')
    const source = ref<EditorSettingsDto | undefined>(
      createSettings({ font_family: 'default', font_size_px: 16 }),
    )
    const c = useEditorFontSettings(source)

    // Confirm only the family locally, then a settings event changes the size.
    await c.onFamilyChange('arial')
    source.value = createSettings({ font_family: 'arial', font_size_px: 28 })
    await nextTick()

    expect(c.family.value).toBe('arial')
    expect(c.sizeInput.value).toBe(28)
  })

  it('exposes the preview font stack and size', () => {
    const source = ref<EditorSettingsDto | undefined>(
      createSettings({ font_family: 'arial', font_size_px: 18 }),
    )
    const c = useEditorFontSettings(source)
    expect(c.previewFontFamily.value).toBe('Arial, sans-serif')
    expect(c.previewFontSize.value).toBe(18)
  })

  it('accepts later external updates after the saved values are acknowledged', async () => {
    const source = ref(createSettings())
    const c = useEditorFontSettings(source)
    mockInvoke.mockResolvedValueOnce('arial').mockResolvedValueOnce(24)
    await c.onFamilyChange('arial')
    await c.onSizeChange(24)
    source.value = createSettings({ font_family: 'arial', font_size_px: 24 })
    await nextTick()
    source.value = createSettings({ font_family: 'georgia', font_size_px: 28 })
    await nextTick()
    expect(c.family.value).toBe('georgia')
    expect(c.sizeInput.value).toBe(28)
  })

  it('reconciles settings received during a pending save, including the other field', async () => {
    let resolveSave!: (value: string) => void
    mockInvoke.mockImplementation(() => new Promise<string>(resolve => { resolveSave = resolve }))
    const source = ref(createSettings())
    const c = useEditorFontSettings(source)
    const pending = c.onFamilyChange('arial')
    source.value = createSettings({ font_family: 'arial', font_size_px: 28 })
    await nextTick()
    resolveSave('arial')
    await pending
    expect(c.sizeInput.value).toBe(28)
    source.value.font_family = 'consolas'
    await nextTick()
    expect(c.family.value).toBe('consolas')
  })

  it('does not publish late failures after disposal', async () => {
    let rejectSave!: (error: Error) => void
    mockInvoke.mockImplementation(() => new Promise((_resolve, reject) => { rejectSave = reject }))
    const scope = effectScope()
    const c = scope.run(() => useEditorFontSettings(ref(createSettings())))!
    const pending = c.onFamilyChange('arial')
    scope.stop()
    rejectSave(new Error('late error'))
    await pending
    expect(c.saveError.value).toBeNull()
    expect(c.family.value).toBe('arial')
  })

  it('exposes the quick options followed by the system catalog', async () => {
    mockInvoke.mockResolvedValue(['Arial', 'default', 'PT Sans', 'system'])
    const c = useEditorFontSettings(ref(createSettings()))
    await vi.waitFor(() => {
      expect(c.fontOptions.value.map((o) => o.id)).toEqual(['default', 'system', 'Arial', 'PT Sans'])
      expect(c.fontOptions.value.map((o) => o.label)).toEqual([
        'По умолчанию',
        'Системный',
        'Arial',
        'PT Sans',
      ])
    })
  })

  it('keeps only the quick options when the catalog is unavailable', async () => {
    mockInvoke.mockRejectedValue(new Error('catalog unavailable'))
    const c = useEditorFontSettings(ref(createSettings()))
    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('get_system_font_families')
    })
    expect(c.fontOptions.value.map((o) => o.id)).toEqual(['default', 'system'])
  })

  it('shows a saved family that is no longer in the catalog', async () => {
    mockInvoke.mockResolvedValue(['Arial'])
    const c = useEditorFontSettings(ref(createSettings({ font_family: 'PT Sans' })))
    await vi.waitFor(() => {
      expect(c.fontOptions.value.map((o) => o.id)).toEqual(['default', 'system', 'Arial'])
    })
    expect(c.family.value).toBe('PT Sans')
    expect(c.previewFontFamily.value).toBe("'PT Sans', var(--font-mono)")
  })

  it('reset restores both fields through the regular setters', async () => {
    mockInvoke.mockImplementation((command: unknown) =>
      Promise.resolve(
        command === 'get_system_font_families'
          ? []
          : command === 'set_editor_font_size'
            ? 16
            : 'default',
      ),
    )
    const c = useEditorFontSettings(
      ref(createSettings({ font_family: 'georgia', font_size_px: 24 })),
    )
    await c.onReset()
    expect(invokeCalls('set_editor_font_family')).toEqual([
      ['set_editor_font_family', { family: 'default' }],
    ])
    expect(invokeCalls('set_editor_font_size')).toEqual([['set_editor_font_size', { sizePx: 16 }]])
    expect(c.family.value).toBe('default')
    expect(c.sizeInput.value).toBe(16)
    expect(c.saveError.value).toBeNull()
  })

  it('reset does nothing when both fields are already at their defaults', async () => {
    const c = useEditorFontSettings(ref(createSettings()))
    await c.onReset()
    expect(invokeCalls('set_editor_font_family')).toHaveLength(0)
    expect(invokeCalls('set_editor_font_size')).toHaveLength(0)
  })

  it('reset keeps the failed field and still resets the other one', async () => {
    mockInvoke.mockImplementation((command: unknown) =>
      command === 'set_editor_font_family'
        ? Promise.reject(new Error('disk failure'))
        : command === 'set_editor_font_size'
          ? Promise.resolve(16)
          : Promise.resolve([]),
    )
    const c = useEditorFontSettings(
      ref(createSettings({ font_family: 'georgia', font_size_px: 24 })),
    )
    await c.onReset()
    expect(c.family.value).toBe('georgia')
    expect(c.saveError.value).toContain('disk failure')
    expect(c.sizeInput.value).toBe(16)
    expect(c.saving.value).toBe(false)
  })
})
