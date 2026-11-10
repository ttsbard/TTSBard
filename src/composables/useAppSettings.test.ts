import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { effectScope, nextTick, createRenderer, h } from 'vue'

const { mockInvoke, listenCallbacks, unlistenFns } = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
  listenCallbacks: new Map<string, (payload: unknown) => void>(),
  unlistenFns: new Map<string, () => void>(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mockInvoke,
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, callback: (payload: unknown) => void) => {
    listenCallbacks.set(event, callback)
    const unlisten = vi.fn(() => {
      listenCallbacks.delete(event)
    })
    unlistenFns.set(event, unlisten)
    return Promise.resolve(unlisten)
  }),
}))

vi.mock('../utils/debug', () => ({
  debugLog: vi.fn(),
  debugError: vi.fn(),
  debugWarn: vi.fn(),
  debugInfo: vi.fn(),
}))

import { createAppSettings, provideAppSettings, useVrchatSettings, type VrchatSettingsDto } from './useAppSettings'
import { useStartupNotifications } from './useStartupNotifications'
import { useErrorHandler, ErrorLevel } from './useErrorHandler'
import { createMainWindowModeController, type MainWindowModeController, type MainWindowModeSnapshot } from './mainWindowMode'
import type { AppSettingsDto } from '../types/settings'

function mockSettings(): AppSettingsDto & { vrchat: VrchatSettingsDto } {
  return {
    storage: { data_dir: null, audio_cache_dir: null },
    tts: {
      provider: 'silero',
      provider_id: 'id1',
      providers: [],
      openai: { api_key: null, voice: 'alloy', proxy_host: null, proxy_port: null, use_proxy: false },
      local: { url: '' },
      fish: { api_key: null, voices: [], reference_id: '', format: 'wav', temperature: 0.7, sample_rate: 44100, use_proxy: false },
      elevenlabs: { api_key: null, voice_id: '', voices: [], models: [], model_id: '', output_format: 'mp3_44100_128', stability: 0.5, similarity_boost: 0.75, style: 0, use_speaker_boost: true, use_proxy: false },
      telegram: { api_id: null, proxy_mode: 'none', voices: [], current_voice_id: '', synthesis_response_timeout_ms: 10000, download_retry_delay_ms: 1000 },
      network: { proxy: { proxy_url: null }, mtproxy: { host: null, port: 443, secret: null, dc_id: null } },
    },
    webview: { enabled: false, start_on_boot: false, port: 8080, access_token: null, upnp_enabled: false, send_original_text: true },
    twitch: { mode: 'irc', enabled: false, username: '', token: '', channel: '', start_on_boot: false, send_original_text: true },
    windows: {
      global: { exclude_from_capture: false },
      main: { x: null, y: null, custom_background: false, opacity: 100, bg_color: '', custom_opacity: false, opacity_compact_only: false, compact_width: 400, compact_height: 300, compact_view: 'compact', hide_extra_window_buttons: false },
      soundpanel: { x: null, y: null, opacity: 100, bg_color: '', clickthrough: false, stay_visible: false, hide_on_blur: false, appearance_source: '' },
      playback: { x: null, y: null, opacity: 100, bg_color: '', appearance_source: '' },
    },
    audio: { output_format: 'default', speaker_device: null, speaker_enabled: true, speaker_volume: 100, virtual_mic_device: null, virtual_mic_volume: 100 },
    audio_effects: { enabled: false, pitch: 0, speed: 0, volume: 100, enhance_enabled: false, enhance_atten_db: 10, formant_preserved: true, boundary_cleanup_enabled: true },
    dsp: {
      eq: { enabled: false, low_cut_enabled: false, low_cut_hz: 80, low_cut_slope_db: 12, bands: [], high_shelf_enabled: false, high_shelf_hz: 8000, high_shelf_gain_db: 0 },
      compressor: { enabled: false, threshold_db: -20, ratio: 4, attack_ms: 5, release_ms: 50, knee_db: 6, makeup_db: 0 },
      limiter: { enabled: false, ceiling_db: -1, release_ms: 50 },
    },
    general: { hotkey_enabled: true, theme: 'dark', ui_language: 'ru', ui_font_family: 'default', ui_font_size_px: 16, show_playback_on_start: false, start_compact: false, hide_on_minimize: false },
    logging: { enabled: true, level: 'info', module_levels: {} },
    preprocessor: { enabled: false, replacements_count: 0 },
    soundpanel_bindings: [],
    editor: { quick: 'disabled', ai: false, ai_completion: false, autocomplete_enabled: true, number_conversion_enabled: true, spellcheck_enabled: false, spellcheck_source: 'online', editor_height: 200, typing_idle_timeout_ms: 800, typing_enabled: true, default_route: 'everywhere', keep_text_after_send: false, font_family: 'default', font_size_px: 16, homograph_accentor: { enabled: false, accentor_pack_id: null, load_on_start: false } },
    ocr: { enabled: false, model_id: null, capture_target: { type: 'all' } },
    ai: {
      provider: 'openai',
      openai: { api_key: null, use_proxy: false, model: 'gpt-4o-mini' },
      zai: { url: null, api_key: null, model: 'glm-4' },
      deepseek: { api_key: null, use_proxy: false, model: 'deepseek-v4-pro' },
      custom: { url: null, api_key: null, use_proxy: false, model: 'default' },
      prompt: '',
      timeout: 30000,
    },
    hotkeys: {
      main_window: { modifiers: [], key: '' },
      sound_panel: { modifiers: [], key: '' },
      playback_pause: { modifiers: [], key: '' },
      playback_stop: { modifiers: [], key: '' },
      playback_repeat: { modifiers: [], key: '' },
      playback_control_window: { modifiers: [], key: '' },
      return_previous_window: { modifiers: [], key: '' },
      toggle_minimal_mode: { modifiers: [], key: '' },
      ocr_capture: { modifiers: [], key: '' },
      editor: {
        edit_word: { modifiers: [], key: '' },
        submit_continue: { modifiers: [], key: '' },
        submit_keep_text: { modifiers: [], key: '' },
        submit_keep_focus: { modifiers: [], key: '' },
        next_spelling_error: { modifiers: [], key: '' },
        previous_spelling_error: { modifiers: [], key: '' },
        next_tab: { modifiers: [], key: '' },
        previous_tab: { modifiers: [], key: '' },
        cycle_route: { modifiers: [], key: '' },
        toggle_typing: { modifiers: [], key: '' },
        cycle_quick_mode: { modifiers: [], key: '' },
        toggle_history: { modifiers: [], key: '' },
        accent_homographs: { modifiers: [], key: '' },
        approve_next_incoming: { modifiers: [], key: '' },
        edit_next_incoming: { modifiers: [], key: '' },
      },
    },
    vtube_studio: {
      enabled: false,
      host: '127.0.0.1',
      port: 8001,
      start_on_boot: false,
      typingAction: { outputMode: 'Event', parameterName: 'TTSBardTyping', startHotkeyId: '', stopHotkeyId: '', startHotkeyName: '', stopHotkeyName: '', itemFileName: '', itemType: '' },
    },
    vrchat: {
      enabled: false,
      start_on_boot: false,
      host: '127.0.0.1',
      port: 9000,
    },
  }
}

