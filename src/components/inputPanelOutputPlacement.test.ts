// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, computed, h, nextTick, ref, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import InputPanel from './InputPanel.vue'
import {
  disposeMainWindowOutputControls,
  resetOutputControlsPlacement,
  setOutputControlsPlacement,
} from '../composables/useMainWindowOutputControls'
import { MAIN_WINDOW_MODE_KEY, type MainWindowModeController } from '../composables/mainWindowMode'
import { APP_SETTINGS_KEY, type AppSettingsContext, type AppSettingsDto } from '../types/settings'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: vi.fn(),
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({
  save: vi.fn(async () => null),
}))

// The real InputPanel template/branches are under test; only the heavy leaf
// components are stubbed so the harness stays cheap. The stub EditorTabs
// exposes the pinned-incoming selector, which drives the real
// showIncomingTab branch of the action bar.
vi.mock('./editor/TtsEditor.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'StubTtsEditor',
      setup(_, { expose }) {
        expose({ focus: () => {} })
        return () => h('div', { class: 'stub-editor' })
      },
    }),
  }
})

vi.mock('./editor/EditorTabs.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'StubEditorTabs',
      emits: ['select-pinned'],
      setup(_, { emit }) {
        return () =>
          h('button', { class: 'stub-select-pinned', onClick: () => emit('select-pinned') }, 'incoming')
      },
    }),
  }
})

vi.mock('./editor/IncomingTextsTab.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'StubIncomingTextsTab',
      setup(_, { expose }) {
        expose({ focus: () => {} })
        return () => h('div', { class: 'stub-incoming' })
      },
    }),
  }
})

vi.mock('./editor/EditorMenu.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubEditorMenu', render: () => h('div', { class: 'stub-editor-menu' }) }) }
})

vi.mock('./editor/RouteSelector.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubRouteSelector', render: () => h('div', { class: 'stub-route-selector' }) }) }
})

vi.mock('./PhraseHistoryList.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'StubPhraseHistoryList', render: () => h('div', { class: 'stub-history' }) }) }
})

