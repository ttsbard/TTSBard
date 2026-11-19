// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, ref } from 'vue'
import HotkeysPanel from './HotkeysPanel.vue'
import { invoke } from '@tauri-apps/api/core'
import { t } from '../i18n'
import { APP_SETTINGS_KEY, type AppSettingsDto } from '../types/settings'
import {
  isMainWindowOutputControlsRecording,
  setMainWindowOutputControlsRecording,
} from '../composables/useMainWindowOutputControls'

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
        toggle_virtual_mic: { key: '', modifiers: [] },
        toggle_vban: { key: '', modifiers: [] },
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
  onReload?: () => void
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
    reload: vi.fn(async () => {
      options.onReload?.()
    }),
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
  setMainWindowOutputControlsRecording('hotkeys-panel', false)
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

    expect(toggles).toHaveLength(23)
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

  it('labels the default-off editor actions with action verbs and contextual hints', async () => {
    const { getRow } = await mountHotkeysPanel()
    const rows = [
      ['cycle_route', 'Cycle the send route', 'Everywhere → No Twitch'],
      ['toggle_typing', 'Toggle typing status sharing', 'WebView, VTube Studio and VRChat'],
      ['cycle_quick_mode', 'Cycle quick editor mode', 'the window hides after sending'],
    ] as const

    for (const [name, label, hintFragment] of rows) {
      const row = getRow(name)!
      expect(row.querySelector('.hotkey-label')?.textContent).toContain(label)

      const hint = row.querySelector('.hotkey-hint') as HTMLElement | null
      expect(hint?.classList.contains('ui-hint')).toBe(true)
      expect(hint?.textContent).toContain(hintFragment)
      expect(row.querySelector(`input[data-hotkey="${name}"]`)?.getAttribute('aria-describedby'))
        .toBe(`hotkey-hint-${name}`)
    }
  })

  it('renders backend default-off editor actions as switched off with actions disabled', async () => {
    const { getToggle, getRow, settingsRef } = await mountHotkeysPanel()
    const editor = (settingsRef.value as AppSettingsDto).hotkeys!.editor
    const defaultOff = ['cycle_route', 'toggle_typing', 'cycle_quick_mode'] as const

    for (const name of defaultOff) {
      editor[name] = { ...editor[name], enabled: false }
    }
    await flushAsync()

    for (const name of defaultOff) {
      expect(getToggle(name)!.checked).toBe(false)
      const row = getRow(name)!
      expect(row.querySelector('.hotkey-value')?.classList.contains('disabled')).toBe(true)
      expect((row.querySelector('.record-btn') as HTMLButtonElement).disabled).toBe(true)
      expect((row.querySelector('.reset-btn') as HTMLButtonElement).disabled).toBe(true)
    }
  })

  it('persists enabling a default-off editor action without changing its combination', async () => {
    const { getToggle, settingsRef, mockedInvoke } = await mountHotkeysPanel()
    const editor = (settingsRef.value as AppSettingsDto).hotkeys!.editor
    editor.cycle_route = { ...editor.cycle_route, enabled: false }
    await flushAsync()

    const toggle = getToggle('cycle_route')!
    expect(toggle.checked).toBe(false)

    toggle.checked = true
    toggle.dispatchEvent(new Event('change'))
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledWith('set_editor_hotkey_enabled', {
      actionId: 'cycle_route',
      enabled: true,
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
    expect(message?.classList.contains('ui-notice--warning')).toBe(true)
    // Only the shortcut is released: the notice must not claim that OCR itself
    // became unavailable.
    expect(message?.textContent).toContain(
      t('hotkeys.ocr_capture_disabled', { name: t('hotkeys.name.ocr_capture') }),
    )
    expect(message?.textContent).not.toContain(t('ocr.failure.runtime_unavailable'))
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

describe('HotkeysPanel NumPad capture and save-in-flight', () => {
  it('records a bare NumPad digit with an empty modifier list', async () => {
    const { getMainRecordBtn, getRecordingIndicator, mockedInvoke } = await mountHotkeysPanel()
    getMainRecordBtn()!.click()
    await flushAsync()
    expect(getRecordingIndicator()).not.toBeNull()

    dispatchKey('keydown', { key: '7', code: 'Numpad7' })
    await flushAsync()
    expect(getRecordingIndicator()?.textContent).toContain('Num 7')

    dispatchKey('keyup', { key: '7', code: 'Numpad7' })
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey', {
      name: 'main_window',
      hotkey: { key: 'NUMPAD7', modifiers: [], enabled: true },
    })
  })

  it('keeps a captured NumPad binding visible while the save is in flight and swaps it in on success', async () => {
    const gate = deferred<{ key: string; modifiers: string[] } | undefined>()
    let savedHotkey: { key: string; modifiers: string[] } | null = null
    let applySaved: (() => void) | null = null
    const { getMainRecordBtn, getRow, getToggle, getRecordingIndicator, settingsRef, mockedInvoke } = await mountHotkeysPanel({
      onReload: () => applySaved?.(),
    })
    applySaved = () => {
      if (!savedHotkey) return
      settingsRef.value = {
        ...(settingsRef.value as AppSettingsDto),
        hotkeys: {
          ...(settingsRef.value as AppSettingsDto).hotkeys,
          main_window: savedHotkey,
        },
      } as AppSettingsDto
    }

    getMainRecordBtn()!.click()
    await flushAsync()
    expect(getRecordingIndicator()).not.toBeNull()

    mockedInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'set_hotkey') {
        savedHotkey = (args as { hotkey: { key: string; modifiers: string[] } }).hotkey
        return gate.promise
      }
      return undefined
    })

    // Capture NumPad 5 with Ctrl: physical code decides, modifier is real.
    dispatchKey('keydown', { key: '5', code: 'Numpad5', ctrlKey: true })
    dispatchKey('keyup', { key: '5', code: 'Numpad5', ctrlKey: true })
    await nextTick()

    // Between capture and the settled save the row shows the captured value;
    // before the fix this window rendered neither the old nor the new value.
    const row = getRow('main_window')!
    const pending = row.querySelector('.hotkey-value.saving') as HTMLElement | null
    expect(pending).not.toBeNull()
    expect(pending?.textContent).toContain('Ctrl+Num 5')
    expect(row.querySelector('.hotkey-value.recording')).toBeNull()

    // The panel stays locked while the save is in flight.
    expect((getRow('sound_panel')!.querySelector('.record-btn') as HTMLButtonElement).disabled).toBe(true)
    expect(getToggle('sound_panel')!.disabled).toBe(true)

    gate.resolve(undefined)
    await flushAsync()

    // Settled: the confirmed value replaces the pending chip without a blank frame.
    expect(row.querySelector('.hotkey-value.saving')).toBeNull()
    expect(row.querySelector('.hotkey-value')?.textContent).toContain('Ctrl+Num 5')
    expect((getRow('sound_panel')!.querySelector('.record-btn') as HTMLButtonElement).disabled).toBe(false)
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey', {
      name: 'main_window',
      hotkey: { key: 'NUMPAD5', modifiers: ['ctrl'], enabled: true },
    })
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
  })

  it('restores the confirmed value and shows the conflict when an in-flight save fails', async () => {
    const gate = deferred<void>()
    const { getMainRecordBtn, getRow, getErrorMessage, mockedInvoke } = await mountHotkeysPanel()
    getMainRecordBtn()!.click()
    await flushAsync()

    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_hotkey') return gate.promise
      return undefined
    })

    // Ctrl+K is free in the default settings (Ctrl+R is taken by cycle_route).
    dispatchKey('keydown', { key: 'K', code: 'KeyK', ctrlKey: true })
    dispatchKey('keyup', { key: 'K', code: 'KeyK' })
    await nextTick()

    const row = getRow('main_window')!
    expect((row.querySelector('.hotkey-value.saving') as HTMLElement | null)?.textContent).toContain('Ctrl+K')

    gate.reject(new Error('Этот хоткей уже используется для «Перехвата»'))
    await flushAsync()

    // The pending chip is gone, the last confirmed binding is back and the
    // backend conflict stays visible.
    expect(row.querySelector('.hotkey-value.saving')).toBeNull()
    expect(row.querySelector('.hotkey-value')?.textContent).toContain('Ctrl+M')
    expect(getErrorMessage()?.classList.contains('error')).toBe(true)
    expect(getErrorMessage()?.textContent).toContain('Этот хоткей уже используется')
    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
  })

  it('records NumPad on main-window-local and editor rows, ignores NumpadEnter everywhere', async () => {
    const { getRow, getRecordingIndicator, mockedInvoke } = await mountHotkeysPanel()
    const recordInRow = (name: string) => getRow(name)!.querySelector('.record-btn') as HTMLButtonElement

    // Editor rows record NumPad through the editor command.
    recordInRow('edit_word').click()
    await flushAsync()
    dispatchKey('keydown', { key: '5', code: 'Numpad5' })
    dispatchKey('keyup', { key: '5', code: 'Numpad5' })
    await flushAsync()
    expect(mockedInvoke).toHaveBeenCalledWith('set_editor_hotkey', {
      actionId: 'edit_word',
      hotkey: { key: 'NUMPAD5', modifiers: [], enabled: true },
    })
    dispatchKey('keydown', { key: 'Escape', code: 'Escape' })
    await flushAsync()

    // Main-window-local rows record NumPad through the global command.
    recordInRow('toggle_minimal_mode').click()
    await flushAsync()
    // Ctrl+NumPad5: physical code decides, the modifier is real.
    dispatchKey('keydown', { key: '5', code: 'Numpad5', ctrlKey: true })
    dispatchKey('keyup', { key: '5', code: 'Numpad5', ctrlKey: true })
    await flushAsync()
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey', {
      name: 'toggle_minimal_mode',
      hotkey: { key: 'NUMPAD5', modifiers: ['ctrl'], enabled: true },
    })

    // NumpadEnter is excluded from every scope.
    mockedInvoke.mockClear()
    recordInRow('return_previous_window').click()
    await flushAsync()
    dispatchKey('keydown', { key: 'Enter', code: 'NumpadEnter' })
    dispatchKey('keyup', { key: 'Enter', code: 'NumpadEnter' })
    await flushAsync()
    expect(mockedInvoke).not.toHaveBeenCalledWith('set_hotkey', expect.anything())
    dispatchKey('keydown', { key: 'Escape', code: 'Escape' })
    await flushAsync()

    recordInRow('edit_word').click()
    await flushAsync()
    dispatchKey('keydown', { key: 'Enter', code: 'NumpadEnter' })
    dispatchKey('keyup', { key: 'Enter', code: 'NumpadEnter' })
    await flushAsync()
    expect(mockedInvoke).not.toHaveBeenCalledWith('set_editor_hotkey', expect.anything())
    expect(getRecordingIndicator()).not.toBeNull()
  })

  it('shows the NumPad label while recording an editor row', async () => {
    const { getRow, getRecordingIndicator, mockedInvoke } = await mountHotkeysPanel()
    const recordInRow = (name: string) => getRow(name)!.querySelector('.record-btn') as HTMLButtonElement

    recordInRow('cycle_route').click()
    await flushAsync()
    expect(getRecordingIndicator()).not.toBeNull()

    dispatchKey('keydown', { key: '-', code: 'NumpadSubtract' })
    await flushAsync()
    expect(getRecordingIndicator()?.textContent).toContain('Num -')

    dispatchKey('keyup', { key: '-', code: 'NumpadSubtract' })
    await flushAsync()
    expect(mockedInvoke).toHaveBeenCalledWith('set_editor_hotkey', {
      actionId: 'cycle_route',
      hotkey: { key: 'NUMPAD_SUBTRACT', modifiers: [], enabled: true },
    })
  })

  it('blocks a NumPad capture occupied by an enabled local binding and allows a different modifier set', async () => {
    const { getMainRecordBtn, getRecordingIndicator, mockedInvoke } = await mountHotkeysPanel({
      hotkeys: {
        toggle_minimal_mode: { key: 'NUMPAD5', modifiers: [] },
      },
    })

    // Bare NumPad5 is taken by toggle_minimal_mode: blocked before any IPC.
    getMainRecordBtn()!.click()
    await flushAsync()
    dispatchKey('keydown', { key: '5', code: 'Numpad5' })
    dispatchKey('keyup', { key: '5', code: 'Numpad5' })
    await flushAsync()
    expect(getRecordingIndicator()?.textContent).toContain('Already assigned')
    expect(mockedInvoke).not.toHaveBeenCalledWith('set_hotkey', expect.anything())

    // A different modifier set does not shadow the bare local binding.
    dispatchKey('keydown', { key: '5', code: 'Numpad5', ctrlKey: true })
    dispatchKey('keyup', { key: '5', code: 'Numpad5', ctrlKey: true })
    await flushAsync()
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey', {
      name: 'main_window',
      hotkey: { key: 'NUMPAD5', modifiers: ['ctrl'], enabled: true },
    })
  })

  it('keeps the top digit row separate when a NumPad binding is taken', async () => {
    const { getMainRecordBtn, mockedInvoke } = await mountHotkeysPanel({
      hotkeys: {
        sound_panel: { key: 'NUMPAD5', modifiers: [] },
      },
    })

    getMainRecordBtn()!.click()
    await flushAsync()
    // Digit5 on the top row is a different physical key: recorded, not blocked
    // (the capture finishes into a save, so no occupied notice ever shows).
    dispatchKey('keydown', { key: '5', code: 'Digit5' })
    dispatchKey('keyup', { key: '5', code: 'Digit5' })
    await flushAsync()
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey', {
      name: 'main_window',
      hotkey: { key: '5', modifiers: [], enabled: true },
    })
  })
})

