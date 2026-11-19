// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import OutputControlsGroup from './OutputControlsGroup.vue'
import {
  disposeMainWindowOutputControls,
  getMainWindowOutputControls,
  setMainWindowOutputControlsRecording,
} from '../../composables/useMainWindowOutputControls'
import { useAppSettings } from '../../composables/useAppSettings'
import { APP_SETTINGS_KEY, type AppSettingsDto } from '../../types/settings'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

const mockedInvoke = vi.mocked(invoke)

const devices = (ids: string[]) => ids.map((id) => ({ id, name: id, is_default: false }))

function createSettings(overrides?: {
  micDevice?: string | null
  micEnabled?: boolean
  vban?: Partial<AppSettingsDto['audio']['vban']>
  micHotkey?: { modifiers: string[]; key: string; enabled?: boolean } | null
  vbanHotkey?: { modifiers: string[]; key: string; enabled?: boolean } | null
  returnPrevious?: { modifiers: string[]; key: string }
}): AppSettingsDto {
  const micHotkey = overrides?.micHotkey ?? { modifiers: ['ctrl'], key: 'M' }
  return {
    audio: {
      output_format: 'default',
      speaker_device: null,
      speaker_enabled: true,
      speaker_volume: 100,
      virtual_mic_device: overrides?.micDevice ?? 'CABLE Input',
      virtual_mic_enabled: overrides?.micEnabled ?? true,
      virtual_mic_volume: 100,
      vban: {
        enabled: true,
        address: '192.168.1.50',
        port: 6980,
        stream_name: 'TTSBard',
        volume: 100,
        ping_interval: 30,
        ...overrides?.vban,
      },
    },
    hotkeys: {
      editor: {
        toggle_virtual_mic: micHotkey as never,
        toggle_vban: (overrides?.vbanHotkey ?? {
          modifiers: [],
          key: '',
        }) as never,
      },
      return_previous_window: (overrides?.returnPrevious ?? {
        modifiers: [],
        key: '',
      }) as never,
      toggle_minimal_mode: { modifiers: [], key: '' } as never,
    },
  } as unknown as AppSettingsDto
}

const apps: ReturnType<typeof createApp>[] = []
const settingsRef = ref<AppSettingsDto | null>(null)

// Realistic root owner (mirrors App.vue): the controller is created around
// the provided settings context in setup, the listener is attached on mount
// and the final unmount disposes it — exactly the lifecycle under test.
const RootOwner = defineComponent({
  props: { direction: { type: String as () => 'up' | 'down', default: 'down' } },
  setup(props) {
    const controls = getMainWindowOutputControls(useAppSettings())
    onMounted(() => controls.attachListener())
    onUnmounted(() => disposeMainWindowOutputControls())
    return () => h(OutputControlsGroup, { direction: props.direction, compact: true })
  },
})

function mountGroup() {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({ render: () => h(RootOwner) })
  app.provide(APP_SETTINGS_KEY, {
    settings: settingsRef,
    isLoading: ref(false),
    error: ref(null),
    reload: vi.fn(async () => {}),
  })
  apps.push(app)
  app.mount(host)
  return host
}

const keydown = (init: {
  code: string
  ctrlKey?: boolean
  shiftKey?: boolean
  altKey?: boolean
  repeat?: boolean
  isComposing?: boolean
}) => {
  const event = new KeyboardEvent('keydown', {
    code: init.code,
    ctrlKey: init.ctrlKey ?? false,
    shiftKey: init.shiftKey ?? false,
    altKey: init.altKey ?? false,
    repeat: init.repeat ?? false,
    isComposing: init.isComposing ?? false,
    bubbles: true,
    cancelable: true,
  })
  document.dispatchEvent(event)
  return event
}

beforeEach(() => {
  mockedInvoke.mockReset()
  mockedInvoke.mockImplementation(async (cmd: string) =>
    cmd === 'get_virtual_mic_devices' ? devices(['CABLE Input']) : undefined,
  )
  setMainWindowOutputControlsRecording('hotkeys-panel', false)
  settingsRef.value = createSettings()
})

afterEach(() => {
  for (const app of apps.splice(0)) app.unmount()
  document.body.innerHTML = ''
  setMainWindowOutputControlsRecording('hotkeys-panel', false)
})

