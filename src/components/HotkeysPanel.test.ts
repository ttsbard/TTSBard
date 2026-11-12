// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, ref } from 'vue'
import HotkeysPanel from './HotkeysPanel.vue'
import { invoke } from '@tauri-apps/api/core'
import { APP_SETTINGS_KEY, type AppSettingsDto } from '../types/settings'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
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

function dispatchKey(
  type: 'keydown' | 'keyup',
  eventInit: {
    key: string
    code: string
    ctrlKey?: boolean
    shiftKey?: boolean
    altKey?: boolean
    metaKey?: boolean
  },
) {
  const event = new KeyboardEvent(type, {
    key: eventInit.key,
    code: eventInit.code,
    ctrlKey: eventInit.ctrlKey ?? false,
    shiftKey: eventInit.shiftKey ?? false,
    altKey: eventInit.altKey ?? false,
    metaKey: eventInit.metaKey ?? false,
    bubbles: true,
    cancelable: true,
  })
  document.dispatchEvent(event)
}

function createInitialSettings(hotkeysOverrides?: Record<string, unknown>): AppSettingsDto {
  return {
    hotkeys: {
      main_window: { key: 'M', modifiers: ['ctrl'] },
      sound_panel: { key: 'F', modifiers: ['shift', 'ctrl'] },
      playback_control_window: { key: 'P', modifiers: ['ctrl'] },
      return_previous_window: { key: 'B', modifiers: ['ctrl'] },
      toggle_minimal_mode: { key: 'N', modifiers: ['ctrl'] },
      ocr_capture: { key: 'O', modifiers: ['ctrl'] },
      editor: {
        edit_word: { key: 'E', modifiers: ['ctrl'] },
        submit_continue: { key: 'J', modifiers: ['ctrl'] },
        submit_keep_text: { key: 'K', modifiers: ['ctrl', 'shift'] },
        submit_keep_focus: { key: 'L', modifiers: ['alt'] },
        next_spelling_error: { key: 'F8', modifiers: [] },
        previous_spelling_error: { key: 'F8', modifiers: ['shift'] },
        next_tab: { key: 'Tab', modifiers: ['ctrl'] },
        previous_tab: { key: 'Tab', modifiers: ['ctrl', 'shift'] },
        cycle_route: { key: 'R', modifiers: ['ctrl'] },
        toggle_typing: { key: 'T', modifiers: ['ctrl'] },
        cycle_quick_mode: { key: 'Q', modifiers: ['ctrl'] },
        toggle_history: { key: 'H', modifiers: ['ctrl'] },
        accent_homographs: { key: 'A', modifiers: ['ctrl'] },
        approve_next_incoming: { key: 'Y', modifiers: ['ctrl'] },
        edit_next_incoming: { key: 'I', modifiers: ['ctrl'] },
      },
      ...hotkeysOverrides,
    },
  } as unknown as AppSettingsDto
}

let appInstance: ReturnType<typeof createApp> | null = null
let container: HTMLDivElement | null = null

