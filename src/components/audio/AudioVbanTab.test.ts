// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, ref } from 'vue'
import AudioVbanTab from './AudioVbanTab.vue'
import { invoke } from '@tauri-apps/api/core'
import {
  APP_SETTINGS_KEY,
  type AppSettingsDto,
  type AudioSettingsDto,
  type VbanSettingsDto,
  type VbanStatusDto,
} from '../../types/settings'
import { presentCommandError } from '../../ipc/commandError'
import { i18n, t } from '../../i18n'
import { withLocale } from '../../test-utils/i18n'
import ruCatalog from '../../../locales/ru.json'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))

const savedSection: VbanSettingsDto = {
  enabled: true,
  address: '192.168.1.50',
  port: 6980,
  stream_name: 'TTSBard',
  volume: 100,
}

function createAudio(vban: VbanSettingsDto = {
  enabled: false,
  address: '',
  port: 6980,
  stream_name: 'TTSBard',
  volume: 100,
}): AudioSettingsDto {
  return {
    output_format: 'default',
    speaker_device: null,
    speaker_enabled: true,
    speaker_volume: 80,
    virtual_mic_device: null,
    virtual_mic_volume: 100,
    vban,
  }
}

function createSettings(vban?: VbanSettingsDto): AppSettingsDto {
  return { audio: createAudio(vban) } as unknown as AppSettingsDto
}