describe('main-window output controls (ROADMAP-143 root lifetime)', () => {
  it('renders the popover group opening downward from the titlebar', async () => {
    const host = mountGroup()
    await nextTick()
    const group = host.querySelector('.output-controls-group')!
    expect(group).not.toBeNull()
    // The titlebar variant carries its own spacing/layering class; the editor
    // placement (direction up) must not.
    expect(group.classList.contains('titlebar-output-controls')).toBe(true)
    const trigger = host.querySelector<HTMLButtonElement>('.output-toggles-btn')!
    expect(trigger).not.toBeNull()
    trigger.click()
    await nextTick()
    const menu = host.querySelector('.output-menu')!
    expect(menu.classList.contains('output-menu--down')).toBe(true)

    // The editor placement renders the shared view without the titlebar
    // class: its popover keeps the base layer and no titlebar spacing.
    apps.splice(0).forEach((app) => app.unmount())
    const editorHost = document.createElement('div')
    document.body.appendChild(editorHost)
    const app = createApp({ render: () => h(RootOwner, { direction: 'up' }) })
    app.provide(APP_SETTINGS_KEY, {
      settings: settingsRef,
      isLoading: ref(false),
      error: ref(null),
      reload: vi.fn(async () => {}),
    })
    apps.push(app)
    app.mount(editorHost)
    await nextTick()
    expect(editorHost.querySelector('.output-controls-group')!.classList.contains('titlebar-output-controls')).toBe(false)
  })

  it('fires from a main-window keydown even when the focus sits in an input', async () => {
    const host = mountGroup()
    await nextTick()
    // The device list refresh (immediate) confirms the configured mic.
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('get_virtual_mic_devices')
    })
    expect(host.querySelector('.output-toggles-btn')).not.toBeNull()

    const input = document.createElement('input')
    document.body.appendChild(input)
    input.focus()
    keydown({ code: 'KeyM', ctrlKey: true })
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('set_virtual_mic_enabled', { enabled: false })
    })
  })

  it('ignores extra modifiers and disabled bindings', async () => {
    mountGroup()
    await nextTick()
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('get_virtual_mic_devices')
    })
    mockedInvoke.mockClear()

    keydown({ code: 'KeyM', ctrlKey: true, shiftKey: true })
    keydown({ code: 'KeyM' })
    await nextTick()
    expect(mockedInvoke).not.toHaveBeenCalled()

    settingsRef.value = createSettings({
      micHotkey: { modifiers: ['ctrl'], key: 'M', enabled: false },
    })
    await nextTick()
    const event = keydown({ code: 'KeyM', ctrlKey: true })
    await nextTick()
    expect(mockedInvoke).not.toHaveBeenCalled()
    // A disabled binding is not ours: the browser keeps its default.
    expect(event.defaultPrevented).toBe(false)
  })

  it('a matched event on an unavailable output is a consumed no-op, not a browser fallback', async () => {
    mockedInvoke.mockImplementation((cmd: string) =>
      cmd === 'get_virtual_mic_devices'
        ? new Promise(() => {}) // device list pending: unknown = unavailable
        : Promise.resolve(undefined),
    )
    mountGroup()
    await nextTick()
    const event = keydown({ code: 'KeyM', ctrlKey: true })
    await nextTick()
    expect(mockedInvoke).not.toHaveBeenCalledWith('set_virtual_mic_enabled', expect.anything())
    // Consumed: the browser default (print) must not replace the command.
    expect(event.defaultPrevented).toBe(true)

    // The refresh only replaces the list from its own response; the next
    // keydown stays a no-op until the configured device is confirmed.
    settingsRef.value = createSettings() // no-op poke: state contract covered in the composable tests
    const second = keydown({ code: 'KeyM', ctrlKey: true })
    await nextTick()
    expect(mockedInvoke).not.toHaveBeenCalledWith('set_virtual_mic_enabled', expect.anything())
    expect(second.defaultPrevented).toBe(true)
  })

  it('does not fire while a hotkey recording is active', async () => {
    mountGroup()
    await nextTick()
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('get_virtual_mic_devices')
    })
    mockedInvoke.mockClear()

    setMainWindowOutputControlsRecording('hotkeys-panel', true)
    const whileRecording = keydown({ code: 'KeyM', ctrlKey: true })
    await nextTick()
    expect(mockedInvoke).not.toHaveBeenCalled()
    // Not consumed during recording: the recorder owns the combination.
    expect(whileRecording.defaultPrevented).toBe(false)

    setMainWindowOutputControlsRecording('hotkeys-panel', false)
    keydown({ code: 'KeyM', ctrlKey: true })
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('set_virtual_mic_enabled', { enabled: false })
    })
  })

  it('a combination claimed by an existing main-window binding does not double-fire', async () => {
    // Legacy stored state where the same combo belongs to return_previous_window
    // and the vban toggle: the main-window binding keeps priority, no second action.
    settingsRef.value = createSettings({
      vbanHotkey: { modifiers: ['ctrl'], key: 'P' },
      returnPrevious: { modifiers: ['ctrl'], key: 'P' },
    })
    mountGroup()
    await nextTick()
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('get_virtual_mic_devices')
    })
    mockedInvoke.mockClear()

    keydown({ code: 'KeyP', ctrlKey: true })
    await nextTick()
    expect(mockedInvoke).not.toHaveBeenCalledWith('set_vban_enabled', expect.anything())
  })

  it('repeat, IME composition and already-handled events do not toggle either output', async () => {
    mountGroup()
    await nextTick()
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('get_virtual_mic_devices')
    })
    mockedInvoke.mockClear()

    // Held key (auto-repeat) — neither output toggles, but the combo stays
    // consumed: the browser default (print) must not replace the command.
    const heldMic = keydown({ code: 'KeyM', ctrlKey: true, repeat: true })
    const heldVban = keydown({ code: 'KeyN', ctrlKey: true, repeat: true })
    // IME composition — not a command: no toggle and NOT consumed either, so
    // the event reaches the editor composition untouched.
    const imeMic = keydown({ code: 'KeyM', ctrlKey: true, isComposing: true })
    const imeVban = keydown({ code: 'KeyN', ctrlKey: true, isComposing: true })
    await nextTick()
    expect(mockedInvoke).not.toHaveBeenCalled()
    expect(heldMic.defaultPrevented).toBe(true) // assigned combo: consumed even on repeat
    expect(heldVban.defaultPrevented).toBe(false) // Ctrl+N is unassigned here: passes through
    expect(imeMic.defaultPrevented).toBe(false) // IME: never consumed, assigned or not
    expect(imeVban.defaultPrevented).toBe(false)

    // An event another capture handler already handled (defaultPrevented) is
    // skipped — dispatched pre-consumed, the way it arrives after an earlier
    // capture listener got it first.
    const alreadyHandled = (code: string) => {
      const event = new KeyboardEvent('keydown', {
        code,
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      })
      event.preventDefault()
      document.dispatchEvent(event)
    }
    alreadyHandled('KeyM')
    alreadyHandled('KeyN')
    await nextTick()
    expect(mockedInvoke).not.toHaveBeenCalled()

    // A clean keydown still reaches both actions (mic via Ctrl+M; a VBAN
    // binding assigned separately fires its own action).
    settingsRef.value = createSettings({
      vbanHotkey: { modifiers: ['ctrl'], key: 'N' },
    })
    await nextTick()
    keydown({ code: 'KeyM', ctrlKey: true })
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('set_virtual_mic_enabled', { enabled: false })
    })
    keydown({ code: 'KeyN', ctrlKey: true })
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('set_vban_enabled', { enabled: false })
    })
  })

  it('the final unmount disposes the controller: the listener is released', async () => {
    const host = mountGroup()
    await nextTick()
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('get_virtual_mic_devices')
    })
    mockedInvoke.mockClear()
    apps.splice(0).forEach((app) => app.unmount())
    keydown({ code: 'KeyM', ctrlKey: true })
    await nextTick()
    expect(mockedInvoke).not.toHaveBeenCalled()
    expect(host.querySelector('.output-toggles-btn')).toBeNull()
  })

  it('a remounted shell builds a fresh controller around its own context', async () => {
    // First shell: the mic hotkey fires through its own context.
    mountGroup()
    await nextTick()
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('get_virtual_mic_devices')
    })
    mockedInvoke.mockClear()
    apps.splice(0).forEach((app) => app.unmount())

    // Second shell: a new context (fresh reload/isLoading) whose settings
    // differ — the discarded controller must not answer with stale state.
    settingsRef.value = createSettings({
      micHotkey: { modifiers: ['ctrl'], key: 'K' },
      vbanHotkey: { modifiers: ['ctrl'], key: 'N' },
    })
    const reload = vi.fn(async () => {})
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp({ render: () => h(RootOwner) })
    app.provide(APP_SETTINGS_KEY, {
      settings: settingsRef,
      isLoading: ref(false),
      error: ref(null),
      reload,
    })
    apps.push(app)
    app.mount(host)

    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('get_virtual_mic_devices')
    })
    mockedInvoke.mockClear()
    // The old Ctrl+M binding is gone with the disposed controller.
    const oldBinding = keydown({ code: 'KeyM', ctrlKey: true })
    await nextTick()
    expect(mockedInvoke).not.toHaveBeenCalled()
    expect(oldBinding.defaultPrevented).toBe(false)

    // The new bindings work exactly once each, through the new context.
    keydown({ code: 'KeyK', ctrlKey: true })
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('set_virtual_mic_enabled', { enabled: false })
    })
    expect(mockedInvoke).toHaveBeenCalledTimes(1)
    keydown({ code: 'KeyN', ctrlKey: true })
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('set_vban_enabled', { enabled: false })
    })
    expect(mockedInvoke).toHaveBeenCalledTimes(2)
    // The toggles reload the context they were built around — the new one.
    expect(reload).toHaveBeenCalled()
  })
})
