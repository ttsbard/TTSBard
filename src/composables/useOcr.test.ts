import { describe, it, expect, vi, beforeEach, beforeAll } from 'vitest'
import { nextTick, ref } from 'vue'

vi.stubGlobal('window', globalThis)

const mocks = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
  listenCallbacks: new Map<string, (event: { payload: unknown }) => void>(),
  unlistenFns: new Map<string, () => void>(),
  mockDebugError: vi.fn(),
}))

let capturedOnMountedCb: (() => void) | null = null
let capturedOnUnmountedCb: (() => void) | null = null

vi.mock('vue', async () => {
  const actual = await vi.importActual<typeof import('vue')>('vue')
  return {
    ...actual,
    onMounted: (cb: () => void) => { capturedOnMountedCb = cb },
    onUnmounted: (cb: () => void) => { capturedOnUnmountedCb = cb },
  }
})

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mocks.mockInvoke,
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, callback: (event: { payload: unknown }) => void) => {
    mocks.listenCallbacks.set(event, callback)
    const unlisten = vi.fn(() => { mocks.listenCallbacks.delete(event) })
    mocks.unlistenFns.set(event, unlisten)
    return Promise.resolve(unlisten)
  }),
}))

vi.mock('../utils/debug', () => ({
  debugError: mocks.mockDebugError,
  debugLog: vi.fn(),
}))

import {
  useOcr,
  convertOcrStatusFromRust,
  convertOcrSettingsFromRust,
  convertCaptureTargetFromRust,
  convertOcrPackListFromRust,
  convertOcrMonitorListFromRust,
  OCR_RUNTIME_STATES,
  type OcrPackDto,
  type MonitorInfoDto,
} from './useOcr'
import { i18n } from '../i18n'
import ruCatalog from '../../locales/ru.json'

beforeAll(() => {
  i18n.global.setLocaleMessage('ru', (ruCatalog as { messages: Record<string, string> }).messages)
  ;(i18n.global.locale as unknown as { value: string }).value = 'ru'
})

function makePack(id = 'pack-1', overrides: Partial<OcrPackDto> = {}): OcrPackDto {
  return { id, display_name: 'Pack ' + id, languages: ['ru', 'en'], ...overrides }
}

function makeMonitor(devicePath = '\\\\.\\DISPLAY1\\Monitor0', overrides: Partial<MonitorInfoDto> = {}): MonitorInfoDto {
  return {
    devicePath,
    label: 'Display 1',
    sourceName: '\\\\.\\DISPLAY1',
    isPrimary: true,
    geometry: { x: 0, y: 0, width: 1920, height: 1080 },
    ...overrides,
  }
}

interface SnapshotPayloads {
  settings?: unknown
  status?: unknown
  packs?: unknown
  monitors?: unknown
}

/**
 * Stateful mock: `get_ocr_settings` echoes the last persisted settings (updated
 * by `save_ocr_settings`), mirroring the real backend where a save may untick
 * `enabled` and the follow-up refresh reads that persisted value.
 */
function installStatefulInvoke(payloads?: SnapshotPayloads) {
  const rawSettings = payloads?.settings
  const initialSettings =
    typeof rawSettings === 'object' && rawSettings !== null
      ? (rawSettings as Record<string, unknown>)
      : {}
  const persisted: Record<string, unknown> = {
    enabled: false,
    model_id: null,
    capture_target: { type: 'all' },
    ...initialSettings,
  }
  mocks.mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'get_ocr_settings') return { ...persisted }
    if (cmd === 'get_ocr_status') return payloads?.status ?? { state: 'disabled' }
    if (cmd === 'list_ocr_packs') return payloads?.packs ?? []
    if (cmd === 'refresh_ocr_packs') return payloads?.packs ?? []
    if (cmd === 'list_ocr_monitors') return payloads?.monitors ?? []
    if (cmd === 'save_ocr_settings') {
      const settings = (args as { settings?: Record<string, unknown> } | undefined)?.settings
      if (settings && typeof settings === 'object') {
        Object.assign(persisted, settings)
      }
      return undefined
    }
    return undefined
  })
}

function defaultInvoke() {
  installStatefulInvoke()
}

async function setupAndMount(payloads?: SnapshotPayloads): Promise<ReturnType<typeof useOcr>> {
  installStatefulInvoke(payloads)
  const composable = useOcr()
  if (capturedOnMountedCb) {
    await capturedOnMountedCb()
  }
  return composable
}

function emit(event: string, payload: unknown) {
  mocks.listenCallbacks.get(event)?.({ payload })
}

