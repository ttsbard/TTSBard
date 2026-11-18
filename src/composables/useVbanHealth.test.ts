import { describe, it, expect, vi, beforeEach } from 'vitest'

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

// The module keeps a singleton verdict, so every test loads a fresh copy.
async function loadModule() {
  vi.resetModules()
  return await import('./useVbanHealth')
}

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

async function mountAndGet(snapshot: unknown): Promise<
  Awaited<ReturnType<typeof loadModule>>
> {
  const module = await loadModule()
  mocks.mockInvoke.mockResolvedValue(snapshot)
  module.useVbanHealthMonitor()
  await capturedOnMountedCb?.()
  return module
}

describe('useVbanHealth', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.listenCallbacks.clear()
    mocks.unlistenFns.clear()
    capturedOnMountedCb = null
    capturedOnUnmountedCb = null
    installDefaultListen()
    mocks.mockInvoke.mockResolvedValue({ state: 'unknown', reason: null })
  })

  it('starts from the grey unknown verdict before any monitor has run', async () => {
    const { useVbanHealth } = await loadModule()
    expect(useVbanHealth().health.value).toEqual({ state: 'unknown', reason: null })
  })

  it('registers the listener before reading the initial snapshot', async () => {
    const seenAtSnapshot: string[][] = []
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_vban_health') {
        seenAtSnapshot.push([...mocks.listenCallbacks.keys()])
        return { state: 'alive', reason: null }
      }
      return undefined
    })

    const { useVbanHealth, useVbanHealthMonitor } = await loadModule()
    useVbanHealthMonitor()
    await capturedOnMountedCb?.()

    expect(seenAtSnapshot).toHaveLength(1)
    expect(seenAtSnapshot[0]).toContain('vban-health-changed')
    expect(useVbanHealth().health.value).toEqual({ state: 'alive', reason: null })
  })

  it('applies the boot snapshot with its reason', async () => {
    const { useVbanHealth } = await mountAndGet({ state: 'failed', reason: 'no_reply' })
    expect(useVbanHealth().health.value).toEqual({ state: 'failed', reason: 'no_reply' })
  })

  it('follows vban-health-changed events', async () => {
    const { useVbanHealth } = await mountAndGet({ state: 'unknown', reason: null })

    emit('vban-health-changed', { state: 'alive', reason: null })
    expect(useVbanHealth().health.value).toEqual({ state: 'alive', reason: null })

    emit('vban-health-changed', { state: 'failed', reason: 'local_socket' })
    expect(useVbanHealth().health.value).toEqual({ state: 'failed', reason: 'local_socket' })

    emit('vban-health-changed', { state: 'unknown', reason: null })
    expect(useVbanHealth().health.value).toEqual({ state: 'unknown', reason: null })
  })

  it('keeps the previous verdict on malformed payloads', async () => {
    const { useVbanHealth } = await mountAndGet({ state: 'alive', reason: null })

    emit('vban-health-changed', null)
    emit('vban-health-changed', 'alive')
    emit('vban-health-changed', 42)
    emit('vban-health-changed', {})
    emit('vban-health-changed', { state: 'flying' })
    emit('vban-health-changed', { state: 42 })

    expect(useVbanHealth().health.value).toEqual({ state: 'alive', reason: null })
  })

  it('falls back to the generic failed text key data when the reason is unknown', async () => {
    const { useVbanHealth } = await mountAndGet({ state: 'alive', reason: null })

    emit('vban-health-changed', { state: 'failed', reason: 'something_new' })
    expect(useVbanHealth().health.value).toEqual({ state: 'failed', reason: null })

    // A bogus reason on a healthy verdict is dropped as well.
    emit('vban-health-changed', { state: 'alive', reason: 'no_reply' })
    expect(useVbanHealth().health.value).toEqual({ state: 'alive', reason: null })
  })

  it('never lets a pending snapshot overwrite a newer event', async () => {
    const module = await loadModule()
    let resolveSnapshot!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveSnapshot = resolve })
      }
      return undefined
    })

    module.useVbanHealthMonitor()
    const mounted = capturedOnMountedCb!()
    await vi.waitFor(() => expect(mocks.mockInvoke).toHaveBeenCalledWith('get_vban_health'))

    emit('vban-health-changed', { state: 'failed', reason: 'no_reply' })
    resolveSnapshot({ state: 'alive', reason: null })
    await mounted

    expect(module.useVbanHealth().health.value).toEqual({ state: 'failed', reason: 'no_reply' })
  })

  it('applies the snapshot when no event raced it', async () => {
    const module = await loadModule()
    let resolveSnapshot!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveSnapshot = resolve })
      }
      return undefined
    })

    module.useVbanHealthMonitor()
    const mounted = capturedOnMountedCb!()
    await vi.waitFor(() => expect(mocks.mockInvoke).toHaveBeenCalledWith('get_vban_health'))

    resolveSnapshot({ state: 'failed', reason: 'local_socket' })
    await mounted

    expect(module.useVbanHealth().health.value).toEqual({
      state: 'failed',
      reason: 'local_socket',
    })
  })

  it('drops a pending snapshot resolved after unmount', async () => {
    const module = await loadModule()
    let resolveSnapshot!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveSnapshot = resolve })
      }
      return undefined
    })

    module.useVbanHealthMonitor()
    const mounted = capturedOnMountedCb!()
    await vi.waitFor(() => expect(mocks.mockInvoke).toHaveBeenCalledWith('get_vban_health'))

    capturedOnUnmountedCb?.()
    resolveSnapshot({ state: 'alive', reason: null })
    await mounted

    expect(module.useVbanHealth().health.value).toEqual({ state: 'unknown', reason: null })
    expect(mocks.unlistenFns.get('vban-health-changed')).toHaveBeenCalledTimes(1)
  })

  it('ignores health events delivered after unmount', async () => {
    const { useVbanHealth } = await mountAndGet({ state: 'alive', reason: null })

    capturedOnUnmountedCb?.()
    emit('vban-health-changed', { state: 'failed', reason: 'no_reply' })

    expect(useVbanHealth().health.value).toEqual({ state: 'alive', reason: null })
    expect(mocks.unlistenFns.get('vban-health-changed')).toHaveBeenCalledTimes(1)
  })

  it('cleans up a listener whose registration resolves after unmount', async () => {
    const module = await loadModule()
    let resolveListen!: (unlisten: () => void) => void
    const pendingListen = new Promise<() => void>((resolve) => { resolveListen = resolve })
    mocks.mockListen.mockImplementationOnce(
      (event: string, callback: (event: { payload: unknown }) => void) => {
        mocks.listenCallbacks.set(event, callback)
        return pendingListen
      },
    )

    module.useVbanHealthMonitor()
    const mounted = capturedOnMountedCb!()

    capturedOnUnmountedCb?.()
    const unlisten = vi.fn(() => { mocks.listenCallbacks.delete('vban-health-changed') })
    resolveListen(unlisten)
    await mounted

    expect(unlisten).toHaveBeenCalledTimes(1)
    expect(mocks.mockInvoke).not.toHaveBeenCalledWith('get_vban_health')
  })

  it('a remounted monitor re-reads the snapshot and a stale one cannot leak', async () => {
    const module = await loadModule()
    let resolveFirst!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveFirst = resolve })
      }
      return undefined
    })

    // First mount cycle whose snapshot never resolves.
    module.useVbanHealthMonitor()
    const firstMounted = capturedOnMountedCb!()
    await vi.waitFor(() => expect(mocks.mockInvoke).toHaveBeenCalledWith('get_vban_health'))

    // Unmount and mount again: the fresh cycle reads its own snapshot.
    capturedOnUnmountedCb?.()
    mocks.mockInvoke.mockResolvedValue({ state: 'failed', reason: 'no_reply' })
    module.useVbanHealthMonitor()
    const secondMounted = capturedOnMountedCb!()
    await secondMounted
    expect(module.useVbanHealth().health.value).toEqual({ state: 'failed', reason: 'no_reply' })

    // The first cycle's snapshot resolves late and must not win.
    resolveFirst({ state: 'alive', reason: null })
    await firstMounted
    expect(module.useVbanHealth().health.value).toEqual({ state: 'failed', reason: 'no_reply' })
  })

  it('logs a subscription failure and still reads the snapshot', async () => {
    mocks.mockListen.mockRejectedValueOnce(new Error('subscribe boom'))

    const { useVbanHealth } = await mountAndGet({ state: 'failed', reason: 'local_socket' })

    expect(mocks.mockDebugError).toHaveBeenCalled()
    expect(useVbanHealth().health.value).toEqual({ state: 'failed', reason: 'local_socket' })
  })

  it('logs a snapshot read failure without throwing', async () => {
    const module = await loadModule()
    mocks.mockInvoke.mockRejectedValueOnce(new Error('snapshot boom'))

    module.useVbanHealthMonitor()
    await capturedOnMountedCb?.()

    expect(mocks.mockDebugError).toHaveBeenCalled()
    expect(module.useVbanHealth().health.value).toEqual({ state: 'unknown', reason: null })
  })

  it('parses strictly on the exported parse helper', async () => {
    const { parseVbanHealth } = await loadModule()
    expect(parseVbanHealth({ state: 'alive', reason: null })).toEqual({ state: 'alive', reason: null })
    expect(parseVbanHealth({ state: 'failed', reason: 'no_reply' })).toEqual({
      state: 'failed',
      reason: 'no_reply',
    })
    expect(parseVbanHealth({ state: 'failed', reason: undefined })).toEqual({
      state: 'failed',
      reason: null,
    })
    expect(parseVbanHealth({ state: 'other' })).toBeNull()
    expect(parseVbanHealth(undefined)).toBeNull()
    expect(parseVbanHealth('alive')).toBeNull()
  })
})