describe('HotkeysPanel review: save/refresh/cleanup separation', () => {
  it('keeps the saved binding and stays silent when the recording-flag cleanup fails after a successful save', async () => {
    const { getMainRecordBtn, getRow, getErrorMessage, mockedInvoke } = await mountHotkeysPanel()
    getMainRecordBtn()!.click()
    await flushAsync()

    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_hotkey') return undefined
      if (cmd === 'set_hotkey_recording') throw new Error('IPC bridge closed')
      return undefined
    })

    // Ctrl+K is free in the default settings. The save itself succeeds; the
    // subsequent cleanup invoke fails and the settings snapshot stays stale.
    dispatchKey('keydown', { key: 'K', code: 'KeyK', ctrlKey: true })
    dispatchKey('keyup', { key: 'K', code: 'KeyK' })
    await flushAsync()

    const row = getRow('main_window')!
    // The assignment persisted: the new binding stays confirmed on screen even
    // though reload produced no fresh snapshot (the old one still holds
    // Ctrl+M). The cleanup failure must not surface as a failed assignment.
    expect(row.querySelector('.hotkey-value.saving')).toBeNull()
    expect(row.querySelector('.hotkey-value')?.textContent).toContain('Ctrl+K')
    expect(row.querySelector('.hotkey-value')?.textContent).not.toContain('Ctrl+M')
    expect(getErrorMessage()).toBeNull()
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey', expect.anything())
  })

  it('keeps the saved binding when the settings refresh after a successful save fails', async () => {
    let failRefresh = false
    const { getMainRecordBtn, getRow, getErrorMessage, mockedInvoke } = await mountHotkeysPanel({
      onReload: () => {
        if (failRefresh) throw new Error('refresh failed')
      },
    })
    getMainRecordBtn()!.click()
    await flushAsync()

    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_hotkey') return undefined
      return undefined
    })
    failRefresh = true

    dispatchKey('keydown', { key: 'K', code: 'KeyK', ctrlKey: true })
    dispatchKey('keyup', { key: 'K', code: 'KeyK' })
    await flushAsync()

    const row = getRow('main_window')!
    expect(row.querySelector('.hotkey-value.saving')).toBeNull()
    expect(row.querySelector('.hotkey-value')?.textContent).toContain('Ctrl+K')
    expect(getErrorMessage()).toBeNull()
  })

  it('records the decimal key and an operator with their NumPad labels', async () => {
    const { getMainRecordBtn, getRecordingIndicator, mockedInvoke } = await mountHotkeysPanel()
    getMainRecordBtn()!.click()
    await flushAsync()

    // The decimal key is captured like the digits: physical code decides.
    dispatchKey('keydown', { key: '.', code: 'NumpadDecimal' })
    await flushAsync()
    expect(getRecordingIndicator()?.textContent).toContain('Num .')

    dispatchKey('keydown', { key: '+', code: 'NumpadAdd' })
    await flushAsync()
    expect(getRecordingIndicator()?.textContent).toContain('Num +')

    dispatchKey('keyup', { key: '+', code: 'NumpadAdd' })
    await flushAsync()
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey', {
      name: 'main_window',
      hotkey: { key: 'NUMPAD_ADD', modifiers: [], enabled: true },
    })
  })
})


