import { ref, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

/** Device descriptor from `get_virtual_mic_devices` (mirrors the backend DTO). */
interface DeviceInfo {
  id: string
  name: string
  is_default: boolean
}

export interface VirtualMicDeviceCheck {
  /**
   * Whether `deviceId` refers to a device that currently exists. An empty id,
   * an unknown list (not loaded yet / IPC failure) and a missing device all
   * answer `false`: an unknown list never counts as availability.
   */
  isSelectedDeviceAvailable: (deviceId: string | null | undefined) => boolean
  /**
   * Refresh the device list from the IPC. Concurrent calls are safe: only the
   * latest response wins, a stale reply cannot overwrite a fresh one, and an
   * error leaves the list unknown (no false availability) — the next refresh
   * retries.
   */
  refresh: () => Promise<void>
  /** Raw known device ids; `null` while the list is not known. */
  deviceIds: Ref<Set<string> | null>
}

/**
 * Virtual microphone device existence check for the output toggles
 * (ROADMAP-142 polish): a saved device id alone must not count as a
 * configured output — the device has to exist in the actual device list.
 * Refreshes are explicit (menu open / selected device change), no polling.
 */
export function createVirtualMicDeviceCheck(
  invokeFn: (cmd: string) => Promise<unknown> = invoke,
): VirtualMicDeviceCheck {
  const deviceIds: Ref<Set<string> | null> = ref(null)
  let latestRequest = 0

  async function refresh(): Promise<void> {
    const request = ++latestRequest
    // The last confirmed list stays visible while a re-check runs — a user
    // requirement that overrides the earlier "grey immediately" rule: no
    // available→disabled→available flicker on every menu open. Only the
    // outcome of the newest request changes the state: a fresh success
    // replaces the list (a vanished device becomes unavailable), a fresh
    // failure marks the list unknown (= unavailable), and a stale response
    // or error changes nothing.
    try {
      const devices = (await invokeFn('get_virtual_mic_devices')) as DeviceInfo[]
      if (request !== latestRequest) return // a newer refresh owns the list
      deviceIds.value = new Set(devices.map((device) => device.id))
    } catch {
      if (request !== latestRequest) return
      // Unknown is not available: keep the output greyed until a refresh
      // succeeds; the next menu open retries.
      deviceIds.value = null
    }
  }

  function isSelectedDeviceAvailable(deviceId: string | null | undefined): boolean {
    if (!deviceId) return false
    if (deviceIds.value === null) return false
    return deviceIds.value.has(deviceId)
  }

  return { isSelectedDeviceAvailable, refresh, deviceIds }
}
