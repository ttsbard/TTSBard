// @vitest-environment jsdom
import { createApp } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '../App.vue'
import {
  resetOutputControlsPlacement,
  setMainWindowOutputControlsRecording,
  setOutputControlsPlacement,
} from '../composables/useMainWindowOutputControls'

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(async () => () => {}),
  getCurrentWindow: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: tauri.getCurrentWindow }))
vi.mock('@tauri-apps/api/event', () => ({ listen: tauri.listen }))

const shell = vi.hoisted(() => ({
  inputMounts: 0,
  inputUnmounts: 0,
  focusEditorCalls: 0,
}))

vi.mock('../composables/useIntegrationStatusSlots', async () => {
  const { ref } = await import('vue')
  return { useIntegrationStatusSlots: () => ({ errorSlots: ref([]) }) }
})

vi.mock('./Sidebar.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubSidebar', render: () => h('div', { class: 'stub-sidebar' }) }) }
})

vi.mock('./InputPanel.vue', async () => {
  const { defineComponent, h, inject, onMounted, onUnmounted } = await import('vue')
  const { MAIN_WINDOW_MODE_KEY } = await import('../composables/mainWindowMode')
  return {
    default: defineComponent({
      name: 'StubInputPanel',
      setup(_, { expose }) {
        const controller = inject(MAIN_WINDOW_MODE_KEY)
        onMounted(() => {
          shell.inputMounts += 1
        })
        onUnmounted(() => {
          shell.inputUnmounts += 1
        })
        expose({
          focusEditor: () => {
            shell.focusEditorCalls += 1
          },
        })
        return () =>
          h('div', { class: 'input-panel' }, [
            h(
              'button',
              {
                class: 'enable-mono-btn',
                onClick: () => {
                  void controller?.setCompactView('mono')
                },
              },
              'enable mono',
            ),
          ])
      },
    }),
  }
})

vi.mock('./TtsPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubTtsPanel', render: () => h('div', { class: 'stub-tts' }) }) }
})

vi.mock('./AudioPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubAudioPanel', render: () => h('div', { class: 'stub-audio' }) }) }
})

vi.mock('./PreprocessorPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubPreprocessorPanel', render: () => h('div', { class: 'stub-preprocessor' }) }) }
})

vi.mock('./WebViewPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubWebViewPanel', render: () => h('div', { class: 'stub-webview' }) }) }
})

vi.mock('./TwitchPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubTwitchPanel', render: () => h('div', { class: 'stub-twitch' }) }) }
})

vi.mock('./SettingsPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubSettingsPanel', render: () => h('div', { class: 'stub-settings' }) }) }
})

vi.mock('./HotkeysPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubHotkeysPanel', render: () => h('div', { class: 'stub-hotkeys' }) }) }
})

vi.mock('./InterceptPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubInterceptPanel', render: () => h('div', { class: 'stub-intercept' }) }) }
})

vi.mock('./VTubeStudioPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubVTubeStudioPanel', render: () => h('div', { class: 'stub-vtube' }) }) }
})

vi.mock('./InputServerPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubInputServerPanel', render: () => h('div', { class: 'stub-input-server' }) }) }
})

vi.mock('./OcrPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubOcrPanel', render: () => h('div', { class: 'stub-ocr' }) }) }
})

vi.mock('./ErrorToasts.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubErrorToasts', render: () => h('div', { class: 'stub-error-toasts' }) }) }
})

vi.mock('./titlebar/IntegrationStatusCluster.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubIntegrationCluster', render: () => h('div', { class: 'stub-integration-cluster' }) }) }
})

