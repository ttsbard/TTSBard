// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, ref } from 'vue'
import AudioPanel from './AudioPanel.vue'
import { invoke } from '@tauri-apps/api/core'
import { useVbanHealthMonitor } from '../composables/useVbanHealth'
import { t } from '../i18n'
import {
  APP_SETTINGS_KEY,
  type AppSettingsDto,
  type VbanSettingsDto,
  type VbanStatusDto,
} from '../types/settings'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

const eventMocks = vi.hoisted(() => ({
  listenCallbacks: new Map<string, (event: { payload: unknown }) => void>(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, callback: (event: { payload: unknown }) => void) => {
    eventMocks.listenCallbacks.set(event, callback)
    return Promise.resolve(() => {
      eventMocks.listenCallbacks.delete(event)
    })
  }),
}))

const savedVban: VbanSettingsDto = {
  enabled: true,
  address: '192.168.1.50',
  port: 6980,
  stream_name: 'TTSBard',
  volume: 100,
  ping_interval: 30,
}

function createSettings(vbanEnabled: boolean): AppSettingsDto {
  return {
    audio: {
      output_format: 'default',
      speaker_device: null,
      speaker_enabled: true,
      speaker_volume: 80,
      virtual_mic_device: null,
      virtual_mic_enabled: true,
      virtual_mic_volume: 100,
      vban: { ...savedVban, enabled: vbanEnabled },
    },
    audio_effects: {
      enabled: false,
      pitch: 0,
      speed: 0,
      volume: 100,
      enhance_enabled: false,
      enhance_atten_db: 12,
      formant_preserved: true,
      boundary_cleanup_enabled: true,
    },
    dsp: {
      eq: {
        enabled: false,
        low_cut_enabled: false,
        low_cut_hz: 80,
        low_cut_slope_db: 12,
        bands: [
          { enabled: false, frequency_hz: 2500, gain_db: 0, q: 0.7 },
          { enabled: false, frequency_hz: 2500, gain_db: 0, q: 0.7 },
          { enabled: false, frequency_hz: 2500, gain_db: 0, q: 0.7 },
        ],
        high_shelf_enabled: false,
        high_shelf_hz: 8000,
        high_shelf_gain_db: 0,
      },
      compressor: {
        enabled: false,
        threshold_db: -18,
        ratio: 2,
        attack_ms: 8,
        release_ms: 120,
        knee_db: 6,
        makeup_db: 0,
      },
      limiter: { enabled: false, ceiling_db: -1, release_ms: 50 },
    },
  } as unknown as AppSettingsDto
}