describe('createAppSettings', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    listenCallbacks.clear()
    unlistenFns.clear()
    mockInvoke.mockResolvedValue(undefined)
    vi.useFakeTimers()
  })

  afterEach(() => {
    useErrorHandler().clearAllErrors()
    vi.useRealTimers()
  })

  it('loads settings when backend is ready', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'is_backend_ready') return true
      if (cmd === 'get_all_app_settings') return mockSettings()
    })

    const ctx = createAppSettings()
    await vi.runAllTimersAsync()

    expect(ctx.settings.value).toEqual(mockSettings())
    expect(ctx.isLoading.value).toBe(false)
    expect(ctx.error.value).toBeNull()
    expect(mockInvoke).toHaveBeenCalledWith('is_backend_ready')
    expect(mockInvoke).toHaveBeenCalledWith('get_all_app_settings', { consumeStartupNotifications: false })
  })

  it('provideAppSettings opts the main window into consuming startup notifications', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'is_backend_ready') return true
      if (cmd === 'get_all_app_settings') return mockSettings()
    })

    const renderer = createRenderer({
      createElement: () => ({}),
      createText: () => ({}),
      createComment: () => ({}),
      insert: () => {},
      remove: () => {},
      setText: () => {},
      setElementText: () => {},
      parentNode: () => null,
      nextSibling: () => null,
      patchProp: () => {},
    })

    const app = renderer.createApp({
      setup() {
        provideAppSettings()
        return () => h('div')
      },
    })
    app.mount({})
    await vi.runAllTimersAsync()

    const settingsCalls = mockInvoke.mock.calls.filter(([cmd]) => cmd === 'get_all_app_settings')
    expect(settingsCalls.length).toBeGreaterThan(0)
    expect(settingsCalls[0]).toEqual(['get_all_app_settings', { consumeStartupNotifications: true }])
    app.unmount()
  })

  it('sets error when backend is not ready', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'is_backend_ready') return false
    })

    const ctx = createAppSettings()
    await vi.advanceTimersByTimeAsync(10000)
    await Promise.resolve()

    expect(ctx.settings.value).toBeNull()
    expect(ctx.isLoading.value).toBe(false)
    expect(ctx.error.value).toContain('Backend not ready')
  })

  it('reloads settings when settings-changed event fires', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'is_backend_ready') return true
      if (cmd === 'get_all_app_settings') return mockSettings()
    })

    createAppSettings()
    await vi.runAllTimersAsync()
    mockInvoke.mockClear()

    const cb = listenCallbacks.get('settings-changed')
    expect(cb).toBeDefined()
    cb?.(undefined)

    await vi.runAllTimersAsync()

    expect(mockInvoke).toHaveBeenCalledWith('is_backend_ready')
    expect(mockInvoke).toHaveBeenCalledWith('get_all_app_settings', { consumeStartupNotifications: false })
  })

  it('reloads when backend-ready event fires after failed initial load', async () => {
    let backendReady = false
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'is_backend_ready') return backendReady
      if (cmd === 'get_all_app_settings') return mockSettings()
    })

    const ctx = createAppSettings()
    await vi.advanceTimersByTimeAsync(10000)
    await Promise.resolve()

    expect(ctx.settings.value).toBeNull()
    expect(ctx.error.value).toContain('Backend not ready')

    backendReady = true
    mockInvoke.mockClear()

    const cb = listenCallbacks.get('backend-ready')
    expect(cb).toBeDefined()
    cb?.(undefined)

    await vi.runAllTimersAsync()

    expect(ctx.settings.value).toEqual(mockSettings())
    expect(ctx.isLoading.value).toBe(false)
  })

  it('cleans up all listeners when cleanup() is called', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'is_backend_ready') return true
      if (cmd === 'get_all_app_settings') return mockSettings()
    })

    const ctx = createAppSettings()
    await vi.runAllTimersAsync()

    expect(unlistenFns.has('settings-changed')).toBe(true)

    ctx.cleanup?.()

    expect(unlistenFns.get('settings-changed')).toHaveBeenCalled()
    expect(unlistenFns.get('backend-ready')).toHaveBeenCalled()
    expect(unlistenFns.get('tts-provider-changed')).toHaveBeenCalled()
    expect(unlistenFns.get('soundpanel-bindings-changed')).toHaveBeenCalled()
  })

  it('registers four event listeners on startup', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'is_backend_ready') return true
      if (cmd === 'get_all_app_settings') return mockSettings()
    })

    createAppSettings()
    await vi.runAllTimersAsync()

    expect(listenCallbacks.has('backend-ready')).toBe(true)
    expect(listenCallbacks.has('settings-changed')).toBe(true)
    expect(listenCallbacks.has('tts-provider-changed')).toBe(true)
    expect(listenCallbacks.has('soundpanel-bindings-changed')).toBe(true)
  })
})

