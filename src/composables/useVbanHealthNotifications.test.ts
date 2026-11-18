import { describe, it, expect, vi, beforeEach, afterEach, beforeAll } from 'vitest'

vi.stubGlobal('window', globalThis)

const mocks = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
  mockListen: vi.fn(),
  listenCallbacks: new Map<string, (event: { payload: unknown }) => void>(),
  unlistenFns: new Map<string, () => void>(),
  mockDebugError: vi.fn(),
}))

let capturedOnMountedCb: (() => Promise<void>) | null = null
let capturedOnUnmountedCb: (() => void) | null = null

vi.mock('vue', async () => {
  const actual = await vi.importActual<typeof import('vue')>('vue')
  return {
    ...actual,
    onMounted: (cb: () => Promise<void>) => { capturedOnMountedCb = cb },
    onUnmounted: (cb: () => void) => { capturedOnUnmountedCb = cb },
  }
})

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mocks.mockInvoke,
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: mocks.mockListen,
}))

vi.mock('../utils/debug', () => ({
  debugLog: vi.fn(),
  debugError: mocks.mockDebugError,
}))

// The real toast store backs the dedup: "visible" means the shown error is
// still in the shared list, so the 5 s expiry is exercised for real.
import { useVbanHealthNotifications, vbanHealthToastMessage } from './useVbanHealthNotifications'
import { useErrorHandler } from './useErrorHandler'
import { i18n } from '../i18n'
import ruCatalog from '../../locales/ru.json'

const ALERT_EVENT = 'vban-health-alert'
const CHANGED_EVENT = 'vban-health-changed'
const FAILED_NO_REPLY = { state: 'failed', reason: 'no_reply' }
const FAILED_LOCAL_SOCKET = { state: 'failed', reason: 'local_socket' }

beforeAll(() => {
  i18n.global.setLocaleMessage('ru', (ruCatalog as { messages: Record<string, string> }).messages)
  ;(i18n.global.locale as unknown as { value: string }).value = 'ru'
})

function installDefaultListen(): void {
  mocks.mockListen.mockImplementation(
    (event: string, callback: (event: { payload: unknown }) => void) => {
      mocks.listenCallbacks.set(event, callback)
      const unlisten = vi.fn(() => { mocks.listenCallbacks.delete(event) })
      mocks.unlistenFns.set(event, unlisten)
      return Promise.resolve(unlisten)
    },
  )
}

function emit(event: string, payload: unknown): void {
  mocks.listenCallbacks.get(event)?.({ payload })
}

/** Drain the microtasks that resolve the listener registration and start the snapshot read. */
async function flushAsync(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
}

async function mount(snapshot: unknown = { state: 'unknown', reason: null }): Promise<void> {
  mocks.mockInvoke.mockResolvedValue(snapshot)
  useVbanHealthNotifications()
  await capturedOnMountedCb?.()
}