async function flushAsync() {
  for (let i = 0; i < 10; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

interface MountOptions {
  settings?: AppSettingsDto | null
  active?: boolean
}

let appInstance: ReturnType<typeof createApp> | null = null
let container: HTMLDivElement | null = null

async function mountTab({ settings = createSettings(undefined), active = false }: MountOptions = {}) {
  container = document.createElement('div')
  document.body.appendChild(container)

  const settingsRef = ref<AppSettingsDto | null>(settings)
  const activeRef = ref(active)
  const dirtyChanges: boolean[] = []
  const statusChanges: VbanStatusDto[] = []
  const mockedInvoke = vi.mocked(invoke)
  // Well-formed baseline for every test (individual tests override it).
  mockedInvoke.mockImplementation(async (cmd) => {
    if (cmd === 'get_vban_status') return { state: 'disabled', error: null }
    return undefined
  })

  appInstance = createApp({
    render() {
      return h(AudioVbanTab, {
        active: activeRef.value,
        onDirtyChange: (dirty: boolean) => dirtyChanges.push(dirty),
        onStatusChange: (status: VbanStatusDto) => statusChanges.push(status),
      })
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
    activeRef,
    dirtyChanges,
    statusChanges,
    mockedInvoke,
    async setActive(value: boolean) {
      activeRef.value = value
      await nextTick()
      await flushAsync()
    },
    async updateSettings(vban?: VbanSettingsDto) {
      settingsRef.value = createSettings(vban)
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

function byId<T extends HTMLElement>(root: HTMLElement, id: string): T {
  const element = root.querySelector<T>(`#${id}`)
  if (!element) throw new Error(`#${id} not found`)
  return element
}

function getTestButton(root: HTMLElement): HTMLButtonElement {
  const buttons = Array.from(root.querySelectorAll<HTMLButtonElement>('.save-section button'))
  const button = buttons.find((candidate) => candidate.textContent?.trim() === t('audio.vban.test'))
  if (!button) throw new Error('Test button not found')
  return button
}

function getSaveButton(root: HTMLElement): HTMLButtonElement {
  const buttons = Array.from(root.querySelectorAll<HTMLButtonElement>('.save-section button'))
  const button = buttons.find((candidate) => candidate.textContent?.includes('Save'))
  if (!button) throw new Error('Save button not found')
  return button
}

function getAddressError(root: HTMLElement): string {
  const hint = root.querySelector<HTMLElement>('#vban-address-error')
  return hint?.textContent?.trim() ?? ''
}

function getPortError(root: HTMLElement): string {
  const hint = root.querySelector<HTMLElement>('#vban-port-error')
  return hint?.textContent?.trim() ?? ''
}

function getStreamError(root: HTMLElement): string {
  const hint = root.querySelector<HTMLElement>('#vban-stream-error')
  return hint?.textContent?.trim() ?? ''
}

function getActionMessage(root: HTMLElement): string {
  const element = root.querySelector<HTMLElement>('.save-status')
  return element?.textContent?.trim() ?? ''
}

function getAddressInput(root: HTMLElement): HTMLInputElement {
  return byId<HTMLInputElement>(root, 'vban-address')
}

function setTypeValue(input: HTMLInputElement, value: string) {
  input.value = value
  input.dispatchEvent(new Event('input', { bubbles: true }))
}

function getToggle(root: HTMLElement): HTMLInputElement {
  const toggle = root.querySelector<HTMLInputElement>('.toggle-switch input[type="checkbox"]')
  if (!toggle) throw new Error('VBAN enable toggle not found')
  return toggle
}

function getHelpPanel(root: HTMLElement): HTMLElement {
  const panel = root.querySelector<HTMLElement>('#vban-help-panel')
  if (!panel) throw new Error('VBAN help panel not found')
  return panel
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

describe('AudioVbanTab loading and form population', () => {
  it('shows the loading state until the settings cache arrives', async () => {
    const tab = await mountTab({ settings: null })
    try {
      expect(tab.container!.textContent).toContain('Loading VBAN settings...')
      expect(tab.container!.querySelector('.save-section')).toBeNull()
    } finally {
      await tab.unmount()
    }
  })

  it('uses disabled VBAN defaults returned for an old config', async () => {
    const tab = await mountTab({ settings: createSettings(undefined) })
    try {
      const root = tab.container!
      expect(getToggle(root).checked).toBe(false)
      expect(getAddressInput(root).value).toBe('')
      expect(byId<HTMLInputElement>(root, 'vban-port').value).toBe('6980')
      expect(byId<HTMLInputElement>(root, 'vban-stream-name').value).toBe('TTSBard')
      expect(byId<HTMLInputElement>(root, 'vban-volume').value).toBe('100')
    } finally {
      await tab.unmount()
    }
  })

  it('populates the form from the saved vban section', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const root = tab.container!
      expect(getToggle(root).checked).toBe(true)
      expect(getAddressInput(root).value).toBe('192.168.1.50')
      expect(byId<HTMLInputElement>(root, 'vban-port').value).toBe('6980')
      expect(byId<HTMLInputElement>(root, 'vban-stream-name').value).toBe('TTSBard')
      expect(root.querySelector('.volume-value')?.textContent).toContain('100%')
    } finally {
      await tab.unmount()
    }
  })

  it('keeps unsaved edits when the settings cache reloads in the background', async () => {
    const tab = await mountTab({ settings: createSettings(undefined) })
    try {
      const root = tab.container!
      setTypeValue(getAddressInput(root), '192.168.1.77')
      await tab.updateSettings({ ...savedSection, address: '10.0.0.9' })
      expect(getAddressInput(root).value).toBe('192.168.1.77')
    } finally {
      await tab.unmount()
    }
  })

  it('resyncs the form from the cache when there are no unsaved edits', async () => {
    const tab = await mountTab({ settings: createSettings(undefined) })
    try {
      await tab.updateSettings(savedSection)
      const root = tab.container!
      expect(getToggle(root).checked).toBe(true)
      expect(getAddressInput(root).value).toBe('192.168.1.50')
    } finally {
      await tab.unmount()
    }
  })
})

describe('AudioVbanTab validation', () => {
  it('requires a saved destination before enabling output', async () => {
    const tab = await mountTab({ settings: createSettings(undefined) })
    try {
      const root = tab.container!
      getToggle(root).click()
      await flushAsync()
      expect(getToggle(root).disabled).toBe(true)
      expect(tab.mockedInvoke).not.toHaveBeenCalledWith('set_vban_enabled', expect.anything())
      expect(getSaveButton(root).disabled).toBe(true)
    } finally {
      await tab.unmount()
    }
  })

  it('rejects a malformed IPv4 address', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const root = tab.container!
      setTypeValue(getAddressInput(root), '999.1.1.1')
      await flushAsync()
      expect(getAddressError(root)).toBe(t('audio.vban.address.invalid'))
      expect(getSaveButton(root).disabled).toBe(true)
      expect(getTestButton(root).disabled).toBe(true)
    } finally {
      await tab.unmount()
    }
  })

  it('waits for address blur before showing an error while keeping invalid actions blocked', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const root = tab.container!
      const address = getAddressInput(root)
      address.focus()
      setTypeValue(address, '192.168.8.')
      await flushAsync()
      expect(address.hasAttribute('placeholder')).toBe(false)
      expect(getAddressError(root)).toBe('')
      expect(getSaveButton(root).disabled).toBe(true)
      expect(getTestButton(root).disabled).toBe(true)

      address.blur()
      await flushAsync()
      expect(getAddressError(root)).toBe(t('audio.vban.address.invalid'))
      expect(root.querySelector('#vban-address-error')?.parentElement?.classList.contains('vban-validation')).toBe(true)
      expect(address.getAttribute('aria-describedby')).toBe('vban-address-error')
    } finally {
      await tab.unmount()
    }
  })

  it.each([
    ['vban-port', '0', 'vban-port-error', 'audio.vban.port.invalid'],
    ['vban-stream-name', '', 'vban-stream-error', 'audio.vban.stream_name.invalid'],
  ])('shows %s errors after blur in the shared vertical list', async (fieldId, value, errorId, key) => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const root = tab.container!
      const field = byId<HTMLInputElement>(root, fieldId)
      field.focus()
      setTypeValue(field, value)
      await flushAsync()
      expect(root.querySelector(`#${errorId}`)).toBeNull()
      expect(getSaveButton(root).disabled).toBe(true)
      expect(getTestButton(root).disabled).toBe(true)

      field.blur()
      await flushAsync()
      const error = root.querySelector(`#${errorId}`)
      expect(error?.textContent).toBe(t(key))
      expect(error?.parentElement?.classList.contains('vban-validation')).toBe(true)
      expect(field.getAttribute('aria-describedby')).toBe(errorId)
    } finally {
      await tab.unmount()
    }
  })

  it('rejects IPv4 octets with leading zeroes like Rust Ipv4Addr', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const root = tab.container!

      setTypeValue(getAddressInput(root), '010.1.1.1')
      await flushAsync()
      expect(getAddressError(root)).toBe(t('audio.vban.address.invalid'))
      expect(getSaveButton(root).disabled).toBe(true)
      expect(getTestButton(root).disabled).toBe(true)

      // A single leading zero octet ("0") stays valid, matching Rust.
      setTypeValue(getAddressInput(root), '0.1.1.1')
      await flushAsync()
      expect(getAddressError(root)).toBe('')
      expect(getSaveButton(root).disabled).toBe(false)
      expect(getTestButton(root).disabled).toBe(false)
    } finally {
      await tab.unmount()
    }
  })

  it('does not block Save for a malformed address while VBAN is disabled', async () => {
    const tab = await mountTab({ settings: createSettings({ ...savedSection, enabled: false }) })
    try {
      const root = tab.container!
      setTypeValue(getAddressInput(root), '999.1.1.1')
      await flushAsync()
      // The hint warns about the unusable address, but a disabled VBAN stores
      // any string: the Save button stays available for definitely valid input.
      expect(getAddressError(root)).toBe(t('audio.vban.address.invalid'))
      expect(getTestButton(root).disabled).toBe(true)
      expect(getSaveButton(root).disabled).toBe(false)
    } finally {
      await tab.unmount()
    }
  })

  it('rejects an out-of-range port', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const root = tab.container!
      setTypeValue(byId<HTMLInputElement>(root, 'vban-port'), '70000')
      await flushAsync()
      expect(getPortError(root)).toBe(t('audio.vban.port.invalid'))
      expect(getSaveButton(root).disabled).toBe(true)
      expect(getTestButton(root).disabled).toBe(true)
    } finally {
      await tab.unmount()
    }
  })

  it('rejects a stream name outside printable ASCII', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const root = tab.container!
      setTypeValue(byId<HTMLInputElement>(root, 'vban-stream-name'), 'Поток')
      await flushAsync()
      expect(getStreamError(root)).toBe(t('audio.vban.stream_name.invalid'))
      expect(getSaveButton(root).disabled).toBe(true)
    } finally {
      await tab.unmount()
    }
  })

  it('applies the volume slider to the 0-100 range', async () => {
    const tab = await mountTab({ settings: createSettings(undefined) })
    try {
      const root = tab.container!
      const volume = byId<HTMLInputElement>(root, 'vban-volume')
      volume.value = '40'
      volume.dispatchEvent(new Event('input', { bubbles: true }))
      await flushAsync()
      expect(root.querySelector('.volume-value')?.textContent).toContain('40%')
    } finally {
      await tab.unmount()
    }
  })
})

