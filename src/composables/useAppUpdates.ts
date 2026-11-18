/**
 * Composable owning the app-level update state (ROADMAP-141, stage 2).
 *
 * Created once in `App.vue` and provided to the tree: the update banner in the
 * normal main window and the «Обновления» card in Settings both read the same
 * snapshot. The backend pushes `update-status-changed` after every check, so
 * a background check performed in the tray updates an open card without it
 * doing anything. The outcome of the last completed attempt (up to date,
 * release found, failed) is derived from the snapshot, so it survives
 * restarts, works for every initiator and stays until the next completed
 * attempt — never faked while a new check is in flight.
 */

import {
  computed,
  getCurrentScope,
  inject,
  onScopeDispose,
  provide,
  ref,
  type ComputedRef,
  type InjectionKey,
  type Ref,
} from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { createAsyncCleanupScope } from '../utils/asyncCleanup'
import { debugError } from '../utils/debug'
import { presentCommandError } from '../ipc/commandError'
import { t } from '../i18n'
import {
  CHECK_FOR_UPDATES_COMMAND,
  DISMISS_UPDATE_PANEL_COMMAND,
  GET_UPDATE_STATUS_COMMAND,
  parseUpdateStatus,
  SET_UPDATES_CHECK_INTERVAL_COMMAND,
  SET_UPDATES_PERIODIC_CHECK_COMMAND,
  SET_UPDATES_STARTUP_CHECK_COMMAND,
  UPDATE_STATUS_CHANGED_EVENT,
  type ReleaseInfoDto,
  type UpdateStatusDto,
} from '../ipc/appUpdates'

export interface AppUpdatesContext {
  /** Latest known state; `null` until the first snapshot arrives. */
  status: Ref<UpdateStatusDto | null>
  /** A check is running (local in-flight or reported by the backend). */
  checking: ComputedRef<boolean>
  /**
   * The release the update panel may announce: strictly newer than the
   * running version and not the version the user dismissed. The card shows
   * the release regardless of this flag.
   */
  panelRelease: ComputedRef<ReleaseInfoDto | null>
  /** Unexpected command failure for the card to display (localized). */
  commandError: Ref<string | null>
  /**
   * The last completed attempt confirmed the running version is current
   * (an attempt finished, it did not fail and found nothing newer). Derived
   * from the snapshot for every initiator — startup, periodic and manual
   * alike; it is not cleared while a new check is in flight.
   */
  upToDate: ComputedRef<boolean>
  checkNow(): Promise<void>
  setStartupCheck(enabled: boolean): Promise<void>
  setPeriodicCheck(enabled: boolean): Promise<void>
  setIntervalDays(days: number): Promise<void>
  dismissPanel(): Promise<void>
}

/**
 * The «up to date» outcome of the last completed attempt, derived from one
 * snapshot: it needs a finished attempt (`last_attempt_at`), no failure in
 * it, and nothing newer than the running version. Before the first attempt
 * and after a failed one the card never claims the running version is
 * current; while a new check is in flight the previous outcome stays shown
 * until the attempt that replaces it completes.
 */
export function upToDateFromStatus(snapshot: UpdateStatusDto | null): boolean {
  return snapshot !== null
    && snapshot.last_attempt_at !== null
    && snapshot.last_attempt_error === null
    && snapshot.update_available === false
}

/** Validate and apply a wire snapshot; malformed payloads are ignored. */
function applyStatus(status: Ref<UpdateStatusDto | null>, payload: unknown): boolean {
  const parsed = parseUpdateStatus(payload)
  if (!parsed) return false
  status.value = parsed
  return true
}

/**
 * Guard against stale IPC replies: a `update-status-changed` event that lands
 * while a request is in flight is strictly newer than the snapshot that
 * request will return, so the reply must not overwrite it. Each request
 * captures the event-applied snapshot counter before invoking and its reply is
 * discarded when an event was applied meanwhile.
 */
function createSnapshotGuard(status: Ref<UpdateStatusDto | null>) {
  // Bumped only when a live event applies a snapshot; IPC replies order
  // among themselves through their own requests.
  let eventApplied = 0

  function beginRequest(): number {
    return eventApplied
  }

  /** Apply an event payload, marking the state as newer than pending replies. */
  function applyEvent(payload: unknown): boolean {
    if (!applyStatus(status, payload)) return false
    eventApplied += 1
    return true
  }

  /**
   * Apply a reply snapshot. A stale reply (an event was applied while the
   * request was in flight) is discarded; the caller decides on the fallback.
   */
  function applyReply(
    beginVersion: number,
    payload: unknown,
  ): 'applied' | 'stale' | 'malformed' {
    if (eventApplied !== beginVersion) return 'stale'
    return applyStatus(status, payload) ? 'applied' : 'malformed'
  }

  async function load(): Promise<void> {
    const beginVersion = beginRequest()
    try {
      const snapshot = await invoke<unknown>(GET_UPDATE_STATUS_COMMAND)
      applyReply(beginVersion, snapshot)
    } catch (e) {
      debugError('[useAppUpdates] Failed to read the update status:', e)
    }
  }

  return { beginRequest, applyEvent, applyReply, load }
}

