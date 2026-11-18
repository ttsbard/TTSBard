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

import { createAppUpdates, upToDateFromStatus } from './useAppUpdates'
import { UPDATE_STATUS_CHANGED_EVENT, type UpdateStatusDto } from '../ipc/appUpdates'

function validStatus(overrides: Partial<UpdateStatusDto> = {}): UpdateStatusDto {
  return {
    current_version: '1.8.0',
    update_available: true,
    latest: {
      version: '1.9.0',
      name: 'TTSBard 1.9.0',
      notes: 'What is new',
      url: 'https://github.com/ttsbard/TTSBard/releases/tag/v1.9.0',
      published_at: '2026-11-16T10:00:00Z',
    },
    last_attempt_at: '2026-11-18T09:00:00Z',
    last_attempt_error: null,
    last_check_at: '2026-11-18T09:00:00Z',
    checking: false,
    startup_check_enabled: true,
    periodic_check_enabled: false,
    check_interval_days: 1,
    dismissed_version: null,
    install_available: true,
    ...overrides,
  }
}

beforeEach(() => {
  mocks.mockInvoke.mockReset()
  mocks.mockDebugError.mockReset()
  mocks.listenCallbacks.clear()
})

function emitStatus(payload: unknown): void {
  const callback = mocks.listenCallbacks.get(UPDATE_STATUS_CHANGED_EVENT)
  expect(callback, 'status listener must be registered').toBeDefined()
  callback?.({ payload })
}