describe('AudioVbanTab save', () => {
  it('sends the current form to set_vban_connection_settings and clears the dirty state', async () => {
    const tab = await mountTab({ settings: createSettings(undefined) })
    try {
      const root = tab.container!
      const toggle = getToggle(root)
      toggle.click()
      await flushAsync()
      setTypeValue(getAddressInput(root), '192.168.1.55')
      await flushAsync()
      expect(tab.dirtyChanges[tab.dirtyChanges.length - 1]).toBe(true)

      getSaveButton(root).click()
      await flushAsync()

      expect(tab.mockedInvoke).toHaveBeenCalledWith('set_vban_connection_settings', {
        settings: {
          enabled: false,
          address: '192.168.1.55',
          port: 6980,
          stream_name: 'TTSBard',
          volume: 100,
        },
      })
      expect(getActionMessage(root)).toBe(t('audio.save.saved'))
      expect(tab.dirtyChanges[tab.dirtyChanges.length - 1]).toBe(false)
      expect(getSaveButton(root).disabled).toBe(true)
    } finally {
      await tab.unmount()
    }
  })

  it('marks only the sent snapshot as saved when the form changes during the save', async () => {
    const tab = await mountTab({ settings: createSettings(undefined) })
    const releaseSave: { current: (() => void) | null } = { current: null }
    try {
      const root = tab.container!
      getToggle(root).click()
      await flushAsync()
      setTypeValue(getAddressInput(root), '192.168.1.55')
      await flushAsync()

      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'set_vban_connection_settings') {
          await new Promise<void>((resolve) => { releaseSave.current = resolve })
        }
        if (cmd === 'get_vban_status') return { state: 'disabled', error: null }
        return undefined
      })

      getSaveButton(root).click()
      await flushAsync()

      // The user edits the port while the save request is in flight.
      setTypeValue(byId<HTMLInputElement>(root, 'vban-port'), '7000')
      await flushAsync()

      releaseSave.current?.()
      await flushAsync()

      // The server received the values as of the click; the status says so.
      expect(getActionMessage(root)).toBe(t('audio.save.saved'))
      // The newer port edit stays unsaved: the dirty dot remains and Save is
      // available again to apply the newer value.
      expect(tab.dirtyChanges[tab.dirtyChanges.length - 1]).toBe(true)
      expect(getSaveButton(root).disabled).toBe(false)

      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'get_vban_status') return { state: 'disabled', error: null }
        return undefined
      })
      getSaveButton(root).click()
      await flushAsync()

      const payloads = tab.mockedInvoke.mock.calls
        .filter(([cmd]) => cmd === 'set_vban_connection_settings')
        .map(([, args]) => (args as { settings: { port: number } }).settings)
      expect(payloads[0].port).toBe(6980)
      expect(payloads[1].port).toBe(7000)
      expect(tab.dirtyChanges[tab.dirtyChanges.length - 1]).toBe(false)
    } finally {
      releaseSave.current?.()
      await tab.unmount()
    }
  })

  it('shows a localized fallback instead of raw backend text and allows a retry', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const root = tab.container!
      setTypeValue(byId<HTMLInputElement>(root, 'vban-stream-name'), 'NewStream')
      await flushAsync()

      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'set_vban_connection_settings') {
          throw 'Внутренняя ошибка IPC при сохранении настроек VBAN'
        }
        return undefined
      })
      getSaveButton(root).click()
      await flushAsync()

      expect(getActionMessage(root)).toBe(t('audio.vban.error.save'))
      expect(getActionMessage(root)).not.toMatch(/[А-Яа-яЁё]/)
      // The failed save keeps the form dirty: Save stays available for a retry.
      expect(getSaveButton(root).disabled).toBe(false)

      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'get_vban_status') return { state: 'disabled', error: null }
        return undefined
      })
      getSaveButton(root).click()
      await flushAsync()
      expect(getActionMessage(root)).toBe(t('audio.save.saved'))
      const saveCalls = tab.mockedInvoke.mock.calls.filter(([cmd]) => cmd === 'set_vban_connection_settings').length
      expect(saveCalls).toBe(2)
    } finally {
      await tab.unmount()
    }
  })
})