function makeSettingsDto(overrides: {
  compactView?: 'compact' | 'mono'
  compactWidth?: number
  compactHeight?: number
  startCompact?: boolean
  micDevice?: string | null
  micHotkey?: { modifiers: string[]; key: string; enabled?: boolean } | null
} = {}): Record<string, unknown> {
  const compactView = overrides.compactView ?? 'compact'
  const compactWidth = overrides.compactWidth ?? 450
  const compactHeight = overrides.compactHeight ?? 400
  const startCompact = overrides.startCompact ?? false
  return {
    storage: { data_dir: null, audio_cache_dir: null },
    notifications: [],
    startup_errors: [],
    tts: {},
    webview: {},
    twitch: {},
    windows: {
      global: { exclude_from_capture: false },
      main: {
        x: null,
        y: null,
        custom_background: false,
        opacity: 100,
        bg_color: '#090b0f',
        custom_opacity: false,
        opacity_compact_only: false,
        compact_width: compactWidth,
        compact_height: compactHeight,
        compact_view: compactView,
        hide_extra_window_buttons: false,
      },
      soundpanel: {},
      playback: {},
    },
    audio: overrides.micDevice === undefined && overrides.micHotkey === undefined
      ? {}
      : {
          output_format: 'default',
          speaker_device: null,
          speaker_enabled: true,
          speaker_volume: 100,
          virtual_mic_device: overrides.micDevice ?? null,
          virtual_mic_enabled: true,
          virtual_mic_volume: 100,
          vban: {
            enabled: true,
            address: '192.168.1.50',
            port: 6980,
            stream_name: 'TTSBard',
            volume: 100,
            ping_interval: 30,
          },
        },
    audio_effects: {},
    dsp: {},
    general: {
      hotkey_enabled: true,
      theme: 'dark',
      ui_language: 'en',
      ui_font_family: 'default',
      ui_font_size_px: 16,
      show_playback_on_start: false,
      start_compact: startCompact,
      hide_on_minimize: false,
    },
    logging: {},
    preprocessor: {},
    soundpanel_bindings: [],
    editor: {},
    ocr: {},
    ai: {},
    hotkeys: {
      toggle_minimal_mode: { modifiers: ['ctrl'], key: 'M' },
      return_previous_window: { modifiers: [], key: '' },
      sound_panel: { modifiers: [], key: '' },
      playback_control_window: { modifiers: [], key: '' },
      // Mirror the real dto: the editor hotkey section always defines the
      // output bindings (an empty binding matches nothing).
      editor: {
        toggle_vban: { modifiers: [], key: '' },
        ...(overrides.micHotkey === undefined
          ? {}
          : { toggle_virtual_mic: overrides.micHotkey }),
      },
    },
    vtube_studio: {},
  }
}

function defaultInvoke(cmd: string): Promise<unknown> {
  switch (cmd) {
    case 'is_backend_ready':
      return Promise.resolve(true)
    case 'get_all_app_settings':
      return Promise.resolve(makeSettingsDto())
    case 'telegram_auto_restore':
      return Promise.resolve(false)
    case 'get_visibility_snapshot':
      return Promise.resolve({ soundpanel_visible: false, playback_control_visible: false })
    default:
      return Promise.resolve(undefined)
  }
}

