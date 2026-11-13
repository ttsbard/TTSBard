import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { createAsyncCleanupScope } from '../utils/asyncCleanup'
import { debugError } from '../utils/debug'
import type { CaptureTargetDto, OcrSettingsDto } from '../types/settings'
import { t } from '../i18n'

// ============================================================================
// Typed DTOs (mirror the Rust OCR backend contract)
// ============================================================================

export interface OcrPackDto {
  id: string
  display_name: string
  languages: string[]
}

/** Position and size of a display in virtual-screen physical pixels. */
export interface MonitorGeometryDto {
  x: number
  y: number
  width: number
  height: number
}

/**
 * One selectable display, as returned by `list_ocr_monitors`. Mirrors the Rust
 * `MonitorInfo` (`devicePath`, `label`, `sourceName`, `isPrimary`, `geometry`).
 */
export interface MonitorInfoDto {
  devicePath: string
  label: string
  sourceName: string
  isPrimary: boolean
  geometry: MonitorGeometryDto
}

export const OCR_RUNTIME_STATES = [
  'disabled',
  'starting',
  'ready',
  'selectingArea',
  'recognizing',
  'error',
] as const

export type OcrRuntimeState = (typeof OCR_RUNTIME_STATES)[number]

/** Wire shape of `OcrStatus`: `{ state, message? }` (camelCase tag). */
export interface OcrStatusDto {
  state: OcrRuntimeState
  message?: string
}

const OCR_STATUS_CHANGED_EVENT = 'ocr-status-changed'
const SETTINGS_CHANGED_EVENT = 'settings-changed'

const DEFAULT_SETTINGS: OcrSettingsDto = {
  enabled: false,
  model_id: null,
  capture_target: { type: 'all' },
}

// Last known display names of ever-seen packs, so a model held in memory after
// its pack left the disk still renders a human-readable label.
const packLabelMemo = new Map<string, string>()

function rememberPackLabels(list: OcrPackDto[]): void {
  for (const pack of list) packLabelMemo.set(pack.id, pack.display_name)
}

// ============================================================================
// Pure validators / converters
// ============================================================================

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0
}

function isOcrRuntimeState(value: unknown): value is OcrRuntimeState {
  return typeof value === 'string' && (OCR_RUNTIME_STATES as readonly string[]).includes(value)
}

/**
 * Strict status converter. Unknown payloads and invalid states fall back to
 * `disabled`; only a string `message` on an `error` state is trusted.
 */
export function convertOcrStatusFromRust(raw: unknown): OcrStatusDto {
  if (!isRecord(raw)) return { state: 'disabled' }
  const state = raw.state
  if (!isOcrRuntimeState(state)) return { state: 'disabled' }
  if (state !== 'error') return { state }
  const message = raw.message
  return typeof message === 'string' ? { state: 'error', message } : { state: 'error' }
}

/**
 * Strict capture-target converter. A missing or malformed payload falls back to
 * the default `{"type":"all"}`; a `monitor` variant without a non-empty device
 * path is rejected the same way (backwards-compat default All).
 */
export function convertCaptureTargetFromRust(raw: unknown): CaptureTargetDto {
  if (!isRecord(raw)) return { type: 'all' }
  const type = raw.type
  if (type === 'all') return { type: 'all' }
  if (type === 'primary') return { type: 'primary' }
  if (type === 'monitor') {
    const devicePath = raw.devicePath
    return isNonEmptyString(devicePath) ? { type: 'monitor', devicePath } : { type: 'all' }
  }
  return { type: 'all' }
}

export function convertOcrSettingsFromRust(raw: unknown): OcrSettingsDto {
  if (!isRecord(raw)) return { ...DEFAULT_SETTINGS }
  const modelId = raw.model_id
  return {
    enabled: typeof raw.enabled === 'boolean' ? raw.enabled : DEFAULT_SETTINGS.enabled,
    model_id: isNonEmptyString(modelId) ? modelId : null,
    capture_target: convertCaptureTargetFromRust(raw.capture_target),
  }
}

function captureTargetsEqual(a: CaptureTargetDto, b: CaptureTargetDto): boolean {
  if (a.type !== b.type) return false
  if (a.type === 'monitor') return a.devicePath === b.devicePath
  return true
}

function ocrSettingsEqual(a: OcrSettingsDto, b: OcrSettingsDto): boolean {
  return (
    a.enabled === b.enabled
    && a.model_id === b.model_id
    && captureTargetsEqual(a.capture_target, b.capture_target)
  )
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}

/**
 * Strict monitor-inventory converter. Only the whitelisted fields are carried
 * into state; malformed entries are rejected and arbitrary extra fields are
 * never accepted as trusted state.
 */