describe('AudioVbanTab live controls', () => {
  it('saves volume independently of an invalid network draft and preserves that draft on reload', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    const root = tab.container!
    setTypeValue(getAddressInput(root), 'unfinished')
    setTypeValue(byId<HTMLInputElement>(root, 'vban-volume'), '35')
    await flushAsync()

    expect(tab.mockedInvoke).toHaveBeenCalledWith('set_vban_volume', { volume: 35 })
    expect(tab.mockedInvoke).not.toHaveBeenCalledWith('set_vban_connection_settings', expect.anything())
    await tab.updateSettings({ ...savedSection, volume: 35 })
    expect(getAddressInput(root).value).toBe('unfinished')
    expect(byId<HTMLInputElement>(root, 'vban-volume').value).toBe('35')
    expect(tab.dirtyChanges[tab.dirtyChanges.length - 1]).toBe(true)
    await tab.unmount()
  })

  it('coalesces volume writes and keeps the latest slider position while a write is pending', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    const root = tab.container!
    let release: (() => void) | undefined
    tab.mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === 'set_vban_volume') await new Promise<void>((resolve) => { release = resolve })
    })
    const slider = byId<HTMLInputElement>(root, 'vban-volume')
    setTypeValue(slider, '80')
    await flushAsync()
    setTypeValue(slider, '60')
    setTypeValue(slider, '25')
    await tab.updateSettings({ ...savedSection, volume: 80 })
    expect(slider.value).toBe('25')
    release?.()
    await flushAsync()
    release?.()
    await flushAsync()
    expect(tab.mockedInvoke.mock.calls.filter(([cmd]) => cmd === 'set_vban_volume').map(([, args]) => args))
      .toEqual([{ volume: 80 }, { volume: 25 }])
    expect(getSaveButton(root).disabled).toBe(true)
    expect(tab.dirtyChanges[tab.dirtyChanges.length - 1]).toBe(false)
    await tab.unmount()
  })

  it('restores confirmed volume and shows an error if persistence fails', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    tab.mockedInvoke.mockRejectedValueOnce(new Error('disk unavailable'))
    setTypeValue(byId<HTMLInputElement>(tab.container!, 'vban-volume'), '20')
    await flushAsync()
    expect(byId<HTMLInputElement>(tab.container!, 'vban-volume').value).toBe('100')
    expect(tab.container!.textContent).toContain(t('audio.vban.error.volume'))
    expect(getSaveButton(tab.container!).disabled).toBe(true)
    await tab.unmount()
  })

  it('switches the saved output immediately without saving the connection draft', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    const root = tab.container!
    setTypeValue(getAddressInput(root), '192.168.1.99')
    getToggle(root).click()
    await flushAsync()
    expect(tab.mockedInvoke).toHaveBeenCalledWith('set_vban_enabled', { enabled: false })
    expect(tab.mockedInvoke).not.toHaveBeenCalledWith('set_vban_connection_settings', expect.anything())
    expect(tab.dirtyChanges[tab.dirtyChanges.length - 1]).toBe(true)
    await tab.updateSettings({ ...savedSection, enabled: false, volume: 65 })
    expect(getToggle(root).checked).toBe(false)
    expect(byId<HTMLInputElement>(root, 'vban-volume').value).toBe('65')
    expect(getAddressInput(root).value).toBe('192.168.1.99')
    getSaveButton(root).click()
    await flushAsync()
    expect(tab.mockedInvoke).toHaveBeenCalledWith('set_vban_connection_settings', {
      settings: { ...savedSection, address: '192.168.1.99', enabled: false, volume: 65 },
    })
    await tab.unmount()
  })
})