describe('createAppSettings startup notification ownership', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    listenCallbacks.clear()
    unlistenFns.clear()
    mockInvoke.mockReset()
  })

  afterEach(() => {
    useErrorHandler().clearAllErrors()
  })

  it('does not consume startup notifications for secondary contexts and consumes them once for the main context', async () => {
    // Backend one-shot queue: notifications and startup_errors are returned and
    // cleared only when a read explicitly requests consumption.
    let startupQueue: { notifications: string[]; startup_errors: string[] } | null = {
      notifications: ['warning'],
      startup_errors: ['Piper unavailable'],
    }
    mockInvoke.mockImplementation(async (cmd: string, args?: { consumeStartupNotifications?: boolean }) => {
      if (cmd === 'is_backend_ready') return true
      if (cmd === 'get_all_app_settings') {
        const settings = mockSettings()
        if (args?.consumeStartupNotifications && startupQueue) {
          settings.notifications = startupQueue.notifications
          settings.startup_errors = startupQueue.startup_errors
          startupQueue = null
        }
        return settings
      }
      throw new Error(`Unexpected command: ${cmd}`)
    })

    // A default secondary context (font sync) must not consume the queue.
    const secondary = createAppSettings()
    await vi.waitFor(() => expect(secondary.settings.value).not.toBeNull())
    expect(startupQueue).not.toBeNull()
    expect(mockInvoke).toHaveBeenCalledWith('get_all_app_settings', { consumeStartupNotifications: false })
    expect(useErrorHandler().errors.value).toHaveLength(0)

    // The consuming main context surfaces the global warning + error exactly once.
    const scope = effectScope()
    const main = createAppSettings({ consumeStartupNotifications: true })
    scope.run(() => useStartupNotifications(main.settings))
    await vi.waitFor(() => expect(main.settings.value).not.toBeNull())
    await nextTick()
    expect(startupQueue).toBeNull()
    expect(useErrorHandler().errors.value.map(({ message, level }) => ({ message, level }))).toEqual([
      { message: 'warning', level: ErrorLevel.WARNING },
      { message: 'Piper unavailable', level: ErrorLevel.ERROR },
    ])

    // Secondary reload cannot consume; primary reload produces no duplicate.
    await secondary.reload()
    await nextTick()
    expect(useErrorHandler().errors.value).toHaveLength(2)

    await main.reload()
    await nextTick()
    expect(useErrorHandler().errors.value).toHaveLength(2)

    scope.stop()
    secondary.cleanup?.()
    main.cleanup?.()
    useErrorHandler().clearAllErrors()
  })

  it('consumes the startup queue first when the main context loads before secondary contexts', async () => {
    let startupQueue: { notifications: string[]; startup_errors: string[] } | null = {
      notifications: ['warning'],
      startup_errors: ['Piper unavailable'],
    }
    mockInvoke.mockImplementation(async (cmd: string, args?: { consumeStartupNotifications?: boolean }) => {
      if (cmd === 'is_backend_ready') return true
      if (cmd === 'get_all_app_settings') {
        const settings = mockSettings()
        if (args?.consumeStartupNotifications && startupQueue) {
          settings.notifications = startupQueue.notifications
          settings.startup_errors = startupQueue.startup_errors
          startupQueue = null
        }
        return settings
      }
      throw new Error(`Unexpected command: ${cmd}`)
    })

    const scope = effectScope()
    const main = createAppSettings({ consumeStartupNotifications: true })
    scope.run(() => useStartupNotifications(main.settings))
    await vi.waitFor(() => expect(main.settings.value).not.toBeNull())
    await nextTick()

    const secondary = createAppSettings()
    await vi.waitFor(() => expect(secondary.settings.value).not.toBeNull())

    expect(startupQueue).toBeNull()
    expect(useErrorHandler().errors.value).toHaveLength(2)

    scope.stop()
    main.cleanup?.()
    secondary.cleanup?.()
    useErrorHandler().clearAllErrors()
  })

  it('exposes compact_view on windows.main and updates upon reload', async () => {
    let currentCompactView: 'compact' | 'mono' = 'compact'
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'is_backend_ready') return true
      if (cmd === 'get_all_app_settings') {
        const s = mockSettings()
        s.windows.main.compact_view = currentCompactView
        return s
      }
      throw new Error(`Unexpected command: ${cmd}`)
    })

    const context = createAppSettings()
    await vi.waitFor(() => expect(context.settings.value).not.toBeNull())

    expect(context.settings.value?.windows.main.compact_view).toBe('compact')

    // Simulate backend update to mono
    currentCompactView = 'mono'
    await context.reload()

    expect(context.settings.value?.windows.main.compact_view).toBe('mono')
    context.cleanup?.()
  })
})