export function createAppUpdates(): AppUpdatesContext {
  const status = ref<UpdateStatusDto | null>(null)
  const commandError = ref<string | null>(null)
  const localChecking = ref(false)
  const listenerScope = createAsyncCleanupScope()
  const guard = createSnapshotGuard(status)

  const checking = computed(() => localChecking.value || status.value?.checking === true)

  /** Derived «up to date» outcome of the last completed attempt. */
  const upToDate = computed(() => upToDateFromStatus(status.value))

  /**
   * The release behind the panel: strictly newer than the running version and
   * not dismissed. A dismissed version stays hidden across re-checks and
   * restarts; a newer release shows the panel again, and an installed version
   * hides it through `update_available` turning false.
   */
  const panelRelease = computed<ReleaseInfoDto | null>(() => {
    const snapshot = status.value
    if (!snapshot?.update_available || !snapshot.latest) return null
    if (snapshot.latest.version === snapshot.dismissed_version) return null
    return snapshot.latest
  })

  const load = guard.load

  async function checkNow(): Promise<void> {
    if (localChecking.value) return
    localChecking.value = true
    commandError.value = null
    try {
      // The backend records the attempt (time and outcome) and broadcasts it;
      // a snapshot that is still checking means a scheduled check held the
      // guard and the command returned the current state instead of our
      // result — the real outcome arrives through `update-status-changed`.
      // The previous completed outcome stays visible meanwhile.
      const beginVersion = guard.beginRequest()
      const snapshot = await invoke<unknown>(CHECK_FOR_UPDATES_COMMAND)
      // A live event applied meanwhile is newer; the stale reply is dropped.
      guard.applyReply(beginVersion, snapshot)
    } catch (e) {
      commandError.value = presentCommandError(e, t('general.updates.errors.generic'))
      debugError('[useAppUpdates] Manual update check failed:', e)
    } finally {
      localChecking.value = false
    }
  }

  // All settings commands share the same shape: literal command names keep
  // the IPC inventory exact; a stale reply is discarded and refreshed (so the
  // saved setting still shows up), a malformed one falls back to a reload.
  async function applySettingsCommand(invokeCall: () => Promise<unknown>): Promise<void> {
    const beginVersion = guard.beginRequest()
    const snapshot = await invokeCall()
    const outcome = guard.applyReply(beginVersion, snapshot)
    // A stale reply was superseded by a live event, but no event carries the
    // saved setting itself — reload so it still appears.
    if (outcome !== 'applied') await load()
  }

  function reportSettingsError(source: string, e: unknown): void {
    // The toggle in the card snaps back because `status` is unchanged; the
    // localized failure explains why. Kept separate from the check-failure
    // plate so a save problem is never masked by the network phrase.
    commandError.value = presentCommandError(e, t('general.updates.errors.save'))
    debugError('[useAppUpdates] Update settings command failed:', source, e)
  }

  async function setStartupCheck(enabled: boolean): Promise<void> {
    commandError.value = null
    try {
      await applySettingsCommand(() =>
        invoke<unknown>(SET_UPDATES_STARTUP_CHECK_COMMAND, { enabled }))
    } catch (e) {
      reportSettingsError('set_updates_startup_check', e)
    }
  }

  async function setPeriodicCheck(enabled: boolean): Promise<void> {
    commandError.value = null
    try {
      await applySettingsCommand(() =>
        invoke<unknown>(SET_UPDATES_PERIODIC_CHECK_COMMAND, { enabled }))
    } catch (e) {
      reportSettingsError('set_updates_periodic_check', e)
    }
  }

  async function setIntervalDays(days: number): Promise<void> {
    commandError.value = null
    try {
      await applySettingsCommand(() =>
        invoke<unknown>(SET_UPDATES_CHECK_INTERVAL_COMMAND, { days }))
    } catch (e) {
      reportSettingsError('set_updates_check_interval', e)
    }
  }

  async function dismissPanel(): Promise<void> {
    const version = status.value?.latest?.version
    if (!version) return
    try {
      const beginVersion = guard.beginRequest()
      const snapshot = await invoke<unknown>(DISMISS_UPDATE_PANEL_COMMAND, { version })
      // Like settings: a stale reply is superseded by a live event, reload so
      // the persisted dismissal still appears.
      if (guard.applyReply(beginVersion, snapshot) !== 'applied') await load()
    } catch (e) {
      commandError.value = presentCommandError(e, t('general.updates.errors.save'))
      debugError('[useAppUpdates] Failed to dismiss the update panel:', e)
    }
  }

  // Live updates: every backend check (startup, periodic, manual from any
  // window) rebroadcasts the snapshot; malformed payloads are ignored.
  listenerScope
    .track(
      listen<unknown>(UPDATE_STATUS_CHANGED_EVENT, (event) => {
        if (!guard.applyEvent(event.payload)) {
          debugError('[useAppUpdates] Ignoring a malformed update status payload')
        }
      }),
    )
    .catch((e) => {
      debugError('[useAppUpdates] Failed to subscribe to update status events:', e)
    })

  void load()

  if (getCurrentScope()) {
    onScopeDispose(() => {
      listenerScope.dispose()
    })
  }

  if (import.meta.env.DEV && import.meta.hot) {
    import.meta.hot.dispose(() => {
      listenerScope.dispose()
    })
  }

  return {
    status,
    checking,
    panelRelease,
    commandError,
    upToDate,
    checkNow,
    setStartupCheck,
    setPeriodicCheck,
    setIntervalDays,
    dismissPanel,
  }
}

export const APP_UPDATES_KEY: InjectionKey<AppUpdatesContext> = Symbol('app-updates')

/** Owner call: create the context once and provide it to the tree. */
export function provideAppUpdates(): AppUpdatesContext {
  const context = createAppUpdates()
  provide(APP_UPDATES_KEY, context)
  return context
}

/** Consumer call for the banner and the settings card. */
export function useAppUpdates(): AppUpdatesContext {
  const context = inject<AppUpdatesContext>(APP_UPDATES_KEY)
  if (!context) {
    throw new Error('useAppUpdates must be used within a component that provides app updates')
  }
  return context
}
