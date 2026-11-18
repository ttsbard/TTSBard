import { onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { createAsyncCleanupScope } from '../utils/asyncCleanup'
import { debugError } from '../utils/debug'
import {
  GET_VBAN_HEALTH_COMMAND,
  parseVbanHealth,
  VBAN_HEALTH_ALERT_EVENT,
  VBAN_HEALTH_CHANGED_EVENT,
  type VbanHealth,
} from './useVbanHealth'
import { useErrorHandler } from './useErrorHandler'
import { t } from '../i18n'

/**
 * Fixed locale key per failure reason (plus the generic fallback for a
 * failed verdict without a known reason). Raw backend text never reaches the
 * toast.
 */
export function vbanHealthToastMessage(reason: VbanHealth['reason']): string {
  if (reason === 'no_reply') return t('audio.vban.toast.no_reply')
  if (reason === 'local_socket') return t('audio.vban.toast.local_socket')
  return t('audio.vban.toast.failed')
}

/**
 * Global (App-level) VBAN receiver failure toasts, independent of the Audio
 * panel ever being opened (ROADMAP-139).
 *
 * Only the `vban-health-alert` stream toasts: plain status transitions
 * (`alive`/`unknown`) and malformed payloads stay silent, and sound is never
 * blocked. While one of these toasts is still visible a repeat alert is
 * suppressed; once it is gone the next alert — e.g. a production send start
 * into the still-failed receiver — warns again. The boot snapshot provides
 * the catch-up for a failure established before the alert listener existed,
 * and any valid alert or status transition delivered while the snapshot is
 * in flight supersedes it, so a disable/retarget reset or a fresh reply can
 * never let the stale failed read toast. The shared verdict itself stays
 * owned by `useVbanHealthMonitor`; the changed stream never toasts here.
 */
export function useVbanHealthNotifications(): void {
  const { showError, errors } = useErrorHandler()
  const listenerScope = createAsyncCleanupScope()
  let disposed = false
  let visibleToastId: string | null = null
  // Bumped by every valid alert event and by every valid status transition so
  // an in-flight boot snapshot never toasts on a verdict a live event has
  // already superseded (same guard as the monitor).
  let catchUpToken = 0

  /** The dedup target is the visible toast itself, not a fixed timer. */
  function isToastVisible(): boolean {
    return visibleToastId !== null && errors.value.some((error) => error.id === visibleToastId)
  }

  function applyAlert(health: VbanHealth | null): void {
    if (disposed || health === null) return
    if (health.state !== 'failed') return
    if (isToastVisible()) return
    visibleToastId = showError(vbanHealthToastMessage(health.reason))
  }

  onMounted(async () => {
    try {
      await listenerScope.track(
        listen<unknown>(VBAN_HEALTH_ALERT_EVENT, (event) => {
          catchUpToken += 1
          applyAlert(parseVbanHealth(event.payload))
        }),
      )
    } catch (e) {
      debugError('[useVbanHealthNotifications] Failed to subscribe to VBAN health alerts:', e)
    }

    // Second stream, invalidation-only: a valid status transition (alive,
    // unknown after a disable/retarget, or failed) is newer than the boot
    // read and cancels the catch-up. It never applies a verdict and never
    // toasts; a malformed payload proves nothing and must not invalidate.
    try {
      await listenerScope.track(
        listen<unknown>(VBAN_HEALTH_CHANGED_EVENT, (event) => {
          if (parseVbanHealth(event.payload) !== null) catchUpToken += 1
        }),
      )
    } catch (e) {
      debugError('[useVbanHealthNotifications] Failed to subscribe to VBAN health changes:', e)
    }

    if (disposed) return

    // Catch-up: the failure may have been established before this listener
    // existed, so the boot snapshot toasts it too.
    const token = catchUpToken
    try {
      const payload = await invoke<unknown>(GET_VBAN_HEALTH_COMMAND)
      if (disposed || token !== catchUpToken) return
      applyAlert(parseVbanHealth(payload))
    } catch (e) {
      if (disposed || token !== catchUpToken) return
      debugError('[useVbanHealthNotifications] Failed to read the initial VBAN health:', e)
    }
  })

  onUnmounted(() => {
    disposed = true
    listenerScope.dispose()
  })
}