describe('pure converters', () => {
  it('converts all six runtime status states', () => {
    expect(convertOcrStatusFromRust({ state: 'disabled' })).toEqual({ state: 'disabled' })
    expect(convertOcrStatusFromRust({ state: 'starting' })).toEqual({ state: 'starting' })
    expect(convertOcrStatusFromRust({ state: 'ready' })).toEqual({ state: 'ready' })
    expect(convertOcrStatusFromRust({ state: 'selectingArea' })).toEqual({ state: 'selectingArea' })
    expect(convertOcrStatusFromRust({ state: 'recognizing' })).toEqual({ state: 'recognizing' })
    expect(convertOcrStatusFromRust({ state: 'error', message: 'boom' })).toEqual({
      state: 'error',
      message: 'boom',
    })
    expect(OCR_RUNTIME_STATES).toEqual([
      'disabled',
      'starting',
      'ready',
      'selectingArea',
      'recognizing',
      'error',
    ])
  })

  it('falls back to disabled for invalid status payloads', () => {
    expect(convertOcrStatusFromRust(null)).toEqual({ state: 'disabled' })
    expect(convertOcrStatusFromRust('ready')).toEqual({ state: 'disabled' })
    expect(convertOcrStatusFromRust(42)).toEqual({ state: 'disabled' })
    expect(convertOcrStatusFromRust(undefined)).toEqual({ state: 'disabled' })
    expect(convertOcrStatusFromRust({})).toEqual({ state: 'disabled' })
    expect(convertOcrStatusFromRust({ state: 'Ready' })).toEqual({ state: 'disabled' })
    expect(convertOcrStatusFromRust({ state: 'unknown' })).toEqual({ state: 'disabled' })
    expect(convertOcrStatusFromRust({ state: 42 })).toEqual({ state: 'disabled' })
    expect(convertOcrStatusFromRust([{ state: 'ready' }])).toEqual({ state: 'disabled' })
  })

  it('only trusts a string message on the error state', () => {
    expect(convertOcrStatusFromRust({ state: 'error', message: 42 })).toEqual({ state: 'error' })
    expect(convertOcrStatusFromRust({ state: 'error' })).toEqual({ state: 'error' })
    expect(convertOcrStatusFromRust({ state: 'ready', message: 'ignored' })).toEqual({
      state: 'ready',
    })
  })

  it('converts valid settings payloads', () => {
    expect(convertOcrSettingsFromRust({ enabled: true, model_id: 'pack-a' })).toEqual({
      enabled: true,
      model_id: 'pack-a',
      capture_target: { type: 'all' },
    })
    expect(convertOcrSettingsFromRust({ enabled: false, model_id: null })).toEqual({
      enabled: false,
      model_id: null,
      capture_target: { type: 'all' },
    })
  })

  it('converts a full capture target and defaults a missing or malformed one to All', () => {
    expect(convertCaptureTargetFromRust({ type: 'all' })).toEqual({ type: 'all' })
    expect(convertCaptureTargetFromRust({ type: 'primary' })).toEqual({ type: 'primary' })
    expect(
      convertCaptureTargetFromRust({ type: 'monitor', devicePath: '\\\\.\\DISPLAY2\\Monitor0' }),
    ).toEqual({ type: 'monitor', devicePath: '\\\\.\\DISPLAY2\\Monitor0' })
    expect(convertCaptureTargetFromRust(undefined)).toEqual({ type: 'all' })
    expect(convertCaptureTargetFromRust(null)).toEqual({ type: 'all' })
    expect(convertCaptureTargetFromRust('primary')).toEqual({ type: 'all' })
    expect(convertCaptureTargetFromRust({})).toEqual({ type: 'all' })
    expect(convertCaptureTargetFromRust({ type: 'unknown' })).toEqual({ type: 'all' })
    expect(convertCaptureTargetFromRust({ type: 'monitor' })).toEqual({ type: 'all' })
    expect(convertCaptureTargetFromRust({ type: 'monitor', devicePath: '' })).toEqual({ type: 'all' })
    expect(convertCaptureTargetFromRust({ type: 'monitor', devicePath: '  ' })).toEqual({ type: 'all' })
  })

  it('normalizes invalid or blank model ids to null and untrusted fields to defaults', () => {
    expect(convertOcrSettingsFromRust(null)).toEqual({
      enabled: false,
      model_id: null,
      capture_target: { type: 'all' },
    })
    expect(convertOcrSettingsFromRust({ enabled: 'yes', model_id: 7 })).toEqual({
      enabled: false,
      model_id: null,
      capture_target: { type: 'all' },
    })
    expect(convertOcrSettingsFromRust({ enabled: true, model_id: '  ' })).toEqual({
      enabled: true,
      model_id: null,
      capture_target: { type: 'all' },
    })
    expect(convertOcrSettingsFromRust({})).toEqual({
      enabled: false,
      model_id: null,
      capture_target: { type: 'all' },
    })
  })

  it('accepts a valid pack list preserving only the whitelisted fields', () => {
    const payload = [
      makePack('a', { languages: ['ru'] }),
      {
        id: 'b',
        display_name: 'Pack B',
        languages: ['en'],
        det_file: 'det.onnx',
        path: 'C:\\secrets',
      },
    ]
    const result = convertOcrPackListFromRust(payload)
    expect(result).toEqual([
      { id: 'a', display_name: 'Pack a', languages: ['ru'] },
      { id: 'b', display_name: 'Pack B', languages: ['en'] },
    ])
    expect(Object.keys(result[1])).toEqual(['id', 'display_name', 'languages'])
  })

  it('rejects malformed pack entries without trusting arbitrary fields', () => {
    const bad: unknown[] = [
      null,
      'pack',
      42,
      { display_name: 'No id' },
      { id: '', display_name: 'Blank id' },
      { id: 42, display_name: 'Numeric id' },
      { id: 'ok', display_name: '' },
      { id: 'ok' },
      { id: 'ok', display_name: 'No langs' },
      { id: 'ok', display_name: 'Langs not array', languages: 'ru' },
      { id: 'ok', display_name: 'Empty langs', languages: [] },
      { id: 'ok', display_name: 'Blank tag', languages: ['  '] },
      { id: 'ok', display_name: 'Numeric tag', languages: [7] },
      { id: 'ok', display_name: 'Ok', languages: ['ru'] },
    ]
    const result = convertOcrPackListFromRust(bad)
    expect(result).toEqual([{ id: 'ok', display_name: 'Ok', languages: ['ru'] }])
  })

  it('returns an empty list for non-array pack payloads', () => {
    expect(convertOcrPackListFromRust(null)).toEqual([])
    expect(convertOcrPackListFromRust({ packs: [] })).toEqual([])
    expect(convertOcrPackListFromRust('nope')).toEqual([])
  })

  it('accepts a valid monitor list preserving only the whitelisted fields', () => {
    const payload = [
      makeMonitor('\\\\.\\DISPLAY1\\Monitor0'),
      {
        devicePath: '\\\\.\\DISPLAY2\\Monitor0',
        label: 'Display 2',
        sourceName: '\\\\.\\DISPLAY2',
        isPrimary: false,
        geometry: { x: 1920, y: 0, width: 2560, height: 1440 },
        hmonitor: 1234,
        adapter: 'leaked-internal',
      },
    ]
    const result = convertOcrMonitorListFromRust(payload)
    expect(result).toEqual([
      makeMonitor('\\\\.\\DISPLAY1\\Monitor0'),
      {
        devicePath: '\\\\.\\DISPLAY2\\Monitor0',
        label: 'Display 2',
        sourceName: '\\\\.\\DISPLAY2',
        isPrimary: false,
        geometry: { x: 1920, y: 0, width: 2560, height: 1440 },
      },
    ])
    expect(Object.keys(result[1])).toEqual(['devicePath', 'label', 'sourceName', 'isPrimary', 'geometry'])
  })

  it('rejects malformed monitor entries without trusting arbitrary fields', () => {
    const good = makeMonitor()
    const bad: unknown[] = [
      null,
      'monitor',
      42,
      { label: 'No path', sourceName: 'x', isPrimary: true, geometry: { x: 0, y: 0, width: 1, height: 1 } },
      { devicePath: '', label: 'Blank path', sourceName: 'x', isPrimary: true, geometry: { x: 0, y: 0, width: 1, height: 1 } },
      { devicePath: 'd', label: '', sourceName: 'x', isPrimary: true, geometry: { x: 0, y: 0, width: 1, height: 1 } },
      { devicePath: 'd', label: 'L', sourceName: '', isPrimary: true, geometry: { x: 0, y: 0, width: 1, height: 1 } },
      { devicePath: 'd', label: 'L', sourceName: 'x', isPrimary: 'yes', geometry: { x: 0, y: 0, width: 1, height: 1 } },
      { devicePath: 'd', label: 'L', sourceName: 'x', isPrimary: true },
      { devicePath: 'd', label: 'L', sourceName: 'x', isPrimary: true, geometry: { x: 0, y: 0, width: 'w', height: 1 } },
      { devicePath: 'd', label: 'L', sourceName: 'x', isPrimary: true, geometry: { x: 0, y: 0, width: 1, height: NaN } },
    ]
    const result = convertOcrMonitorListFromRust([...bad, good])
    expect(result).toEqual([good])
  })

  it('returns an empty list for non-array monitor payloads', () => {
    expect(convertOcrMonitorListFromRust(null)).toEqual([])
    expect(convertOcrMonitorListFromRust({ monitors: [] })).toEqual([])
    expect(convertOcrMonitorListFromRust('nope')).toEqual([])
  })
})

