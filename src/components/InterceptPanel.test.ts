// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, ref } from 'vue'
import InterceptPanel from './InterceptPanel.vue'
import { invoke } from '@tauri-apps/api/core'

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

function dispatchKeyEvent(eventInit: { code: string; key: string; keyCode?: number }) {
  const event = new KeyboardEvent('keydown', {
    code: eventInit.code,
    key: eventInit.key,
    bubbles: true,
    cancelable: true,
  })
  if (eventInit.keyCode !== undefined) {
    Object.defineProperty(event, 'keyCode', { value: eventInit.keyCode })
  }
  document.dispatchEvent(event)
}

let appInstance: ReturnType<typeof createApp> | null = null
let container: HTMLDivElement | null = null

async function mountPanel(options: {
  active?: boolean
  allowAnyKey?: boolean
  bindings?: Array<{ key: string; action: string }>
} = {}) {
  container = document.createElement('div')
  document.body.appendChild(container)

  const activeRef = ref(options.active ?? true)
  const mockedInvoke = vi.mocked(invoke)

  const defaultHandler = async (cmd: string): Promise<unknown> => {
    if (cmd === 'get_intercept_settings') {
      return {
        enabled: true,
        allow_any_key: options.allowAnyKey ?? false,
        bindings: options.bindings ?? [{ key: 'NUMPAD1', action: 'playback_stop' }],
      }
    }
    return undefined
  }

  mockedInvoke.mockImplementation(defaultHandler)

  appInstance = createApp({
    render() {
      return h(InterceptPanel, { active: activeRef.value })
    },
  })

  appInstance.mount(container)

  await flushAsync()
  mockedInvoke.mockClear()

  return {
    container,
    activeRef,
    mockedInvoke,
    defaultHandler,
    getRecordBtn: () => container?.querySelector('.record-btn') as HTMLButtonElement | null,
    getRecordingIndicator: () => container?.querySelector('.record-btn.recording') as HTMLButtonElement | null,
    getCancelBtn: () => container?.querySelector('.record-actions button:last-child') as HTMLButtonElement | null,
    getNewBindingRow: () => container?.querySelector('.new-binding-row') as HTMLDivElement | null,
    getErrorMessage: () => container?.querySelector('.message-box') as HTMLDivElement | null,
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

describe('InterceptPanel mounted lifecycle and recording', () => {
  it('waits for backend preparation before installing the recording listener', async () => {
    const { getRecordBtn, getRecordingIndicator, mockedInvoke, defaultHandler } = await mountPanel()
    const gate = deferred()
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === 'set_hotkey_recording') return gate.promise
      return defaultHandler(cmd)
    })

    const recordBtn = getRecordBtn()!
    recordBtn.click()
    await nextTick()

    // While backend preparation is pending, indicator is not active
    expect(getRecordingIndicator()).toBeNull()

    // Dispatching keydown does not register key because listener is not yet attached
    dispatchKeyEvent({ code: 'Numpad2', key: '2', keyCode: 98 })
    await nextTick()

    // Now let backend preparation resolve
    gate.resolve()
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: true })
    expect(mockedInvoke).toHaveBeenCalledWith('unregister_hotkeys')
    expect(getRecordingIndicator()).not.toBeNull()

    // Now listener is active, dispatch keydown
    dispatchKeyEvent({ code: 'Numpad2', key: '2', keyCode: 98 })
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')
  })

  it('keeps recording on an occupied key and restores after a free one', async () => {
    const { getRecordBtn, getRecordingIndicator, getNewBindingRow, mockedInvoke } = await mountPanel()
    getRecordBtn()!.click()
    await flushAsync()

    expect(getRecordingIndicator()).not.toBeNull()

    // Numpad1 is already bound in initial settings
    dispatchKeyEvent({ code: 'Numpad1', key: '1', keyCode: 97 })
    await flushAsync()

    // Still recording, and set_hotkey_recording(false) was not called
    expect(getRecordingIndicator()).not.toBeNull()
    expect(mockedInvoke).not.toHaveBeenCalledWith('set_hotkey_recording', { recording: false })

    // Now press a free key Numpad2
    dispatchKeyEvent({ code: 'Numpad2', key: '2', keyCode: 98 })
    await flushAsync()

    // Recording finished, new binding row shown
    expect(getRecordingIndicator()).toBeNull()
    expect(getNewBindingRow()).not.toBeNull()
    expect(getNewBindingRow()!.textContent).toContain('NUMPAD2')
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')
  })

  it('restores an active session on cancel button click', async () => {
    const { getRecordBtn, getRecordingIndicator, getCancelBtn, mockedInvoke } = await mountPanel()
    getRecordBtn()!.click()
    await flushAsync()

    expect(getRecordingIndicator()).not.toBeNull()

    // Click cancel button
    getCancelBtn()!.click()
    await flushAsync()

    expect(getRecordingIndicator()).toBeNull()
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')

    // Further keydown should do nothing
    mockedInvoke.mockClear()
    dispatchKeyEvent({ code: 'Numpad2', key: '2', keyCode: 98 })
    await flushAsync()
    expect(mockedInvoke).not.toHaveBeenCalled()
  })

  it('restores an active session on props.active = false (hide)', async () => {
    const { getRecordBtn, getRecordingIndicator, activeRef, mockedInvoke } = await mountPanel()
    getRecordBtn()!.click()
    await flushAsync()

    expect(getRecordingIndicator()).not.toBeNull()

    // Deactivate panel
    activeRef.value = false
    await flushAsync()

    expect(getRecordingIndicator()).toBeNull()
    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')

    // Dispatched keydown does not trigger anything
    mockedInvoke.mockClear()
    dispatchKeyEvent({ code: 'Numpad2', key: '2', keyCode: 98 })
    await flushAsync()
    expect(mockedInvoke).not.toHaveBeenCalled()
  })

  it('restores an active session on unmount', async () => {
    const { getRecordBtn, getRecordingIndicator, unmount, mockedInvoke } = await mountPanel()
    getRecordBtn()!.click()
    await flushAsync()

    expect(getRecordingIndicator()).not.toBeNull()

    // Unmount
    await unmount()

    expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')

    // Dispatched keydown does not trigger anything
    mockedInvoke.mockClear()
    dispatchKeyEvent({ code: 'Numpad2', key: '2', keyCode: 98 })
    await flushAsync()
    expect(mockedInvoke).not.toHaveBeenCalled()
  })

  it.each(['cancel', 'leave', 'unmount'] as const)(
    'rolls back a delayed start after %s',
    async (reason) => {
      const { getRecordBtn, getRecordingIndicator, getCancelBtn, activeRef, unmount, mockedInvoke, defaultHandler } =
        await mountPanel()
      const gate = deferred()
      mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'set_hotkey_recording') return gate.promise
        return defaultHandler(cmd)
      })

      getRecordBtn()!.click()
      await nextTick()

      if (reason === 'cancel') {
        getCancelBtn()!.click()
      } else if (reason === 'leave') {
        activeRef.value = false
      } else {
        await unmount()
      }
      await nextTick()

      // Now delayed start resolves
      gate.resolve()
      await flushAsync()

      expect(getRecordingIndicator() ?? null).toBeNull()
      expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
    },
  )

  it('restores registrations when cancelled during unregister and blocks a competing start', async () => {
    const { getRecordBtn, getRecordingIndicator, getCancelBtn, mockedInvoke, defaultHandler } = await mountPanel()
    const gate = deferred()
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === 'unregister_hotkeys') return gate.promise
      return defaultHandler(cmd)
    })

    getRecordBtn()!.click()
    await flushAsync()

    // Cancel while unregister_hotkeys is pending
    getCancelBtn()!.click()
    await flushAsync()

    // A competing native click must be disabled and cannot prepare another session.
    const recordBtn = getRecordBtn()!
    expect(recordBtn).not.toBeNull()
    expect(recordBtn.disabled).toBe(true)
    recordBtn.click()
    await flushAsync()
    const startCalls = () => mockedInvoke.mock.calls.filter(
      ([cmd, args]) => cmd === 'set_hotkey_recording' && (args as { recording: boolean })?.recording,
    )
    const unregisterCalls = () => mockedInvoke.mock.calls.filter(([cmd]) => cmd === 'unregister_hotkeys')
    expect(startCalls()).toHaveLength(1)
    expect(unregisterCalls()).toHaveLength(1)

    mockedInvoke.mockImplementation(defaultHandler)
    gate.resolve()
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')
    expect(startCalls()).toHaveLength(1)
    expect(unregisterCalls()).toHaveLength(1)

    // Now clicking record starts a fresh clean session
    expect(getRecordBtn()!.disabled).toBe(false)
    getRecordBtn()!.click()
    await flushAsync()
    expect(getRecordingIndicator()).not.toBeNull()
    expect(startCalls()).toHaveLength(2)
    expect(unregisterCalls()).toHaveLength(2)
  })

  it.each(['set_hotkey_recording', 'unregister_hotkeys'] as const)(
    'rolls back failed %s and permits retry',
    async (failedCmd) => {
      const { getRecordBtn, getRecordingIndicator, getErrorMessage, mockedInvoke, defaultHandler } =
        await mountPanel()
      let fail = true
      mockedInvoke.mockImplementation(async (cmd, args) => {
        if (
          fail &&
          cmd === failedCmd &&
          (failedCmd !== 'set_hotkey_recording' || (args as { recording: boolean })?.recording)
        ) {
          fail = false
          throw new Error('setup failed')
        }
        return defaultHandler(cmd)
      })

      getRecordBtn()!.click()
      await flushAsync()

      expect(getErrorMessage()?.textContent).toContain('setup failed')
      expect(getRecordingIndicator()).toBeNull()
      expect(mockedInvoke).toHaveBeenCalledWith('set_hotkey_recording', { recording: false })
      if (failedCmd === 'unregister_hotkeys') {
        expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')
      }

      // Retry succeeds
      mockedInvoke.mockImplementation(defaultHandler)
      getRecordBtn()!.click()
      await flushAsync()

      expect(getRecordingIndicator()).not.toBeNull()
    },
  )

  it('attempts registration restoration even if the flag reset fails', async () => {
    const { getRecordBtn, getCancelBtn, getErrorMessage, mockedInvoke, defaultHandler } = await mountPanel()
    getRecordBtn()!.click()
    await flushAsync()

    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === 'set_hotkey_recording') throw new Error('reset failed')
      return defaultHandler(cmd)
    })

    getCancelBtn()!.click()
    await flushAsync()

    expect(mockedInvoke).toHaveBeenCalledWith('reregister_hotkeys_cmd')
    expect(getErrorMessage()?.textContent).toContain('reset failed')
  })

  it('blocks a new session until restoration completes', async () => {
    const { getRecordBtn, getRecordingIndicator, getCancelBtn, mockedInvoke, defaultHandler } = await mountPanel()
    getRecordBtn()!.click()
    await flushAsync()

    const gate = deferred()
    mockedInvoke.mockImplementation(async (cmd, args) => {
      if (cmd === 'set_hotkey_recording' && !(args as { recording: boolean })?.recording) {
        return gate.promise
      }
      return defaultHandler(cmd)
    })

    getCancelBtn()!.click()
    await flushAsync()

    // While restoration is in progress, record button is disabled or not recording
    const recordBtn = getRecordBtn()
    if (recordBtn && !recordBtn.disabled) {
      recordBtn.click()
    }
    await flushAsync()

    // set_hotkey_recording(true) was not called again
    const startCalls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) => cmd === 'set_hotkey_recording' && (args as { recording: boolean })?.recording,
    )
    expect(startCalls).toHaveLength(1)

    // Complete restoration
    mockedInvoke.mockImplementation(defaultHandler)
    gate.resolve()
    await flushAsync()

    // Now start works
    getRecordBtn()!.click()
    await flushAsync()
    expect(getRecordingIndicator()).not.toBeNull()
  })

  it.each([false, true] as const)(
    'preserves Escape semantics with allow_any_key=%s',
    async (allowAnyKey) => {
      const { getRecordBtn, getNewBindingRow, getRecordingIndicator } = await mountPanel({
        allowAnyKey,
      })
      getRecordBtn()!.click()
      await flushAsync()

      dispatchKeyEvent({ code: 'Escape', key: 'Escape', keyCode: 27 })
      await flushAsync()

      if (allowAnyKey) {
        // With unrestricted mode, Escape is recorded as a key binding (label Escape)
        expect(getNewBindingRow()).not.toBeNull()
        expect(getNewBindingRow()!.textContent).toContain('Escape')
      } else {
        // With restricted mode, Escape cancels recording
        expect(getRecordingIndicator()).toBeNull()
        expect(getNewBindingRow()).toBeNull()
      }
    },
  )
})

