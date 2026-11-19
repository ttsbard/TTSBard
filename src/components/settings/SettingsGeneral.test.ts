// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { computed, createApp, h, nextTick, ref, type Ref } from 'vue'
import SettingsGeneral from './SettingsGeneral.vue'
import { invoke } from '@tauri-apps/api/core'
import { APP_SETTINGS_KEY, type AppSettingsDto } from '../../types/settings'
import { APP_UPDATES_KEY, upToDateFromStatus, type AppUpdatesContext } from '../../composables/useAppUpdates'
import { UPDATE_INSTALL_KEY, type UpdateInstallContext } from '../../composables/useUpdateInstall'
import { i18n } from '../../i18n'
import { resetOutputControlsPlacement } from '../../composables/useMainWindowOutputControls'
import russianCatalog from '../../../locales/ru.json'
import type { UpdateStatusDto } from '../../ipc/appUpdates'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(async () => null),
}))

function deferred<T = void>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

async function flushAsync() {
  for (let i = 0; i < 10; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

function createSettings(hideExtra = false): AppSettingsDto {
  return {
    windows: {
      global: { exclude_from_capture: false },
      main: {
        x: null,
        y: null,
        width: 800,
        height: 600,
        start_compact: false,
        hide_on_minimize: false,
        hide_extra_window_buttons: hideExtra,
      },
      playback: { x: null, y: null, width: 300, height: 200 },
      soundpanel: { x: null, y: null, width: 400, height: 300 },
    },
    general: {
      show_playback_on_start: false,
      start_compact: false,
      hide_on_minimize: false,
    },
    logging: {
      level: 'info',
    },
  } as unknown as AppSettingsDto
}

/** Wire-shaped card snapshot (matches `UpdateStatusDto` of stage 2). */
function cardStatus(overrides: Partial<UpdateStatusDto> = {}): UpdateStatusDto {
  return {
    current_version: '1.7.0',
    update_available: false,
    latest: null,
    last_attempt_at: null,
    last_attempt_error: null,
    last_check_at: null,
    checking: false,
    startup_check_enabled: true,
    periodic_check_enabled: false,
    check_interval_days: 1,
    dismissed_version: null,
    install_available: false,
    ...overrides,
  }
}

function releaseStatus(notes: string, overrides: Partial<UpdateStatusDto> = {}): UpdateStatusDto {
  return cardStatus({
    update_available: true,
    latest: {
      version: '1.8.0',
      name: null,
      notes,
      url: 'https://github.com/ttsbard/TTSBard/releases/tag/v1.8.0',
      published_at: null,
    },
    last_attempt_at: '2026-11-18T09:00:00Z',
    last_attempt_error: null,
    last_check_at: '2026-11-18T09:00:00Z',
    ...overrides,
  })
}

let appInstance: ReturnType<typeof createApp> | null = null
let container: HTMLDivElement | null = null

function getHideButtonsCheckbox(root: HTMLElement): HTMLInputElement {
  const labels = Array.from(root.querySelectorAll<HTMLLabelElement>('label.setting-label'))
  const label = labels.find((l) =>
    l.textContent?.includes('buttons') ||
    l.textContent?.includes('кнопки') ||
    l.textContent?.includes('hide_extra_window_buttons'),
  )
  const input = label?.querySelector('input[type="checkbox"]') as HTMLInputElement | null
  if (!input) throw new Error('hide_extra_window_buttons checkbox not found')
  return input
}

function getStartupCheckbox(root: HTMLElement): HTMLInputElement {
  const input = root.querySelector<HTMLInputElement>('.updates-auto input[type="checkbox"]')
  if (!input) throw new Error('startup check checkbox not found')
  return input
}

function getPeriodicCheckbox(root: HTMLElement): HTMLInputElement {
  const input = root.querySelector<HTMLInputElement>('.updates-interval input[type="checkbox"]')
  if (!input) throw new Error('periodic check checkbox not found')
  return input
}

function getIntervalSelect(root: HTMLElement): HTMLSelectElement {
  const select = root.querySelector<HTMLSelectElement>('#updates-interval')
  if (!select) throw new Error('interval select not found')
  return select
}

async function mountGeneralSettings(
  initialHideButtons = false,
  status: UpdateStatusDto | null = cardStatus(),
) {
  container = document.createElement('div')
  document.body.appendChild(container)

  const settingsRef = ref<AppSettingsDto | null>(createSettings(initialHideButtons))
  const mockedInvoke = vi.mocked(invoke)
  mockedInvoke.mockImplementation(async (cmd) => {
    if (cmd === 'storage_get_data_info') {
      return { path: 'C:\\data', is_default: true }
    }
    return undefined
  })

  const onShowMessage = vi.fn()
  const statusRef = ref<UpdateStatusDto | null>(status)
  const updatesCommandError = ref<string | null>(null)

  appInstance = createApp({
    render() {
      return h(SettingsGeneral, { onShowMessage })
    },
  })

  appInstance.provide(APP_SETTINGS_KEY, {
    settings: settingsRef,
    isLoading: ref(false),
    error: ref(null),
    reload: vi.fn(async () => {}),
  })

  // The updates card (ROADMAP-141) reads the shared app-updates context; the
  // up-to-date outcome uses the real snapshot derivation, so the tests below
  // exercise the same rule the composable ships.
  const checkNow = vi.fn(async () => {})
  const setStartupCheck = vi.fn(async (_enabled: boolean) => {})
  const setPeriodicCheck = vi.fn(async (_enabled: boolean) => {})
  const setIntervalDays = vi.fn(async (_days: number) => {})
  const dismissPanel = vi.fn(async () => {})
  const updatesContext: AppUpdatesContext = {
    status: statusRef,
    checking: computed(() => statusRef.value?.checking === true),
    panelRelease: computed(() => null),
    commandError: updatesCommandError,
    upToDate: computed(() => upToDateFromStatus(statusRef.value)),
    checkNow,
    setStartupCheck,
    setPeriodicCheck,
    setIntervalDays,
    dismissPanel,
  }
  appInstance.provide(APP_UPDATES_KEY, updatesContext)

  // The install button (ROADMAP-141 stage 2) starts the shared install
  // pipeline; the component test below swaps in a real status through the
  // same context shape.
  const installRunning = ref(false)
  const updateInstallContext: UpdateInstallContext = {
    stage: ref(null),
    version: ref(null),
    downloaded: ref(0),
    total: ref(null),
    errorText: ref(null),
    active: computed(() => installRunning.value),
    canCancel: computed(() => false),
    percent: computed(() => null),
    start: vi.fn(async () => {}),
    cancel: vi.fn(async () => false),
    reset: vi.fn(),
  }
  appInstance.provide(UPDATE_INSTALL_KEY, updateInstallContext)

  appInstance.mount(container)

  await flushAsync()
  mockedInvoke.mockClear()

  return {
    container,
    settingsRef,
    statusRef,
    mockedInvoke,
    onShowMessage,
    updatesContext,
    checkNow,
    setStartupCheck,
    setPeriodicCheck,
    setIntervalDays,
    dismissPanel,
    updatesCommandError,
    installRunning,
    updateInstallContext,
    getCheckbox: () => getHideButtonsCheckbox(container!),
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

afterEach(async () => {
  if (appInstance) {
    appInstance.unmount()
    appInstance = null
  }
  if (container) {
    container.remove()
    container = null
  }
  vi.clearAllTimers()
  vi.useRealTimers()
  vi.clearAllMocks()
  document.body.innerHTML = ''
})

async function withLocale<T>(language: 'en' | 'ru', run: () => Promise<T>): Promise<T> {
  const activeLocale: Ref<string> = i18n.global.locale
  const previousLocale = activeLocale.value
  i18n.global.setLocaleMessage('ru', russianCatalog.messages)
  activeLocale.value = language
  try {
    return await run()
  } finally {
    activeLocale.value = previousLocale
  }
}

describe('SettingsGeneral update intervals', () => {
  it('shows formatted release notes immediately and keeps remote content inert', async () => {
    const { container } = await mountGeneralSettings(false, releaseStatus([
      '### Added',
      '',
      '- **New feature**',
      '',
      '[Release link](https://example.com) ![Picture](https://example.com/picture.png)',
      '',
      '<img src="https://example.com/raw.png">',
    ].join('\n')))
    const notes = container.querySelector('.updates-notes')
    expect(notes?.querySelector('h3')?.textContent).toBe('Added')
    expect(notes?.querySelector('li strong')?.textContent).toBe('New feature')
    expect(notes?.textContent).toContain('Release link')
    expect(notes?.querySelector('a, img')).toBeNull()
  })

  it('renders nested release-note lists for the indented-marker layout', async () => {
    const { container } = await mountGeneralSettings(false, releaseStatus([
      '- top item',
      '  - nested item',
      '',
      '1. first',
      '   1. deeper',
    ].join('\n')))
    const notes = container.querySelector('.updates-notes')
    // The restored ul/ol padding indents outer and nested lists alike; this
    // pins the DOM shape the styling relies on (markers stay inside the
    // scroll container instead of clipping at its left edge).
    expect(notes?.querySelector('ul li ul li')?.textContent).toBe('nested item')
    expect(notes?.querySelector('ol li ol li')?.textContent).toBe('deeper')
  })

  it.each([
    ['en', ['1 day', '2 days', '3 days', '4 days', '5 days', '6 days', '7 days']],
    ['ru', ['1 день', '2 дня', '3 дня', '4 дня', '5 дней', '6 дней', '7 дней']],
  ] as const)('shows all seven intervals in %s', async (language, labels) => {
    await withLocale(language, async () => {
      const { container } = await mountGeneralSettings()
      const options = container.querySelectorAll('#updates-interval option')
      expect(Array.from(options, (option) => option.textContent?.trim())).toEqual(labels)
    })
  })
})

describe('SettingsGeneral update switches (stage 2)', () => {
  it.each([
    [true, true],
    [true, false],
    [false, true],
    [false, false],
  ] as const)('reflects the startup/periodic combination %j/%j and sends toggles', async (startup, periodic) => {
    const { container, updatesContext } = await mountGeneralSettings(false, cardStatus({
      startup_check_enabled: startup,
      periodic_check_enabled: periodic,
    }))

    const startupInput = getStartupCheckbox(container)
    const periodicInput = getPeriodicCheckbox(container)
    const select = getIntervalSelect(container)
    expect(startupInput.checked).toBe(startup)
    expect(periodicInput.checked).toBe(periodic)
    // The interval selector is enabled only while the schedule is on.
    expect(select.disabled).toBe(!periodic)

    startupInput.checked = !startup
    startupInput.dispatchEvent(new Event('change'))
    await flushAsync()
    expect(updatesContext.setStartupCheck).toHaveBeenCalledWith(!startup)

    periodicInput.checked = !periodic
    periodicInput.dispatchEvent(new Event('change'))
    await flushAsync()
    expect(updatesContext.setPeriodicCheck).toHaveBeenCalledWith(!periodic)
  })

  it('keeps the interval value while disabled and restores it when re-enabled', async () => {
    const { container, statusRef } = await mountGeneralSettings(false, cardStatus({
      periodic_check_enabled: false,
      check_interval_days: 5,
    }))
    const select = getIntervalSelect(container)
    expect(select.disabled).toBe(true)
    expect(select.value).toBe('5')

    // Turning the schedule back on re-enables the kept interval.
    statusRef.value = cardStatus({ periodic_check_enabled: true, check_interval_days: 5 })
    await flushAsync()
    expect(select.disabled).toBe(false)
    expect(select.value).toBe('5')
  })

  it('rolls the periodic switch back when the save fails and shows the save error', async () => {
    const mount = await mountGeneralSettings(false, cardStatus({ periodic_check_enabled: false }))
    const target = getPeriodicCheckbox(mount.container)
    expect(target.checked).toBe(false)

    // A failed save sends no snapshot: the context stays unchanged and the
    // card snaps the native input back to the persisted value.
    mount.setPeriodicCheck.mockImplementation(async () => {
      mount.updatesCommandError.value = 'Could not save the update check settings.'
    })
    target.checked = true
    target.dispatchEvent(new Event('change'))
    await flushAsync()

    expect(target.checked).toBe(false)
    expect(mount.container.querySelector('.updates-error')).not.toBeNull()
  })

  it('rolls the startup switch back when the save fails', async () => {
    const mount = await mountGeneralSettings(false, cardStatus({ startup_check_enabled: true }))
    const target = getStartupCheckbox(mount.container)
    expect(target.checked).toBe(true)

    mount.setStartupCheck.mockImplementation(async () => {
      mount.updatesCommandError.value = 'Could not save the update check settings.'
    })
    target.checked = false
    target.dispatchEvent(new Event('change'))
    await flushAsync()

    expect(target.checked).toBe(true)
    expect(mount.container.querySelector('.updates-error')).not.toBeNull()
  })

  it('keeps the toggled value after a successful save snapshot', async () => {
    const mount = await mountGeneralSettings(false, cardStatus({ periodic_check_enabled: false, check_interval_days: 3 }))
    const target = getPeriodicCheckbox(mount.container)
    expect(target.checked).toBe(false)

    mount.setPeriodicCheck.mockImplementation(async () => {
      mount.statusRef.value = cardStatus({ periodic_check_enabled: true, check_interval_days: 3 })
    })
    target.checked = true
    target.dispatchEvent(new Event('change'))
    await flushAsync()

    expect(target.checked).toBe(true)
    const select = getIntervalSelect(mount.container)
    expect(select.disabled).toBe(false)
    expect(select.value).toBe('3')
  })

  it('sends a kept interval after the user changes it while enabled', async () => {
    const { container, updatesContext } = await mountGeneralSettings(false, cardStatus({ periodic_check_enabled: true }))
    const select = getIntervalSelect(container)
    select.value = '4'
    select.dispatchEvent(new Event('change'))
    await flushAsync()
    expect(updatesContext.setIntervalDays).toHaveBeenCalledWith(4)
  })

  it('disables the manual check button only while a check is running', async () => {
    const mount = await mountGeneralSettings(false, cardStatus())
    const button = mount.container.querySelector<HTMLButtonElement>('.updates-check-button')
    expect(button?.disabled).toBe(false)
    button?.dispatchEvent(new Event('click'))
    await flushAsync()
    expect(mount.updatesContext.checkNow).toHaveBeenCalledTimes(1)

    mount.statusRef.value = cardStatus({ checking: true })
    await flushAsync()
    expect(button?.disabled).toBe(true)
  })
})

describe('SettingsGeneral last attempt outcome (stage 2)', () => {
  it('shows no result line before the first attempt completes', async () => {
    const { container } = await mountGeneralSettings(false, cardStatus())
    expect(container.querySelector('.updates-ok')).toBeNull()
    expect(container.querySelector('.updates-error-plate')).toBeNull()
    // The attempt time item is hidden until an attempt finishes.
    expect(container.querySelector('.updates-last-attempt')).toBeNull()
    expect(container.querySelector('.updates-last-check')?.textContent?.trim())
      .toBe('Not checked yet')
  })

  it.each([
    ['en', 'You’re up to date'],
    ['ru', 'Установлена актуальная версия'],
  ] as const)('shows the up-to-date outcome in %s once an attempt finished clean', async (language, text) => {
    await withLocale(language, async () => {
      const { container } = await mountGeneralSettings(false, cardStatus({
        update_available: false,
        latest: null,
        last_attempt_at: '2026-11-18T09:00:00Z',
        last_check_at: '2026-11-18T09:00:00Z',
      }))
      const note = container.querySelector('.updates-ok')
      expect(note?.textContent?.trim()).toContain(text)
      expect(container.querySelector('.updates-last-attempt')).toBeNull()
      // The outcome stays in the card: it is a persistent state, not a toast.
      expect(container.querySelector('.updates-section')?.contains(note ?? null)).toBe(true)
    })
  })

  it.each([
    ['en', 'Could not get the latest version information from GitHub'],
    ['ru', 'Не удалось получить информацию о последней версии с GitHub'],
  ] as const)('shows the calm plate with the exact base phrase in %s', async (language, text) => {
    await withLocale(language, async () => {
      const { container } = await mountGeneralSettings(false, cardStatus({
        last_attempt_at: '2026-11-18T10:00:00Z',
        last_attempt_error: 'network',
      }))
      const plate = container.querySelector('.updates-error-plate')
      expect(plate?.textContent?.trim()).toBe(text)
      // The plate lives inside the card (no popup, toast or bubble).
      expect(container.querySelector('.updates-section')?.contains(plate ?? null)).toBe(true)
    })
  })

  it('shows the failed attempt beside the previously found release and keeps both times', async () => {
    const initial = releaseStatus('What is new', { install_available: true })
    const { container, statusRef } = await mountGeneralSettings(false, initial)
    expect(container.querySelector('.updates-error-plate')).toBeNull()

    // A fresh failure keeps the release, its notes, the install offer and the
    // success stamp; only the attempt time moves.
    statusRef.value = releaseStatus('What is new', {
      install_available: true,
      last_attempt_at: '2026-11-18T12:00:00Z',
      last_attempt_error: 'rate_limited',
      last_check_at: '2026-11-18T09:00:00Z',
    })
    await flushAsync()

    expect(container.querySelector('.updates-error-plate')).not.toBeNull()
    expect(container.querySelector('.updates-release')).not.toBeNull()
    expect(container.querySelector('.updates-notes')?.textContent).toContain('What is new')
    expect(container.querySelector('.updates-install-row')).not.toBeNull()
    // The «up to date» line must not appear next to a failure.
    expect(container.querySelector('.updates-ok')).toBeNull()
  })

  it('keeps the success stamp unchanged while the attempt time moves after a failure', async () => {
    const lastCheck = new Date(2026, 0, 5, 8, 3).toISOString()
    const lastAttempt = new Date(2026, 0, 5, 16, 42).toISOString()
    const { container, statusRef } = await mountGeneralSettings(false, releaseStatus('What is new', {
      last_attempt_at: lastCheck,
      last_check_at: lastCheck,
    }))
    const successTime = () => container.querySelector('.updates-last-check')?.textContent?.trim()
    const attemptTime = () => container.querySelector('.updates-last-attempt')?.textContent?.trim()
    const before = successTime()
    expect(before).toBe('05.01.2026 08:03')

    statusRef.value = releaseStatus('What is new', {
      last_attempt_at: lastAttempt,
      last_attempt_error: 'network',
      last_check_at: lastCheck,
    })
    await flushAsync()

    expect(attemptTime()).toBe('05.01.2026 16:42')
    expect(successTime()).toBe(before)
  })

  it('clears the plate when a later attempt succeeds', async () => {
    const { container, statusRef } = await mountGeneralSettings(false, cardStatus({
      last_attempt_at: '2026-11-18T10:00:00Z',
      last_attempt_error: 'parse',
    }))
    expect(container.querySelector('.updates-error-plate')).not.toBeNull()
    expect(container.querySelector('.updates-ok')).toBeNull()

    statusRef.value = cardStatus({
      last_attempt_at: '2026-11-18T11:00:00Z',
      last_attempt_error: null,
      last_check_at: '2026-11-18T11:00:00Z',
    })
    await flushAsync()

    expect(container.querySelector('.updates-error-plate')).toBeNull()
    expect(container.querySelector('.updates-ok')).not.toBeNull()
    expect(container.querySelector('.updates-last-attempt')).toBeNull()
  })

  it('renders the settings save error line apart from the check plate', async () => {
    const { container, updatesCommandError } = await mountGeneralSettings(false, cardStatus({
      last_attempt_at: '2026-11-18T10:00:00Z',
      last_attempt_error: 'network',
    }))
    updatesCommandError.value = 'Could not save the update check settings.'
    await flushAsync()
    // Both render: the persistent check plate and the action feedback line.
    expect(container.querySelector('.updates-error-plate')).not.toBeNull()
    expect(container.querySelector('.updates-error:not(.updates-error-plate)')?.textContent)
      .toContain('Could not save the update check settings.')
  })
})

describe('SettingsGeneral hide_extra_window_buttons', () => {
  it('starts a single save command during rapid repeated change events', async () => {
    const { getCheckbox, mockedInvoke } = await mountGeneralSettings(false)
    const checkbox = getCheckbox()
    expect(checkbox.checked).toBe(false)
    expect(checkbox.disabled).toBe(false)

    const gate = deferred()
    mockedInvoke.mockImplementationOnce(() => gate.promise)

    // First toggle
    checkbox.dispatchEvent(new Event('change'))
    await nextTick()

    expect(checkbox.checked).toBe(true)
    expect(checkbox.disabled).toBe(true)

    // Repeated toggle while pending
    checkbox.dispatchEvent(new Event('change'))
    await flushAsync()

    // Only one invoke call started
    expect(mockedInvoke).toHaveBeenCalledTimes(1)
    expect(mockedInvoke).toHaveBeenCalledWith('set_hide_extra_window_buttons', { value: true })

    // Resolve in-flight save
    gate.resolve()
    await flushAsync()

    expect(checkbox.disabled).toBe(false)
    expect(checkbox.checked).toBe(true)
  })

  it('rolls back on save failure, displays error message, and permits retry', async () => {
    const { getCheckbox, mockedInvoke, onShowMessage } = await mountGeneralSettings(false)
    const checkbox = getCheckbox()
    expect(checkbox.checked).toBe(false)

    mockedInvoke.mockRejectedValueOnce(new Error('disk full'))

    // Attempt toggle -> fails
    checkbox.dispatchEvent(new Event('change'))
    await flushAsync()

    // Rollback
    expect(checkbox.checked).toBe(false)
    expect(checkbox.disabled).toBe(false)
    expect(onShowMessage).toHaveBeenCalledWith(expect.any(String), 'error')
    expect(mockedInvoke).toHaveBeenCalledWith('set_hide_extra_window_buttons', { value: true })

    // Retry succeeds
    mockedInvoke.mockResolvedValueOnce(undefined)
    checkbox.dispatchEvent(new Event('change'))
    await flushAsync()

    expect(checkbox.checked).toBe(true)
    expect(mockedInvoke).toHaveBeenLastCalledWith('set_hide_extra_window_buttons', { value: true })
  })

  it('does not let an external settings update overwrite a pending toggle', async () => {
    const { getCheckbox, settingsRef, mockedInvoke } = await mountGeneralSettings(false)
    const checkbox = getCheckbox()
    expect(checkbox.checked).toBe(false)

    const gate = deferred()
    mockedInvoke.mockImplementationOnce(() => gate.promise)

    checkbox.dispatchEvent(new Event('change'))
    await nextTick()
    expect(checkbox.checked).toBe(true)

    // External settings update arrives with old false value while toggle is pending
    settingsRef.value = createSettings(false)
    await flushAsync()

    // User's choice must NOT be overwritten
    expect(checkbox.checked).toBe(true)

    // In-flight save finishes
    gate.resolve()
    await flushAsync()

    // External settings update arrives with true value
    settingsRef.value = createSettings(true)
    await flushAsync()

    expect(checkbox.checked).toBe(true)
  })
})

describe('SettingsGeneral update install button', () => {
  it('shows the centered install button only when the backend offers the install', async () => {
    const withCapability = await mountGeneralSettings(false, releaseStatus('What is new', { install_available: true }))
    const button = withCapability.container!.querySelector<HTMLButtonElement>('.updates-install-row .updates-install-button')
    expect(button, 'install button under the notes').not.toBeNull()
    // The suite runs in the default locale; accept either catalog's label.
    expect(button?.textContent?.trim()).toMatch(/Обновить|Update/)
    // Centering comes from `.updates-install-row` (scoped CSS, verified in
    // the mock-run UI report — jsdom does not apply scoped styles).
    expect(withCapability.container!.querySelector('.updates-install-row > .updates-install-button'))
      .not.toBeNull()
    await withCapability.unmount()

    const withoutCapability = await mountGeneralSettings(false, releaseStatus('What is new', { install_available: false }))
    expect(withoutCapability.container!.querySelector('.updates-install-row')).toBeNull()
    await withoutCapability.unmount()
  })

  it('starts the shared install pipeline from the button and disables it while running', async () => {
    const mount = await mountGeneralSettings(false, releaseStatus('What is new', { install_available: true }))
    const button = mount.container!.querySelector<HTMLButtonElement>('.updates-install-row .updates-install-button')
    expect(button).not.toBeNull()
    expect(button?.disabled).toBe(false)
    button!.dispatchEvent(new Event('click'))
    await flushAsync()
    expect(mount.updateInstallContext.start).toHaveBeenCalledTimes(1)

    // An active pipeline disables the card button; the progress modal is
    // hosted by App.vue and not part of this component tree.
    mount.installRunning.value = true
    await flushAsync()
    expect(button!.disabled).toBe(true)
    await mount.unmount()
  })
})


describe('SettingsGeneral audio output placement (ROADMAP-143)', () => {
  const placementSelect = (root: HTMLElement | null) => {
    const select = root?.querySelector<HTMLSelectElement>('#output-controls-placement')
    if (!select) throw new Error('output-controls-placement select not found')
    return select
  }

  afterEach(() => {
    resetOutputControlsPlacement()
    localStorage.removeItem('app-output-controls-placement')
  })

  it('defaults to hidden when nothing (or an invalid value) is stored', async () => {
    const mount = await mountGeneralSettings()
    expect(localStorage.getItem('app-output-controls-placement')).toBeNull()
    expect(placementSelect(mount.container).value).toBe('hidden')
    await mount.unmount()
    // An invalid stored value is ignored, the default stays.
    localStorage.setItem('app-output-controls-placement', 'sidebar')
    const invalid = await mountGeneralSettings()
    expect(placementSelect(invalid.container).value).toBe('hidden')
    await invalid.unmount()
  })

  it('keeps the default when reading the stored value fails', async () => {
    const getItem = vi.spyOn(Storage.prototype, 'getItem')
    getItem.mockImplementation(() => {
      throw new Error('storage unavailable')
    })
    const { container } = await mountGeneralSettings()
    expect(placementSelect(container).value).toBe('hidden')
    getItem.mockRestore()
  })

  it.each(['titlebar', 'editor', 'hidden'])('persists and restores display choice %s', async (value) => {
    const mount = await mountGeneralSettings()
    const select = placementSelect(mount.container)
    select.value = value
    select.dispatchEvent(new Event('change'))
    await flushAsync()

    expect(localStorage.getItem('app-output-controls-placement')).toBe(value)
    expect(select.value).toBe(value)
    expect(mount.onShowMessage).not.toHaveBeenCalled()
    await mount.unmount()
    resetOutputControlsPlacement()
    const restored = await mountGeneralSettings()
    expect(placementSelect(restored.container).value).toBe(value)
    await restored.unmount()
  })

  it('rolls a failed write back to the previous placement and reports the error', async () => {
    const { container, onShowMessage } = await mountGeneralSettings()
    const select = placementSelect(container)
    expect(select.value).toBe('hidden')

    const setItem = vi.spyOn(Storage.prototype, 'setItem')
    setItem.mockImplementation(() => {
      throw new Error('disk full')
    })
    select.value = 'titlebar'
    select.dispatchEvent(new Event('change'))
    await flushAsync()

    // Nothing was stored and the previous choice stays selected; the user
    // is told the placement was not saved (no fake success).
    expect(localStorage.getItem('app-output-controls-placement')).toBeNull()
    expect(select.value).toBe('hidden')
    expect(onShowMessage).toHaveBeenCalledWith('Could not save the audio output placement', 'error')

    // A retry that succeeds applies the choice.
    setItem.mockRestore()
    select.value = 'titlebar'
    select.dispatchEvent(new Event('change'))
    await flushAsync()
    expect(localStorage.getItem('app-output-controls-placement')).toBe('titlebar')
    expect(select.value).toBe('titlebar')
  })
})