describe('useVbanHealthNotifications', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.listenCallbacks.clear()
    mocks.unlistenFns.clear()
    capturedOnMountedCb = null
    capturedOnUnmountedCb = null
    installDefaultListen()
    mocks.mockInvoke.mockResolvedValue({ state: 'unknown', reason: null })
    useErrorHandler().clearAllErrors()
    vi.useFakeTimers()
  })

  afterEach(() => {
    useErrorHandler().clearAllErrors()
    vi.useRealTimers()
  })

  it('exports the fixed safe Russian toast texts per reason', () => {
    expect(vbanHealthToastMessage('no_reply'))
      .toBe('VBAN-приёмник не отвечает. Звук на другом компьютере может не воспроизводиться')
    expect(vbanHealthToastMessage('local_socket'))
      .toBe('Проверка VBAN-приёмника не запустилась: локальный UDP-порт занят другой программой. Освободите порт — проверка восстановится сама; воспроизведение при этом не блокируется')
    // A failed verdict without a known reason keeps its own generic text.
    expect(vbanHealthToastMessage(null))
      .toBe('VBAN-приёмник недоступен. Звук на другом компьютере может не воспроизводиться')
  })

  it('registers the alert listener before reading the boot snapshot', async () => {
    const seenAtSnapshot: string[][] = []
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_vban_health') {
        seenAtSnapshot.push([...mocks.listenCallbacks.keys()])
        return { state: 'unknown', reason: null }
      }
      return undefined
    })

    useVbanHealthNotifications()
    await capturedOnMountedCb?.()

    expect(seenAtSnapshot).toHaveLength(1)
    expect(seenAtSnapshot[0]).toContain(ALERT_EVENT)
  })

  it('shows one fixed error toast for a no_reply alert', async () => {
    await mount()

    emit(ALERT_EVENT, FAILED_NO_REPLY)
    const { errors } = useErrorHandler()
    expect(errors.value).toHaveLength(1)
    expect(errors.value[0].message)
      .toBe('VBAN-приёмник не отвечает. Звук на другом компьютере может не воспроизводиться')
  })

  it('uses the separate local socket explanation for that reason', async () => {
    await mount()

    emit(ALERT_EVENT, FAILED_LOCAL_SOCKET)
    const { errors } = useErrorHandler()
    expect(errors.value).toHaveLength(1)
    expect(errors.value[0].message).toBe(vbanHealthToastMessage('local_socket'))
  })

  it('falls back to the generic text when the failed reason is unknown', async () => {
    await mount()

    emit(ALERT_EVENT, { state: 'failed', reason: 'something_new' })
    const { errors } = useErrorHandler()
    expect(errors.value).toHaveLength(1)
    expect(errors.value[0].message).toBe(vbanHealthToastMessage(null))
  })

  it('never toasts for alive or unknown alert payloads', async () => {
    await mount()

    emit(ALERT_EVENT, { state: 'alive', reason: null })
    emit(ALERT_EVENT, { state: 'unknown', reason: null })

    expect(useErrorHandler().errors.value).toHaveLength(0)
  })

  it('ignores malformed alert payloads without toasting', async () => {
    await mount()

    emit(ALERT_EVENT, null)
    emit(ALERT_EVENT, 'failed')
    emit(ALERT_EVENT, 42)
    emit(ALERT_EVENT, {})
    emit(ALERT_EVENT, { state: 'flying' })
    emit(ALERT_EVENT, { state: 42 })

    expect(useErrorHandler().errors.value).toHaveLength(0)
  })

  it('catches up on a boot failure captured by the snapshot', async () => {
    await mount(FAILED_NO_REPLY)

    const { errors } = useErrorHandler()
    expect(errors.value).toHaveLength(1)
    expect(errors.value[0].message).toBe(vbanHealthToastMessage('no_reply'))
  })

  it('does not toast for ordinary boot states', async () => {
    await mount({ state: 'alive', reason: null })
    expect(useErrorHandler().errors.value).toHaveLength(0)
  })

  it('dedupes an alert that echoes the failed snapshot while it is visible', async () => {
    await mount(FAILED_NO_REPLY)
    expect(useErrorHandler().errors.value).toHaveLength(1)

    emit(ALERT_EVENT, FAILED_NO_REPLY)
    emit(ALERT_EVENT, FAILED_NO_REPLY)
    expect(useErrorHandler().errors.value).toHaveLength(1)
  })

  it('dedupes repeated alerts while the toast is visible', async () => {
    await mount()

    emit(ALERT_EVENT, FAILED_NO_REPLY)
    emit(ALERT_EVENT, FAILED_NO_REPLY)
    vi.advanceTimersByTime(4999)
    emit(ALERT_EVENT, FAILED_LOCAL_SOCKET)

    expect(useErrorHandler().errors.value).toHaveLength(1)
    expect(useErrorHandler().errors.value[0].message).toBe(vbanHealthToastMessage('no_reply'))
  })

  it('warns again after the 5 s toast is gone', async () => {
    await mount()

    emit(ALERT_EVENT, FAILED_NO_REPLY)
    expect(useErrorHandler().errors.value).toHaveLength(1)

    // The default error toast lives 5 s, then the next alert warns again.
    vi.advanceTimersByTime(5000)
    expect(useErrorHandler().errors.value).toHaveLength(0)

    emit(ALERT_EVENT, FAILED_NO_REPLY)
    expect(useErrorHandler().errors.value).toHaveLength(1)
  })

  it('skips a stale snapshot when an alert raced it', async () => {
    let resolveSnapshot!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveSnapshot = resolve })
      }
      return undefined
    })

    useVbanHealthNotifications()
    // Calling onMounted registers the listener synchronously; drain the
    // microtasks so the pending snapshot promise exists, then resolve it.
    const mounted = capturedOnMountedCb!()
    await flushAsync()

    emit(ALERT_EVENT, FAILED_NO_REPLY)
    resolveSnapshot(FAILED_NO_REPLY)
    await mounted

    expect(useErrorHandler().errors.value).toHaveLength(1)
  })

  it('keeps a single toast when a later alert beats the pending snapshot', async () => {
    let resolveSnapshot!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveSnapshot = resolve })
      }
      return undefined
    })

    useVbanHealthNotifications()
    const mounted = capturedOnMountedCb!()
    await flushAsync()

    emit(ALERT_EVENT, FAILED_LOCAL_SOCKET)
    // The snapshot echoes the same failure while the toast is still visible.
    resolveSnapshot(FAILED_LOCAL_SOCKET)
    await mounted

    expect(useErrorHandler().errors.value).toHaveLength(1)
  })

  it('stays silent when a newer alive transition beats the failed boot snapshot', async () => {
    let resolveSnapshot!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveSnapshot = resolve })
      }
      return undefined
    })

    useVbanHealthNotifications()
    const mounted = capturedOnMountedCb!()
    await flushAsync()

    // The receiver answered after the boot read started: the stale failed
    // snapshot must not toast.
    emit(CHANGED_EVENT, { state: 'alive', reason: null })
    resolveSnapshot(FAILED_NO_REPLY)
    await mounted

    expect(useErrorHandler().errors.value).toHaveLength(0)
  })

  it('stays silent when a disable reset (unknown) beats the failed boot snapshot', async () => {
    let resolveSnapshot!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveSnapshot = resolve })
      }
      return undefined
    })

    useVbanHealthNotifications()
    const mounted = capturedOnMountedCb!()
    await flushAsync()

    // VBAN was disabled or retargeted after the boot read started: the
    // synchronous verdict reset supersedes the stale failed snapshot.
    emit(CHANGED_EVENT, { state: 'unknown', reason: null })
    resolveSnapshot(FAILED_NO_REPLY)
    await mounted

    expect(useErrorHandler().errors.value).toHaveLength(0)
  })

  it('never toasts from a changed event on its own', async () => {
    await mount()

    emit(CHANGED_EVENT, { state: 'failed', reason: 'no_reply' })
    emit(CHANGED_EVENT, { state: 'alive', reason: null })

    expect(useErrorHandler().errors.value).toHaveLength(0)
  })

  it('a malformed changed event does not invalidate the catch-up', async () => {
    let resolveSnapshot!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveSnapshot = resolve })
      }
      return undefined
    })

    useVbanHealthNotifications()
    const mounted = capturedOnMountedCb!()
    await flushAsync()

    emit(CHANGED_EVENT, { state: 'flying' })
    emit(CHANGED_EVENT, 'garbage')
    resolveSnapshot(FAILED_NO_REPLY)
    await mounted

    expect(useErrorHandler().errors.value).toHaveLength(1)
  })

  it('unmount unlistens both streams and a late changed event stays silent', async () => {
    await mount()
    capturedOnUnmountedCb?.()

    emit(CHANGED_EVENT, { state: 'failed', reason: 'no_reply' })

    expect(useErrorHandler().errors.value).toHaveLength(0)
    expect(mocks.unlistenFns.get(ALERT_EVENT)).toHaveBeenCalledTimes(1)
    expect(mocks.unlistenFns.get(CHANGED_EVENT)).toHaveBeenCalledTimes(1)
  })

  it('logs a subscription failure and still reads the boot snapshot', async () => {
    mocks.mockListen.mockRejectedValueOnce(new Error('subscribe boom'))
    await mount(FAILED_NO_REPLY)

    expect(mocks.mockDebugError).toHaveBeenCalled()
    expect(useErrorHandler().errors.value).toHaveLength(1)
  })

  it('logs a snapshot read failure without showing a toast or rejecting', async () => {
    mocks.mockInvoke.mockRejectedValueOnce(new Error('snapshot boom'))

    useVbanHealthNotifications()
    await capturedOnMountedCb?.()

    expect(mocks.mockDebugError).toHaveBeenCalled()
    expect(useErrorHandler().errors.value).toHaveLength(0)
  })

  it('ignores alerts delivered after unmount and unlistens once', async () => {
    await mount()
    capturedOnUnmountedCb?.()

    emit(ALERT_EVENT, FAILED_NO_REPLY)

    expect(useErrorHandler().errors.value).toHaveLength(0)
    expect(mocks.unlistenFns.get(ALERT_EVENT)).toHaveBeenCalledTimes(1)
  })

  it('drops a pending snapshot result resolved after unmount', async () => {
    let resolveSnapshot!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveSnapshot = resolve })
      }
      return undefined
    })

    useVbanHealthNotifications()
    const mounted = capturedOnMountedCb!()
    await flushAsync()

    capturedOnUnmountedCb?.()
    resolveSnapshot(FAILED_NO_REPLY)
    await mounted

    expect(useErrorHandler().errors.value).toHaveLength(0)
    expect(mocks.unlistenFns.get(ALERT_EVENT)).toHaveBeenCalledTimes(1)
  })

  it('cleans up a listener whose registration resolves after unmount', async () => {
    let resolveListen!: (unlisten: () => void) => void
    const pendingListen = new Promise<() => void>((resolve) => { resolveListen = resolve })
    mocks.mockListen.mockImplementationOnce(
      (event: string, callback: (event: { payload: unknown }) => void) => {
        mocks.listenCallbacks.set(event, callback)
        return pendingListen
      },
    )

    useVbanHealthNotifications()
    const mounted = capturedOnMountedCb!()

    capturedOnUnmountedCb?.()
    const unlisten = vi.fn(() => { mocks.listenCallbacks.delete(ALERT_EVENT) })
    resolveListen(unlisten)
    await mounted

    expect(unlisten).toHaveBeenCalledTimes(1)
    expect(mocks.mockInvoke).not.toHaveBeenCalledWith('get_vban_health')
    expect(useErrorHandler().errors.value).toHaveLength(0)
  })
})