describe('AudioVbanTab test', () => {
  it('sends the unsaved form to test_vban_output and shows the result', async () => {
    const tab = await mountTab({ settings: createSettings(undefined) })
    try {
      const root = tab.container!
      setTypeValue(getAddressInput(root), '192.168.1.99')
      await flushAsync()

      getTestButton(root).click()
      await flushAsync()

      expect(tab.mockedInvoke).toHaveBeenCalledWith('test_vban_output', {
        settings: {
          enabled: false,
          address: '192.168.1.99',
          port: 6980,
          stream_name: 'TTSBard',
          volume: 100,
        },
      })
      expect(getActionMessage(root)).toBe(t('audio.vban.test.success'))
    } finally {
      await tab.unmount()
    }
  })

  it('keeps the test available while VBAN is disabled with a valid target', async () => {
    const tab = await mountTab({ settings: createSettings({ ...savedSection, enabled: false }) })
    try {
      const root = tab.container!
      expect(getToggle(root).checked).toBe(false)
      expect(getTestButton(root).disabled).toBe(false)
      expect(getSaveButton(root).disabled).toBe(true)

      getTestButton(root).click()
      await flushAsync()
      expect(tab.mockedInvoke).toHaveBeenCalledWith('test_vban_output', {
        settings: { enabled: false, address: '192.168.1.50', port: 6980, stream_name: 'TTSBard', volume: 100 },
      })
      expect(getActionMessage(root)).toBe(t('audio.vban.test.success'))
    } finally {
      await tab.unmount()
    }
  })

  it('blocks the test until the target is valid', async () => {
    const tab = await mountTab({ settings: createSettings(undefined) })
    try {
      const root = tab.container!
      expect(getTestButton(root).disabled).toBe(true)
      expect(tab.mockedInvoke).not.toHaveBeenCalledWith('test_vban_output', expect.anything())
    } finally {
      await tab.unmount()
    }
  })

  it('shows a localized test error and re-enables the button for a retry', async () => {
    const tab = await mountTab({ settings: createSettings({ ...savedSection, enabled: false }) })
    try {
      const root = tab.container!
      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'test_vban_output') {
          throw 'Внутренняя ошибка IPC при тестировании VBAN'
        }
        return undefined
      })

      getTestButton(root).click()
      await flushAsync()

      expect(getActionMessage(root)).toBe(t('audio.vban.error.test'))
      expect(getActionMessage(root)).not.toMatch(/[А-Яа-яЁё]/)
      expect(getTestButton(root).disabled).toBe(false)

      tab.mockedInvoke.mockImplementation(async () => undefined)
      getTestButton(root).click()
      await flushAsync()
      expect(getActionMessage(root)).toBe(t('audio.vban.test.success'))
      expect(tab.mockedInvoke.mock.calls.filter(([cmd]) => cmd === 'test_vban_output')).toHaveLength(2)
    } finally {
      await tab.unmount()
    }
  })

  it('sends the form as it was at the click time, ignoring later edits', async () => {
    const tab = await mountTab({ settings: createSettings({ ...savedSection, enabled: false }) })
    const releaseTest: { current: (() => void) | null } = { current: null }
    try {
      const root = tab.container!
      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'test_vban_output') {
          await new Promise<void>((resolve) => { releaseTest.current = resolve })
        }
        return undefined
      })

      getTestButton(root).click()
      await flushAsync()
      expect(getTestButton(root).disabled).toBe(true)
      expect(getActionMessage(root)).toBe(t('audio.vban.test.sending'))

      // The user edits the form while the test is in flight.
      setTypeValue(getAddressInput(root), '10.10.10.10')
      await flushAsync()

      releaseTest.current?.()
      await flushAsync()

      expect(getActionMessage(root)).toBe(t('audio.vban.test.success'))
    } finally {
      releaseTest.current?.()
      await tab.unmount()
    }
  })

  it('survives switching to a neighbour tab while the test request is in flight', async () => {
    const tab = await mountTab({ settings: createSettings({ ...savedSection, enabled: false }), active: true })
    const releaseTest: { current: (() => void) | null } = { current: null }
    try {
      const root = tab.container!
      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'test_vban_output') {
          await new Promise<void>((resolve) => { releaseTest.current = resolve })
        }
        return undefined
      })

      getTestButton(root).click()
      await flushAsync()

      // Switch to a neighbour tab: the async result must not be lost or crash.
      await tab.setActive(false)

      releaseTest.current?.()
      await flushAsync()

      expect(getActionMessage(root)).toBe(t('audio.vban.test.success'))
      expect(getTestButton(root).disabled).toBe(false)
    } finally {
      releaseTest.current?.()
      await tab.unmount()
    }
  })
})

