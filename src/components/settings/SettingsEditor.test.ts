// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import SettingsEditor from './SettingsEditor.vue'
import { APP_SETTINGS_KEY, type AppSettingsDto, type EditorSettingsDto } from '../../types/settings'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))

vi.mock('./EditorFontSettings.vue', () => ({
  default: { name: 'EditorFontSettings', render: () => null },
}))

vi.mock('../../composables/useRuAccentRuntime', () => ({
  useRuAccentRuntime: () => ({
    statusFor: () => ({ value: 'not_loaded' }),
    load: vi.fn(async () => {}),
    refreshPacks: vi.fn(async () => []),
    rescanPacks: vi.fn(async () => []),
    packLabelFor: (id: string) => id,
  }),
}))

function deferred<T = void>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

async function flushAsync() {
  for (let i = 0; i < 10; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

function createEditorSettings(overrides?: Partial<EditorSettingsDto>): EditorSettingsDto {
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

function createSettings(numberConversion = true): AppSettingsDto {
  return {
    editor: createEditorSettings({ number_conversion_enabled: numberConversion }),
  } as unknown as AppSettingsDto
}

function getNumberConversionCheckbox(root: HTMLElement): HTMLInputElement {
  const labels = Array.from(root.querySelectorAll<HTMLLabelElement>('label.setting-label'))
  const label = labels.find((entry) => entry.textContent?.includes('Convert numbers to words'))
  const input = label?.querySelector('input[type="checkbox"]') as HTMLInputElement | null
  if (!input) throw new Error('number conversion checkbox not found')
  return input
}

let appInstance: ReturnType<typeof createApp> | null = null
let container: HTMLDivElement | null = null

async function mountEditorSettings(initialValue = true) {
  container = document.createElement('div')
  document.body.appendChild(container)

  const settingsRef = ref<AppSettingsDto | null>(createSettings(initialValue))
  const mockedInvoke = vi.mocked(invoke)
  mockedInvoke.mockResolvedValue(undefined)

  const onShowMessage = vi.fn()

  appInstance = createApp({
    render() {
      return h(SettingsEditor, { onShowMessage })
    },
  })

  appInstance.provide(APP_SETTINGS_KEY, {
    settings: settingsRef,
    isLoading: ref(false),
    error: ref(null),
    reload: vi.fn(async () => {}),
  })

  appInstance.mount(container)

  await flushAsync()
  mockedInvoke.mockClear()

  return {
    container,
    settingsRef,
    mockedInvoke,
    onShowMessage,
    getCheckbox: () => getNumberConversionCheckbox(container!),
    async unmount() {
      if (appInstance) {
        appInstance.unmount()
        appInstance = null
      }
      if (container) {
        container.remove()
        container = null
      }
      await flushAsync()
    },
  }
}

afterEach(async () => {
  if (appInstance) {
    appInstance.unmount()
    appInstance = null
  }
  if (container) {
    container.remove()
    container = null
  }
  vi.clearAllMocks()
  document.body.innerHTML = ''
})

describe('SettingsEditor number conversion', () => {
  it('starts enabled and starts a single save during rapid changes', async () => {
    const { getCheckbox, mockedInvoke, onShowMessage } = await mountEditorSettings(true)
    const checkbox = getCheckbox()
    expect(checkbox.checked).toBe(true)
    expect(checkbox.disabled).toBe(false)

    const gate = deferred()
    mockedInvoke.mockImplementationOnce(() => gate.promise)

    checkbox.click()
    await nextTick()

    expect(checkbox.checked).toBe(false)
    expect(checkbox.disabled).toBe(true)

    checkbox.click()
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledTimes(1)
    expect(mockedInvoke).toHaveBeenCalledWith('set_editor_number_conversion_enabled', { enabled: false })

    gate.resolve()
    await flushAsync()

    expect(checkbox.disabled).toBe(false)
    expect(checkbox.checked).toBe(false)
    expect(onShowMessage).toHaveBeenCalledWith(expect.any(String), 'success')
  })

  it('rolls back on save failure, reports an error and permits retry', async () => {
    const { getCheckbox, mockedInvoke, onShowMessage } = await mountEditorSettings(true)
    const checkbox = getCheckbox()

    mockedInvoke.mockRejectedValueOnce(new Error('disk full'))

    checkbox.click()
    await flushAsync()

    expect(checkbox.checked).toBe(true)
    expect(checkbox.disabled).toBe(false)
    expect(onShowMessage).toHaveBeenCalledWith(expect.any(String), 'error')
    expect(mockedInvoke).toHaveBeenCalledWith('set_editor_number_conversion_enabled', { enabled: false })

    mockedInvoke.mockResolvedValueOnce(undefined)
    checkbox.click()
    await flushAsync()

    expect(checkbox.checked).toBe(false)
    expect(mockedInvoke).toHaveBeenLastCalledWith('set_editor_number_conversion_enabled', { enabled: false })
  })

  it('does not let a stale settings event overwrite a pending choice', async () => {
    const { getCheckbox, settingsRef, mockedInvoke } = await mountEditorSettings(true)
    const checkbox = getCheckbox()

    const gate = deferred()
    mockedInvoke.mockImplementationOnce(() => gate.promise)

    checkbox.click()
    await nextTick()
    expect(checkbox.checked).toBe(false)

    settingsRef.value = createSettings(true)
    await flushAsync()
    expect(checkbox.checked).toBe(false)

    gate.resolve()
    await flushAsync()

    settingsRef.value = createSettings(false)
    await flushAsync()
    expect(checkbox.checked).toBe(false)
  })

  it('syncs a non-pending external update and unmounts cleanly', async () => {
    const { getCheckbox, settingsRef, unmount } = await mountEditorSettings(true)
    const checkbox = getCheckbox()

    settingsRef.value = createSettings(false)
    await flushAsync()
    expect(checkbox.checked).toBe(false)

    await unmount()
    expect(document.body.textContent).not.toContain('Convert numbers to words')
  })
})
