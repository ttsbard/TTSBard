// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick } from 'vue'
import type { PocketModelDto } from './pocketTts'

const { invokeMock, openMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  openMock: vi.fn(),
  listenMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: openMock }))

import TtsPocketCard from './TtsPocketCard.vue'

const mounted: Array<{ unmount(): void }> = []

function shtorm(overrides: Partial<PocketModelDto> = {}): PocketModelDto {
  return {
    id: 'local-pocket:shtorm',
    display_name: 'Shtorm RU v2 Fast',
    language: 'ru',
    format: 'pocket-tts-onnx',
    format_version: 1,
    package_id: 'shtorm',
    voice_mode: 'reference_wav',
    voices: [],
    selection: null,
    readiness: 'needs_reference',
    error_code: 'pocket.reference_required',
    loaded: false,
    reference_duration_seconds: null,
    ...overrides,
  }
}

function kyutai(overrides: Partial<PocketModelDto> = {}): PocketModelDto {
  return {
    id: 'local-pocket:kyutai',
    display_name: 'Kyutai English',
    language: 'en',
    format: 'pocket-tts-onnx',
    format_version: 1,
    package_id: 'kyutai',
    voice_mode: 'voice_state',
    voices: [{ id: 'alba', display_name: 'Alba', available: true }],
    selection: { kind: 'voice_state', voice_id: 'alba', display_name: 'Alba' },
    readiness: 'ready',
    error_code: null,
    loaded: false,
    reference_duration_seconds: null,
    ...overrides,
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

async function flush() {
  await Promise.resolve()
  await nextTick()
  await new Promise(resolve => setTimeout(resolve, 0))
  await nextTick()
}

async function mountCard(activeModelId: string | null = null) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const activate = vi.fn()
  const app = createApp({
    render: () => h(TtsPocketCard, {
      activeModelId,
      speakerDevice: null,
      speakerVolume: 100,
      onActivate: activate,
    }),
  })
  app.mount(host)
  mounted.push({
    unmount() {
      app.unmount()
      host.remove()
    },
  })
  await flush()
  return { host, activate }
}

function installList(models: PocketModelDto[] | (() => PocketModelDto[])) {
  invokeMock.mockImplementation((command: string) => {
    if (command === 'list_pocket_models' || command === 'refresh_pocket_models') {
      return Promise.resolve(typeof models === 'function' ? models() : models)
    }
    if (command === 'stop_preview') return Promise.resolve()
    return Promise.resolve()
  })
}

function modelRadios(host: HTMLElement) {
  return [...host.querySelectorAll<HTMLInputElement>('input[name="pocket-model"]')]
}

describe('TtsPocketCard', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    openMock.mockReset()
    listenMock.mockReset().mockResolvedValue(vi.fn())
  })

  afterEach(() => {
    for (const item of mounted.splice(0)) item.unmount()
  })

  it('renders an explicit empty catalog state and an accessible refresh control', async () => {
    installList([])
    const { host } = await mountCard()

    expect(host.querySelector('.pocket-empty')).not.toBeNull()
    expect(host.textContent).toContain('No Pocket TTS models found.')
    expect(modelRadios(host)).toHaveLength(0)

    const refresh = host.querySelector('.pocket-refresh-button') as HTMLButtonElement
    expect(refresh).not.toBeNull()
    expect(refresh.getAttribute('title')).toBeTruthy()
    expect(refresh.getAttribute('aria-label')).toBeTruthy()
  })

  it('renders as an always-expanded, non-selectable card with a Cpu icon and header refresh', async () => {
    installList([shtorm()])
    const { host } = await mountCard()

    expect(host.querySelector('.provider-card')).not.toBeNull()
    expect(host.querySelector('.expand-icon')).toBeNull()
    expect(host.querySelector('.card-content')).not.toBeNull()
    expect(host.querySelector('.card-header input[type="radio"]')).toBeNull()
    expect(host.querySelector('.card-header')?.firstElementChild?.classList.contains('provider-icon')).toBe(true)
    expect(host.querySelector('.provider-icon.lucide-cpu')).not.toBeNull()
    expect(host.querySelector('.header-actions .pocket-refresh-button')).not.toBeNull()
    expect(host.querySelector('.card-subtitle')).toBeNull()
    expect(host.textContent).not.toContain('Local CPU synthesis')
    expect(host.querySelector('.local-model-list')).not.toBeNull()
  })

  it('uses compact native radios for models and ready voices', async () => {
    const english = kyutai()
    installList([english])
    const { host } = await mountCard(english.id)

    const modelRadio = modelRadios(host)[0]
    expect(modelRadio.classList.contains('local-model-radio')).toBe(true)
    expect(modelRadio.classList.contains('ui-choice-input')).toBe(false)

    const voiceRadio = host.querySelector<HTMLInputElement>(
      `input[name="pocket-voice-${english.id}"]`,
    )
    expect(voiceRadio).not.toBeNull()
    expect(voiceRadio!.classList.contains('local-model-radio')).toBe(true)
    expect(voiceRadio!.classList.contains('ui-choice-input')).toBe(false)
    expect(host.textContent).toContain('Ready')
  })

  it('reloads the Pocket registry when a model has been installed', async () => {
    let models: PocketModelDto[] = []
    installList(() => models)
    const { host } = await mountCard()
    expect(modelRadios(host)).toHaveLength(0)

    const [, onModelsChanged] = listenMock.mock.calls.find(([event]) => event === 'pocket-models-changed')!
    models = [shtorm()]
    onModelsChanged()
    await flush()

    expect(modelRadios(host)).toHaveLength(1)
  })

  it('lists a discovered model with its language and needs-reference state', async () => {
    installList([shtorm()])
    const { host } = await mountCard()

    expect(modelRadios(host)).toHaveLength(1)
    expect(host.querySelector('.pocket-model-language')?.textContent).toContain('RU')
    expect(host.textContent).toContain('Shtorm RU v2 Fast')
    expect(host.textContent).toContain('WAV needed')
    expect(host.querySelector('.pocket-choose-button')?.textContent).toContain('Choose WAV reference')
    expect(host.querySelector('.pocket-reference-name')).toBeNull()

    expect(host.textContent).toContain('Up to 12 s')
    expect(host.querySelector('.pocket-test-button')).toBeNull()
    expect(host.textContent?.match(/Shtorm RU v2 Fast/g)).toHaveLength(1)
  })

  it('keeps the calm readiness hint and the pending activation for a needs-reference model', async () => {
    const russian = shtorm()
    const english = kyutai()
    const readyRussian = shtorm({
      selection: { kind: 'reference_wav', path: 'C:\\Voices\\me.wav', file_name: 'me.wav' },
      readiness: 'ready',
      error_code: null,
    })
    let selectCalls = 0
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_pocket_models') return Promise.resolve([russian, english])
      if (command === 'stop_preview') return Promise.resolve()
      if (command === 'select_tts_provider_by_id') {
        selectCalls += 1
        return selectCalls === 1
          ? Promise.reject({ code: 'tts_provider.selection_failed', message: 'not ready', retryable: false })
          : Promise.resolve()
      }
      if (command === 'validate_pocket_reference') {
        return Promise.resolve({
          status: 'ready',
          error_code: null,
          file_name: 'me.wav',
          duration_seconds: 5,
          sample_rate: 24000,
          channels: 1,
        })
      }
      if (command === 'set_pocket_voice') return Promise.resolve(readyRussian)
      return Promise.resolve()
    })
    openMock.mockResolvedValue('C:\\Voices\\me.wav')

    const { host, activate } = await mountCard(english.id)
    const initialRows = [...host.querySelectorAll<HTMLElement>('.local-model-row')]
    expect(initialRows[0].classList.contains('local-model-row--focused')).toBe(false)
    expect(host.querySelector('.pocket-field-label')?.textContent?.trim()).toBe('Ready voice')

    modelRadios(host)[0].click()
    await flush()

    expect(invokeMock).toHaveBeenCalledWith('select_tts_provider_by_id', { id: russian.id })
    const rows = [...host.querySelectorAll<HTMLElement>('.local-model-row')]
    expect(rows[0].classList.contains('local-model-row--focused')).toBe(true)
    expect(rows[0].classList.contains('local-model-row--active')).toBe(false)
    expect(rows[1].classList.contains('local-model-row--focused')).toBe(false)
    expect(host.querySelector('.pocket-field-label')?.textContent?.trim())
      .toBe('Voice reference — Shtorm RU v2 Fast')
    expect(host.querySelector('.pocket-details .ui-hint')?.textContent)
      .toContain('Choose a WAV reference to make this model ready.')
    expect(host.querySelector('.local-model-message--error')).toBeNull()
    expect(host.querySelector('[role="alert"]')).toBeNull()

    ;(host.querySelector('.pocket-choose-button') as HTMLButtonElement).click()
    await flush()

    expect(invokeMock.mock.calls.filter(([command]) => command === 'select_tts_provider_by_id')).toHaveLength(2)
    expect(activate).toHaveBeenCalledWith(russian.id)
  })

  it('still shows a red alert when a broken reference model cannot be activated', async () => {
    const missing = shtorm({
      selection: { kind: 'reference_wav', path: 'C:\\Voices\\gone.wav', file_name: 'gone.wav' },
      readiness: 'reference_missing',
      error_code: 'pocket.reference_missing',
    })
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_pocket_models') return Promise.resolve([missing])
      if (command === 'select_tts_provider_by_id') {
        return Promise.reject({ code: 'tts_provider.selection_failed', message: 'missing', retryable: false })
      }
      if (command === 'stop_preview') return Promise.resolve()
      return Promise.resolve()
    })

    const { host } = await mountCard()
    modelRadios(host)[0].click()
    await flush()

    const alert = host.querySelector('.local-model-message--error[role="alert"]')
    expect(alert).not.toBeNull()
    expect(alert?.textContent)
      .toContain('The saved reference file is gone. Choose the WAV again.')
  })

  it('keeps the previous reference when the WAV dialog is cancelled', async () => {
    const ready = shtorm({
      selection: { kind: 'reference_wav', path: 'C:\\Voices\\old.wav', file_name: 'old.wav' },
      readiness: 'ready',
      error_code: null,
    })
    installList([ready])
    openMock.mockResolvedValue(null)
    const { host } = await mountCard(ready.id)

    expect(host.querySelector('.pocket-reference-name')?.textContent).toContain('old.wav')
    ;(host.querySelector('.pocket-replace-button') as HTMLButtonElement).click()
    await flush()

    expect(openMock).toHaveBeenCalledTimes(1)
    expect(openMock.mock.calls[0][0]).toMatchObject({
      multiple: false,
      filters: [{ extensions: ['wav'] }],
    })
    expect(host.querySelector('.pocket-reference-name')?.textContent).toContain('old.wav')
    expect(invokeMock.mock.calls.some(([command]) => command === 'validate_pocket_reference')).toBe(false)
    expect(invokeMock.mock.calls.some(([command]) => command === 'set_pocket_voice')).toBe(false)
  })

  it('shows a validation error, retries, and activates the model after a successful check', async () => {
    const initial = shtorm()
    const checked = shtorm({
      selection: {
        kind: 'reference_wav',
        path: 'C:\\Voices\\good.wav',
        file_name: 'good.wav',
      },
      readiness: 'ready',
      error_code: null,
    })
    installList([initial])
    openMock
      .mockResolvedValueOnce('C:\\Voices\\bad.wav')
      .mockResolvedValueOnce('C:\\Voices\\good.wav')
    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === 'list_pocket_models') return Promise.resolve([initial])
      if (command === 'stop_preview') return Promise.resolve()
      if (command === 'select_tts_provider_by_id') {
        return invokeMock.mock.calls.filter(([name]) => name === 'select_tts_provider_by_id').length === 1
          ? Promise.reject({ code: 'tts_provider.selection_failed', message: 'not ready', retryable: false })
          : Promise.resolve()
      }
      if (command === 'validate_pocket_reference') {
        return Promise.resolve(args?.path === 'C:\\Voices\\good.wav'
          ? { status: 'ready', error_code: null, file_name: 'good.wav', duration_seconds: 5, sample_rate: 24000, channels: 1 }
          : { status: 'reference_invalid', error_code: 'pocket.reference_invalid', file_name: 'bad.wav', duration_seconds: null, sample_rate: null, channels: null })
      }
      if (command === 'set_pocket_voice') return Promise.resolve(checked)
      return Promise.resolve()
    })

    const { host, activate } = await mountCard()
    modelRadios(host)[0].click()
    await flush()
    expect(host.textContent).toContain('Choose a WAV reference to make this model ready.')

    ;(host.querySelector('.pocket-choose-button') as HTMLButtonElement).click()
    await flush()
    expect(host.textContent).toContain('The WAV file cannot be read or contains no speech. Choose another file.')
    expect(invokeMock.mock.calls.some(([command]) => command === 'set_pocket_voice')).toBe(false)

    ;(host.querySelector('.pocket-choose-button') as HTMLButtonElement).click()
    await flush()

    expect(host.querySelector('.pocket-reference-name')?.textContent).toContain('good.wav')
    expect(host.textContent).toContain('Ready')
    expect(invokeMock.mock.calls.filter(([command]) => command === 'set_pocket_voice')).toHaveLength(1)
    expect(invokeMock.mock.calls.filter(([command]) => command === 'select_tts_provider_by_id')).toHaveLength(2)
    expect(activate).toHaveBeenCalledWith(checked.id)
  })

  it('detects a disappeared reference file and keeps the file name for replacement', async () => {
    const ready = shtorm({
      selection: { kind: 'reference_wav', path: 'C:\\Voices\\gone.wav', file_name: 'gone.wav' },
      readiness: 'ready',
      error_code: null,
    })
    const missing = {
      ...ready,
      readiness: 'reference_missing' as const,
      error_code: 'pocket.reference_missing',
    }
    let listCalls = 0
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_pocket_models') {
        const result = listCalls === 0 ? [ready] : [missing]
        listCalls += 1
        return Promise.resolve(result)
      }
      if (command === 'stop_preview') return Promise.resolve()
      if (command === 'preview_audio_file') {
        return Promise.reject({
          code: 'pocket.reference_missing',
          message: 'missing',
          retryable: false,
        })
      }
      return Promise.resolve()
    })

    const { host } = await mountCard(ready.id)
    ;(host.querySelector('.pocket-listen-button') as HTMLButtonElement).click()
    await flush()
    await vi.waitFor(() => {
      if (!host.textContent?.includes('Reference missing')) throw new Error('readiness did not refresh')
    })

    expect(invokeMock.mock.calls.filter(([command]) => command === 'preview_audio_file')).toHaveLength(1)
    expect(invokeMock.mock.calls.find(([command]) => command === 'preview_audio_file')?.[1]).toMatchObject({
      filePath: 'C:\\Voices\\gone.wav',
      voiceTransformEnabled: false,
      enhanceEnabled: false,
      dspSettings: null,
    })
    expect(host.textContent).toContain('The Pocket TTS reference file is missing. Choose the WAV again.')
    expect(host.querySelector('.pocket-reference-name')?.textContent).toContain('gone.wav')
  })

  it('serializes model activation while a selection is pending', async () => {
    const first = shtorm({ id: 'local-pocket:a', display_name: 'Model A' })
    const second = shtorm({ id: 'local-pocket:b', display_name: 'Model B' })
    const firstSelect = deferred<void>()
    const secondSelect = deferred<void>()
    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === 'list_pocket_models') return Promise.resolve([first, second])
      if (command === 'stop_preview') return Promise.resolve()
      if (command === 'select_tts_provider_by_id') {
        return args?.id === first.id ? firstSelect.promise : secondSelect.promise
      }
      return Promise.resolve()
    })

    const { host, activate } = await mountCard()
    const radios = modelRadios(host)
    radios[0].click()
    await flush()
    expect(radios[1].disabled).toBe(true)
    radios[1].click()
    expect(invokeMock).not.toHaveBeenCalledWith('select_tts_provider_by_id', { id: second.id })

    firstSelect.resolve()
    await flush()
    expect(modelRadios(host)[1].disabled).toBe(false)
    modelRadios(host)[1].click()
    await flush()
    secondSelect.resolve()
    await flush()

    expect(activate.mock.calls).toEqual([[first.id], [second.id]])
    expect(modelRadios(host)[0].checked).toBe(false)
    expect(modelRadios(host)[1].checked).toBe(true)
    expect(host.querySelector('.local-model-message--error')).toBeNull()
  })

  it('ignores a late reference response after switching to another model', async () => {
    const first = shtorm({ id: 'local-pocket:a', display_name: 'Model A' })
    const second = kyutai({ id: 'local-pocket:b', display_name: 'Model B' })
    const validate = deferred<Record<string, unknown>>()
    const checkedFirst = shtorm({
      id: first.id,
      display_name: first.display_name,
      selection: { kind: 'reference_wav', path: 'C:\\Voices\\me.wav', file_name: 'me.wav' },
      readiness: 'ready',
      error_code: null,
    })
    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === 'list_pocket_models') return Promise.resolve([first, second])
      if (command === 'stop_preview') return Promise.resolve()
      if (command === 'select_tts_provider_by_id') {
        return args?.id === first.id
          ? Promise.reject({ code: 'tts_provider.selection_failed', message: 'not ready', retryable: false })
          : Promise.resolve()
      }
      if (command === 'validate_pocket_reference') return validate.promise
      if (command === 'set_pocket_voice') return Promise.resolve(checkedFirst)
      return Promise.resolve()
    })
    openMock.mockResolvedValue('C:\\Voices\\me.wav')

    const { host } = await mountCard(second.id)
    modelRadios(host)[0].click()
    await flush()
    ;(host.querySelector('.pocket-choose-button') as HTMLButtonElement).click()
    await flush()
    expect(host.querySelector('.pocket-choose-button')).not.toBeNull()

    modelRadios(host)[1].click()
    await flush()
    await vi.waitFor(() => {
      if (!host.textContent?.includes('Alba')) throw new Error('second model details not shown')
    })

    validate.resolve({ status: 'ready', error_code: null, file_name: 'me.wav', duration_seconds: 5, sample_rate: 44100, channels: 1 })
    await flush()
    await vi.waitFor(() => {
      const rows = host.querySelectorAll('.local-model-row')
      if (!rows[0]?.textContent?.includes('Ready')) throw new Error('first row was not refreshed')
    })

    expect(host.textContent).toContain('Alba')
    expect(host.querySelector('.pocket-reference-name')).toBeNull()
    expect(host.querySelector('.local-model-message')).toBeNull()
  })

  it('shows the saved WAV duration and stops an active preview', async () => {
    const ready = shtorm({
      selection: { kind: 'reference_wav', path: 'sample.wav', file_name: 'sample.wav' },
      readiness: 'ready',
      error_code: null,
      reference_duration_seconds: 6.25,
    })
    const preview = deferred<void>()
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_pocket_models') return Promise.resolve([ready])
      if (command === 'preview_audio_file') return preview.promise
      return Promise.resolve()
    })

    const { host } = await mountCard(ready.id)
    const play = host.querySelector<HTMLButtonElement>('.pocket-listen-button')!
    const stop = host.querySelector<HTMLButtonElement>('.pocket-stop-button')!
    expect(host.querySelector('.pocket-reference-duration')?.textContent).toContain('6.25 s')
    expect(stop.disabled).toBe(true)

    play.click()
    await flush()
    expect(play.disabled).toBe(true)
    expect(stop.disabled).toBe(false)

    stop.click()
    preview.resolve()
    await flush()
    expect(invokeMock).toHaveBeenCalledWith('stop_preview')
    expect(stop.disabled).toBe(true)
    expect(play.disabled).toBe(false)
  })

  it('explains the duration limit when a WAV is too long', async () => {
    openMock.mockResolvedValue('long.wav')
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_pocket_models') return Promise.resolve([shtorm()])
      if (command === 'validate_pocket_reference') {
        return Promise.resolve({ status: 'reference_invalid', error_code: 'pocket.reference_too_long' })
      }
      return Promise.resolve()
    })

    const { host } = await mountCard()
    host.querySelector<HTMLButtonElement>('.pocket-choose-button')!.click()
    await flush()
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('The WAV must be no longer than 12 seconds.')
    expect(invokeMock.mock.calls.some(([command]) => command === 'set_pocket_voice')).toBe(false)
  })

  it('normalizes the selected reference and updates the card', async () => {
    const ready = shtorm({
      selection: { kind: 'reference_wav', path: 'C:\\Voices\\me.wav', file_name: 'me.wav' },
      readiness: 'ready',
      error_code: null,
    })
    const normalized = shtorm({
      selection: {
        kind: 'reference_wav',
        path: 'C:\\AppData\\ttsbard\\pocket-references\\shtorm.wav',
        file_name: 'shtorm.wav',
      },
      readiness: 'ready',
      error_code: null,
    })
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_pocket_models') return Promise.resolve([ready])
      if (command === 'normalize_pocket_reference') return Promise.resolve(normalized)
      return Promise.resolve()
    })

    const { host } = await mountCard(ready.id)
    const button = host.querySelector<HTMLButtonElement>('.pocket-normalize-button')!
    expect(button).not.toBeNull()
    expect(button.textContent).toContain('Normalize')
    expect(button.disabled).toBe(false)

    button.click()
    await flush()

    expect(invokeMock).toHaveBeenCalledWith('normalize_pocket_reference', { modelId: ready.id })
    expect(host.querySelector('.pocket-reference-name')?.textContent).toContain('shtorm.wav')
    expect(host.textContent).toContain('Reference prepared: 24 kHz, mono, normalized level.')
  })

  it('reports a failed reference normalization and keeps the previous file', async () => {
    const ready = shtorm({
      selection: { kind: 'reference_wav', path: 'C:\\Voices\\me.wav', file_name: 'me.wav' },
      readiness: 'ready',
      error_code: null,
    })
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_pocket_models') return Promise.resolve([ready])
      if (command === 'normalize_pocket_reference') {
        return Promise.reject(new Error('normalize failed'))
      }
      return Promise.resolve()
    })

    const { host } = await mountCard(ready.id)
    ;(host.querySelector('.pocket-normalize-button') as HTMLButtonElement).click()
    await flush()

    expect(host.querySelector('[role="alert"]')?.textContent)
      .toContain('Could not prepare the WAV reference')
    expect(host.querySelector('.pocket-reference-name')?.textContent).toContain('me.wav')
  })

  it('restores the Shtorm reference when switching RU → EN → RU', async () => {
    const russian = shtorm({
      selection: { kind: 'reference_wav', path: 'C:\\Voices\\me.wav', file_name: 'me.wav' },
      readiness: 'ready',
      error_code: null,
    })
    const english = kyutai()
    installList([russian, english])
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_pocket_models') return Promise.resolve([russian, english])
      if (command === 'stop_preview') return Promise.resolve()
      return Promise.resolve()
    })

    const { host, activate } = await mountCard(russian.id)
    expect(host.querySelector('.pocket-reference-name')?.textContent).toContain('me.wav')

    modelRadios(host)[1].click()
    await flush()
    await vi.waitFor(() => {
      if (!host.textContent?.includes('Alba')) throw new Error('Khutai details not shown')
    })

    modelRadios(host)[0].click()
    await flush()
    await vi.waitFor(() => {
      if (!host.textContent?.includes('me.wav')) throw new Error('Shtorm reference not restored')
    })

    expect(activate).toHaveBeenNthCalledWith(1, english.id)
    expect(activate).toHaveBeenNthCalledWith(2, russian.id)
  })
})