async function flushPromises(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await Promise.resolve()
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

const apps: ReturnType<typeof createApp>[] = []

function mountShellRaw(): HTMLDivElement {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp(App)
  app.config.warnHandler = () => {}
  app.mount(host)
  apps.push(app)
  return host
}

async function mountShell(): Promise<HTMLDivElement> {
  const host = mountShellRaw()
  await vi.waitFor(() => expect(document.documentElement.getAttribute('data-theme')).toBe('dark'))
  return host
}

beforeEach(() => {
  tauri.getCurrentWindow.mockReturnValue({
    minimize: vi.fn(async () => {}),
    onFocusChanged: vi.fn(async () => () => {}),
    onCloseRequested: vi.fn(async () => () => {}),
  })
  tauri.invoke.mockImplementation((cmd: string) => defaultInvoke(cmd))
})

afterEach(() => {
  for (const app of apps.splice(0)) app.unmount()
  document.body.innerHTML = ''
  document.documentElement.removeAttribute('data-theme')
  localStorage.clear()
  resetOutputControlsPlacement()
  shell.inputMounts = 0
  shell.inputUnmounts = 0
  shell.focusEditorCalls = 0
  vi.clearAllMocks()
})

describe('window mode shell wiring', () => {
  it('renders the ordinary shell with titlebar and sidebar, without the mono bar', async () => {
    const host = await mountShell()
    expect(host.querySelector('.app-titlebar')).toBeTruthy()
    expect(host.querySelector('.stub-sidebar')).toBeTruthy()
    expect(host.querySelector('.mono-mode-bar')).toBeNull()
    expect(host.querySelector('.minimal-mode-toggle')).toBeTruthy()
  })

  it('toggles ordinary -> compact through the real minimal button', async () => {
    const host = await mountShell()
    const toggle = host.querySelector('.minimal-mode-toggle') as HTMLElement
    toggle.click()
    await vi.waitFor(() => expect(host.querySelector('.stub-sidebar')).toBeNull())

    expect(host.querySelector('.app-titlebar')).toBeTruthy()
    expect(host.querySelector('.mono-mode-bar')).toBeNull()
    expect(tauri.invoke).toHaveBeenCalledWith('set_main_bounds')
    expect(tauri.invoke).toHaveBeenCalledWith('resize_main_window', { width: 450, height: 400, compact: true })
  })

  it('keeps the input/editor mounted across compact <-> mono', async () => {
    localStorage.setItem('app-start-compact', 'true')
    localStorage.setItem('app-compact-view', 'compact')
    const host = await mountShell()

    expect(host.querySelector('.mono-mode-bar')).toBeNull()
    expect(host.querySelector('.app-titlebar')).toBeTruthy()
    expect(shell.inputMounts).toBe(1)
    expect(shell.inputUnmounts).toBe(0)

    const enableMono = host.querySelector('.enable-mono-btn') as HTMLElement
    enableMono.click()
    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeTruthy())
    expect(host.querySelector('.app-titlebar')).toBeNull()
    expect(tauri.invoke).toHaveBeenCalledWith('set_main_compact_view', { view: 'mono' })

    const inputEl = host.querySelector('.input-panel') as HTMLElement
    expect(inputEl).toBeTruthy()
    expect(document.body.contains(inputEl)).toBe(true)
    expect(shell.inputUnmounts).toBe(0)

    const returnBtn = host.querySelector('.mono-return-btn') as HTMLElement
    returnBtn.click()
    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeNull())
    expect(host.querySelector('.app-titlebar')).toBeTruthy()
    expect(shell.inputUnmounts).toBe(0)
    expect(shell.inputMounts).toBe(1)
    expect(document.body.contains(host.querySelector('.input-panel'))).toBe(true)
    expect(shell.focusEditorCalls).toBeGreaterThan(0)
  })

  it('minimizes from the mono bar without leaving mono or touching mode IPC', async () => {
    localStorage.setItem('app-start-compact', 'true')
    localStorage.setItem('app-compact-view', 'mono')
    tauri.invoke.mockImplementation((cmd: string) =>
      cmd === 'get_all_app_settings'
        ? Promise.resolve(makeSettingsDto({ compactView: 'mono' }))
        : defaultInvoke(cmd),
    )
    const host = await mountShell()
    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeTruthy())

    const minimizeSpy = tauri.getCurrentWindow().minimize
    const inputEl = host.querySelector('.input-panel') as HTMLElement
    expect(inputEl).toBeTruthy()

    const minBtn = host.querySelector('.mono-minimize-btn') as HTMLButtonElement
    expect(minBtn).toBeTruthy()
    minBtn.click()
    await vi.waitFor(() => expect(minimizeSpy).toHaveBeenCalledTimes(1))

    expect(host.querySelector('.mono-mode-bar')).toBeTruthy()
    expect(host.querySelector('.app-titlebar')).toBeNull()
    expect(shell.inputMounts).toBe(1)
    expect(shell.inputUnmounts).toBe(0)
    expect(document.body.contains(host.querySelector('.input-panel'))).toBe(true)

    expect(tauri.invoke).not.toHaveBeenCalledWith('set_main_bounds')
    expect(tauri.invoke).not.toHaveBeenCalledWith('remove_main_bounds')
    expect(tauri.invoke).not.toHaveBeenCalledWith('resize_main_window', expect.anything())
    expect(tauri.invoke).not.toHaveBeenCalledWith('set_main_compact_view', expect.anything())
  })

  it('remembers mono across an ordinary round trip using the local hotkey', async () => {
    tauri.invoke.mockImplementation((cmd: string) =>
      cmd === 'get_all_app_settings'
        ? Promise.resolve(makeSettingsDto({ compactView: 'mono' }))
        : defaultInvoke(cmd),
    )
    const host = await mountShell()

    const toggle = host.querySelector('.minimal-mode-toggle') as HTMLElement
    toggle.click()
    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeTruthy())
    expect(host.querySelector('.app-titlebar')).toBeNull()

    await sleep(550)

    document.body.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'm', code: 'KeyM', ctrlKey: true, bubbles: true }),
    )
    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeNull())
    expect(host.querySelector('.app-titlebar')).toBeTruthy()

    await sleep(550)

    toggle.click()
    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeTruthy())
    expect(host.querySelector('.app-titlebar')).toBeNull()
  })

  it('shows the enabled corner restore button in mono and exits directly to ordinary', async () => {
    localStorage.setItem('app-start-compact', 'true')
    localStorage.setItem('app-compact-view', 'mono')
    tauri.invoke.mockImplementation((cmd: string) =>
      cmd === 'get_all_app_settings'
        ? Promise.resolve(makeSettingsDto({ compactView: 'mono' }))
        : defaultInvoke(cmd),
    )
    const host = await mountShell()

    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeTruthy())
    expect(host.querySelector('.app-titlebar')).toBeNull()
    expect(host.querySelector('.stub-sidebar')).toBeNull()

    const corner = host.querySelector('.minimal-mode-toggle') as HTMLButtonElement
    expect(corner).toBeTruthy()
    expect(getComputedStyle(corner).display).not.toBe('none')
    expect(corner.disabled).toBe(false)
    expect(corner.getAttribute('aria-label')).toBe('Restore window')

    corner.click()
    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeNull())
    expect(host.querySelector('.app-titlebar')).toBeTruthy()
    expect(host.querySelector('.stub-sidebar')).toBeTruthy()
    expect(tauri.invoke).toHaveBeenCalledWith('remove_main_bounds')
    expect(tauri.invoke).toHaveBeenCalledWith('resize_main_window', { width: 800, height: 630 })

    await sleep(550)

    const cornerAfterExit = host.querySelector('.minimal-mode-toggle') as HTMLButtonElement
    expect(cornerAfterExit).toBeTruthy()
    cornerAfterExit.click()
    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeTruthy())
    expect(host.querySelector('.app-titlebar')).toBeNull()
    expect(host.querySelector('.stub-sidebar')).toBeNull()
  })

  it('rolls back the remembered view when persistence fails', async () => {
    localStorage.setItem('app-start-compact', 'true')
    localStorage.setItem('app-compact-view', 'compact')
    tauri.invoke.mockImplementation((cmd: string) =>
      cmd === 'set_main_compact_view' ? Promise.reject(new Error('disk full')) : defaultInvoke(cmd),
    )
    const host = await mountShell()

    expect(host.querySelector('.mono-mode-bar')).toBeNull()

    const enableMono = host.querySelector('.enable-mono-btn') as HTMLElement
    enableMono.click()
    await vi.waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('set_main_compact_view', { view: 'mono' }))
    await flushPromises()

    expect(host.querySelector('.mono-mode-bar')).toBeNull()
    expect(host.querySelector('.app-titlebar')).toBeTruthy()
    expect(localStorage.getItem('app-compact-view')).toBe('compact')
  })

  it('rejects a stale initial settings snapshot and consumes the reload when idle', async () => {
    let resolveInitial!: (dto: unknown) => void
    let settingsCalls = 0
    tauri.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_all_app_settings') {
        settingsCalls += 1
        if (settingsCalls === 1) {
          return new Promise((resolve) => {
            resolveInitial = resolve
          })
        }
        return Promise.resolve(makeSettingsDto({ compactView: 'mono', compactWidth: 600, compactHeight: 500 }))
      }
      return defaultInvoke(cmd)
    })

    const host = mountShellRaw()
    await flushPromises()

    expect(host.querySelector('.app-titlebar')).toBeTruthy()

    const toggle = host.querySelector('.minimal-mode-toggle') as HTMLElement
    toggle.click()
    await vi.waitFor(() => expect(host.querySelector('.stub-sidebar')).toBeNull())

    resolveInitial(makeSettingsDto({ compactView: 'compact', compactWidth: 111, compactHeight: 111 }))
    await flushPromises()
    expect(localStorage.getItem('app-compact-view')).toBeNull()

    await vi.waitFor(() => expect(settingsCalls).toBe(2), { timeout: 3000 })
    await vi.waitFor(() => expect(localStorage.getItem('app-compact-view')).toBe('mono'), { timeout: 3000 })

    expect(host.querySelector('.mono-mode-bar')).toBeTruthy()
  })
})