describe('useOcr', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.listenCallbacks.clear()
    mocks.unlistenFns.clear()
    capturedOnMountedCb = null
    capturedOnUnmountedCb = null
  })

  it('starts with default settings, disabled runtime and no derived errors', () => {
    const { settings, status, packs, monitors, message, statusLabel, statusErrorMessage, missingModelError } =
      useOcr()

    expect(settings.value).toEqual({ enabled: false, model_id: null, capture_target: { type: 'all' } })
    expect(status.value).toEqual({ state: 'disabled' })
    expect(packs.value).toEqual([])
    expect(monitors.value).toEqual([])
    expect(message.value).toBeNull()
    expect(statusLabel.value).toBe('Отключено')
    expect(statusErrorMessage.value).toBeNull()
    expect(missingModelError.value).toBeNull()
  })

  it('loads settings, status and packs snapshots on mount', async () => {
    const { settings, status, packs, statusLabel, isEnabled, isReady, missingModelError } =
      await setupAndMount({
        settings: { enabled: true, model_id: 'pack-1' },
        status: { state: 'ready' },
        packs: [makePack('pack-1')],
      })

    expect(settings.value).toEqual({ enabled: true, model_id: 'pack-1', capture_target: { type: 'all' } })
    expect(status.value).toEqual({ state: 'ready' })
    expect(isEnabled.value).toBe(true)
    expect(isReady.value).toBe(true)
    expect(statusLabel.value).toBe('Готов')
    expect(packs.value).toEqual([makePack('pack-1')])
    expect(missingModelError.value).toBeNull()
  })

  it('registers listeners before loading the initial snapshots', async () => {
    const seenListeners: string[][] = []
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      seenListeners.push([...mocks.listenCallbacks.keys()])
      if (cmd === 'get_ocr_settings') return { enabled: false, model_id: null }
      if (cmd === 'get_ocr_status') return { state: 'disabled' }
      if (cmd === 'list_ocr_packs') return []
      if (cmd === 'list_ocr_monitors') return []
      return undefined
    })

    useOcr()
    await capturedOnMountedCb?.()

    expect(seenListeners).toHaveLength(4)
    for (const names of seenListeners) {
      expect(names).toContain('ocr-status-changed')
      expect(names).toContain('settings-changed')
    }
  })

  it('updates the runtime state on ocr-status-changed events with derived labels', async () => {
    const { status, statusLabel, statusErrorMessage } = await setupAndMount()

    emit('ocr-status-changed', { state: 'starting' })
    expect(status.value.state).toBe('starting')
    expect(statusLabel.value).toBe('Запускается')

    emit('ocr-status-changed', { state: 'selectingArea' })
    expect(statusLabel.value).toBe('Выбор области')

    emit('ocr-status-changed', { state: 'recognizing' })
    expect(statusLabel.value).toBe('Распознавание…')

    emit('ocr-status-changed', { state: 'error', message: 'worker died' })
    expect(status.value).toEqual({ state: 'error', message: 'worker died' })
    expect(statusLabel.value).toBe('Ошибка')
    expect(statusErrorMessage.value).toBe('worker died')

    emit('ocr-status-changed', { state: 'ready' })
    expect(statusLabel.value).toBe('Готов')
    expect(statusErrorMessage.value).toBeNull()
  })

  it('ignores invalid ocr-status-changed payloads by falling back to disabled', async () => {
    const { status } = await setupAndMount()

    emit('ocr-status-changed', 'garbage')
    expect(status.value.state).toBe('disabled')

    emit('ocr-status-changed', { state: 'flying' })
    expect(status.value.state).toBe('disabled')

    emit('ocr-status-changed', null)
    expect(status.value.state).toBe('disabled')
  })

  it('prefers a status event received while the snapshot is in flight', async () => {
    let resolveStatus!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_ocr_settings') return Promise.resolve({ enabled: true, model_id: 'pack-1' })
      if (cmd === 'get_ocr_status') {
        // A newer transition fires after the snapshot request, before it resolves.
        emit('ocr-status-changed', { state: 'ready' })
        return new Promise((resolve) => { resolveStatus = resolve })
      }
      if (cmd === 'list_ocr_packs') return Promise.resolve([makePack('pack-1')])
      return Promise.resolve(undefined)
    })

    const { status } = useOcr()
    const mounted = capturedOnMountedCb!()

    // Wait until the snapshot request is actually in flight (the event fires
    // synchronously inside the request), then let the stale snapshot resolve
    // later — it must not regress the state.
    await vi.waitFor(() => expect(mocks.mockInvoke).toHaveBeenCalledWith('get_ocr_status'))
    resolveStatus({ state: 'disabled' })
    await mounted

    expect(status.value.state).toBe('ready')
  })

  it('reloads settings and status on the global settings-changed event', async () => {
    const { settings, status } = await setupAndMount()

    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_ocr_settings') return { enabled: true, model_id: 'pack-2' }
      if (cmd === 'get_ocr_status') return { state: 'ready' }
      return undefined
    })

    emit('settings-changed', undefined)

    await vi.waitFor(() =>
      expect(settings.value).toEqual({ enabled: true, model_id: 'pack-2', capture_target: { type: 'all' } }),
    )
    await vi.waitFor(() => expect(status.value.state).toBe('ready'))
  })

  it('saveSettings persists the current settings payload', async () => {
    const { settings, saveSettings, message } = await setupAndMount()

    settings.value.enabled = true
    settings.value.model_id = 'pack-1'
    await saveSettings()

    expect(mocks.mockInvoke).toHaveBeenCalledWith('save_ocr_settings', {
      settings: { enabled: true, model_id: 'pack-1', capture_target: { type: 'all' } },
    })
    expect(message.value).toBe('Настройки сохранены')
  })

  it('saveSettings rolls back to the last confirmed settings on failure', async () => {
    const { settings, saveSettings } = await setupAndMount()
    mocks.mockInvoke.mockRejectedValueOnce(new Error('backend down'))

    settings.value.enabled = true
    settings.value.model_id = 'pack-1'

    await saveSettings()

    expect(settings.value).toEqual({ enabled: false, model_id: null, capture_target: { type: 'all' } })
    expect(mocks.mockDebugError).not.toHaveBeenCalled()
  })

  it('keeps OCR disabled and surfaces the error when the capture hotkey is off', async () => {
    const { settings, saveSettings, message } = await setupAndMount()
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'save_ocr_settings') {
        throw new Error('Нельзя включить OCR: горячая клавиша «OCR: захват области» выключена')
      }
      if (cmd === 'get_ocr_settings') {
        return { enabled: false, model_id: null, capture_target: { type: 'all' } }
      }
      return undefined
    })

    settings.value.enabled = true
    settings.value.model_id = 'pack-1'
    await saveSettings()

    expect(settings.value.enabled).toBe(false)
    expect(message.value).toContain('Нельзя включить OCR')
  })

  it('a successful save becomes the fallback snapshot for later failures', async () => {
    const { settings, saveSettings } = await setupAndMount()

    settings.value.enabled = true
    settings.value.model_id = 'pack-1'
    await saveSettings()

    settings.value.enabled = false
    settings.value.model_id = 'pack-2'
    mocks.mockInvoke.mockRejectedValueOnce(new Error('backend down'))
    await saveSettings()

    expect(settings.value).toEqual({ enabled: true, model_id: 'pack-1', capture_target: { type: 'all' } })
  })

  it('guards duplicate in-flight saves', async () => {
    const { saveSettings } = await setupAndMount()

    let resolveSave!: (value: unknown) => void
    mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
      if (cmd === 'save_ocr_settings') {
        return new Promise((resolve) => { resolveSave = resolve })
      }
      return undefined
    })

    const first = saveSettings()
    await Promise.resolve()
    await saveSettings()

    resolveSave(undefined)
    await first

    const saveCalls = mocks.mockInvoke.mock.calls.filter((call) => call[0] === 'save_ocr_settings')
    expect(saveCalls).toHaveLength(1)
  })

  it('rapid model switches persist the latest value', async () => {
    const { settings, saveSettings, savePending, message } = await setupAndMount()
    mocks.mockInvoke.mockClear()

    const saveCalls: unknown[] = []
    const persisted: Record<string, unknown> = {
      enabled: false,
      model_id: null,
      capture_target: { type: 'all' },
    }
    let resolveFirst!: () => void
    mocks.mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === 'save_ocr_settings') {
        saveCalls.push(args)
        const settings = (args as { settings?: Record<string, unknown> } | undefined)?.settings
        if (settings && typeof settings === 'object') Object.assign(persisted, settings)
        if (saveCalls.length === 1) {
          return new Promise((resolve) => {
            resolveFirst = () => resolve(undefined)
          })
        }
        return Promise.resolve(undefined)
      }
      if (cmd === 'get_ocr_settings') return Promise.resolve({ ...persisted })
      return Promise.resolve(undefined)
    })

    settings.value = { enabled: true, model_id: 'pack-a', capture_target: { type: 'all' } }
    const first = saveSettings()
    await vi.waitFor(() => expect(saveCalls).toHaveLength(1))

    // A second edit while the first save is in flight must not be dropped:
    // the concurrent call returns early, but the drain loop picks the edit up.
    settings.value = { enabled: true, model_id: 'pack-b', capture_target: { type: 'all' } }
    await saveSettings()
    expect(saveCalls).toHaveLength(1)

    resolveFirst()
    await first

    expect(saveCalls).toEqual([
      { settings: { enabled: true, model_id: 'pack-a', capture_target: { type: 'all' } } },
      { settings: { enabled: true, model_id: 'pack-b', capture_target: { type: 'all' } } },
    ])
    expect(settings.value).toEqual({ enabled: true, model_id: 'pack-b', capture_target: { type: 'all' } })
    expect(savePending.value).toBe(false)
    expect(message.value).toBe('Настройки сохранены')
  })

  it('rolls back to the last persisted snapshot when a mid-drain save fails', async () => {
    const { settings, saveSettings, message } = await setupAndMount()

    settings.value = { enabled: true, model_id: 'pack-a', capture_target: { type: 'all' } }
    await saveSettings()

    let resolveFirst!: () => void
    let saveCount = 0
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'save_ocr_settings') {
        saveCount += 1
        if (saveCount === 1) {
          return new Promise((resolve) => {
            resolveFirst = () => resolve(undefined)
          })
        }
        return Promise.reject(new Error('backend down'))
      }
      // The post-save refresh re-reads the last persisted snapshot (pack-b).
      if (cmd === 'get_ocr_settings') {
        return Promise.resolve({ enabled: true, model_id: 'pack-b' })
      }
      return Promise.resolve(undefined)
    })

    settings.value = { enabled: true, model_id: 'pack-b', capture_target: { type: 'all' } }
    const second = saveSettings()
    await vi.waitFor(() => expect(saveCount).toBe(1))

    settings.value = { enabled: true, model_id: 'pack-c', capture_target: { type: 'all' } }
    resolveFirst()
    await second

    // pack-b persisted, pack-c failed: roll back to pack-b (really saved),
    // not to the lost edit pack-c.
    expect(saveCount).toBe(2)
    expect(settings.value).toEqual({ enabled: true, model_id: 'pack-b', capture_target: { type: 'all' } })
    expect(message.value).toContain('Не удалось сохранить настройки')
  })

  it('does not apply a settings echo snapshot while a save is in flight', async () => {
    const { settings, saveSettings } = await setupAndMount()

    let resolveSave!: () => void
    const persisted: Record<string, unknown> = {
      enabled: false,
      model_id: null,
      capture_target: { type: 'all' },
    }
    mocks.mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === 'save_ocr_settings') {
        return new Promise((resolve) => {
          resolveSave = () => {
            const settings = (args as { settings?: Record<string, unknown> } | undefined)?.settings
            if (settings && typeof settings === 'object') Object.assign(persisted, settings)
            resolve(undefined)
          }
        })
      }
      // Echo the persisted state: stale pre-save snapshot while the save is in
      // flight, the saved value after the drain completes.
      if (cmd === 'get_ocr_settings') {
        return Promise.resolve({ ...persisted })
      }
      return Promise.resolve(undefined)
    })

    settings.value = { enabled: true, model_id: 'pack-a', capture_target: { type: 'all' } }
    const pending = saveSettings()
    await vi.waitFor(() =>
      expect(mocks.mockInvoke).toHaveBeenCalledWith('save_ocr_settings', expect.anything()),
    )

    emit('settings-changed', undefined)
    await vi.waitFor(() => expect(mocks.mockInvoke).toHaveBeenCalledWith('get_ocr_settings'))
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(settings.value).toEqual({ enabled: true, model_id: 'pack-a', capture_target: { type: 'all' } })

    resolveSave()
    await pending

    expect(settings.value).toEqual({ enabled: true, model_id: 'pack-a', capture_target: { type: 'all' } })
  })

  it('discards an older settings snapshot when a newer refresh overlaps', async () => {
    defaultInvoke()
    const { settings, refreshSettings } = useOcr()

    let resolveFirst!: (value: unknown) => void
    let resolveSecond!: (value: unknown) => void
    mocks.mockInvoke
      .mockImplementationOnce(() => {
        return new Promise((resolve) => { resolveFirst = resolve })
      })
      .mockImplementationOnce(() => {
        return new Promise((resolve) => { resolveSecond = resolve })
      })

    const first = refreshSettings()
    const second = refreshSettings()

    resolveSecond({ enabled: true, model_id: 'newer' })
    await second
    resolveFirst({ enabled: false, model_id: 'older' })
    await first

    expect(settings.value).toEqual({ enabled: true, model_id: 'newer', capture_target: { type: 'all' } })
  })

  it('reports a missing selected model without substituting another pack', async () => {
    const { settings, packs, missingModelError } = await setupAndMount({
      settings: { enabled: true, model_id: 'missing-pack' },
      status: { state: 'error', message: 'model not found' },
      packs: [makePack('pack-a')],
    })

    expect(missingModelError.value).not.toBeNull()
    expect(settings.value.model_id).toBe('missing-pack')
    expect(packs.value.map((pack) => pack.id)).toEqual(['pack-a'])
  })

  it('clears the missing-model error when the selected pack is present', async () => {
    const { missingModelError } = await setupAndMount({
      settings: { enabled: true, model_id: 'pack-a' },
      status: { state: 'ready' },
      packs: [makePack('pack-a')],
    })

    expect(missingModelError.value).toBeNull()
  })

  it('recomputes the missing-model error as the pack list changes', async () => {
    const { settings, missingModelError, rescanPacks } = await setupAndMount({
      settings: { enabled: true, model_id: 'pack-a' },
      status: { state: 'ready' },
      packs: [],
    })

    expect(missingModelError.value).not.toBeNull()

    mocks.mockInvoke.mockImplementationOnce(async (cmd: string) => {
      if (cmd === 'refresh_ocr_packs') return [makePack('pack-a')]
      return undefined
    })
    await rescanPacks()

    expect(settings.value.model_id).toBe('pack-a')
    expect(missingModelError.value).toBeNull()
  })

  it('does not flag a missing model when none is selected', async () => {
    const { missingModelError } = await setupAndMount({
      settings: { enabled: false, model_id: null },
      status: { state: 'disabled' },
      packs: [],
    })

    expect(missingModelError.value).toBeNull()
  })

  it('rescanPacks reloads the pack list from the backend', async () => {
    const { packs, rescanPacks } = await setupAndMount()

    mocks.mockInvoke.mockImplementationOnce(async (cmd: string) => {
      if (cmd === 'refresh_ocr_packs') return [makePack('pack-1'), makePack('pack-2')]
      return undefined
    })

    await rescanPacks()

    expect(mocks.mockInvoke).toHaveBeenCalledWith('refresh_ocr_packs')
    expect(packs.value).toEqual([makePack('pack-1'), makePack('pack-2')])
  })

  it('rejects malformed entries received by a rescan', async () => {
    const { packs, rescanPacks } = await setupAndMount()

    mocks.mockInvoke.mockImplementationOnce(async (cmd: string) => {
      if (cmd === 'refresh_ocr_packs') {
        return [makePack('good'), { id: 42, display_name: 'bad', path: 'C:\\x' }]
      }
      return undefined
    })

    await rescanPacks()

    expect(packs.value).toEqual([makePack('good')])
  })

  it('guards duplicate in-flight rescans', async () => {
    const { rescanPacks, packs } = await setupAndMount()
    mocks.mockInvoke.mockClear()

    let resolveList!: (value: unknown) => void
    mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
      if (cmd === 'refresh_ocr_packs') {
        return new Promise((resolve) => { resolveList = resolve })
      }
      return undefined
    })

    const first = rescanPacks()
    await Promise.resolve()
    await rescanPacks()

    resolveList([makePack('pack-1')])
    await first

    const rescanCalls = mocks.mockInvoke.mock.calls.filter((call) => call[0] === 'refresh_ocr_packs')
    expect(rescanCalls).toHaveLength(1)
    expect(packs.value).toEqual([makePack('pack-1')])
  })

  it('openPacksFolder invokes the backend without any arguments', async () => {
    const { openPacksFolder } = await setupAndMount()

    await openPacksFolder()

    expect(mocks.mockInvoke).toHaveBeenCalledWith('open_ocr_packs_folder')
  })

  it('guards duplicate in-flight open calls and surfaces failures', async () => {
    const { openPacksFolder, message } = await setupAndMount()
    mocks.mockInvoke.mockClear()

    let resolveOpen!: (value: unknown) => void
    mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
      if (cmd === 'open_ocr_packs_folder') {
        return new Promise((resolve) => { resolveOpen = resolve })
      }
      return undefined
    })

    const first = openPacksFolder()
    await Promise.resolve()
    await openPacksFolder()

    resolveOpen(undefined)
    await first

    const openCalls = mocks.mockInvoke.mock.calls.filter(
      (call) => call[0] === 'open_ocr_packs_folder',
    )
    expect(openCalls).toHaveLength(1)

    mocks.mockInvoke.mockRejectedValueOnce(new Error('explorer missing'))
    await openPacksFolder()
    expect(message.value).toBe('Не удалось открыть папку моделей')
  })

  it('unregisters listeners and clears the message timer on unmount', async () => {
    const { saveSettings, message } = await setupAndMount()

    await saveSettings()
    expect(message.value).toBe('Настройки сохранены')

    const clearSpy = vi.spyOn(globalThis, 'clearTimeout')
    capturedOnUnmountedCb?.()

    expect(mocks.unlistenFns.get('ocr-status-changed')).toHaveBeenCalledTimes(1)
    expect(mocks.unlistenFns.get('settings-changed')).toHaveBeenCalledTimes(1)
    expect(clearSpy).toHaveBeenCalled()
    clearSpy.mockRestore()
  })

  it('ignores a stale settings snapshot that resolves after unmount', async () => {
    defaultInvoke()
    const { settings, refreshSettings } = useOcr()

    let resolveSettings!: (value: unknown) => void
    mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
      if (cmd === 'get_ocr_settings') {
        return new Promise((resolve) => { resolveSettings = resolve })
      }
      return undefined
    })

    const pending = refreshSettings()
    capturedOnUnmountedCb?.()
    resolveSettings({ enabled: true, model_id: 'late' })
    await pending

    expect(settings.value).toEqual({ enabled: false, model_id: null, capture_target: { type: 'all' } })
  })

  it('ignores a stale status snapshot that resolves after unmount', async () => {
    defaultInvoke()
    const { status, refreshStatus } = useOcr()

    let resolveStatus!: (value: unknown) => void
    mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
      if (cmd === 'get_ocr_status') {
        return new Promise((resolve) => { resolveStatus = resolve })
      }
      return undefined
    })

    const pending = refreshStatus()
    capturedOnUnmountedCb?.()
    resolveStatus({ state: 'ready' })
    await pending

    expect(status.value).toEqual({ state: 'disabled' })
  })

  it('ignores a stale pack snapshot that resolves after unmount', async () => {
    defaultInvoke()
    const { packs, refreshPacks } = useOcr()

    let resolvePacks!: (value: unknown) => void
    mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
      if (cmd === 'list_ocr_packs') {
        return new Promise((resolve) => { resolvePacks = resolve })
      }
      return undefined
    })

    const pending = refreshPacks()
    capturedOnUnmountedCb?.()
    resolvePacks([makePack('late')])
    await pending

    expect(packs.value).toEqual([])
  })

  it('rescanPacks invokes refresh_ocr_packs, updates packs and refreshes settings', async () => {
    const { packs, rescanPacks } = await setupAndMount()
    mocks.mockInvoke.mockClear()

    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'refresh_ocr_packs') return [makePack('pack-1'), makePack('pack-2')]
      if (cmd === 'get_ocr_settings') return { enabled: false, model_id: null }
      return undefined
    })

    await rescanPacks()

    expect(mocks.mockInvoke).toHaveBeenCalledWith('refresh_ocr_packs')
    expect(mocks.mockInvoke).not.toHaveBeenCalledWith('list_ocr_packs')
    expect(packs.value).toEqual([makePack('pack-1'), makePack('pack-2')])
    await vi.waitFor(() => expect(mocks.mockInvoke).toHaveBeenCalledWith('get_ocr_settings'))
  })

  it('rescanPacks surfaces an error and leaves packs untouched', async () => {
    const { packs, rescanPacks, message } = await setupAndMount()
    mocks.mockInvoke.mockClear()

    mocks.mockInvoke.mockRejectedValueOnce(new Error('scan failed'))

    await rescanPacks()

    expect(mocks.mockInvoke).toHaveBeenCalledWith('refresh_ocr_packs')
    expect(packs.value).toEqual([])
    expect(message.value).toBe('Не удалось обновить список моделей')
  })

  describe('monitor inventory and capture target', () => {
    it('loads the monitor inventory on mount', async () => {
      const { monitors, monitorListPending, monitorListError } = await setupAndMount({
        monitors: [makeMonitor()],
      })

      expect(monitors.value).toEqual([makeMonitor()])
      expect(monitorListPending.value).toBe(false)
      expect(monitorListError.value).toBe(false)
    })

    it('refreshes inventory when the mounted panel becomes active and stops after unmount', async () => {
      defaultInvoke()
      const active = ref(false)
      useOcr(() => active.value)
      expect(mocks.mockInvoke).not.toHaveBeenCalled()
      active.value = true
      await nextTick()
      expect(mocks.mockInvoke).toHaveBeenCalledWith('list_ocr_monitors')
      active.value = false
      await nextTick()
      mocks.mockInvoke.mockClear()
      capturedOnUnmountedCb?.()
      active.value = true
      await nextTick()
      expect(mocks.mockInvoke).not.toHaveBeenCalled()
    })

    it('does not declare the saved monitor missing before its first inventory response', async () => {
      defaultInvoke()
      const { settings, selectedMonitorMissing, refreshMonitors } = useOcr()
      settings.value.capture_target = { type: 'monitor', devicePath: 'saved-device' }
      expect(selectedMonitorMissing.value).toBe(false)
      mocks.mockInvoke.mockResolvedValueOnce([])
      await refreshMonitors()
      expect(selectedMonitorMissing.value).toBe(true)
    })

    it('rejects nonphysical monitor geometry', () => {
      for (const geometry of [
        { x: 0, y: 0, width: 0, height: 1080 },
        { x: 0, y: 0, width: -1, height: 1080 },
        { x: 0, y: 0, width: 1920, height: 1.5 },
        { x: 0.5, y: 0, width: 1920, height: 1080 },
        { x: 2147483648, y: 0, width: 1920, height: 1080 },
      ]) {
        expect(convertOcrMonitorListFromRust([{ ...makeMonitor(), geometry }])).toEqual([])
      }
    })

    it('tracks pending and clears it after the monitor list resolves', async () => {
      defaultInvoke()
      const { monitors, monitorListPending, refreshMonitors } = useOcr()

      let resolveList!: (value: unknown) => void
      mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
        if (cmd === 'list_ocr_monitors') {
          return new Promise((resolve) => { resolveList = resolve })
        }
        return undefined
      })

      const pending = refreshMonitors()
      expect(monitorListPending.value).toBe(true)

      resolveList([makeMonitor()])
      await pending

      expect(monitorListPending.value).toBe(false)
      expect(monitors.value).toEqual([makeMonitor()])
    })

    it('surfaces a persistent monitor list error and clears it on a successful retry', async () => {
      const { monitors, monitorListPending, monitorListError, refreshMonitors } = await setupAndMount()

      mocks.mockInvoke.mockRejectedValueOnce(new Error('enum failed'))
      await refreshMonitors()
      expect(monitorListError.value).toBe(true)
      expect(monitorListPending.value).toBe(false)

      mocks.mockInvoke.mockResolvedValueOnce([makeMonitor()])
      await refreshMonitors()
      expect(monitorListError.value).toBe(false)
      expect(monitors.value).toEqual([makeMonitor()])
    })

    it('guards duplicate in-flight monitor refreshes', async () => {
      const { refreshMonitors, monitors } = await setupAndMount()
      mocks.mockInvoke.mockClear()

      let resolveList!: (value: unknown) => void
      mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
        if (cmd === 'list_ocr_monitors') {
          return new Promise((resolve) => { resolveList = resolve })
        }
        return undefined
      })

      const first = refreshMonitors()
      await Promise.resolve()
      await refreshMonitors()

      resolveList([makeMonitor()])
      await first

      const calls = mocks.mockInvoke.mock.calls.filter((call) => call[0] === 'list_ocr_monitors')
      expect(calls).toHaveLength(1)
      expect(monitors.value).toEqual([makeMonitor()])
    })

    it('reports a specific monitor absent from a successful inventory as missing', async () => {
      const target = { type: 'monitor', devicePath: '\\\\.\\DISPLAY9\\Monitor0' } as const
      const { settings, selectedMonitorMissing } = await setupAndMount({
        settings: { enabled: false, model_id: null, capture_target: target },
        monitors: [makeMonitor()],
      })

      expect(settings.value.capture_target).toEqual(target)
      expect(selectedMonitorMissing.value).toBe(true)
    })

    it('does not report a missing monitor while enumeration fails', async () => {
      const { selectedMonitorMissing, monitorListError, refreshMonitors } = await setupAndMount({
        settings: {
          enabled: false,
          model_id: null,
          capture_target: { type: 'monitor', devicePath: '\\\\.\\DISPLAY9\\Monitor0' },
        },
        monitors: [makeMonitor()],
      })

      expect(selectedMonitorMissing.value).toBe(true)

      mocks.mockInvoke.mockRejectedValueOnce(new Error('enum failed'))
      await refreshMonitors()

      // An enumeration failure must not masquerade as a missing monitor.
      expect(monitorListError.value).toBe(true)
      expect(selectedMonitorMissing.value).toBe(false)
    })

    it('preserves a missing selected monitor across a refresh without changing selection', async () => {
      const target = { type: 'monitor', devicePath: '\\\\.\\DISPLAY9\\Monitor0' } as const
      const { settings, selectedMonitorMissing, refreshMonitors } = await setupAndMount({
        settings: { enabled: false, model_id: null, capture_target: target },
        monitors: [makeMonitor()],
      })

      expect(selectedMonitorMissing.value).toBe(true)

      mocks.mockInvoke.mockResolvedValueOnce([makeMonitor()])
      await refreshMonitors()

      expect(settings.value.capture_target).toEqual(target)
      expect(selectedMonitorMissing.value).toBe(true)
    })

    it('persists the latest capture target when a target edit lands during a save drain', async () => {
      const { settings, saveSettings } = await setupAndMount()
      mocks.mockInvoke.mockClear()

      const saveCalls: unknown[] = []
      const persisted: Record<string, unknown> = {
        enabled: false,
        model_id: null,
        capture_target: { type: 'all' },
      }
      let resolveFirst!: () => void
      mocks.mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
        if (cmd === 'save_ocr_settings') {
          saveCalls.push(args)
          const s = (args as { settings?: Record<string, unknown> } | undefined)?.settings
          if (s && typeof s === 'object') Object.assign(persisted, s)
          if (saveCalls.length === 1) {
            return new Promise((resolve) => { resolveFirst = () => resolve(undefined) })
          }
          return Promise.resolve(undefined)
        }
        if (cmd === 'get_ocr_settings') return Promise.resolve({ ...persisted })
        return Promise.resolve(undefined)
      })

      settings.value = { enabled: true, model_id: 'pack-a', capture_target: { type: 'all' } }
      const first = saveSettings()
      await vi.waitFor(() => expect(saveCalls).toHaveLength(1))

      settings.value = { ...settings.value, capture_target: { type: 'primary' } }
      await saveSettings()
      expect(saveCalls).toHaveLength(1)

      resolveFirst()
      await first

      expect(saveCalls).toEqual([
        { settings: { enabled: true, model_id: 'pack-a', capture_target: { type: 'all' } } },
        { settings: { enabled: true, model_id: 'pack-a', capture_target: { type: 'primary' } } },
      ])
      expect(settings.value.capture_target).toEqual({ type: 'primary' })
    })

    it('rolls back the capture target to the last confirmed value on failure', async () => {
      const { settings, saveSettings } = await setupAndMount()

      settings.value.capture_target = { type: 'primary' }
      await saveSettings()

      settings.value.capture_target = { type: 'monitor', devicePath: '\\\\.\\DISPLAY2\\Monitor0' }
      mocks.mockInvoke.mockRejectedValueOnce(new Error('backend down'))
      await saveSettings()

      expect(settings.value.capture_target).toEqual({ type: 'primary' })
    })

    it('ignores a stale monitor list reply that resolves after unmount', async () => {
      defaultInvoke()
      const { monitors, refreshMonitors } = useOcr()

      let resolveMonitors!: (value: unknown) => void
      mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
        if (cmd === 'list_ocr_monitors') {
          return new Promise((resolve) => { resolveMonitors = resolve })
        }
        return undefined
      })

      const pending = refreshMonitors()
      capturedOnUnmountedCb?.()
      resolveMonitors([makeMonitor('late')])
      await pending

      expect(monitors.value).toEqual([])
    })
  })

  describe('runtimeHoldsModel', () => {
    it('is true when the runtime is ready and the model is absent from packs', async () => {
      const { runtimeHoldsModel } = await setupAndMount({
        settings: { enabled: true, model_id: 'gone' },
        status: { state: 'ready' },
        packs: [makePack('other')],
      })

      expect(runtimeHoldsModel.value).toBe(true)
    })

    it('is false when the selected model is present in packs', async () => {
      const { runtimeHoldsModel } = await setupAndMount({
        settings: { enabled: true, model_id: 'pack-1' },
        status: { state: 'ready' },
        packs: [makePack('pack-1')],
      })

      expect(runtimeHoldsModel.value).toBe(false)
    })

    it('is false when the runtime is disabled', async () => {
      const { runtimeHoldsModel } = await setupAndMount({
        settings: { enabled: true, model_id: 'gone' },
        status: { state: 'disabled' },
        packs: [makePack('other')],
      })

      expect(runtimeHoldsModel.value).toBe(false)
    })

    it('is false when no model is selected', async () => {
      const { runtimeHoldsModel } = await setupAndMount({
        settings: { enabled: false, model_id: null },
        status: { state: 'ready' },
        packs: [],
      })

      expect(runtimeHoldsModel.value).toBe(false)
    })
  })
})
