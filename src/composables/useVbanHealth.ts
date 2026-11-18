import { onMounted, onUnmounted, shallowRef, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { createAsyncCleanupScope } from '../utils/asyncCleanup'
import { debugError } from '../utils/debug'

/**
 * Контракт дорожки 139: снапшот и событие возвращают один и тот же DTO
 * ({ state, reason }) из `get_vban_health` / `vban-health-changed`.
 * `unknown` — вердикта ещё нет, `alive` — приёмник ответил на PING,
 * `failed` — ошибка установлена; `reason` объясняет `failed`
 * (`no_reply` — нет ответа, `local_socket` — локальная проверка не смогла
 * запуститься, например занят UDP-порт ответа).
 */
export const VBAN_HEALTH_CHANGED_EVENT = 'vban-health-changed'
/**
 * Alert stream for global toasts (ROADMAP-139): the backend emits it when the
 * failed verdict is established and when a production send starts while
 * failed, pre-deduplicated inside its own 5 s window. The payload is the same
 * {@link VbanHealth} DTO as `vban-health-changed`.
 */
export const VBAN_HEALTH_ALERT_EVENT = 'vban-health-alert'
export const GET_VBAN_HEALTH_COMMAND = 'get_vban_health'

export type VbanHealthStateName = 'unknown' | 'alive' | 'failed'
export type VbanHealthReason = 'no_reply' | 'local_socket'

export interface VbanHealth {
  state: VbanHealthStateName
  reason: VbanHealthReason | null
}

const HEALTH_STATES: readonly VbanHealthStateName[] = ['unknown', 'alive', 'failed']
const HEALTH_REASONS: readonly VbanHealthReason[] = ['no_reply', 'local_socket']

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

/**
 * Strict payload extraction. A payload with an unknown state is malformed and
 * is ignored entirely so it can never reset a shown verdict; a reason is
 * meaningful only next to a valid `failed` state — an unknown reason there
 * falls back to `null` (the generic failed text) so the red verdict itself
 * stays visible instead of being dropped.
 */
export function parseVbanHealth(raw: unknown): VbanHealth | null {
  if (!isRecord(raw)) return null
  const state = raw.state
  if (typeof state !== 'string' || !(HEALTH_STATES as readonly string[]).includes(state)) {
    return null
  }
  let reason: VbanHealthReason | null = null
  if (state === 'failed') {
    const rawReason = raw.reason
    reason =
      typeof rawReason === 'string' && (HEALTH_REASONS as readonly string[]).includes(rawReason)
        ? (rawReason as VbanHealthReason)
        : null
  }
  return { state: state as VbanHealthStateName, reason }
}

// Module-level singleton: one shared verdict for every consumer (Audio panel
// tab header now, global toasts later), independent of any panel being open.
// The listener lifecycle lives with the App (see `useVbanHealthMonitor`).
const health = shallowRef<VbanHealth>({ state: 'unknown', reason: null })

let monitorGeneration = 0
// Bumped by every health event so an in-flight snapshot never overwrites a
// newer transition (same guard as the OCR runtime notifications).
let eventToken = 0

/**
 * App-level lifecycle: registers the `vban-health-changed` listener first and
 * only then reads the boot snapshot, so nothing that happened before the
 * listener was registered is missed. Unmount unregisters the listener and a
 * late snapshot/event can no longer touch the shared verdict.
 * Call once; consumers read the status through `useVbanHealth`.
 */
export function useVbanHealthMonitor(): void {
  const listenerScope = createAsyncCleanupScope()
  let disposed = false
  const generation = ++monitorGeneration

  function applyHealth(value: VbanHealth | null): void {
    if (disposed || generation !== monitorGeneration || value === null) return
    health.value = value
  }

  onMounted(async () => {
    try {
      await listenerScope.track(
        listen<unknown>(VBAN_HEALTH_CHANGED_EVENT, (event) => {
          eventToken += 1
          applyHealth(parseVbanHealth(event.payload))
        }),
      )
    } catch (e) {
      debugError('[useVbanHealth] Failed to subscribe to VBAN health events:', e)
    }

    if (disposed || generation !== monitorGeneration) return

    const token = eventToken
    try {
      const payload = await invoke<unknown>(GET_VBAN_HEALTH_COMMAND)
      if (disposed || generation !== monitorGeneration || token !== eventToken) return
      applyHealth(parseVbanHealth(payload))
    } catch (e) {
      if (disposed || generation !== monitorGeneration || token !== eventToken) return
      debugError('[useVbanHealth] Failed to read the initial VBAN health:', e)
    }
  })

  onUnmounted(() => {
    disposed = true
    listenerScope.dispose()
  })
}

/** Read-only access to the shared VBAN receiver health verdict. */
export function useVbanHealth(): { health: Readonly<Ref<VbanHealth>> } {
  return { health }
}
