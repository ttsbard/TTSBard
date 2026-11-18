import { computed, ref, watch, type ComputedRef, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { VbanSettingsDto } from '../types/settings'

/**
 * Whether the saved VBAN configuration is complete enough to feed a target —
 * the client-side mirror of the existing target validation (valid IPv4
 * destination, port in range, stream name 1–16 chars). `enabled` is
 * intentionally not part of this: an unconfigured output is unavailable even
 * when enabled, and a configured one stays available while merely switched
 * off or while the receiver health is temporarily failed.
 */
export function isVbanTargetConfigured(settings: VbanSettingsDto | undefined): boolean {
  if (!settings) return false
  const address = settings.address.trim()
  const octets = address.split('.')
  const addressValid =
    octets.length === 4 &&
    octets.every((octet) => {
      if (!/^\d{1,3}$/.test(octet)) return false
      const value = Number(octet)
      return Number.isInteger(value) && value >= 0 && value <= 255
    })
  const portValid = Number.isInteger(settings.port) && settings.port >= 1 && settings.port <= 65535
  // Mirror of the backend `validate_stream_name`: non-empty, at most 16
  // bytes (ASCII length is byte length), only printable ASCII 0x20–0x7E —
  // so whitespace-only passes (the backend allows it) and any non-ASCII
  // name makes the output unavailable.
  const name = settings.stream_name
  const nameValid =
    name.length >= 1 &&
    name.length <= 16 &&
    [...name].every((ch) => {
      const code = ch.charCodeAt(0)
      return code >= 0x20 && code <= 0x7e
    })
  return addressValid && portValid && nameValid
}

/**
 * Editor output toggles (ROADMAP-142): one persist-first request per output.
 *
 * Contract:
 * - the displayed state is always the saved snapshot (`read`), never a local
 *   guess; before the first snapshot the toggle reports not ready and ignores
 *   attempts;
 * - while a command is in flight further toggles are ignored (pending guard),
 *   covering both the popover rows and the editor hotkeys — they share the
 *   same controller;
 * - after a successful persist the snapshot is refreshed explicitly. The
 *   coalescing loader contract matters here: `reload()` (useAppSettings.load)
 *   returns immediately when another load is already active and only marks a
 *   pending follow-up, so the controller additionally waits until
 *   `isLoading` settles before releasing the guard. The loader flips
 *   `isLoading` false and back to true synchronously when it starts the
 *   follow-up, and Vue batches watcher notifications, so the watcher never
 *   observes the gap;
 * - `reload` follows the useAppSettings contract: it never rejects, failures
 *   land in the shared settings error state and the old snapshot stays — the
 *   UI simply keeps showing the saved state instead of a fake success;
 * - a command failure is surfaced through `onError` with no fake rollback:
 *   the snapshot remains authoritative;
 * - `pending` is released in `finally`, so neither a failed command, a failed
 *   refresh, nor a settled-but-stale snapshot can lock the toggle;
 * - an unavailable output (`isAvailable` returns false — unconfigured or its
 *   hardware disappeared) is never toggled: the guard covers the popover
 *   click and the editor hotkey alike, because both call this same action.
 */
export interface OutputToggleOptions {
  /** Current saved value; `undefined` until the first settings snapshot. */
  read: () => boolean | undefined
  /** Persist-first backend command, e.g. `set_virtual_mic_enabled`. */
  command: string
  /** Refresh the settings snapshot after a successful persist. */
  reload: () => Promise<unknown>
  /** Shared loader state (`useAppSettings.isLoading`), for the coalesced wait. */
  isLoading: Ref<boolean>
  /**
   * Whether the output currently has what it needs to feed (configured
   * hardware / valid target). Omitted means always available.
   */
  isAvailable?: () => boolean
  /** Present a command failure (the fallback text is already localized). */
  onError: (error: unknown) => void
}

export interface OutputToggleController {
  enabled: ComputedRef<boolean>
  ready: ComputedRef<boolean>
  /** False while the output is unconfigured or its hardware is gone. */
  available: ComputedRef<boolean>
  pending: Ref<boolean>
  toggle: () => Promise<void>
}

/** Resolve when the settings loader (including its follow-up) goes idle. */
function waitUntilIdle(isLoading: Ref<boolean>): Promise<void> {
  return new Promise((resolve) => {
    if (!isLoading.value) {
      resolve()
      return
    }
    const stop = watch(isLoading, (loading) => {
      if (!loading) {
        stop()
        resolve()
      }
    })
  })
}

export function createOutputToggle(options: OutputToggleOptions): OutputToggleController {
  const pending = ref(false)
  const enabled = computed(() => options.read() ?? false)
  const ready = computed(() => options.read() !== undefined)
  const available = computed(() => options.isAvailable?.() ?? true)

  async function toggle(): Promise<void> {
    const current = options.read()
    // Unavailable outputs are blocked here, not in the click handler: the
    // editor hotkey resolves to this same action and must be blocked too.
    if (pending.value || current === undefined || !available.value) return
    pending.value = true
    try {
      await invoke(options.command, { enabled: !current })
      // Persist-first: the resolved command means the value is stored; pull
      // the confirming snapshot even if the settings-changed reload already
      // landed or never fired. A coalesced call returns before the snapshot
      // arrives — wait for the active load (and its follow-up) to settle so
      // the guard never releases ahead of the confirming state.
      await options.reload()
      if (options.isLoading.value) {
        await waitUntilIdle(options.isLoading)
      }
    } catch (error) {
      options.onError(error)
    } finally {
      pending.value = false
    }
  }

  return { enabled, ready, available, pending, toggle }
}