async function mountHotkeysPanel(options: {
  active?: boolean
  hotkeys?: Record<string, unknown>
} = {}) {
  container = document.createElement('div')
  document.body.appendChild(container)

  const activeRef = ref(options.active ?? true)
  const settingsRef = ref<AppSettingsDto | null>(createInitialSettings(options.hotkeys))
  const mockedInvoke = vi.mocked(invoke)
  mockedInvoke.mockResolvedValue(undefined)

  appInstance = createApp({
    render() {
      return h(HotkeysPanel, { active: activeRef.value })
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
    activeRef,
    settingsRef,
    mockedInvoke,
    getMainRecordBtn: () => container?.querySelector('.record-btn') as HTMLButtonElement | null,
    getRecordingIndicator: () => container?.querySelector('.hotkey-value.recording') as HTMLDivElement | null,
    getCancelBtn: () => container?.querySelector('.cancel-btn') as HTMLButtonElement | null,
    getErrorMessage: () => container?.querySelector('.message-box') as HTMLDivElement | null,
    getToggle: (name: string) => container?.querySelector(`input[data-hotkey="${name}"]`) as HTMLInputElement | null,
    getRow: (name: string) => container?.querySelector(`input[data-hotkey="${name}"]`)?.closest('.hotkey-row') as HTMLDivElement | null,
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
  vi.clearAllTimers()
  vi.useRealTimers()
  vi.clearAllMocks()
  document.body.innerHTML = ''
})

describe('HotkeysPanel mounted lifecycle and recording', () => {
  it('keeps recording after a duplicate, clears its notice, and accepts another shortcut', async () => {
    vi.useFakeTimers()
    try {
      const { getMainRecordBtn, getRecordingIndicator, mockedInvoke } = await mountHotkeysPanel({
        hotkeys: {
          sound_panel: { key: 'F', modifiers: ['shift', 'ctrl'] },
        },
      })

      getMainRecordBtn()!.click()
      await flushAsync()
      expect(getRecordingIndicator()).not.toBeNull()

      // Press duplicate Ctrl+Shift+F
      dispatchKey('keydown', { key: 'F', code: 'KeyF', ctrlKey: true, shiftKey: true })
      dispatchKey('keyup', { key: 'F', code: 'KeyF', ctrlKey: true, shiftKey: true })
      await flushAsync()

      expect(getRecordingIndicator()?.textContent).toContain('Already assigned')
      expect(mockedInvoke).not.toHaveBeenCalledWith('set_hotkey', expect.anything())

      // Advance 2s to clear occupied notice
      vi.advanceTimersByTime(2000)
      await flushAsync()
      expect(getRecordingIndicator()).not.toBeNull()
      expect(getRecordingIndicator()?.textContent).not.toContain('Already assigned')
      expect(mockedInvoke).not.toHaveBeenCalledWith('set_hotkey', expect.anything())

      // Press another shortcut Ctrl+Shift+G
      dispatchKey('keydown', { key: 'G', code: 'KeyG', ctrlKey: true, shiftKey: true })
      dispatchKey('keyup', { key: 'G', code: 'KeyG', ctrlKey: false, shiftKey: false })
      await flushAsync()

      expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey', {
        name: 'main_window',
        hotkey: { key: 'G', modifiers: ['ctrl', 'shift'], enabled: true },
      })
    } finally {
      vi.useRealTimers()
    }
  })

  it('cancels recording when the panel is hidden', async () => {
    const { getMainRecordBtn, getRecordingIndicator, activeRef, mockedInvoke } = await mountHotkeysPanel()
    getMainRecordBtn()!.click()
    await flushAsync()
    expect(getRecordingIndicator()).not.toBeNull()

    activeRef.value = false
    await flushAsync()

    expect(getRecordingIndicator()).toBeNull()
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')

    mockedInvoke.mockClear()
    dispatchKey('keydown', { key: 'R', code: 'KeyR', ctrlKey: true, shiftKey: true })
    dispatchKey('keyup', { key: 'R', code: 'KeyR' })
    await flushAsync()
    expect(mockedInvoke).not.toHaveBeenCalledWith('set_hotkey', expect.anything())
  })

  it('does not start recording after leaving during IPC setup', async () => {
    const { getMainRecordBtn, getRecordingIndicator, activeRef, mockedInvoke } = await mountHotkeysPanel()
    const gate = deferred()
    mockedInvoke.mockImplementationOnce(async (cmd) => {
      if (cmd === 'set_hotkey_recording') return gate.promise
      return undefined
    })

    getMainRecordBtn()!.click()
    await nextTick()

    activeRef.value = false
    await nextTick()

    gate.resolve()
    await flushAsync()

    expect(getRecordingIndicator()).toBeNull()
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')
  })

  it.each([
    ['К', 'KeyR', 'R'],
    ['R', 'KeyR', 'R'],
    ['!', 'Digit1', '1'],
    ['F3', 'F3', 'F3'],
    [' ', 'Space', 'SPACE'],
    ['Enter', 'Enter', 'Enter'],
  ])('records physical %s / %s as %s', async (key, code, expected) => {
    const { getMainRecordBtn, getRecordingIndicator, mockedInvoke } = await mountHotkeysPanel()
    getMainRecordBtn()!.click()
    await flushAsync()
    expect(getRecordingIndicator()).not.toBeNull()

    dispatchKey('keydown', { key, code, ctrlKey: true, shiftKey: true })
    dispatchKey('keyup', { key: key.toLowerCase(), code, ctrlKey: false, shiftKey: false })
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey', {
      name: 'main_window',
      hotkey: { modifiers: ['ctrl', 'shift'], key: expected, enabled: true },
    })
  })

  it.each([
    ['Control', 'ControlLeft'],
    [';', 'Semicolon'],
  ])('ignores %s', async (key, code) => {
    const { getMainRecordBtn, mockedInvoke } = await mountHotkeysPanel()
    getMainRecordBtn()!.click()
    await flushAsync()

    dispatchKey('keydown', { key, code })
    dispatchKey('keyup', { key, code })
    await flushAsync()

    expect(mockedInvoke).not.toHaveBeenCalledWith('set_hotkey', expect.anything())
  })

  it('cancels on Escape and restores registration', async () => {
    const { getMainRecordBtn, getRecordingIndicator, mockedInvoke } = await mountHotkeysPanel()
    getMainRecordBtn()!.click()
    await flushAsync()
    expect(getRecordingIndicator()).not.toBeNull()

    dispatchKey('keydown', { key: 'Escape', code: 'Escape' })
    await flushAsync()

    expect(getRecordingIndicator()).toBeNull()
    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')
    expect(mockedInvoke).not.toHaveBeenCalledWith('set_hotkey', expect.anything())
  })

  it.each(['Этот хоткей уже используется', new Error('Registration failed')])(
    'shows IPC error details: %s',
    async (error) => {
      const { getMainRecordBtn, getErrorMessage, mockedInvoke } = await mountHotkeysPanel()
      getMainRecordBtn()!.click()
      await flushAsync()

      mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'set_hotkey') throw error
        return undefined
      })

      dispatchKey('keydown', { key: 'К', code: 'KeyR', ctrlKey: true, shiftKey: true })
      dispatchKey('keyup', { key: 'К', code: 'KeyR' })
      await flushAsync()

      const detail = typeof error === 'string' ? error : error.message
      expect(getErrorMessage()?.textContent).toContain(detail)
      expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')
    },
  )

  it('unmount cleans up listeners and stops in-flight session', async () => {
    const { getMainRecordBtn, getRecordingIndicator, unmount, mockedInvoke } = await mountHotkeysPanel()
    getMainRecordBtn()!.click()
    await flushAsync()
    expect(getRecordingIndicator()).not.toBeNull()

    await unmount()

    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')

    mockedInvoke.mockClear()
    dispatchKey('keydown', { key: 'R', code: 'KeyR', ctrlKey: true, shiftKey: true })
    dispatchKey('keyup', { key: 'R', code: 'KeyR' })
    await flushAsync()
    expect(mockedInvoke).not.toHaveBeenCalledWith('set_hotkey', expect.anything())
  })
})