describe('InterceptPanel binding save error reporting', () => {
  it('shows the full conflict text when set_intercept_binding rejects with a string', async () => {
    const { getRecordBtn, getNewBindingRow, getErrorMessage, mockedInvoke, defaultHandler } =
      await mountPanel()

    // Record NumPad 9, which the backend will reject as already assigned globally.
    getRecordBtn()!.click()
    await flushAsync()
    dispatchKeyEvent({ code: 'Numpad9', key: '9', keyCode: 105 })
    await flushAsync()
    expect(getNewBindingRow()).not.toBeNull()
    expect(getNewBindingRow()!.textContent).toContain('NUMPAD9')

    // Tauri Result<(), String> rejects with a plain string, not an Error instance.
    const conflictMessage =
      'Клавиша NUMPAD9 уже назначена в разделе «Глобальные» (действие: Пауза воспроизведения)'
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === 'set_intercept_binding') throw conflictMessage
      return defaultHandler(cmd)
    })

    const confirmBtn = container?.querySelector('.new-binding-row button') as HTMLButtonElement | null
    expect(confirmBtn).not.toBeNull()
    confirmBtn!.click()
    await flushAsync()

    // The localized prefix survives and the backend detail names key, section and action.
    const message = getErrorMessage()?.textContent ?? ''
    expect(message).toContain('Error:')
    expect(message).toContain(conflictMessage)
    expect(message).toContain('NUMPAD9')
    expect(message).toContain('Глобальные')
    expect(message).toContain('Пауза воспроизведения')
  })
})