describe('createAppUpdates', () => {
  it('loads the boot snapshot on creation', async () => {
    const snapshot = validStatus({ update_available: false })
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      expect(cmd).toBe('get_update_status')
      return snapshot
    })

    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).toEqual(snapshot))
    expect(context.panelRelease.value).toBeNull()
  })

  it('survives a failing boot read with a null status', async () => {
    mocks.mockInvoke.mockRejectedValue('backend not ready')
    const context = createAppUpdates()
    await vi.waitFor(() => expect(mocks.mockDebugError).toHaveBeenCalled())
    expect(context.status.value).toBeNull()
  })

  it('exposes the panel release only when newer and not dismissed', async () => {
    mocks.mockInvoke.mockResolvedValue(validStatus())
    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())
    expect(context.panelRelease.value?.version).toBe('1.9.0')

    // Dismissing the shown version hides the panel but keeps the snapshot.
    emitStatus(validStatus({ dismissed_version: '1.9.0' }))
    expect(context.panelRelease.value).toBeNull()
    expect(context.status.value?.latest?.version).toBe('1.9.0')

    // A newer release shows the panel again.
    emitStatus(validStatus({
      latest: { ...validStatus().latest!, version: '1.9.1' },
    }))
    expect(context.panelRelease.value?.version).toBe('1.9.1')

    // The release matching the running version never shows the panel.
    emitStatus(validStatus({ update_available: false }))
    expect(context.panelRelease.value).toBeNull()
  })

  it('ignores malformed event payloads instead of corrupting the state', async () => {
    mocks.mockInvoke.mockResolvedValue(validStatus())
    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())

    emitStatus({ current_version: 1 })
    expect(context.status.value?.current_version).toBe('1.8.0')
    expect(mocks.mockDebugError).toHaveBeenCalled()

    // Unknown attempt error codes and wrong switch types drop the payload too.
    emitStatus({ ...validStatus(), last_attempt_error: 'boom: crashed' })
    emitStatus({ ...validStatus(), periodic_check_enabled: 'no' })
    expect(context.status.value?.last_attempt_error).toBeNull()
    expect(context.status.value?.periodic_check_enabled).toBe(false)
  })

  it('applies the manual check result and clears the in-flight flag', async () => {
    const checked = validStatus({
      last_attempt_at: '2026-11-18T12:00:00Z',
      last_check_at: '2026-11-18T12:00:00Z',
      update_available: false,
    })
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      expect(cmd).toBe('check_for_updates')
      return checked
    })

    const context = createAppUpdates()
    await context.checkNow()
    expect(context.status.value).toEqual(checked)
    expect(context.checking.value).toBe(false)
    expect(context.commandError.value).toBeNull()
  })

  it('reports a manual check failure without losing the found release', async () => {
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_update_status') return validStatus()
      if (cmd === 'check_for_updates') throw { code: 'errors.ipc_timeout', message: 'timed out' }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())
    await context.checkNow()

    expect(context.commandError.value).toBeTruthy()
    // The last snapshot (including the release) stays intact.
    expect(context.status.value?.latest?.version).toBe('1.9.0')
    expect(context.checking.value).toBe(false)
  })

  it('sends both switches and the interval with full snapshots back', async () => {
    mocks.mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'get_update_status') {
        return validStatus({ startup_check_enabled: true, periodic_check_enabled: true })
      }
      if (cmd === 'set_updates_startup_check') {
        expect(args).toEqual({ enabled: false })
        return validStatus({ startup_check_enabled: false, periodic_check_enabled: true })
      }
      if (cmd === 'set_updates_periodic_check') {
        expect(args).toEqual({ enabled: false })
        return validStatus({ startup_check_enabled: false, periodic_check_enabled: false })
      }
      if (cmd === 'set_updates_check_interval') {
        expect(args).toEqual({ days: 5 })
        return validStatus({ check_interval_days: 5 })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())

    await context.setStartupCheck(false)
    expect(context.status.value?.startup_check_enabled).toBe(false)
    // The two switches are independent: turning one off keeps the other on.
    expect(context.status.value?.periodic_check_enabled).toBe(true)

    await context.setPeriodicCheck(false)
    expect(context.status.value?.periodic_check_enabled).toBe(false)
    expect(context.status.value?.startup_check_enabled).toBe(false)

    // The interval is kept even with the schedule off.
    await context.setIntervalDays(5)
    expect(context.status.value?.check_interval_days).toBe(5)
  })

  it('rolls a failed settings command back by leaving the snapshot unchanged', async () => {
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_update_status') return validStatus({ periodic_check_enabled: true })
      if (cmd === 'set_updates_periodic_check') throw { code: 'errors.io_write', message: 'disk full' }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value?.periodic_check_enabled).toBe(true))

    await context.setPeriodicCheck(false)
    // No snapshot arrived, so the toggle snaps back to the persisted value.
    expect(context.status.value?.periodic_check_enabled).toBe(true)
    expect(context.commandError.value).toBeTruthy()
  })

  it('dismisses the panel with the shown release version', async () => {
    mocks.mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'get_update_status') return validStatus()
      if (cmd === 'dismiss_update_panel') {
        expect(args).toEqual({ version: '1.9.0' })
        return validStatus({ dismissed_version: '1.9.0' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())
    await context.dismissPanel()
    expect(context.status.value?.dismissed_version).toBe('1.9.0')
    expect(context.panelRelease.value).toBeNull()
  })

  it('ignores dismiss when no release is known', async () => {
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_update_status') return validStatus({ latest: null, update_available: false })
      throw new Error(`unexpected command: ${cmd}`)
    })

    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())
    await context.dismissPanel()
    expect(mocks.mockInvoke).not.toHaveBeenCalledWith('dismiss_update_panel', expect.anything())
  })

  it('drops a stale settings reply that lost to a live event and reloads the saved setting', async () => {
    // The saved periodic=false snapshot arrives through the event while the
    // settings IPC is still pending; the older reply (periodic=true) must not
    // overwrite it, and a reload picks up the persisted switch.
    let resolveSettings!: (v: UpdateStatusDto) => void
    let reloads = 0
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_update_status') {
        reloads += 1
        // The reload (second read) returns the saved switch with the fresh
        // check stamp; the boot read still has the old schedule.
        return reloads > 1
          ? validStatus({ periodic_check_enabled: false, last_check_at: '2026-11-18T10:00:00Z' })
          : validStatus({ periodic_check_enabled: true })
      }
      if (cmd === 'set_updates_periodic_check') {
        return new Promise<UpdateStatusDto>((resolve) => { resolveSettings = resolve })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value?.periodic_check_enabled).toBe(true))

    const pending = context.setPeriodicCheck(false)
    // A live event (another initiator's check) lands mid-request.
    emitStatus(validStatus({ periodic_check_enabled: true, last_check_at: '2026-11-18T10:00:00Z' }))
    // The deferred reply carries an older snapshot with a recognizable marker.
    resolveSettings(validStatus({
      periodic_check_enabled: true,
      last_attempt_at: '2026-12-31T00:00:00Z',
    }))
    await pending

    // The stale reply was dropped (no marker), the reload shows the saved
    // switch together with the fresh stamp.
    expect(context.status.value?.last_attempt_at).not.toBe('2026-12-31T00:00:00Z')
    expect(context.status.value?.last_check_at).toBe('2026-11-18T10:00:00Z')
    await vi.waitFor(() => expect(context.status.value?.periodic_check_enabled).toBe(false))
    expect(context.commandError.value).toBeNull()
  })

  it('drops a stale dismiss reply that lost to a live event and reloads the dismissal', async () => {
    let resolveDismiss!: (v: UpdateStatusDto) => void
    let reloads = 0
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_update_status') {
        reloads += 1
        // The reload (second read) returns the persisted dismissal with the
        // fresh check stamp; the boot read has no dismissal yet.
        return reloads > 1
          ? validStatus({ dismissed_version: '1.9.0', last_check_at: '2026-11-18T11:00:00Z' })
          : validStatus()
      }
      if (cmd === 'dismiss_update_panel') {
        return new Promise<UpdateStatusDto>((resolve) => { resolveDismiss = resolve })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())

    const pending = context.dismissPanel()
    emitStatus(validStatus({ last_check_at: '2026-11-18T11:00:00Z' }))
    // The deferred reply carries an older snapshot with a recognizable marker.
    resolveDismiss(validStatus({
      dismissed_version: null,
      last_attempt_at: '2026-12-31T00:00:00Z',
    }))
    await pending

    // The stale reply was dropped (no marker, no panel resurrection)...
    expect(context.status.value?.last_attempt_at).not.toBe('2026-12-31T00:00:00Z')
    // ...and the reload applied the persisted dismissal with the fresh stamp.
    expect(context.status.value?.last_check_at).toBe('2026-11-18T11:00:00Z')
    await vi.waitFor(() => expect(context.panelRelease.value).toBeNull())
    expect(context.status.value?.dismissed_version).toBe('1.9.0')
  })

  it('drops a stale check reply when a live event finished first', async () => {
    let resolveCheck!: (v: UpdateStatusDto) => void
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_update_status') return validStatus({ update_available: false, latest: null })
      if (cmd === 'check_for_updates') {
        return new Promise<UpdateStatusDto>((resolve) => { resolveCheck = resolve })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())

    const pending = context.checkNow()
    // The backend broadcast arrives before the command resolves.
    const finished = validStatus({
      update_available: false,
      latest: null,
      last_attempt_at: '2026-11-18T12:00:00Z',
      last_check_at: '2026-11-18T12:00:00Z',
    })
    emitStatus(finished)
    // The deferred command answer is the older pre-check snapshot.
    resolveCheck(validStatus())
    await pending

    expect(context.status.value).toEqual(finished)
    expect(context.upToDate.value).toBe(true)
    expect(context.checking.value).toBe(false)
    expect(context.commandError.value).toBeNull()
  })
})