describe('createAppSettings compact_view snapshot hydration', () => {
  function makeController() {
    const adapter = {
      setBounds: vi.fn(async () => {}),
      removeBounds: vi.fn(async () => {}),
      resize: vi.fn(async () => {}),
      persistCompactView: vi.fn(async () => {}),
    }
    const storage = { updateStoredView: vi.fn() }
    const controller = createMainWindowModeController({
      adapter,
      storage,
      boot: { startCompact: true, compactView: 'compact', compactWidth: 450, compactHeight: 400 },
    })
    return { controller, adapter, storage }
  }

  function controllerHooks(controller: MainWindowModeController) {
    return {
      captureSnapshotToken: () => controller.captureSnapshotRevision(),
      applySnapshot: (snapshot: MainWindowModeSnapshot, token: number) => controller.applySettingsSnapshot(snapshot, token),
    }
  }

  beforeEach(() => {
    vi.clearAllMocks()
    listenCallbacks.clear()
    unlistenFns.clear()
    mockInvoke.mockReset()
  })

  afterEach(() => {
    useErrorHandler().clearAllErrors()
  })

  it('delayed initial snapshot after a local switch does not roll back view/cache', async () => {
    let resolveSettings!: (s: AppSettingsDto) => void
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'is_backend_ready') return Promise.resolve(true)
      if (cmd === 'get_all_app_settings') {
        return new Promise<AppSettingsDto>((resolve) => { resolveSettings = resolve })
      }
      return Promise.reject(new Error(`Unexpected command: ${cmd}`))
    })

    const { controller, storage } = makeController()
    const ctx = createAppSettings({ compactSnapshotHooks: controllerHooks(controller) })
    await vi.waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('get_all_app_settings', { consumeStartupNotifications: false }),
    )

    const ok = await controller.setCompactView('mono')
    expect(ok).toBe(true)
    expect(controller.rememberedView.value).toBe('mono')

    const stale = mockSettings()
    stale.windows.main.compact_view = 'compact'
    resolveSettings(stale)
    await vi.waitFor(() => expect(ctx.settings.value).not.toBeNull())

    expect(controller.rememberedView.value).toBe('mono')
    expect(storage.updateStoredView).toHaveBeenLastCalledWith('mono')
    expect(controller.confirmedCompactDimensions.value).toEqual({ width: 450, height: 400 })
    ctx.cleanup?.()
  })

  it('snapshot resolving during a switch does not roll back view/cache', async () => {
    let resolveSettings!: (s: AppSettingsDto) => void
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'is_backend_ready') return Promise.resolve(true)
      if (cmd === 'get_all_app_settings') {
        return new Promise<AppSettingsDto>((resolve) => { resolveSettings = resolve })
      }
      return Promise.reject(new Error(`Unexpected command: ${cmd}`))
    })

    const { controller, adapter, storage } = makeController()
    let resolveSetView!: () => void
    adapter.persistCompactView.mockReturnValueOnce(new Promise<void>((resolve) => { resolveSetView = resolve }))

    const ctx = createAppSettings({ compactSnapshotHooks: controllerHooks(controller) })
    await vi.waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('get_all_app_settings', { consumeStartupNotifications: false }),
    )

    const switchPromise = controller.setCompactView('mono')
    expect(controller.busy.value).toBe(true)

    const stale = mockSettings()
    stale.windows.main.compact_view = 'compact'
    resolveSettings(stale)
    await vi.waitFor(() => expect(ctx.settings.value).not.toBeNull())

    expect(controller.rememberedView.value).toBe('mono')

    resolveSetView()
    await switchPromise

    expect(controller.rememberedView.value).toBe('mono')
    expect(storage.updateStoredView).toHaveBeenLastCalledWith('mono')
    ctx.cleanup?.()
  })

  it('a load that started during a setter and resolved after it cannot roll back the choice', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'is_backend_ready') return Promise.resolve(true)
      if (cmd === 'get_all_app_settings') return Promise.resolve(mockSettings())
      return Promise.reject(new Error(`Unexpected command: ${cmd}`))
    })

    const { controller, adapter, storage } = makeController()
    const ctx = createAppSettings({ compactSnapshotHooks: controllerHooks(controller) })
    await vi.waitFor(() => expect(ctx.settings.value).not.toBeNull())
    expect(controller.rememberedView.value).toBe('compact')

    let resolveSettings!: (s: AppSettingsDto) => void
    let resolveSetView!: () => void
    adapter.persistCompactView.mockReturnValueOnce(new Promise<void>((resolve) => { resolveSetView = resolve }))
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'is_backend_ready') return Promise.resolve(true)
      if (cmd === 'get_all_app_settings') {
        return new Promise<AppSettingsDto>((resolve) => { resolveSettings = resolve })
      }
      return Promise.reject(new Error(`Unexpected command: ${cmd}`))
    })

    const switchPromise = controller.setCompactView('mono')
    expect(controller.busy.value).toBe(true)

    // A fresh settings load starts while the setter is in flight and captures
    // the mid-setter revision.
    const reloadPromise = ctx.reload()
    await vi.waitFor(() => expect(resolveSettings).toBeDefined())

    // Setter completes, invalidating any snapshot captured during the switch.
    resolveSetView()
    await switchPromise
    expect(controller.busy.value).toBe(false)

    // The stale snapshot resolves after the setter and must be rejected.
    const stale = mockSettings()
    stale.windows.main.compact_view = 'compact'
    resolveSettings(stale)
    await reloadPromise

    expect(controller.rememberedView.value).toBe('mono')
    expect(storage.updateStoredView).toHaveBeenLastCalledWith('mono')

    // A genuinely fresh load is still applied.
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'is_backend_ready') return Promise.resolve(true)
      if (cmd === 'get_all_app_settings') {
        const s = mockSettings()
        s.windows.main.compact_view = 'compact'
        return Promise.resolve(s)
      }
      return Promise.reject(new Error(`Unexpected command: ${cmd}`))
    })
    await ctx.reload()
    await vi.waitFor(() => expect(controller.rememberedView.value).toBe('compact'))
    expect(storage.updateStoredView).toHaveBeenLastCalledWith('compact')

    ctx.cleanup?.()
  })

  it('applies a fresh subsequent snapshot when no local switch intervened', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'is_backend_ready') return Promise.resolve(true)
      if (cmd === 'get_all_app_settings') return Promise.resolve(mockSettings())
      return Promise.reject(new Error(`Unexpected command: ${cmd}`))
    })

    const { controller, storage } = makeController()
    const ctx = createAppSettings({ compactSnapshotHooks: controllerHooks(controller) })
    await vi.waitFor(() => expect(ctx.settings.value).not.toBeNull())
    expect(controller.rememberedView.value).toBe('compact')

    let currentView: 'compact' | 'mono' = 'mono'
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'is_backend_ready') return Promise.resolve(true)
      if (cmd === 'get_all_app_settings') {
        const s = mockSettings()
        s.windows.main.compact_view = currentView
        return Promise.resolve(s)
      }
      return Promise.reject(new Error(`Unexpected command: ${cmd}`))
    })

    await ctx.reload()

    expect(controller.rememberedView.value).toBe('mono')
    expect(storage.updateStoredView).toHaveBeenLastCalledWith('mono')
    ctx.cleanup?.()
  })

  it('applies compact_view after a failed load is retried', async () => {
    let failOnce = true
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'is_backend_ready') return Promise.resolve(true)
      if (cmd === 'get_all_app_settings') {
        if (failOnce) {
          failOnce = false
          return Promise.reject(new Error('boom'))
        }
        const s = mockSettings()
        s.windows.main.compact_view = 'mono'
        return Promise.resolve(s)
      }
      return Promise.reject(new Error(`Unexpected command: ${cmd}`))
    })

    const { controller } = makeController()
    const ctx = createAppSettings({ compactSnapshotHooks: controllerHooks(controller) })
    await vi.waitFor(() => expect(ctx.error.value).toBe('boom'))

    await ctx.reload()
    await vi.waitFor(() => expect(ctx.settings.value).not.toBeNull())

    expect(controller.rememberedView.value).toBe('mono')
    ctx.cleanup?.()
  })
})

describe('useVrchatSettings', () => {
  it('returns vrchat settings when provided', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'is_backend_ready') return true
      if (cmd === 'get_all_app_settings') return mockSettings()
    })

    let vrchatSettings: ReturnType<typeof useVrchatSettings> | undefined
    const renderer = createRenderer({
      createElement: () => ({}),
      createText: () => ({}),
      createComment: () => ({}),
      insert: () => {},
      remove: () => {},
      setText: () => {},
      setElementText: () => {},
      parentNode: () => null,
      nextSibling: () => null,
      patchProp: () => {},
    })

    const Child = {
      setup() {
        vrchatSettings = useVrchatSettings()
        return () => h('span')
      },
    }

    const Root = {
      setup() {
        provideAppSettings()
        return () => h(Child)
      },
    }

    renderer.render(h(Root), {} as any)
    await vi.waitFor(() => expect(vrchatSettings?.value).toBeDefined())
    expect(vrchatSettings?.value).toEqual({
      enabled: false,
      start_on_boot: false,
      host: '127.0.0.1',
      port: 9000,
    })
  })
})