describe('HotkeysPanel per-hotkey enable switch', () => {
  it('renders one accessible toggle per hotkey row', async () => {
    const { container } = await mountHotkeysPanel()
    const toggles = container.querySelectorAll('.toggle-switch input')

    expect(toggles).toHaveLength(21)
    for (const toggle of Array.from(toggles)) {
      expect(toggle.getAttribute('aria-label')).toBeTruthy()
    }
    expect((toggles[0] as HTMLInputElement).getAttribute('data-hotkey')).toBe('main_window')
  })

  it('dims a disabled binding and disables both action buttons', async () => {
    const { getRow, getToggle } = await mountHotkeysPanel({
      hotkeys: {
        main_window: { key: 'M', modifiers: ['ctrl'], enabled: false },
      },
    })

    const toggle = getToggle('main_window')!
    const row = getRow('main_window')!
    expect(toggle.checked).toBe(false)
    expect(row.querySelector('.hotkey-value')?.classList.contains('disabled')).toBe(true)
    expect((row.querySelector('.record-btn') as HTMLButtonElement).disabled).toBe(true)
    expect((row.querySelector('.reset-btn') as HTMLButtonElement).disabled).toBe(true)
  })

  it('persists a global toggle without changing the combination', async () => {
    const { getToggle, mockedInvoke } = await mountHotkeysPanel()
    const toggle = getToggle('sound_panel')!

    toggle.checked = false
    toggle.dispatchEvent(new Event('change'))
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_binding_enabled', {
      name: 'sound_panel',
      enabled: false,
    })
  })

  it('persists an editor toggle through the editor command', async () => {
    const { getToggle, mockedInvoke } = await mountHotkeysPanel()
    const toggle = getToggle('edit_word')!

    toggle.checked = false
    toggle.dispatchEvent(new Event('change'))
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledWith('set_editor_hotkey_enabled', {
      actionId: 'edit_word',
      enabled: false,
    })
  })

  it('shows a warning when disabling ocr_capture while OCR is enabled', async () => {
    const { getToggle, getErrorMessage, settingsRef } = await mountHotkeysPanel()
    settingsRef.value = {
      ...(settingsRef.value as AppSettingsDto),
      ocr: { enabled: true, model_id: null, capture_target: { type: 'all' } },
    } as AppSettingsDto
    await flushAsync()

    const toggle = getToggle('ocr_capture')!
    toggle.checked = false
    toggle.dispatchEvent(new Event('change'))
    await flushAsync()

    const message = getErrorMessage()
    expect(message?.classList.contains('warning')).toBe(true)
    expect(message?.textContent).toContain('OCR')
  })

  it('reverts the switch and shows the conflict when the backend rejects enabling', async () => {
    const { getToggle, getErrorMessage, mockedInvoke } = await mountHotkeysPanel({
      hotkeys: {
        main_window: { key: 'M', modifiers: ['ctrl'], enabled: false },
      },
    })
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_hotkey_binding_enabled') {
        throw new Error('Этот хоткей уже используется для главного окна')
      }
      return undefined
    })

    const toggle = getToggle('main_window')!
    toggle.checked = true
    toggle.dispatchEvent(new Event('change'))
    await flushAsync()

    expect(toggle.checked).toBe(false)
    expect(getErrorMessage()?.classList.contains('error')).toBe(true)
    expect(getErrorMessage()?.textContent).toContain('Этот хоткей уже используется')
  })

  it('does not treat a disabled hotkey as occupied while recording', async () => {
    const { getMainRecordBtn, getRecordingIndicator, mockedInvoke } = await mountHotkeysPanel({
      hotkeys: {
        sound_panel: { key: 'G', modifiers: ['ctrl', 'shift'], enabled: false },
      },
    })

    getMainRecordBtn()!.click()
    await flushAsync()
    expect(getRecordingIndicator()).not.toBeNull()

    dispatchKey('keydown', { key: 'G', code: 'KeyG', ctrlKey: true, shiftKey: true })
    dispatchKey('keyup', { key: 'G', code: 'KeyG' })
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey', {
      name: 'main_window',
      hotkey: { key: 'G', modifiers: ['ctrl', 'shift'], enabled: true },
    })
  })
})