describe('main output recording guard lifecycle (ROADMAP-142 move)', () => {
  function micHotkeyOverride(): Record<string, unknown> {
    const base = {
      edit_word: { key: '', modifiers: [] },
      submit_continue: { key: '', modifiers: [] },
      submit_keep_text: { key: '', modifiers: [] },
      submit_keep_focus: { key: '', modifiers: [] },
      next_spelling_error: { key: '', modifiers: [] },
      previous_spelling_error: { key: '', modifiers: [] },
      next_tab: { key: '', modifiers: [] },
      previous_tab: { key: '', modifiers: [] },
      cycle_route: { key: '', modifiers: [] },
      toggle_typing: { key: '', modifiers: [] },
      cycle_quick_mode: { key: '', modifiers: [] },
      toggle_history: { key: '', modifiers: [] },
      accent_homographs: { key: '', modifiers: [] },
      approve_next_incoming: { key: '', modifiers: [] },
      edit_next_incoming: { key: '', modifiers: [] },
      toggle_virtual_mic: { key: 'M', modifiers: ['ctrl'] },
      toggle_vban: { key: '', modifiers: [] },
    }
    return { editor: base }
  }

  async function clickMicRecordButton() {
    const checkbox = document.querySelector<HTMLInputElement>(
      'input[data-hotkey="toggle_virtual_mic"]',
    )!
    const row = checkbox.closest('.hotkey-row')!
    ;(row.querySelector<HTMLButtonElement>('.record-btn')!).click()
    await flushAsync()
  }

  it('blocks the output action window during a deferred recording start', async () => {
    // The backend start never resolves in this test: the frontend guard must
    // already be up while the IPC is pending.
    vi.mocked(invoke).mockImplementation((cmd: string) =>
      cmd === 'set_hotkey_recording' ? new Promise(() => {}) : Promise.resolve(undefined),
    )
    mountHotkeysPanel({ hotkeys: micHotkeyOverride() })
    expect(isMainWindowOutputControlsRecording()).toBe(false)

    await clickMicRecordButton()
    expect(isMainWindowOutputControlsRecording()).toBe(true)
  })

  it('releases the guard when a failed backend cleanup cancels recording', async () => {
    let failCancel = false
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'set_hotkey_recording') {
        if (failCancel) return Promise.reject(new Error('cleanup IPC failed'))
        return Promise.resolve(undefined)
      }
      return Promise.resolve(undefined)
    })
    mountHotkeysPanel({ hotkeys: micHotkeyOverride() })

    await clickMicRecordButton()
    expect(isMainWindowOutputControlsRecording()).toBe(true)

    // Cancel (Escape) while the backend cleanup IPC fails: the frontend
    // guard is released regardless, so the output action stays usable.
    failCancel = true
    ;(document.querySelector<HTMLButtonElement>('.cancel-btn')!).click()
    await flushAsync()
    expect(isMainWindowOutputControlsRecording()).toBe(false)
  })
})