function countInvokes(cmd: string): number {
  return tauri.invoke.mock.calls.filter(([c]) => c === cmd).length
}

function pressCtrl(
  code: string,
  target?: EventTarget,
  opts?: { repeat?: boolean },
): { defaultPrevented: boolean } {
  let prevented = false
  const event = new KeyboardEvent('keydown', {
    key: code.replace(/^Key/, '').toLowerCase(),
    code,
    ctrlKey: true,
    repeat: opts?.repeat ?? false,
    bubbles: true,
    cancelable: true,
  })
  Object.defineProperty(event, 'defaultPrevented', { get: () => prevented })
  event.preventDefault = () => {
    prevented = true
  }
  ;(target ?? document).dispatchEvent(event)
  return { defaultPrevented: prevented }
}

function pressCtrlP(target?: EventTarget, opts?: { repeat?: boolean }): { defaultPrevented: boolean } {
  return pressCtrl('KeyP', target, opts)
}

describe('output controls placement and shortcut lifetime (ROADMAP-143)', () => {
  it('moves the group between titlebar and editor placement without duplicating state', async () => {
    localStorage.setItem('app-output-controls-placement', 'editor')
    const host = await mountShell()
    // No mic device is configured in this fixture: per the refresh contract
    // there is nothing to confirm, so no device IPC runs here at all.
    await flushPromises()
    expect(tauri.invoke).not.toHaveBeenCalledWith('get_virtual_mic_devices')

    // Editor placement: no group in the titlebar (the editor action bar owns
    // the view; the real InputPanel is stubbed in this harness).
    expect(host.querySelector('.app-titlebar .output-controls-group')).toBeNull()

    // Switching to the titlebar reactively mounts exactly one group view.
    setOutputControlsPlacement('titlebar')
    await vi.waitFor(() =>
      expect(host.querySelectorAll('.app-titlebar .output-controls-group')).toHaveLength(1),
    )
    expect(host.querySelectorAll('.output-controls-group')).toHaveLength(1)

    // Back to editor: the titlebar group disappears again.
    setOutputControlsPlacement('editor')
    await vi.waitFor(() =>
      expect(host.querySelector('.app-titlebar .output-controls-group')).toBeNull(),
    )
  })

  it('assigned Ctrl+P toggles once, is consumed, and survives normal → mono → compact', async () => {
    localStorage.setItem('app-output-controls-placement', 'titlebar')
    // The backend persists the compact view: the mock mirrors it so a toggle
    // reload cannot apply a stale view over a mode switch (that race is
    // impossible with the real backend).
    let compactView: 'compact' | 'mono' = 'compact'
    tauri.invoke.mockImplementation((cmd: string, args?: { view?: string }) => {
      if (cmd === 'get_virtual_mic_devices') {
        return Promise.resolve([{ id: 'CABLE Input', name: 'CABLE Input', is_default: false }])
      }
      if (cmd === 'get_all_app_settings') {
        return Promise.resolve(
          makeSettingsDto({
            compactView,
            micDevice: 'CABLE Input',
            micHotkey: { modifiers: ['ctrl'], key: 'P' },
          }),
        )
      }
      if (cmd === 'set_main_compact_view') {
        compactView = args?.view === 'mono' ? 'mono' : 'compact'
        return Promise.resolve(undefined)
      }
      return defaultInvoke(cmd)
    })
    const host = await mountShell()
    await vi.waitFor(() => expect(countInvokes('get_virtual_mic_devices')).toBeGreaterThan(0))
    await flushPromises()

    // Normal mode: the assigned print shortcut toggles the mic exactly once
    // and is consumed (no browser print, no editor dispatch).
    const trigger = host.querySelector('.output-toggles-btn')
    // Availability gate: the label flips to "on" only after the device list
    // confirms the configured mic.
    await vi.waitFor(() =>
      expect(trigger?.getAttribute('aria-label')).toMatch(/virtual microphone on/i),
    )
    const pressed = pressCtrlP()
    await vi.waitFor(() => expect(countInvokes('set_virtual_mic_enabled')).toBe(1))
    expect(pressed.defaultPrevented).toBe(true)
    expect(countInvokes('set_virtual_mic_enabled')).toBe(1)
    await flushPromises() // let the confirming reload settle before the next step

    // Repeat of a held key is consumed (no print) but does not toggle again.
    const repeated = pressCtrlP(undefined, { repeat: true })
    await flushPromises()
    expect(repeated.defaultPrevented).toBe(true)
    expect(countInvokes('set_virtual_mic_enabled')).toBe(1)

    // Compact: keep working with the titlebar visible.
    ;(host.querySelector('.minimal-mode-toggle') as HTMLElement).click()
    await vi.waitFor(() => expect(host.querySelector('.stub-sidebar')).toBeNull())
    await flushPromises()
    const compactPress = pressCtrlP()
    await vi.waitFor(() => expect(countInvokes('set_virtual_mic_enabled')).toBe(2))
    expect(compactPress.defaultPrevented).toBe(true)
    await flushPromises()

    // Mono: the group moves to the micro bar, while the root listener
    // still consumes the shortcut and toggles exactly once. The 500 ms
    // post-native guard must be respected before the next mode switch.
    await sleep(550)
    ;(host.querySelector('.enable-mono-btn') as HTMLElement).click()
    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeTruthy())
    expect(host.querySelector('.app-titlebar')).toBeNull()
    expect(host.querySelectorAll('.output-toggles-btn')).toHaveLength(1)
    const monoTrigger = host.querySelector<HTMLButtonElement>('.mono-mode-bar .micro .output-toggles-btn')!
    expect(monoTrigger).toBeTruthy()
    monoTrigger.click()
    await flushPromises()
    expect(host.querySelector('.mono-mode-bar .output-menu--down')).toBeTruthy()
    monoTrigger.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    setOutputControlsPlacement('hidden')
    await flushPromises()
    expect(host.querySelector('.output-toggles-btn')).toBeNull()
    setOutputControlsPlacement('editor')
    await flushPromises()
    expect(host.querySelector('.mono-mode-bar .output-toggles-btn')).toBeNull()
    setOutputControlsPlacement('titlebar')
    await flushPromises()
    expect(host.querySelectorAll('.mono-mode-bar .output-toggles-btn')).toHaveLength(1)
    const monoPress = pressCtrlP()
    await vi.waitFor(() => expect(countInvokes('set_virtual_mic_enabled')).toBe(3))
    expect(monoPress.defaultPrevented).toBe(true)
    await flushPromises()

    // Back to compact: the view returns, still a single action per press.
    ;(host.querySelector('.mono-return-btn') as HTMLElement).click()
    await vi.waitFor(() => expect(host.querySelector('.mono-mode-bar')).toBeNull())
    await flushPromises()
    const backPress = pressCtrlP()
    await vi.waitFor(() => expect(countInvokes('set_virtual_mic_enabled')).toBe(4))
    expect(backPress.defaultPrevented).toBe(true)
  })

  it('works with the focus inside a text input and stays a single action', async () => {
    localStorage.setItem('app-output-controls-placement', 'titlebar')
    tauri.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_virtual_mic_devices') {
        return Promise.resolve([{ id: 'CABLE Input', name: 'CABLE Input', is_default: false }])
      }
      if (cmd === 'get_all_app_settings') {
        return Promise.resolve(
          makeSettingsDto({
            micDevice: 'CABLE Input',
            micHotkey: { modifiers: ['ctrl'], key: 'P' },
          }),
        )
      }
      return defaultInvoke(cmd)
    })
    await mountShell()
    await vi.waitFor(() => expect(countInvokes('get_virtual_mic_devices')).toBeGreaterThan(0))
    await flushPromises()

    const input = document.createElement('input')
    document.body.appendChild(input)
    input.focus()
    const pressed = pressCtrlP(input)
    await vi.waitFor(() => expect(countInvokes('set_virtual_mic_enabled')).toBe(1))
    expect(pressed.defaultPrevented).toBe(true)
    input.remove()
    await flushPromises() // the toggle must settle before the next press

    // Same for an editor-like surface (CodeMirror renders a contenteditable):
    // the root capture listener does not depend on the focused element.
    const editorSurface = document.createElement('div')
    editorSurface.setAttribute('contenteditable', 'true')
    document.body.appendChild(editorSurface)
    editorSurface.focus()
    const editorPress = pressCtrlP(editorSurface)
    await vi.waitFor(() => expect(countInvokes('set_virtual_mic_enabled')).toBe(2))
    expect(editorPress.defaultPrevented).toBe(true)
    editorSurface.remove()
  })

  it('keeps the shortcut inert while a hotkey recording is active', async () => {
    localStorage.setItem('app-output-controls-placement', 'titlebar')
    tauri.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_virtual_mic_devices') {
        return Promise.resolve([{ id: 'CABLE Input', name: 'CABLE Input', is_default: false }])
      }
      if (cmd === 'get_all_app_settings') {
        return Promise.resolve(
          makeSettingsDto({
            micDevice: 'CABLE Input',
            micHotkey: { modifiers: ['ctrl'], key: 'P' },
          }),
        )
      }
      return defaultInvoke(cmd)
    })
    await mountShell()
    await vi.waitFor(() => expect(countInvokes('get_virtual_mic_devices')).toBeGreaterThan(0))
    await flushPromises()

    // The recorder owns the combination: no action, and the event stays
    // unconsumed so the recorder can capture it.
    setMainWindowOutputControlsRecording('hotkeys-panel', true)
    const recorded = pressCtrlP()
    await flushPromises()
    expect(countInvokes('set_virtual_mic_enabled')).toBe(0)
    expect(recorded.defaultPrevented).toBe(false)

    setMainWindowOutputControlsRecording('hotkeys-panel', false)
    const after = pressCtrlP()
    await vi.waitFor(() => expect(countInvokes('set_virtual_mic_enabled')).toBe(1))
    expect(after.defaultPrevented).toBe(true)
  })

  it('treats a matched shortcut on an unavailable output as a consumed no-op', async () => {
    localStorage.setItem('app-output-controls-placement', 'titlebar')
    tauri.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_virtual_mic_devices') {
        // The configured device is missing from the live list: unavailable.
        return Promise.resolve([{ id: 'Other Mic', name: 'Other Mic', is_default: false }])
      }
      if (cmd === 'get_all_app_settings') {
        return Promise.resolve(
          makeSettingsDto({
            micDevice: 'CABLE Input',
            micHotkey: { modifiers: ['ctrl'], key: 'P' },
          }),
        )
      }
      return defaultInvoke(cmd)
    })
    await mountShell()
    await vi.waitFor(() => expect(countInvokes('get_virtual_mic_devices')).toBeGreaterThan(0))
    await flushPromises()

    const pressed = pressCtrlP()
    await flushPromises()
    // Consumed (no browser print) but no toggle: the unavailable output
    // never falls back to the browser command.
    expect(pressed.defaultPrevented).toBe(true)
    expect(countInvokes('set_virtual_mic_enabled')).toBe(0)
  })

  it('rebuilds the controller around the new context after a final unmount', async () => {
    localStorage.setItem('app-output-controls-placement', 'titlebar')
    // Shell A: Ctrl+K assigned to the mic, 'CABLE Input' confirmed available.
    tauri.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_virtual_mic_devices') {
        return Promise.resolve([{ id: 'CABLE Input', name: 'CABLE Input', is_default: false }])
      }
      if (cmd === 'get_all_app_settings') {
        return Promise.resolve(
          makeSettingsDto({
            micDevice: 'CABLE Input',
            micHotkey: { modifiers: ['ctrl'], key: 'K' },
          }),
        )
      }
      return defaultInvoke(cmd)
    })
    await mountShell()
    await vi.waitFor(() => expect(countInvokes('get_virtual_mic_devices')).toBeGreaterThan(0))
    await flushPromises()

    const firstPress = pressCtrl('KeyK')
    await vi.waitFor(() => expect(countInvokes('set_virtual_mic_enabled')).toBe(1))
    expect(firstPress.defaultPrevented).toBe(true)
    await flushPromises()

    // Final unmount of the first shell: listener, reactive scope and the
    // singleton must be released.
    apps.splice(0).forEach((app) => app.unmount())
    const afterUnmount = pressCtrl('KeyK')
    await flushPromises()
    expect(countInvokes('set_virtual_mic_enabled')).toBe(1)
    expect(afterUnmount.defaultPrevented).toBe(false)

    // Shell B: a different settings ref (different availability and
    // binding): the same physical mic is configured but the NEW device list
    // no longer contains it, and the shortcut moved to Ctrl+J.
    tauri.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_virtual_mic_devices') {
        return Promise.resolve([{ id: 'Live Mic', name: 'Live Mic', is_default: false }])
      }
      if (cmd === 'get_all_app_settings') {
        return Promise.resolve(
          makeSettingsDto({
            micDevice: 'CABLE Input',
            micHotkey: { modifiers: ['ctrl'], key: 'J' },
          }),
        )
      }
      return defaultInvoke(cmd)
    })
    const host = await mountShell()
    await vi.waitFor(() =>
      expect(countInvokes('get_virtual_mic_devices')).toBeGreaterThan(1),
    )
    await flushPromises()

    // The stale binding from the disposed shell neither fires nor consumes.
    const stalePress = pressCtrl('KeyK')
    await flushPromises()
    expect(countInvokes('set_virtual_mic_enabled')).toBe(1)
    expect(stalePress.defaultPrevented).toBe(false)

    // The new binding answers through the NEW context. Availability comes
    // from the new device check ('CABLE Input' is not in the new list), so
    // the press stays a consumed no-op — a leaked stale state would toggle.
    const newPress = pressCtrl('KeyJ')
    await flushPromises()
    expect(newPress.defaultPrevented).toBe(true)
    expect(countInvokes('set_virtual_mic_enabled')).toBe(1)

    // Shell B's own state/reload: reopen the popover view — its open event
    // refreshes availability through the NEW controller. The refreshed list
    // now contains the configured device, so the same press toggles exactly
    // once (a leaked stale listener would double the count).
    const devicesBefore = countInvokes('get_virtual_mic_devices')
    tauri.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_virtual_mic_devices') {
        return Promise.resolve([
          { id: 'CABLE Input', name: 'CABLE Input', is_default: false },
          { id: 'Live Mic', name: 'Live Mic', is_default: false },
        ])
      }
      if (cmd === 'get_all_app_settings') {
        return Promise.resolve(
          makeSettingsDto({
            micDevice: 'CABLE Input',
            micHotkey: { modifiers: ['ctrl'], key: 'J' },
          }),
        )
      }
      return defaultInvoke(cmd)
    })
    const trigger = host.querySelector('.output-toggles-btn') as HTMLButtonElement
    expect(trigger).toBeTruthy()
    trigger.click()
    await vi.waitFor(() =>
      expect(countInvokes('get_virtual_mic_devices')).toBeGreaterThan(devicesBefore),
    )
    await flushPromises()
    const availablePress = pressCtrl('KeyJ')
    await vi.waitFor(() => expect(countInvokes('set_virtual_mic_enabled')).toBe(2))
    expect(availablePress.defaultPrevented).toBe(true)
  })

  it('passes an unassigned or disabled shortcut through to the browser', async () => {
    localStorage.setItem('app-output-controls-placement', 'titlebar')
    tauri.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_virtual_mic_devices') {
        return Promise.resolve([{ id: 'CABLE Input', name: 'CABLE Input', is_default: false }])
      }
      if (cmd === 'get_all_app_settings') {
        return Promise.resolve(
          makeSettingsDto({
            micDevice: 'CABLE Input',
            micHotkey: { modifiers: ['ctrl'], key: 'P', enabled: false },
          }),
        )
      }
      return defaultInvoke(cmd)
    })
    await mountShell()
    await vi.waitFor(() => expect(countInvokes('get_virtual_mic_devices')).toBeGreaterThan(0))
    await flushPromises()

    // Disabled binding: the browser keeps its default print behaviour.
    const pressed = pressCtrlP()
    await flushPromises()
    expect(pressed.defaultPrevented).toBe(false)
    expect(countInvokes('set_virtual_mic_enabled')).toBe(0)
  })
})