async function flushAsync() {
  for (let i = 0; i < 10; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

let appInstance: ReturnType<typeof createApp> | null = null
let container: HTMLDivElement | null = null

interface MountOptions {
  vbanEnabled?: boolean
  /** Snapshot served by `get_vban_health` (boot state of the monitor). */
  healthSnapshot?: unknown
}

// The health listener lives in App; the host mirrors that so the panel is
// mounted with the global monitor running, exactly like in production.
async function mountPanel({ vbanEnabled = false, healthSnapshot = { state: 'unknown', reason: null } }: MountOptions = {}) {
  container = document.createElement('div')
  document.body.appendChild(container)

  const settingsRef = ref<AppSettingsDto | null>(createSettings(vbanEnabled))
  const mockedInvoke = vi.mocked(invoke)

  mockedInvoke.mockImplementation(async (cmd) => {
    switch (cmd) {
      case 'get_vban_health':
        return healthSnapshot
      case 'get_vban_status':
        return { state: 'disabled', error: null } satisfies VbanStatusDto
      case 'get_output_devices':
      case 'get_virtual_mic_devices':
        return []
      case 'get_audio_output_format_pending':
        return false
      default:
        return undefined
    }
  })

  appInstance = createApp({
    setup() {
      useVbanHealthMonitor()
      return () => h(AudioPanel)
    },
  })

  appInstance.provide(APP_SETTINGS_KEY, {
    settings: settingsRef,
    isLoading: ref(false),
    error: ref(null),
    reload: vi.fn(async () => {}),
  })

  appInstance.mount(container)
  await flushAsync()

  return {
    container,
    settingsRef,
    mockedInvoke,
    emitHealth(payload: unknown) {
      eventMocks.listenCallbacks.get('vban-health-changed')?.({ payload })
    },
    async setVbanEnabled(enabled: boolean) {
      settingsRef.value = createSettings(enabled)
      await flushAsync()
    },
    async unmount() {
      if (appInstance) {
        appInstance.unmount()
        appInstance = null
      }
      if (container) {
        container.remove()
        container = null
      }
      await flushAsync()
    },
  }
}

function getTabs(root: HTMLElement): HTMLButtonElement[] {
  return Array.from(root.querySelectorAll<HTMLButtonElement>('.audio-tabs .ui-tab'))
}

function getVbanTab(root: HTMLElement): HTMLButtonElement {
  const tab = getTabs(root)[2]
  if (!tab) throw new Error('VBAN tab not found')
  return tab
}

function getVbanTabStatus(root: HTMLElement): HTMLElement | null {
  return root.querySelector<HTMLElement>('.vban-tab-dot')
}

function getVbanSaveButton(root: HTMLElement): HTMLButtonElement {
  const button = Array.from(root.querySelectorAll<HTMLButtonElement>('.vban-settings .save-section button'))
    .find((candidate) => candidate.textContent?.includes('Save'))
  if (!button) throw new Error('VBAN Save button not found')
  return button
}

beforeEach(() => {
  vi.clearAllMocks()
  eventMocks.listenCallbacks.clear()
})

afterEach(async () => {
  if (appInstance) {
    appInstance.unmount()
    appInstance = null
  }
  if (container) {
    container.remove()
    container = null
  }
  document.body.innerHTML = ''
  vi.clearAllMocks()
  eventMocks.listenCallbacks.clear()
})

describe('AudioPanel VBAN tab availability indicator', () => {
  it('shows the availability dot only for a saved enabled output', async () => {
    const panel = await mountPanel({ vbanEnabled: false })
    try {
      const root = panel.container!
      expect(getVbanTabStatus(root)).toBeNull()
      expect(getVbanTab(root).getAttribute('aria-label')).toBe(t('audio.tabs.vban'))
      expect(getVbanTab(root).getAttribute('title')).toBeNull()

      // A local toggle of the draft is not a saved change: still hidden.
      const toggle = root.querySelector<HTMLInputElement>('.vban-card .toggle-switch input')
      if (!toggle) throw new Error('VBAN toggle not found')
      toggle.click()
      await flushAsync()
      expect(getVbanTabStatus(root)).toBeNull()
      toggle.click()
      await flushAsync()

      // The indicator follows the shared durable settings snapshot.
      await panel.setVbanEnabled(true)
      const status = getVbanTabStatus(root)
      expect(status?.classList.contains('vban-tab-dot--unknown')).toBe(true)
      expect(getVbanTab(root).textContent?.trim()).toBe(t('audio.tabs.vban'))
      expect(getVbanTab(root).getAttribute('aria-label'))
        .toBe(`${t('audio.tabs.vban')} — ${t('audio.vban.health.unknown')}`)
      expect(getVbanTab(root).getAttribute('title')).toBe(t('audio.vban.health.unknown'))

      await panel.setVbanEnabled(false)
      expect(getVbanTabStatus(root)).toBeNull()
    } finally {
      await panel.unmount()
    }
  })

  it('renders the boot snapshot verdicts: grey unknown, green alive, red failed', async () => {
    const panel = await mountPanel({
      vbanEnabled: true,
      healthSnapshot: { state: 'failed', reason: 'no_reply' },
    })
    try {
      const root = panel.container!
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--failed')).toBe(true)
      expect(getVbanTab(root).getAttribute('title'))
        .toBe(t('audio.vban.health.failed.no_reply'))
      expect(getVbanTab(root).getAttribute('aria-label'))
        .toBe(`${t('audio.tabs.vban')} — ${t('audio.vban.health.failed.no_reply')}`)
    } finally {
      await panel.unmount()
    }

    const alivePanel = await mountPanel({
      vbanEnabled: true,
      healthSnapshot: { state: 'alive', reason: null },
    })
    try {
      const root = alivePanel.container!
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--alive')).toBe(true)
      expect(getVbanTab(root).getAttribute('title')).toBe(t('audio.vban.health.alive'))
    } finally {
      await alivePanel.unmount()
    }
  })

  it('follows vban-health-changed events, including the local socket reason', async () => {
    const panel = await mountPanel({ vbanEnabled: true })
    try {
      const root = panel.container!

      panel.emitHealth({ state: 'alive', reason: null })
      await flushAsync()
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--alive')).toBe(true)
      expect(getVbanTab(root).getAttribute('title')).toBe(t('audio.vban.health.alive'))

      panel.emitHealth({ state: 'failed', reason: 'local_socket' })
      await flushAsync()
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--failed')).toBe(true)
      expect(getVbanTab(root).getAttribute('title'))
        .toBe(t('audio.vban.health.failed.local_socket'))

      panel.emitHealth({ state: 'unknown', reason: null })
      await flushAsync()
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--unknown')).toBe(true)
    } finally {
      await panel.unmount()
    }
  })

  it('never lets the boot snapshot overwrite a newer event verdict', async () => {
    container = document.createElement('div')
    document.body.appendChild(container)

    const settingsRef = ref<AppSettingsDto | null>(createSettings(true))
    const mockedInvoke = vi.mocked(invoke)
    let resolveHealth!: (value: unknown) => void
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === 'get_vban_health') {
        return new Promise((resolve) => { resolveHealth = resolve })
      }
      if (cmd === 'get_vban_status') {
        return { state: 'disabled', error: null } satisfies VbanStatusDto
      }
      if (cmd === 'get_output_devices' || cmd === 'get_virtual_mic_devices') return []
      if (cmd === 'get_audio_output_format_pending') return false
      return undefined
    })

    appInstance = createApp({
      setup() {
        useVbanHealthMonitor()
        return () => h(AudioPanel)
      },
    })
    appInstance.provide(APP_SETTINGS_KEY, {
      settings: settingsRef,
      isLoading: ref(false),
      error: ref(null),
      reload: vi.fn(async () => {}),
    })
    appInstance.mount(container)
    await flushAsync()
    expect(mockedInvoke).toHaveBeenCalledWith('get_vban_health')

    // The event lands while the snapshot is still in flight and must win.
    eventMocks.listenCallbacks.get('vban-health-changed')?.({
      payload: { state: 'alive', reason: null },
    })
    resolveHealth({ state: 'failed', reason: 'no_reply' })
    await flushAsync()

    const root = container!
    expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--alive')).toBe(true)
    expect(getVbanTab(root).getAttribute('title')).toBe(t('audio.vban.health.alive'))
  })

  it('keeps the verdict current across tab navigation and a panel remount', async () => {
    const panel = await mountPanel({ vbanEnabled: true })
    try {
      const root = panel.container!

      // An event that arrived while another tab was open is still shown.
      panel.emitHealth({ state: 'failed', reason: 'no_reply' })
      await flushAsync()
      getTabs(root)[0].click()
      await flushAsync()
      getTabs(root)[2].click()
      await flushAsync()
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--failed')).toBe(true)
      expect(getVbanTab(root).getAttribute('aria-label'))
        .toContain(t('audio.vban.health.failed.no_reply'))

      // A dirty draft is marked in the accessible name alongside the verdict.
      const streamName = root.querySelector<HTMLInputElement>('#vban-stream-name')
      if (!streamName) throw new Error('VBAN stream name not found')
      streamName.value = 'Changed'
      streamName.dispatchEvent(new Event('input', { bubbles: true }))
      await flushAsync()
      expect(getVbanTab(root).getAttribute('aria-label'))
        .toContain(t('audio.tabs.vban.dirty'))

      getVbanSaveButton(root).click()
      await flushAsync()
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--failed')).toBe(true)
      expect(getVbanTab(root).getAttribute('aria-label'))
        .toBe(`${t('audio.tabs.vban')} — ${t('audio.vban.health.failed.no_reply')}`)
    } finally {
      await panel.unmount()
    }

    // After closing and reopening the panel, the fresh monitor restores the
    // verdict from the backend snapshot.
    const reopened = await mountPanel({
      vbanEnabled: true,
      healthSnapshot: { state: 'alive', reason: null },
    })
    try {
      const root = reopened.container!
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--alive')).toBe(true)
    } finally {
      await reopened.unmount()
    }
  })
})