describe('up-to-date outcome (ROADMAP-141 stage 2)', () => {
  it('marks the card up to date after a finished manual check, with no expiry timer', async () => {
    const checked = validStatus({ update_available: false, latest: null })
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_update_status') return validStatus()
      if (cmd === 'check_for_updates') return checked
      throw new Error(`unexpected command: ${cmd}`)
    })

    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())
    expect(context.upToDate.value).toBe(false)

    // Fake timers are installed before the attempt, so a scheduled expiry
    // would fire: the outcome must survive any amount of time instead.
    vi.useFakeTimers()
    try {
      await context.checkNow()
      expect(context.upToDate.value).toBe(true)
      vi.advanceTimersByTime(60_000)
      expect(context.upToDate.value).toBe(true)
    } finally {
      vi.useRealTimers()
    }
  })

  it('marks the card up to date from a background event without any manual action', async () => {
    // A scheduled (startup or periodic) check finished and found nothing
    // newer: the outcome belongs to every initiator alike, no manual-only
    // note and no timer.
    const scheduled = validStatus({
      update_available: false,
      latest: null,
      last_attempt_at: '2026-11-18T07:30:00Z',
      last_check_at: '2026-11-18T07:30:00Z',
    })
    mocks.mockInvoke.mockResolvedValue(validStatus({ update_available: false, latest: null, last_attempt_at: null, last_check_at: null }))
    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())
    expect(context.upToDate.value).toBe(false)

    emitStatus(scheduled)
    expect(context.upToDate.value).toBe(true)
  })

  it('never claims up to date before the first attempt completes', () => {
    expect(upToDateFromStatus(validStatus({
      update_available: false,
      latest: null,
      last_attempt_at: null,
      last_attempt_error: null,
    }))).toBe(false)
    expect(upToDateFromStatus(null)).toBe(false)
  })

  it('does not claim up to date when the last attempt failed inside the snapshot', async () => {
    const failed = validStatus({ update_available: false, latest: null, last_attempt_error: 'network' })
    mocks.mockInvoke.mockResolvedValue(failed)
    const context = createAppUpdates()

    await context.checkNow()
    expect(context.upToDate.value).toBe(false)
  })

  it('keeps the previous up-to-date outcome while a new check is in flight', async () => {
    // The manual command collides with a scheduled check and answers with the
    // current snapshot (`checking: true`); the completed outcome stays shown
    // and nothing fabricates a fresh success.
    mocks.mockInvoke.mockResolvedValue(validStatus({ update_available: false, latest: null }))
    const context = createAppUpdates()
    await context.checkNow()
    expect(context.upToDate.value).toBe(true)

    emitStatus(validStatus({
      update_available: false,
      latest: null,
      checking: true,
      last_attempt_at: '2026-11-18T09:00:00Z',
    }))
    expect(context.checking.value).toBe(true)
    expect(context.upToDate.value).toBe(true)
  })

  it('shows a failed attempt beside the previously found release', async () => {
    mocks.mockInvoke.mockResolvedValue(validStatus({ last_attempt_error: null }))
    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value).not.toBeNull())

    // The fresh failure keeps the known release, its notes and the last
    // success time; it does not overwrite the success stamp.
    emitStatus(validStatus({
      last_attempt_at: '2026-11-18T10:00:00Z',
      last_attempt_error: 'rate_limited',
      last_check_at: '2026-11-18T09:00:00Z',
    }))
    expect(context.status.value?.last_attempt_error).toBe('rate_limited')
    expect(context.upToDate.value).toBe(false)
    expect(context.status.value?.latest?.version).toBe('1.9.0')
    expect(context.panelRelease.value?.version).toBe('1.9.0')
    expect(context.status.value?.last_check_at).toBe('2026-11-18T09:00:00Z')
  })

  it('clears the previous error when a later attempt succeeds', async () => {
    mocks.mockInvoke.mockResolvedValue(validStatus({
      update_available: false,
      latest: null,
      last_attempt_at: '2026-11-18T10:00:00Z',
      last_attempt_error: 'network',
      last_check_at: null,
    }))
    const context = createAppUpdates()
    await vi.waitFor(() => expect(context.status.value?.last_attempt_error).toBe('network'))
    expect(context.upToDate.value).toBe(false)

    // The success replaces the error; the outcome becomes up to date again.
    emitStatus(validStatus({
      update_available: false,
      latest: null,
      last_attempt_at: '2026-11-18T11:00:00Z',
      last_attempt_error: null,
      last_check_at: '2026-11-18T11:00:00Z',
    }))
    expect(context.status.value?.last_attempt_error).toBeNull()
    expect(context.upToDate.value).toBe(true)
  })

  it('stays up to date when a later background attempt is also up to date', async () => {
    mocks.mockInvoke.mockResolvedValue(validStatus({ update_available: false, latest: null }))
    const context = createAppUpdates()
    await context.checkNow()
    expect(context.upToDate.value).toBe(true)

    emitStatus(validStatus({
      update_available: false,
      latest: null,
      last_attempt_at: '2026-11-18T18:00:00Z',
      last_check_at: '2026-11-18T18:00:00Z',
    }))
    expect(context.upToDate.value).toBe(true)
  })
})