function makeSettingsDto(): AppSettingsDto {
  return {
    tts: {},
    ai: {},
    editor: { quick: 'disabled', default_route: 'everywhere' },
    ocr: { enabled: false },
    vrchat: { enabled: false },
    preprocessor: {},
    audio: {
      output_format: 'default',
      speaker_device: null,
      speaker_enabled: true,
      speaker_volume: 100,
      virtual_mic_device: 'CABLE Input',
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
    hotkeys: {
      editor: {
        toggle_virtual_mic: { modifiers: ['ctrl'], key: 'M' } as never,
        toggle_vban: { modifiers: [], key: '' } as never,
        submit_continue: { modifiers: ['ctrl'], key: 'Enter' } as never,
        accent_homographs: { modifiers: [], key: '' } as never,
      },
      return_previous_window: { modifiers: [], key: '' } as never,
      toggle_minimal_mode: { modifiers: [], key: '' } as never,
    },
  } as unknown as AppSettingsDto
}

interface ModeRefs {
  isMinimalMode: Ref<boolean>
  isMono: Ref<boolean>
}

/** Minimal MainWindowModeController around mutable refs the test can flip. */
function makeModeController(refs: ModeRefs): MainWindowModeController {
  const { isMinimalMode, isMono } = refs
  return {
    mode: computed(() => (isMono.value ? 'mono' : isMinimalMode.value ? 'compact' : 'ordinary')),
    rememberedView: computed(() => 'compact'),
    isMinimalMode: computed(() => isMinimalMode.value),
    isMono: computed(() => isMono.value),
    isCompact: computed(() => isMinimalMode.value && !isMono.value),
    busy: computed(() => false),
    suppressed: computed(() => false),
    confirmedCompactDimensions: computed(() => ({ width: 450, height: 400 })),
    toggleMinimalMode: async () => {
      isMinimalMode.value = !isMinimalMode.value
      return true
    },
    setCompactView: async () => true,
    setResizeFlush: () => () => {},
    acquireSuppressionLease: () => () => {},
    captureSnapshotRevision: () => 0,
    applySettingsSnapshot: () => false,
    invalidateCompactDimensions: () => {},
    confirmCompactDimensions: () => {},
    dispose: () => {},
  }
}

function defaultInvoke(cmd: string): Promise<unknown> {
  switch (cmd) {
    case 'get_tabs':
      return Promise.resolve({ active_id: 't1', tabs: [{ id: 't1', title: 'Tab 1', text: '' }] })
    case 'list_incoming_texts':
      return Promise.resolve([])
    case 'get_speech_queue_state':
      return Promise.resolve({})
    case 'get_incoming_settings':
      return Promise.resolve({})
    case 'load_preprocessor_data':
      return Promise.resolve({ replacements: {}, usernames: {} })
    case 'get_virtual_mic_devices':
      return Promise.resolve([{ id: 'CABLE Input', name: 'CABLE Input', is_default: false }])
    default:
      return Promise.resolve(undefined)
  }
}

const apps: ReturnType<typeof createApp>[] = []

function mountInputPanel(refs: ModeRefs): HTMLElement {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({ render: () => h(InputPanel) })
  app.config.warnHandler = () => {}
  app.provide(MAIN_WINDOW_MODE_KEY, makeModeController(refs))
  app.provide(APP_SETTINGS_KEY, {
    settings: ref(makeSettingsDto()),
    isLoading: ref(false),
    error: ref(null),
    reload: vi.fn(async () => {}),
  } satisfies AppSettingsContext)
  apps.push(app)
  app.mount(host)
  return host
}

async function flushAsync(): Promise<void> {
  for (let i = 0; i < 8; i += 1) {
    await Promise.resolve()
    await nextTick()
  }
}

beforeEach(() => {
  vi.mocked(invoke).mockImplementation((cmd: string) => defaultInvoke(cmd))
  // The resize controller subscribes to window bounds/scale events.
  vi.mocked(getCurrentWindow).mockReturnValue({
    innerSize: async () => ({ width: 800, height: 630 }),
    scaleFactor: async () => 1,
    onResized: async () => () => {},
    onCloseRequested: async () => () => {},
    minimize: async () => {},
    onFocusChanged: async () => () => {},
  } as never)
})

afterEach(async () => {
  for (const app of apps.splice(0)) app.unmount()
  disposeMainWindowOutputControls()
  document.body.innerHTML = ''
  localStorage.clear()
  resetOutputControlsPlacement()
  vi.clearAllMocks()
})

describe('InputPanel editor placement of the output controls (real branch)', () => {
  beforeEach(() => {
    setOutputControlsPlacement('editor')
  })

  it('hides output controls on first launch without a saved preference', async () => {
    localStorage.removeItem('app-output-controls-placement')
    resetOutputControlsPlacement()
    const host = mountInputPanel({ isMinimalMode: ref(false), isMono: ref(false) })
    await flushAsync()
    expect(host.querySelector('.editor-action-bar')).not.toBeNull()
    expect(host.querySelector('.output-controls-group')).toBeNull()
  })

  it('renders the group under the editor with an upward menu in normal mode', async () => {
    const host = mountInputPanel({ isMinimalMode: ref(false), isMono: ref(false) })
    await flushAsync()

    // The real action-bar branch: the group lives between the route selector
    // and the typing toggle, inside .editor-action-bar.
    const bar = host.querySelector('.editor-action-bar')!
    expect(bar).not.toBeNull()
    const group = bar.querySelector('.output-controls-group')
    expect(group).not.toBeNull()
    expect(group!.classList.contains('titlebar-output-controls')).toBe(false)

    // Up direction: the open menu is the up variant (unlike the titlebar's).
    const trigger = bar.querySelector<HTMLButtonElement>('.output-toggles-btn')!
    expect(trigger.disabled).toBe(false)
    trigger.click()
    await nextTick()
    expect(bar.querySelector('.output-menu--up')).not.toBeNull()
    expect(bar.querySelector('.output-menu--down')).toBeNull()
  })

  it('follows the compact flag from the mode controller', async () => {
    const isMinimalMode = ref(false)
    const host = mountInputPanel({ isMinimalMode, isMono: ref(false) })
    await flushAsync()

    // The group class falls through onto the OutputToggles root element.
    const root = host.querySelector('.editor-action-bar .output-toggles')!
    expect(root.classList.contains('compact')).toBe(false)

    // The real binding :compact="isMinimalMode" reacts to the controller.
    isMinimalMode.value = true
    await nextTick()
    expect(host.querySelector('.editor-action-bar .output-toggles')!.classList.contains('compact')).toBe(true)
  })

  it('hides the group with the action bar while the incoming tab is shown', async () => {
    const host = mountInputPanel({ isMinimalMode: ref(false), isMono: ref(false) })
    await flushAsync()
    expect(host.querySelector('.editor-action-bar .output-controls-group')).not.toBeNull()

    // The pinned incoming tab replaces the editor surface: the whole action
    // bar (and the group with it) leaves the DOM.
    ;(host.querySelector('.stub-select-pinned') as HTMLElement).click()
    await nextTick()
    expect(host.querySelector('.editor-action-bar')).toBeNull()
    expect(host.querySelector('.output-controls-group')).toBeNull()
  })

  it('hides the group in mono, keeps it via placement=editor only', async () => {
    const isMono = ref(false)
    const host = mountInputPanel({ isMinimalMode: ref(true), isMono })
    await flushAsync()
    expect(host.querySelector('.editor-action-bar .output-controls-group')).not.toBeNull()

    isMono.value = true
    await nextTick()
    expect(host.querySelector('.editor-action-bar')).toBeNull()
    expect(host.querySelector('.output-controls-group')).toBeNull()
  })

  it('renders nothing in the panel when the placement preference is the titlebar', async () => {
    setOutputControlsPlacement('titlebar')
    const host = mountInputPanel({ isMinimalMode: ref(false), isMono: ref(false) })
    await flushAsync()

    // The action bar still exists — only the group is gone; the titlebar
    // (owned by the root shell) is outside this panel.
    expect(host.querySelector('.editor-action-bar')).not.toBeNull()
    expect(host.querySelector('.output-controls-group')).toBeNull()
  })
})