export function convertOcrMonitorListFromRust(raw: unknown): MonitorInfoDto[] {
  if (!Array.isArray(raw)) return []
  const monitors: MonitorInfoDto[] = []
  for (const entry of raw) {
    if (!isRecord(entry)) continue
    if (!isNonEmptyString(entry.devicePath)) continue
    if (!isNonEmptyString(entry.label)) continue
    if (!isNonEmptyString(entry.sourceName)) continue
    if (typeof entry.isPrimary !== 'boolean') continue
    const geometry = entry.geometry
    if (!isRecord(geometry)) continue
    if (!isFiniteNumber(geometry.x) || !Number.isInteger(geometry.x) || geometry.x < -2147483648 || geometry.x > 2147483647) continue
    if (!isFiniteNumber(geometry.y) || !Number.isInteger(geometry.y) || geometry.y < -2147483648 || geometry.y > 2147483647) continue
    if (!isFiniteNumber(geometry.width) || !Number.isInteger(geometry.width) || geometry.width <= 0 || geometry.width > 4294967295) continue
    if (!isFiniteNumber(geometry.height) || !Number.isInteger(geometry.height) || geometry.height <= 0 || geometry.height > 4294967295) continue
    monitors.push({
      devicePath: entry.devicePath,
      label: entry.label,
      sourceName: entry.sourceName,
      isPrimary: entry.isPrimary,
      geometry: {
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
      },
    })
  }
  return monitors
}

function isNonEmptyStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.length > 0 && value.every(isNonEmptyString)
}

/**
 * Strict pack-list converter. Only the whitelisted presentation fields are
 * carried into state; malformed entries are rejected and arbitrary extra
 * fields (paths, internals) are never accepted as trusted state.
 */
export function convertOcrPackListFromRust(raw: unknown): OcrPackDto[] {
  if (!Array.isArray(raw)) return []
  const packs: OcrPackDto[] = []
  for (const entry of raw) {
    if (!isRecord(entry)) continue
    if (!isNonEmptyString(entry.id)) continue
    if (!isNonEmptyString(entry.display_name)) continue
    if (!isNonEmptyStringArray(entry.languages)) continue
    packs.push({
      id: entry.id,
      display_name: entry.display_name,
      languages: entry.languages,
    })
  }
  return packs
}

// ============================================================================
// Composable
// ============================================================================

/**
 * Owns the OCR panel's backend contract: persisted settings, the actual
 * runtime status (authoritative — never inferred from `settings.enabled`),
 * the discovered model packs, and the rescan / open-folder actions.
 */