describe('AudioVbanTab transient test result', () => {
  it('hides the successful test result after 3 seconds', async () => {
    vi.useFakeTimers()
    const tab = await mountTab({ settings: createSettings({ ...savedSection, enabled: false }) })
    try {
      const root = tab.container!
      const success = t('audio.vban.test.success')

      getTestButton(root).click()
      await flushAsync()
      expect(getActionMessage(root)).toBe(success)

      vi.advanceTimersByTime(2999)
      await flushAsync()
      expect(getActionMessage(root)).toBe(success)

      vi.advanceTimersByTime(1)
      await flushAsync()
      expect(getActionMessage(root)).toBe('')
    } finally {
      await tab.unmount()
      vi.useRealTimers()
    }
  })

  it('restarts the 3 second timer when the test is repeated', async () => {
    vi.useFakeTimers()
    const tab = await mountTab({ settings: createSettings({ ...savedSection, enabled: false }) })
    try {
      const root = tab.container!
      const success = t('audio.vban.test.success')
      // The mounted tab also owns the 2 s production-status poll.
      const baselineTimers = vi.getTimerCount()

      getTestButton(root).click()
      await flushAsync()
      expect(getActionMessage(root)).toBe(success)

      vi.advanceTimersByTime(2000)
      await flushAsync()
      expect(getActionMessage(root)).toBe(success)

      // The second click must replace the pending hide timer, not reset it in
      // addition: the first timer would fire 1 s after this point.
      getTestButton(root).click()
      await flushAsync()
      expect(getActionMessage(root)).toBe(success)
      expect(vi.getTimerCount()).toBe(baselineTimers + 1)

      vi.advanceTimersByTime(2999)
      await flushAsync()
      expect(getActionMessage(root)).toBe(success)

      vi.advanceTimersByTime(1)
      await flushAsync()
      expect(getActionMessage(root)).toBe('')
    } finally {
      await tab.unmount()
      vi.useRealTimers()
    }
  })

  it('clears the test result and its timer when the form changes', async () => {
    vi.useFakeTimers()
    const tab = await mountTab({ settings: createSettings({ ...savedSection, enabled: false }) })
    try {
      const root = tab.container!
      const baselineTimers = vi.getTimerCount()

      getTestButton(root).click()
      await flushAsync()
      expect(getActionMessage(root)).not.toBe('')
      vi.advanceTimersByTime(1000)

      setTypeValue(getAddressInput(root), '192.168.1.51')
      await flushAsync()
      expect(getActionMessage(root)).toBe('')
      expect(vi.getTimerCount()).toBe(baselineTimers)

      vi.advanceTimersByTime(5000)
      await flushAsync()
      expect(getActionMessage(root)).toBe('')
    } finally {
      await tab.unmount()
      vi.useRealTimers()
    }
  })

  it('clears the test hide timer on unmount', async () => {
    vi.useFakeTimers()
    const tab = await mountTab({ settings: createSettings({ ...savedSection, enabled: false }) })
    try {
      const root = tab.container!
      const baselineTimers = vi.getTimerCount()

      getTestButton(root).click()
      await flushAsync()
      expect(getActionMessage(root)).not.toBe('')
      expect(vi.getTimerCount()).toBe(baselineTimers + 1)

      await tab.unmount()
      expect(vi.getTimerCount()).toBe(0)
    } finally {
      await tab.unmount()
      vi.useRealTimers()
    }
  })

  it('keeps a failed test visible after the success timeout', async () => {
    vi.useFakeTimers()
    const tab = await mountTab({ settings: createSettings({ ...savedSection, enabled: false }) })
    try {
      const root = tab.container!
      const baselineTimers = vi.getTimerCount()
      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'test_vban_output') throw 'Внутренняя ошибка IPC при тестировании VBAN'
        return undefined
      })

      getTestButton(root).click()
      await flushAsync()
      expect(getActionMessage(root)).toBe(t('audio.vban.error.test'))
      expect(vi.getTimerCount()).toBe(baselineTimers)

      vi.advanceTimersByTime(10000)
      await flushAsync()
      expect(getActionMessage(root)).toBe(t('audio.vban.error.test'))
    } finally {
      await tab.unmount()
      vi.useRealTimers()
    }
  })
})

