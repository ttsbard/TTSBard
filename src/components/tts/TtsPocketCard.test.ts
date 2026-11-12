// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick } from 'vue'
import type { PocketModelDto } from './pocketTts'

const { invokeMock, openMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  openMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
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
  const expand = host.querySelector('.expand-icon') as HTMLElement | null
  if (expand) expand.click()
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

  it('lists a discovered model with its language and needs-reference state', async () => {
    installList([shtorm()])
    const { host } = await mountCard()

    expect(modelRadios(host)).toHaveLength(1)
    expect(host.querySelector('.pocket-model-language')?.textContent).toContain('RU')
    expect(host.textContent).toContain('Shtorm RU v2 Fast')
    expect(host.textContent).toContain('WAV needed')
    expect(host.querySelector('.pocket-choose-button')).not.toBeNull()
    expect(host.querySelector('.pocket-reference-name')).toBeNull()

    const testButton = host.querySelector('.pocket-test-button') as HTMLButtonElement
    expect(testButton.disabled).toBe(true)
    expect(host.textContent).toContain('The synthesis check becomes available when the model is ready.')
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
    expect((host.querySelector('.pocket-test-button') as HTMLButtonElement).disabled).toBe(true)
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
    expect(host.querySelector('.pocket-message--error')).toBeNull()
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
      const rows = host.querySelectorAll('.pocket-model-row')
      if (!rows[0]?.textContent?.includes('Ready')) throw new Error('first row was not refreshed')
    })

    expect(host.textContent).toContain('Alba')
    expect(host.querySelector('.pocket-reference-name')).toBeNull()
    expect(host.querySelector('.pocket-message')).toBeNull()
  })

  it('selects a ready non-active model before running the synthesis check', async () => {
    const ready = shtorm({
      selection: { kind: 'reference_wav', path: 'C:\\Voices\\me.wav', file_name: 'me.wav' },
      readiness: 'ready',
      error_code: null,
    })
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_pocket_models') return Promise.resolve([ready])
      if (command === 'select_tts_provider_by_id') return Promise.resolve()
      if (command === 'stop_preview') return Promise.resolve()
      if (command === 'submit_speech') return Promise.resolve({ job_id: 'job-1' })
      return Promise.resolve()
    })

    const { host, activate } = await mountCard()
    expect(host.textContent).toContain('The model will be selected for the synthesis check.')

    const testButton = host.querySelector('.pocket-test-button') as HTMLButtonElement
    expect(testButton.disabled).toBe(false)
    testButton.click()
    await flush()

    const selectCalls = invokeMock.mock.calls.filter(([command]) => command === 'select_tts_provider_by_id')
    expect(selectCalls).toHaveLength(1)
    expect(selectCalls[0][1]).toEqual({ id: ready.id })
    expect(activate).toHaveBeenCalledWith(ready.id)
    const speechCalls = invokeMock.mock.calls.filter(([command]) => command === 'submit_speech')
    expect(speechCalls).toHaveLength(1)
    expect(String(speechCalls[0][1]?.text)).toMatch(/[А-Яа-яЁё]/)
  })

  it('shows Alba and runs a new English synthesis phrase for the active Kyutai model', async () => {
    const model = kyutai()
    installList([model])
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_pocket_models') return Promise.resolve([model])
      if (command === 'stop_preview') return Promise.resolve()
      if (command === 'submit_speech') return Promise.resolve({ job_id: 'job-1' })
      return Promise.resolve()
    })

    const { host } = await mountCard(model.id)
    expect(host.textContent).toContain('Alba')
    expect(host.textContent).toContain('Ready')
    expect(host.querySelector('.pocket-choose-button')).toBeNull()

    const testButton = host.querySelector('.pocket-test-button') as HTMLButtonElement
    expect(testButton.disabled).toBe(false)

    testButton.click()
    await flush()
    testButton.click()
    await flush()

    const speechCalls = invokeMock.mock.calls.filter(([command]) => command === 'submit_speech')
    expect(speechCalls).toHaveLength(2)
    const firstPhrase = String(speechCalls[0][1]?.text)
    const secondPhrase = String(speechCalls[1][1]?.text)
    expect(firstPhrase).toMatch(/[A-Za-z]/)
    expect(secondPhrase).toMatch(/[A-Za-z]/)
    expect(firstPhrase).not.toBe(secondPhrase)
    expect(host.textContent).toContain('Synthesis check started:')
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
