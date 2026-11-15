// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, ref } from 'vue'
import AudioPanel from './AudioPanel.vue'
import { invoke } from '@tauri-apps/api/core'
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

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))

const savedVban: VbanSettingsDto = {
  enabled: true,
  address: '192.168.1.50',
  port: 6980,
  stream_name: 'TTSBard',
  volume: 100,
}

function createSettings(vbanEnabled: boolean): AppSettingsDto {
  return {
    audio: {
      output_format: 'default',
      speaker_device: null,
      speaker_enabled: true,
      speaker_volume: 80,
      virtual_mic_device: null,
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
  status?: VbanStatusDto
}

async function mountPanel({ vbanEnabled = false, status = { state: 'disabled', error: null } }: MountOptions = {}) {
  container = document.createElement('div')
  document.body.appendChild(container)

  const settingsRef = ref<AppSettingsDto | null>(createSettings(vbanEnabled))
  const mockedInvoke = vi.mocked(invoke)
  let currentStatus: VbanStatusDto = { ...status }

  mockedInvoke.mockImplementation(async (cmd) => {
    switch (cmd) {
      case 'get_vban_status':
        return { ...currentStatus }
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
    render() {
      return h(AudioPanel)
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
    setStatus(next: VbanStatusDto) {
      currentStatus = { ...next }
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
})

describe('AudioPanel VBAN tab status', () => {
  it('shows the production status only for a saved enabled output', async () => {
    const panel = await mountPanel({ vbanEnabled: false, status: { state: 'idle', error: null } })
    try {
      const root = panel.container!
      expect(getVbanTabStatus(root)).toBeNull()
      expect(getVbanTab(root).getAttribute('aria-label')).toBe(t('audio.tabs.vban'))
      expect(getVbanTab(root).getAttribute('title')).toBeNull()

      // The production indicator follows the shared durable settings snapshot.
      const toggle = root.querySelector<HTMLInputElement>('.vban-card .toggle-switch input')
      if (!toggle) throw new Error('VBAN toggle not found')
      toggle.click()
      await flushAsync()
      expect(getVbanTabStatus(root)).toBeNull()
      toggle.click()
      await flushAsync()

      await panel.setVbanEnabled(true)
      const status = getVbanTabStatus(root)
      expect(status?.classList.contains('vban-tab-dot--idle')).toBe(true)
      expect(getVbanTab(root).textContent?.trim()).toBe(t('audio.tabs.vban'))
      expect(getVbanTab(root).getAttribute('aria-label'))
        .toBe(`${t('audio.tabs.vban')} — ${t('audio.vban.status.idle')}`)
      expect(getVbanTab(root).getAttribute('title')).toBe(t('audio.vban.status.idle'))

      await panel.setVbanEnabled(false)
      expect(getVbanTabStatus(root)).toBeNull()
    } finally {
      await panel.unmount()
    }
  })

  it('keeps the status current after a tab switch and after Save', async () => {
    const panel = await mountPanel({ vbanEnabled: true, status: { state: 'idle', error: null } })
    try {
      const root = panel.container!

      // Switching to VBAN refreshes the production status immediately.
      panel.setStatus({ state: 'sending', error: null })
      getTabs(root)[1].click()
      await flushAsync()
      getTabs(root)[2].click()
      await flushAsync()
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--sending')).toBe(true)
      expect(getVbanTab(root).getAttribute('aria-label')).toContain(t('audio.vban.status.sending'))

      // A dirty draft is marked in the accessible name alongside the status.
      panel.setStatus({ state: 'idle', error: null })
      const volume = root.querySelector<HTMLInputElement>('#vban-stream-name')
      if (!volume) throw new Error('VBAN volume not found')
      volume.value = 'Changed'
      volume.dispatchEvent(new Event('input', { bubbles: true }))
      await flushAsync()
      expect(getVbanTab(root).getAttribute('aria-label'))
        .toContain(t('audio.tabs.vban.dirty'))

      getVbanSaveButton(root).click()
      await flushAsync()
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--idle')).toBe(true)
      expect(getVbanTab(root).getAttribute('aria-label'))
        .toBe(`${t('audio.tabs.vban')} — ${t('audio.vban.status.idle')}`)
    } finally {
      await panel.unmount()
    }
  })

  it('shows the error state in the tab and keeps the backend detail in the body', async () => {
    const panel = await mountPanel({
      vbanEnabled: true,
      status: { state: 'error', error: 'socket send failed: network unreachable' },
    })
    try {
      const root = panel.container!
      expect(getVbanTabStatus(root)?.classList.contains('vban-tab-dot--error')).toBe(true)
      expect(getVbanTab(root).getAttribute('aria-label')).toContain(t('audio.vban.status.error'))

      // The duplicate status line is removed; only the diagnostic text stays.
      expect(root.querySelector('.vban-status')).toBeNull()
      const detail = root.querySelector<HTMLElement>('.vban-status-error')
      expect(detail?.textContent?.trim()).toBe('socket send failed: network unreachable')
      expect(detail?.getAttribute('role')).toBe('status')
    } finally {
      await panel.unmount()
    }
  })
})