describe('AudioVbanTab help', () => {
  it('shows the connection instructions in a separate help section', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const panel = getHelpPanel(tab.container!)
      const section = panel.closest('section')
      expect(section?.getAttribute('aria-labelledby')).toBe('vban-help-title')
      expect(section?.querySelector('h2')?.textContent?.trim()).toBe(t('audio.vban.help.title'))
      expect(panel.textContent?.trim()).toBe(t('audio.vban.help.text'))
    } finally {
      await tab.unmount()
    }
  })

  it('does not render the former permanent CABLE/OBS/Discord instructions', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const text = tab.container!.textContent ?? ''
      expect(text).not.toContain('CABLE Input')
      expect(text).not.toContain('CABLE Output')
      expect(text).not.toContain('OBS')
      expect(text).not.toContain('Discord')
    } finally {
      await tab.unmount()
    }
  })

  it('uses the agreed RU and EN help text', () => {
    // The test i18n starts with English only; register the RU catalog before
    // switching the locale so the runtime lookup itself is covered.
    i18n.global.setLocaleMessage('ru', (ruCatalog as { messages: Record<string, string> }).messages)
    withLocale('ru', () => {
      expect(t('audio.vban.help.text')).toBe('Кнопка «Тест» использует текущие значения формы. В «VBAN Receptor» нажмите «Stream Name» и выберите «TTSBard».')
    })
    withLocale('en', () => {
      expect(t('audio.vban.help.text')).toBe('The Test button uses the current form values. In VBAN Receptor, click “Stream Name” and select “TTSBard”.')
    })
  })
})

