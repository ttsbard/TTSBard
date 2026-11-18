import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.stubGlobal('window', globalThis)

const mocks = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
  listenCallbacks: new Map<string, (event: { payload: unknown }) => void>(),
  mockDebugError: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mocks.mockInvoke,
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, callback: (event: { payload: unknown }) => void) => {
    mocks.listenCallbacks.set(event, callback)
    return Promise.resolve(vi.fn(() => mocks.listenCallbacks.delete(event)))
  }),
}))

vi.mock('../utils/debug', () => ({
  debugLog: vi.fn(),
  debugError: mocks.mockDebugError,
  debugWarn: vi.fn(),
}))

vi.mock('../i18n', () => ({
  t: (key: string) => key,
}))

import { createUpdateInstall } from './useUpdateInstall'
import {
  UPDATE_INSTALL_PROGRESS_EVENT,
  type UpdateInstallProgressDto,
} from '../ipc/appUpdates'

function progress(overrides: Partial<UpdateInstallProgressDto> = {}): UpdateInstallProgressDto {
  return {
    stage: 'downloading',
    version: '1.9.0',
    downloaded: 1024,
    total: 2048,
    error_code: null,
    ...overrides,
  }
}

beforeEach(() => {
  mocks.mockInvoke.mockReset()
  mocks.mockDebugError.mockReset()
  mocks.listenCallbacks.clear()
})

function emitProgress(payload: unknown): void {
  const callback = mocks.listenCallbacks.get(UPDATE_INSTALL_PROGRESS_EVENT)
  expect(callback, 'progress listener must be registered').toBeDefined()
  callback?.({ payload })
}

describe('createUpdateInstall', () => {
  it('starts idle and follows the backend pipeline through its stages', async () => {
    const context = createUpdateInstall()
    expect(context.stage.value).toBeNull()
    expect(context.active.value).toBe(false)

    // install_update resolves with the initial checking stage.
    mocks.mockInvoke.mockResolvedValueOnce(progress({ stage: 'checking', version: null, downloaded: 0, total: null }))
    await context.start()
    expect(mocks.mockInvoke).toHaveBeenCalledWith('install_update')
    expect(context.stage.value).toBe('checking')
    expect(context.canCancel.value).toBe(true)

    emitProgress(progress({ stage: 'downloading', downloaded: 1024, total: 2048 }))
    expect(context.stage.value).toBe('downloading')
    expect(context.downloaded.value).toBe(1024)
    expect(context.total.value).toBe(2048)
    expect(context.percent.value).toBe(50)
    expect(context.version.value).toBe('1.9.0')

    emitProgress(progress({ stage: 'saving', downloaded: 0, total: null }))
    expect(context.stage.value).toBe('saving')
    expect(context.canCancel.value).toBe(false)
    expect(context.percent.value).toBeNull()
  })

  it('cancel() aborts the download and resets the context', async () => {
    const context = createUpdateInstall()
    mocks.mockInvoke.mockResolvedValueOnce(progress({ stage: 'downloading' }))
    await context.start()

    mocks.mockInvoke.mockResolvedValueOnce(progress({ stage: 'cancelled' }))
    const cancelled = await context.cancel()
    expect(mocks.mockInvoke).toHaveBeenCalledWith('cancel_update_install')
    expect(cancelled).toBe(true)
    expect(context.stage.value).toBeNull()
    expect(context.active.value).toBe(false)
  })

  it('cancel() reports false when the backend has nothing cancellable left', async () => {
    const context = createUpdateInstall()
    mocks.mockInvoke.mockResolvedValueOnce(progress({ stage: 'saving' }))
    await context.start()

    mocks.mockInvoke.mockResolvedValueOnce(null)
    expect(await context.cancel()).toBe(false)
    expect(context.stage.value).toBe('saving')
  })

  it('maps a backend rejection code to a localized failed stage', async () => {
    const context = createUpdateInstall()
    mocks.mockInvoke.mockRejectedValueOnce('signature')
    await context.start()
    expect(context.stage.value).toBe('failed')
    expect(context.errorText.value).toBe('general.updates.install.errors.signature')
    // The failed run leaves the context active so the modal can show it.
    expect(context.active.value).toBe(true)
  })

  it('a failed pipeline event carries the localized error and retry resets', async () => {
    const context = createUpdateInstall()
    mocks.mockInvoke.mockResolvedValueOnce(progress({ stage: 'checking' }))
    await context.start()

    emitProgress(progress({ stage: 'failed', error_code: 'network', version: null }))
    expect(context.stage.value).toBe('failed')
    expect(context.errorText.value).toBe('general.updates.install.errors.network')

    context.reset()
    expect(context.stage.value).toBeNull()
    expect(context.errorText.value).toBeNull()
    // Retry: start() runs a fresh pipeline.
    mocks.mockInvoke.mockResolvedValueOnce(progress({ stage: 'checking' }))
    await context.start()
    expect(context.stage.value).toBe('checking')
  })

  it('drops malformed progress payloads instead of applying them', async () => {
    const context = createUpdateInstall()
    mocks.mockInvoke.mockResolvedValueOnce(progress({ stage: 'downloading' }))
    await context.start()

    emitProgress({ stage: 'exploding' })
    expect(context.stage.value).toBe('downloading')
    expect(mocks.mockDebugError).toHaveBeenCalled()
  })

  it('start() is a no-op while a pipeline is active', async () => {
    const context = createUpdateInstall()
    mocks.mockInvoke.mockResolvedValueOnce(progress({ stage: 'downloading' }))
    await context.start()
    await context.start()
    expect(mocks.mockInvoke).toHaveBeenCalledTimes(1)
  })
})