export function useOcr(active?: () => boolean) {
  const settings = ref<OcrSettingsDto>({ ...DEFAULT_SETTINGS })
  let confirmedSettings: OcrSettingsDto = { ...DEFAULT_SETTINGS }
  const status = ref<OcrStatusDto>({ state: 'disabled' })
  const packs = ref<OcrPackDto[]>([])
  const monitors = ref<MonitorInfoDto[]>([])
  const monitorListPending = ref(false)
  const monitorListError = ref(false)
  const monitorListLoaded = ref(false)
  const message = ref<string | null>(null)
  const messageType = ref<'success' | 'error' | null>(null)
  const savePending = ref(false)
  const rescanPending = ref(false)
  const openPending = ref(false)

  const listenerScope = createAsyncCleanupScope()
  let disposed = false
  let messageTimeout: number | null = null
  // Bumped by every status event so an in-flight `get_ocr_status` snapshot
  // never overwrites a newer transition that arrived while it was pending.
  let statusLoadToken = 0
  // Bumped by every `refreshStatus()` call so overlapping snapshots keep only
  // the newest reply: a slower older request must never overwrite a newer one.
  let statusRequestSeq = 0
  // Bumped by every settings refresh so an in-flight `get_ocr_settings`
  // snapshot never overwrites a newer refresh that started while it was
  // pending (out-of-order guard, mirrors statusLoadToken).
  let settingsLoadToken = 0
  // Bumped by every monitor inventory load so a stale reply can never update
  // state after unmount or after a newer load started.
  let monitorListLoadToken = 0

  const isEnabled = computed(() => settings.value.enabled)
  const isReady = computed(() => status.value.state === 'ready')

  const statusLabel = computed(() => {
    switch (status.value.state) {
      case 'starting':
        return t('ocr.status.starting')
      case 'ready':
        return t('ocr.status.ready')
      case 'selectingArea':
        return t('ocr.status.selecting_area')
      case 'recognizing':
        return t('ocr.status.recognizing')
      case 'error':
        return t('ocr.status.error')
      default:
        return t('ocr.status.disabled')
    }
  })

  const statusErrorMessage = computed(() =>
    status.value.state === 'error' ? (status.value.message ?? null) : null,
  )

  /**
   * A non-null `model_id` that is absent from the current valid pack list is an
   * error; the composable never silently substitutes another pack.
   */
  const missingModelError = computed(() => {
    const modelId = settings.value.model_id
    if (modelId === null) return null
    if (packs.value.some((pack) => pack.id === modelId)) return null
    return t('ocr.error.missing_model', { model: modelId })
  })

  /**
   * True while the selected model is absent from the freshly scanned pack list
   * but the runtime is still holding it live in memory (ready / selecting /
   * recognizing). The panel renders it as a selected «(в памяти)» option and
   * must not touch settings in that state.
   */
  const runtimeHoldsModel = computed(() => {
    const modelId = settings.value.model_id
    if (modelId === null) return false
    if (packs.value.some((pack) => pack.id === modelId)) return false
    const state = status.value.state
    return state === 'ready' || state === 'selectingArea' || state === 'recognizing'
  })

  /** Human-readable label of the in-memory model (falls back to its id). */
  const runtimeModelLabel = computed(() => {
    const modelId = settings.value.model_id
    if (modelId === null) return ''
    return packLabelMemo.get(modelId) ?? modelId
  })

  /**
   * True only when the saved capture target is a specific monitor whose device
   * path is absent from a successfully loaded inventory. Deliberately `false`
   * while the inventory is loading or has failed, so an enumeration failure
   * never masquerades as a missing monitor.
   */
  const selectedMonitorMissing = computed(() => {
    const target = settings.value.capture_target
    if (target.type !== 'monitor' || target.devicePath === undefined) return false
    if (!monitorListLoaded.value || monitorListPending.value || monitorListError.value) return false
    return !monitors.value.some((monitor) => monitor.devicePath === target.devicePath)
  })

  function showMessage(text: string, type: 'success' | 'error' = 'success') {
    message.value = text
    messageType.value = type
    if (messageTimeout !== null) clearTimeout(messageTimeout)
    messageTimeout = window.setTimeout(() => {
      message.value = null
      messageType.value = null
      messageTimeout = null
    }, 3000)
  }

  async function refreshSettings(): Promise<void> {
    settingsLoadToken += 1
    const token = settingsLoadToken
    try {
      const payload = await invoke<unknown>('get_ocr_settings')
      // While a save drain is in flight it owns the settings state: applying a
      // (possibly stale) echo snapshot here would clobber the edit the drain
      // is about to persist and could roll state back to an older value.
      if (disposed || savePending.value || token !== settingsLoadToken) return
      settings.value = convertOcrSettingsFromRust(payload)
      confirmedSettings = { ...settings.value }
    } catch (e) {
      if (disposed || token !== settingsLoadToken) return
      debugError('[Ocr] Failed to load settings:', e)
    }
  }

  async function refreshStatus(): Promise<void> {
    statusRequestSeq += 1
    const request = statusRequestSeq
    const token = statusLoadToken
    try {
      const payload = await invoke<unknown>('get_ocr_status')
      // Drop the reply when it belongs to a superseded request (a newer
      // `refreshStatus()` started meanwhile) or when a status event already
      // delivered a newer transition.
      if (disposed || request !== statusRequestSeq || token !== statusLoadToken) return
      status.value = convertOcrStatusFromRust(payload)
    } catch (e) {
      if (disposed || request !== statusRequestSeq || token !== statusLoadToken) return
      debugError('[Ocr] Failed to load status:', e)
    }
  }

  async function loadPacks(): Promise<void> {
    const payload = await invoke<unknown>('list_ocr_packs')
    if (disposed) return
    packs.value = convertOcrPackListFromRust(payload)
    rememberPackLabels(packs.value)
  }

  async function refreshPacks(): Promise<void> {
    try {
      await loadPacks()
    } catch (e) {
      if (disposed) return
      debugError('[Ocr] Failed to load packs:', e)
    }
  }

  async function loadMonitors(): Promise<void> {
    monitorListLoadToken += 1
    const token = monitorListLoadToken
    monitorListPending.value = true
    monitorListError.value = false
    try {
      const payload = await invoke<unknown>('list_ocr_monitors')
      if (disposed || token !== monitorListLoadToken) return
      monitors.value = convertOcrMonitorListFromRust(payload)
      monitorListLoaded.value = true
    } catch (e) {
      if (disposed || token !== monitorListLoadToken) return
      debugError('[Ocr] Failed to load monitors:', e)
      // A persistent error stays visible until a later successful retry.
      monitorListError.value = true
    } finally {
      if (!disposed && token === monitorListLoadToken) {
        monitorListPending.value = false
      }
    }
  }

  async function refreshMonitors(): Promise<void> {
    if (monitorListPending.value) return
    await loadMonitors()
  }

  const stopActivationWatch = active ? watch(active, (isActive) => {
    if (isActive && !disposed) {
      void refreshMonitors()
      // A model may have been installed from the Models settings tab while this
      // panel was hidden. Re-read the on-disk packs and persisted selection so
      // the active model selector stays current without disturbing saved state.
      void refreshPacks()
      void refreshSettings()
      void refreshStatus()
    }
  }) : undefined

  async function saveSettings(): Promise<void> {
    // A concurrent call while a save is in flight is not dropped: the drain
    // loop below re-reads `settings.value` after every persisted snapshot, so
    // the latest edit is always saved exactly once per iteration.
    if (savePending.value) return
    savePending.value = true
    try {
      let payload = { ...settings.value }
      while (true) {
        await invoke('save_ocr_settings', { settings: payload })
        if (disposed) return
        confirmedSettings = payload
        if (ocrSettingsEqual(settings.value, payload)) break
        payload = { ...settings.value }
      }
      showMessage(t('ocr.message.saved'), 'success')
    } catch (e) {
      if (disposed) return
      settings.value = { ...confirmedSettings }
      const errorMessage = e instanceof Error ? e.message : String(e)
      showMessage(t('ocr.message.save_error', { detail: errorMessage }), 'error')
    } finally {
      if (!disposed) {
        savePending.value = false
        // The backend may have unticked `enabled` during `save_ocr_settings`
        // (failed runtime start) and emitted `settings-changed` while the
        // guard above was still dropping echoes. Re-read once the drain is
        // done so the persisted `{ enabled: false }` lands on the checkbox.
        void refreshSettings()
      }
    }
  }

  async function rescanPacks(): Promise<void> {
    if (rescanPending.value) return
    rescanPending.value = true
    try {
      const payload = await invoke<unknown>('refresh_ocr_packs')
      if (disposed) return
      packs.value = convertOcrPackListFromRust(payload)
      rememberPackLabels(packs.value)
      showMessage(t('ocr.message.list_updated'), 'success')
      // The backend reconciles (may untick) during the refresh; re-read the
      // persisted settings. Idempotent and guarded against the save drain.
      void refreshSettings()
    } catch (e) {
      if (disposed) return
      debugError('[Ocr] Failed to rescan packs:', e)
      showMessage(t('ocr.message.list_update_error'), 'error')
    } finally {
      if (!disposed) rescanPending.value = false
    }
  }

  async function openPacksFolder(): Promise<void> {
    if (openPending.value) return
    openPending.value = true
    try {
      await invoke('open_ocr_packs_folder')
    } catch (e) {
      if (disposed) return
      debugError('[Ocr] Failed to open packs folder:', e)
      showMessage(t('ocr.message.open_folder_error'), 'error')
    } finally {
      if (!disposed) openPending.value = false
    }
  }

  onMounted(async () => {
    await listenerScope.track(
      listen<unknown>(OCR_STATUS_CHANGED_EVENT, (event) => {
        statusLoadToken += 1
        status.value = convertOcrStatusFromRust(event.payload)
      }),
    )
    await listenerScope.track(
      listen(SETTINGS_CHANGED_EVENT, () => {
        void refreshSettings()
        void refreshStatus()
      }),
    )
    // Listeners first, then snapshots: an event arriving between registration
    // and a snapshot response reflects the latest transition and wins over the
    // (possibly stale) snapshot.
    await refreshSettings()
    await refreshStatus()
    await refreshPacks()
    await loadMonitors()
  })

  onUnmounted(() => {
    disposed = true
    stopActivationWatch?.()
    if (messageTimeout !== null) {
      clearTimeout(messageTimeout)
      messageTimeout = null
    }
    listenerScope.dispose()
  })

  return {
    settings,
    status,
    packs,
    monitors,
    monitorListPending,
    monitorListError,
    message,
    messageType,
    savePending,
    rescanPending,
    openPending,
    isEnabled,
    isReady,
    statusLabel,
    statusErrorMessage,
    missingModelError,
    runtimeHoldsModel,
    runtimeModelLabel,
    selectedMonitorMissing,
    saveSettings,
    rescanPacks,
    openPacksFolder,
    refreshSettings,
    refreshStatus,
    refreshPacks,
    refreshMonitors,
  }
}