describe('AudioVbanTab backend status', () => {
  it('polls get_vban_status while mounted and refreshes immediately when activated', async () => {
    vi.useFakeTimers()
    try {
      const tab = await mountTab({ settings: createSettings(undefined), active: false })
      try {
        // The Audio panel tab owns the indicator, so polling starts on mount
        // even while another audio tab is visible.
        expect(tab.mockedInvoke).toHaveBeenCalledWith('get_vban_status')

        tab.mockedInvoke.mockClear()
        vi.advanceTimersByTime(2000)
        await flushAsync()
        expect(tab.mockedInvoke).toHaveBeenCalledWith('get_vban_status')

        tab.mockedInvoke.mockClear()
        await tab.setActive(true)
        expect(tab.mockedInvoke).toHaveBeenCalledWith('get_vban_status')

        // Hiding the tab no longer pauses the indicator polling.
        tab.mockedInvoke.mockClear()
        await tab.setActive(false)
        vi.advanceTimersByTime(2000)
        await flushAsync()
        expect(tab.mockedInvoke).toHaveBeenCalledWith('get_vban_status')
      } finally {
        await tab.unmount()
        vi.useRealTimers()
      }
    } finally {
      vi.useRealTimers()
    }
  })

  it('emits the production state and keeps the backend error text', async () => {
    const tab = await mountTab({ settings: createSettings(undefined), active: true })
    try {
      const root = tab.container!
      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'get_vban_status') {
          return { state: 'sending', error: null }
        }
        return undefined
      })
      await tab.setActive(false)
      await tab.setActive(true)
      expect(tab.statusChanges[tab.statusChanges.length - 1]?.state).toBe('sending')

      // The duplicate status line is gone from the body.
      expect(root.querySelector('.vban-status')).toBeNull()
      expect(root.querySelector('.vban-status-text')).toBeNull()

      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'get_vban_status') {
          return { state: 'error', error: 'socket send failed' }
        }
        return undefined
      })
      await tab.setActive(false)
      await tab.setActive(true)
      expect(tab.statusChanges[tab.statusChanges.length - 1]?.state).toBe('error')
      const detail = root.querySelector<HTMLElement>('.vban-status-error')
      expect(detail?.textContent?.trim()).toBe('socket send failed')
      expect(detail?.getAttribute('role')).toBe('status')
    } finally {
      await tab.unmount()
    }
  })

  it('refreshes the production status after a successful save', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const root = tab.container!
      tab.mockedInvoke.mockImplementation(async (cmd) => {
        if (cmd === 'get_vban_status') {
          return { state: 'idle', error: null }
        }
        return undefined
      })

      const volume = byId<HTMLInputElement>(root, 'vban-stream-name')
      volume.value = 'Changed'
      volume.dispatchEvent(new Event('input', { bubbles: true }))
      await flushAsync()

      const statusCallsBefore = tab.mockedInvoke.mock.calls.filter(([cmd]) => cmd === 'get_vban_status').length
      getSaveButton(root).click()
      await flushAsync()

      const statusCallsAfter = tab.mockedInvoke.mock.calls.filter(([cmd]) => cmd === 'get_vban_status').length
      expect(statusCallsAfter).toBeGreaterThan(statusCallsBefore)
      expect(tab.statusChanges[tab.statusChanges.length - 1]?.state).toBe('idle')
    } finally {
      await tab.unmount()
    }
  })
})

describe('AudioVbanTab accessibility and errors', () => {
  it('associates labels with fields and exposes the enable toggle', async () => {
    const tab = await mountTab({ settings: createSettings(savedSection) })
    try {
      const root = tab.container!
      for (const id of ['vban-address', 'vban-port', 'vban-stream-name', 'vban-volume']) {
        const field = byId<HTMLElement>(root, id)
        expect(field).toBeTruthy()
      }
      const toggle = getToggle(root)
      expect(toggle.getAttribute('aria-label')).toBe(t('audio.vban.enable'))
      const address = getAddressInput(root)
      address.focus()
      expect(document.activeElement).toBe(address)
    } finally {
      await tab.unmount()
    }
  })

  it('presents the English fallback instead of a raw Russian error', () => {
    withLocale('en', () => {
      const raw = 'Внутренняя ошибка IPC при работе с VBAN'
      const presented = presentCommandError(raw, t('audio.vban.error.test'))
      expect(presented).not.toBe(raw)
      expect(presented).not.toMatch(/[А-Яа-яЁё]/)
    })
  })
})
