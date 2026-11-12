import { describe, it, expect, vi, beforeEach, beforeAll } from 'vitest'
import type { JobDto } from '../../src-playback/speechQueue'

vi.stubGlobal('window', globalThis)

const mocks = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
  listenCallbacks: new Map<string, (event: { payload: unknown }) => void>(),
  unlistenFns: new Map<string, () => void>(),
  mockDebugError: vi.fn(),
  mockShowError: vi.fn(),
}))

let capturedOnMountedCb: (() => void) | null = null
let capturedOnUnmountedCb: (() => void) | null = null

vi.mock('vue', async () => {
  const actual = await vi.importActual<typeof import('vue')>('vue')
  return {
    ...actual,
    onMounted: (cb: () => void) => { capturedOnMountedCb = cb },
    onUnmounted: (cb: () => void) => { capturedOnUnmountedCb = cb },
  }
})

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mocks.mockInvoke,
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, callback: (event: { payload: unknown }) => void) => {
    mocks.listenCallbacks.set(event, callback)
    const unlisten = vi.fn(() => { mocks.listenCallbacks.delete(event) })
    mocks.unlistenFns.set(event, unlisten)
    return Promise.resolve(unlisten)
  }),
}))

vi.mock('../utils/debug', () => ({
  debugError: mocks.mockDebugError,
  debugLog: vi.fn(),
}))

vi.mock('./useErrorHandler', () => ({
  useErrorHandler: () => ({ showError: mocks.mockShowError }),
}))

import {
  useIncomingTexts,
  isIncomingTextItem,
  isIncomingTextList,
  isActiveExternalJob,
  selectActiveExternalJobs,
  computeIncomingCount,
  ACTIVE_EXTERNAL_JOB_STATUSES,
} from './useIncomingTexts'
import { i18n } from '../i18n'
import ruCatalog from '../../locales/ru.json'

beforeAll(() => {
  i18n.global.setLocaleMessage('ru', (ruCatalog as { messages: Record<string, string> }).messages)
  ;(i18n.global.locale as unknown as { value: string }).value = 'ru'
})

function makeJob(overrides: Partial<JobDto> = {}): JobDto {
  return {
    job_id: 'job-1',
    original_text: 'hello',
    spoken_text: null,
    status: 'queued',
    error: null,
    attempt: 1,
    created_at_ms: 1000,
    last_activity_at_ms: 1000,
    source: 'server',
    ...overrides,
  }
}

function makeDto(jobs: JobDto[] = []) {
  return { jobs }
}

function defaultInvoke() {
  mocks.mockInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'list_incoming_texts') return []
    if (cmd === 'get_speech_queue_state') return { jobs: [] }
    if (cmd === 'get_incoming_settings') return { auto_play: true }
    return undefined
  })
}

async function setupAndMount(invokeImpl?: (cmd: string) => Promise<unknown>) {
  if (invokeImpl) {
    mocks.mockInvoke.mockImplementation(invokeImpl)
  } else {
    defaultInvoke()
  }
  const composable = useIncomingTexts()
  if (capturedOnMountedCb) {
    await capturedOnMountedCb()
  }
  return composable
}

describe('pure helpers', () => {
  it('validates incoming text items', () => {
    expect(isIncomingTextItem({ id: 'a', text: 'hello', source: 'server' })).toBe(true)
    expect(isIncomingTextItem({ id: 'a', text: 'hello', source: 'ocr' })).toBe(true)
    expect(isIncomingTextItem({ id: 'a', text: 'hello' })).toBe(false)
    expect(isIncomingTextItem({ id: 'a', text: 'hello', source: 'editor' })).toBe(false)
    expect(isIncomingTextItem({ id: 'a', text: 'hello', source: 'external' })).toBe(false)
    expect(isIncomingTextItem({ id: 'a', text: 42, source: 'server' })).toBe(false)
    expect(isIncomingTextItem({ id: 1, text: 'hello', source: 'server' })).toBe(false)
    expect(isIncomingTextItem(null)).toBe(false)
  })

  it('validates incoming text lists', () => {
    expect(isIncomingTextList([{ id: 'a', text: 'x', source: 'ocr' }])).toBe(true)
    expect(isIncomingTextList([])).toBe(true)
    expect(isIncomingTextList([{ id: 'a', text: 'x' }])).toBe(false)
    expect(isIncomingTextList([{ id: 'a', text: 'x', source: 'editor' }])).toBe(false)
    expect(isIncomingTextList({ id: 'a', text: 'x', source: 'server' })).toBe(false)
    expect(isIncomingTextList(null)).toBe(false)
  })

  it('counts active incoming jobs from both server and ocr sources', () => {
    expect(isActiveExternalJob(makeJob({ source: 'server', status: 'queued' }))).toBe(true)
    expect(isActiveExternalJob(makeJob({ source: 'ocr', status: 'queued' }))).toBe(true)
    expect(isActiveExternalJob(makeJob({ source: 'server', status: 'generating' }))).toBe(true)
    expect(isActiveExternalJob(makeJob({ source: 'server', status: 'ready' }))).toBe(true)
    expect(isActiveExternalJob(makeJob({ source: 'server', status: 'playing' }))).toBe(true)
    expect(isActiveExternalJob(makeJob({ source: 'server', status: 'failed' }))).toBe(true)
    expect(isActiveExternalJob(makeJob({ source: 'server', status: 'completed' }))).toBe(false)
    expect(isActiveExternalJob(makeJob({ source: 'server', status: 'cancelled' }))).toBe(false)
    expect(isActiveExternalJob(makeJob({ source: 'ocr', status: 'playing' }))).toBe(true)
    expect(isActiveExternalJob(makeJob({ source: 'editor', status: 'queued' }))).toBe(false)
    expect(isActiveExternalJob(makeJob({ source: 'external' as JobDto['source'], status: 'queued' }))).toBe(false)
  })

  it('exposes the expected active status set', () => {
    expect([...ACTIVE_EXTERNAL_JOB_STATUSES].sort()).toEqual(
      ['failed', 'generating', 'playing', 'queued', 'ready'].sort(),
    )
  })

  it('selects active server and ocr jobs while excluding editor and terminal jobs', () => {
    const dto = makeDto([
      makeJob({ job_id: 'server-active', source: 'server' }),
      makeJob({ job_id: 'ocr-active', source: 'ocr' }),
      makeJob({ job_id: 'server-done', source: 'server', status: 'completed' }),
      makeJob({ job_id: 'server-cancelled', source: 'server', status: 'cancelled' }),
      makeJob({ job_id: 'editor', source: 'editor', status: 'queued' }),
    ])
    const result = selectActiveExternalJobs(dto)
    expect(result.map((j) => j.job_id)).toEqual(['server-active', 'ocr-active'])
  })

  it('returns an empty list for an invalid queue payload', () => {
    expect(selectActiveExternalJobs(null)).toEqual([])
    expect(selectActiveExternalJobs({ jobs: 'nope' })).toEqual([])
    expect(selectActiveExternalJobs({})).toEqual([])
  })

  it('computes the incoming count as pending plus active server/ocr jobs', () => {
    const pending = [
      { id: 'a', text: 'one', source: 'server' as const },
      { id: 'b', text: 'two', source: 'ocr' as const },
    ]
    const external = [makeJob()]
    expect(computeIncomingCount(pending, external)).toBe(3)
    expect(computeIncomingCount([], [])).toBe(0)
  })
})

describe('useIncomingTexts', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.listenCallbacks.clear()
    mocks.unlistenFns.clear()
    capturedOnMountedCb = null
    capturedOnUnmountedCb = null
    mocks.mockShowError.mockReturnValue('')
  })

  it('starts empty with auto-play enabled', () => {
    const { pendingItems, externalJobs, autoPlay, count } = useIncomingTexts()
    expect(pendingItems.value).toEqual([])
    expect(externalJobs.value).toEqual([])
    expect(autoPlay.value).toBe(true)
    expect(count.value).toBe(0)
  })

  it('loads the initial snapshot on mount', async () => {
    const { pendingItems, externalJobs, autoPlay, count } = await setupAndMount(async (cmd: string) => {
      if (cmd === 'list_incoming_texts') {
        return [
          { id: 'a', text: 'one', source: 'server' },
          { id: 'b', text: 'two', source: 'ocr' },
        ]
      }
      if (cmd === 'get_speech_queue_state') return makeDto([makeJob()])
      if (cmd === 'get_incoming_settings') return { auto_play: false }
      return undefined
    })

    expect(pendingItems.value).toEqual([
      { id: 'a', text: 'one', source: 'server' },
      { id: 'b', text: 'two', source: 'ocr' },
    ])
    expect(externalJobs.value).toHaveLength(1)
    expect(autoPlay.value).toBe(false)
    expect(count.value).toBe(3)
  })

  it('updates pending items on the incoming-changed event and ignores invalid payloads', async () => {
    const { pendingItems, count } = await setupAndMount()

    const callback = mocks.listenCallbacks.get('input-server-incoming-changed')
    expect(callback).toBeDefined()
    callback?.({
      payload: [
        { id: 'a', text: 'one', source: 'server' },
        { id: 'b', text: 'two', source: 'ocr' },
      ],
    })
    expect(pendingItems.value).toHaveLength(2)
    expect(count.value).toBe(2)

    callback?.({ payload: { not: 'a list' } })
    expect(pendingItems.value).toHaveLength(2)

    callback?.({ payload: [{ id: 1, text: 'bad', source: 'server' }] })
    expect(pendingItems.value).toHaveLength(2)

    callback?.({ payload: [{ id: 'c', text: 'missing source' }] })
    expect(pendingItems.value).toHaveLength(2)
  })

  it('updates external jobs on the queue-changed event, selecting server/ocr and filtering editor and terminal jobs', async () => {
    const { externalJobs } = await setupAndMount()

    const callback = mocks.listenCallbacks.get('speech-queue-changed')
    expect(callback).toBeDefined()
    callback?.({
      payload: makeDto([
        makeJob({ job_id: 'server-active', source: 'server' }),
        makeJob({ job_id: 'ocr-active', source: 'ocr' }),
        makeJob({ job_id: 'done', status: 'completed' }),
        makeJob({ job_id: 'editor', source: 'editor' }),
      ]),
    })
    expect(externalJobs.value.map((j) => j.job_id)).toEqual(['server-active', 'ocr-active'])

    callback?.({ payload: { jobs: 'invalid' } })
    expect(externalJobs.value.map((j) => j.job_id)).toEqual(['server-active', 'ocr-active'])
  })

  it('refreshes auto-play from incoming settings on the global settings-changed event', async () => {
    const { autoPlay } = await setupAndMount()

    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_incoming_settings') return { auto_play: false }
      return undefined
    })

    const callback = mocks.listenCallbacks.get('settings-changed')
    expect(callback).toBeDefined()
    callback?.({ payload: undefined })

    await vi.waitFor(() => expect(autoPlay.value).toBe(false))
  })

  it('auto-play toggle never touches input server settings', async () => {
    const { autoPlay, setAutoPlay } = await setupAndMount()

    mocks.mockInvoke.mockClear()
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_incoming_settings') return { auto_play: false }
      return undefined
    })

    await setAutoPlay(false)

    expect(mocks.mockInvoke).toHaveBeenCalledWith('save_incoming_settings', {
      settings: { auto_play: false, route: 'audio_only' },
    })
    expect(autoPlay.value).toBe(false)
    expect(mocks.mockInvoke.mock.calls.some(([cmd]) => cmd === 'get_input_server_settings')).toBe(false)
    expect(mocks.mockInvoke.mock.calls.some(([cmd]) => cmd === 'save_input_server_settings')).toBe(false)
  })

  it('approves an item via the matching backend command', async () => {
    const { approve } = await setupAndMount()
    await approve('item-1')
    expect(mocks.mockInvoke).toHaveBeenCalledWith('approve_incoming_text', { incomingId: 'item-1' })
  })

  it('discards an item via the matching backend command', async () => {
    const { discard } = await setupAndMount()
    await discard('item-1')
    expect(mocks.mockInvoke).toHaveBeenCalledWith('discard_incoming_text', { incomingId: 'item-1' })
  })

  it('edit returns the text for the caller to route into a new tab', async () => {
    const { edit } = await setupAndMount()
    mocks.mockInvoke.mockResolvedValueOnce({ id: 'item-1', text: 'external text', source: 'server' })

    const text = await edit('item-1')

    expect(mocks.mockInvoke).toHaveBeenCalledWith('take_incoming_text_for_edit', { incomingId: 'item-1' })
    expect(text).toBe('external text')
  })

  it('edit surfaces a user-visible error and returns null on failure', async () => {
    const { edit } = await setupAndMount()
    mocks.mockInvoke.mockRejectedValueOnce(new Error('gone'))

    const text = await edit('item-1')

    expect(text).toBeNull()
    expect(mocks.mockShowError).toHaveBeenCalled()
  })

  it('edit shows the specific message for an unknown-item rejection', async () => {
    const { edit, pendingItems } = await setupAndMount()
    mocks.mockInvoke.mockRejectedValueOnce({
      code: 'input_server.unknown_item',
      message: 'unknown id',
      retryable: false,
    })

    const text = await edit('item-1')

    expect(text).toBeNull()
    expect(mocks.mockShowError).toHaveBeenCalledWith('Входящий текст уже обработан или отсутствует')
    expect(pendingItems.value).toEqual([])
  })

  it('approve and discard show the specific message for an unknown-item rejection', async () => {
    const { approve, discard } = await setupAndMount()
    const envelope = {
      code: 'input_server.unknown_item',
      message: 'unknown id',
      retryable: false,
    }
    mocks.mockInvoke.mockRejectedValueOnce(envelope)
    await approve('item-1')
    mocks.mockInvoke.mockRejectedValueOnce(envelope)
    await discard('item-1')

    expect(mocks.mockShowError).toHaveBeenCalledTimes(2)
    expect(mocks.mockShowError).toHaveBeenCalledWith('Входящий текст уже обработан или отсутствует')
  })

  it('edit keeps the generic message for non-envelope rejections', async () => {
    const { edit } = await setupAndMount()
    mocks.mockInvoke.mockRejectedValueOnce(new Error('boom'))
    await edit('item-1')
    expect(mocks.mockShowError).toHaveBeenCalledWith('Не удалось взять текст для редактирования')

    mocks.mockShowError.mockClear()
    mocks.mockInvoke.mockRejectedValueOnce('string rejection')
    await edit('item-1')
    expect(mocks.mockShowError).toHaveBeenCalledWith('Не удалось взять текст для редактирования')
  })

  it('guards actions while in flight', async () => {
    const { edit, busyIds } = await setupAndMount()

    let resolveEdit!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'take_incoming_text_for_edit') {
        return new Promise((resolve) => { resolveEdit = resolve })
      }
      return Promise.resolve(undefined)
    })

    const first = edit('item-1')
    await Promise.resolve()

    // Re-entrant call while the first is still in flight is ignored.
    const second = await edit('item-1')
    expect(second).toBeNull()
    expect(busyIds.value.has('item-1')).toBe(true)

    resolveEdit({ id: 'item-1', text: 'text', source: 'server' })
    expect(await first).toBe('text')
    expect(busyIds.value.has('item-1')).toBe(false)
  })

  it('exposes a single take-in-flight flag while the take is pending', async () => {
    const { edit, editInFlight } = await setupAndMount()

    let resolveEdit!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'take_incoming_text_for_edit') {
        return new Promise((resolve) => { resolveEdit = resolve })
      }
      return Promise.resolve(undefined)
    })

    expect(editInFlight.value).toBe(false)
    const pending = edit('item-1')
    await Promise.resolve()
    expect(editInFlight.value).toBe(true)

    resolveEdit({ id: 'item-1', text: 'ok', source: 'server' })
    expect(await pending).toBe('ok')
    expect(editInFlight.value).toBe(false)
  })

  it('treats a malformed take payload like a command failure without returning untrusted text', async () => {
    const { edit, editInFlight } = await setupAndMount()

    const malformed: unknown[] = [
      { id: 'item-1', text: 42, source: 'server' },
      { id: 'item-1', text: 'x', source: 'editor' },
      { id: 'item-1' },
      null,
      undefined,
      'not-an-item',
    ]

    for (const payload of malformed) {
      mocks.mockShowError.mockClear()
      mocks.mockInvoke.mockResolvedValueOnce(payload)

      expect(await edit('item-1')).toBeNull()
      expect(mocks.mockShowError).toHaveBeenCalledWith('Не удалось взять текст для редактирования')
      expect(editInFlight.value).toBe(false)
    }
  })

  it('releases the guard after a malformed payload so a later edit is allowed', async () => {
    const { edit } = await setupAndMount()

    mocks.mockInvoke.mockResolvedValueOnce({ id: 'item-1', text: 42, source: 'server' })
    expect(await edit('item-a')).toBeNull()

    mocks.mockInvoke.mockResolvedValueOnce({ id: 'item-b', text: 'recovered', source: 'server' })
    expect(await edit('item-b')).toBe('recovered')

    const takeCalls = mocks.mockInvoke.mock.calls.filter(
      ([cmd]) => cmd === 'take_incoming_text_for_edit',
    )
    expect(takeCalls).toHaveLength(2)
  })

  it('ignores a take that resolves after unmount', async () => {
    defaultInvoke()
    const { edit, editInFlight } = useIncomingTexts()

    let resolveEdit!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'take_incoming_text_for_edit') {
        return new Promise((resolve) => { resolveEdit = resolve })
      }
      return Promise.resolve(undefined)
    })

    const pending = edit('item-1')
    await Promise.resolve()
    capturedOnUnmountedCb?.()
    resolveEdit({ id: 'item-1', text: 'late text' })

    expect(await pending).toBeNull()
    expect(editInFlight.value).toBe(false)
  })

  it('does not surface a take error after unmount', async () => {
    defaultInvoke()
    const { edit } = useIncomingTexts()

    let rejectEdit!: (e: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'take_incoming_text_for_edit') {
        return new Promise((_resolve, reject) => { rejectEdit = reject })
      }
      return Promise.resolve(undefined)
    })

    const pending = edit('item-1')
    await Promise.resolve()
    capturedOnUnmountedCb?.()
    rejectEdit(new Error('late failure'))

    expect(await pending).toBeNull()
    expect(mocks.mockShowError).not.toHaveBeenCalled()
  })

  it('persists auto-play toggles through the source-neutral incoming settings command', async () => {
    const { autoPlay, setAutoPlay } = await setupAndMount()

    mocks.mockInvoke.mockClear()
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'save_incoming_settings') return undefined
      return undefined
    })

    await expect(setAutoPlay(false)).resolves.toBeUndefined()

    expect(mocks.mockInvoke).toHaveBeenCalledWith('save_incoming_settings', {
      settings: { auto_play: false, route: 'audio_only' },
    })
    expect(autoPlay.value).toBe(false)
  })

  it('ignores a duplicate auto-play toggle for the already-active value', async () => {
    const { setAutoPlay } = await setupAndMount()
    mocks.mockInvoke.mockClear()

    await setAutoPlay(true)

    expect(mocks.mockInvoke).not.toHaveBeenCalledWith('save_incoming_settings')
  })

  it('sends only one save when the same toggle fires repeatedly', async () => {
    const { setAutoPlay } = await setupAndMount()

    await setAutoPlay(false)
    await setAutoPlay(false)

    const saveCalls = mocks.mockInvoke.mock.calls.filter((call) => call[0] === 'save_incoming_settings')
    expect(saveCalls).toHaveLength(1)
  })

  it('reverts auto-play when the incoming settings save is rejected', async () => {
    const { autoPlay, setAutoPlay } = await setupAndMount()
    mocks.mockInvoke.mockRejectedValueOnce(new Error('backend down'))

    await setAutoPlay(false)

    expect(autoPlay.value).toBe(true)
    expect(mocks.mockShowError).toHaveBeenCalled()
  })

  it('keeps the latest event when it arrives during a pending-items snapshot', async () => {
    const { pendingItems, refreshPendingItems } = await setupAndMount()

    mocks.mockInvoke.mockClear()
    let resolveList!: (value: unknown) => void
    mocks.mockInvoke.mockImplementationOnce(() => {
      return new Promise((resolve) => { resolveList = resolve })
    })

    const pending = refreshPendingItems()

    mocks.listenCallbacks.get('input-server-incoming-changed')?.({
      payload: [{ id: 'new', text: 'new', source: 'server' }],
    })

    resolveList([{ id: 'old', text: 'old', source: 'server' }])
    await pending

    expect(pendingItems.value).toEqual([{ id: 'new', text: 'new', source: 'server' }])
  })

  it('keeps the latest event when it arrives during an external-jobs snapshot', async () => {
    const { externalJobs, refreshExternalJobs } = await setupAndMount()

    mocks.mockInvoke.mockClear()
    let resolveQueue!: (value: unknown) => void
    mocks.mockInvoke.mockImplementationOnce(() => {
      return new Promise((resolve) => { resolveQueue = resolve })
    })

    const pending = refreshExternalJobs()

    mocks.listenCallbacks.get('speech-queue-changed')?.({
      payload: makeDto([makeJob({ job_id: 'new', source: 'server' })]),
    })

    resolveQueue(makeDto([makeJob({ job_id: 'old', source: 'server' })]))
    await pending

    expect(externalJobs.value.map((j) => j.job_id)).toEqual(['new'])
  })

  it('sets loadError and keeps the last valid list on an invalid snapshot payload', async () => {
    const { pendingItems, loadError, refreshPendingItems } = await setupAndMount(
      async (cmd: string) => {
        if (cmd === 'list_incoming_texts') {
          return [{ id: 'a', text: 'one', source: 'server' }]
        }
        if (cmd === 'get_speech_queue_state') return makeDto([])
        if (cmd === 'get_incoming_settings') return { auto_play: true }
        return undefined
      },
    )

    expect(pendingItems.value).toEqual([{ id: 'a', text: 'one', source: 'server' }])
    expect(loadError.value).toBeNull()

    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_incoming_texts') return { not: 'a list' }
      return undefined
    })

    await refreshPendingItems()

    expect(pendingItems.value).toEqual([{ id: 'a', text: 'one', source: 'server' }])
    expect(loadError.value).toBe('Не удалось загрузить входящие')
  })

  it('clears loadError when a rejected snapshot is followed by a valid snapshot', async () => {
    const { pendingItems, loadError, refreshPendingItems } = await setupAndMount(
      async (cmd: string) => {
        if (cmd === 'list_incoming_texts') {
          throw new Error('backend down')
        }
        if (cmd === 'get_speech_queue_state') return makeDto([])
        if (cmd === 'get_incoming_settings') return { auto_play: true }
        return undefined
      },
    )

    expect(loadError.value).toBe('Не удалось загрузить входящие')

    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_incoming_texts') {
        return [{ id: 'a', text: 'one', source: 'server' }]
      }
      return undefined
    })

    await refreshPendingItems()

    expect(pendingItems.value).toEqual([{ id: 'a', text: 'one', source: 'server' }])
    expect(loadError.value).toBeNull()
  })

  it('clears loadError when a rejected snapshot is followed by a valid incoming-changed event', async () => {
    const { pendingItems, loadError } = await setupAndMount(
      async (cmd: string) => {
        if (cmd === 'list_incoming_texts') {
          throw new Error('backend down')
        }
        if (cmd === 'get_speech_queue_state') return makeDto([])
        if (cmd === 'get_incoming_settings') return { auto_play: true }
        return undefined
      },
    )

    expect(loadError.value).toBe('Не удалось загрузить входящие')

    mocks.listenCallbacks.get('input-server-incoming-changed')?.({
      payload: [{ id: 'a', text: 'one', source: 'server' }],
    })

    expect(pendingItems.value).toEqual([{ id: 'a', text: 'one', source: 'server' }])
    expect(loadError.value).toBeNull()
  })

  it('keeps loadError when a rejected snapshot is followed by an invalid incoming-changed event', async () => {
    const { pendingItems, loadError } = await setupAndMount(
      async (cmd: string) => {
        if (cmd === 'list_incoming_texts') {
          throw new Error('backend down')
        }
        if (cmd === 'get_speech_queue_state') return makeDto([])
        if (cmd === 'get_incoming_settings') return { auto_play: true }
        return undefined
      },
    )

    expect(loadError.value).toBe('Не удалось загрузить входящие')

    mocks.listenCallbacks.get('input-server-incoming-changed')?.({
      payload: { not: 'a list' },
    })

    expect(pendingItems.value).toEqual([])
    expect(loadError.value).toBe('Не удалось загрузить входящие')
  })

  it('skip removes a failed external job from the active list', async () => {
    const { externalJobs, busyIds, skipExternalJob } = await setupAndMount(
      async (cmd: string) => {
        if (cmd === 'list_incoming_texts') return []
        if (cmd === 'get_speech_queue_state') {
          return makeDto([makeJob({ job_id: 'failed-job', status: 'failed', source: 'server' })])
        }
        if (cmd === 'get_incoming_settings') return { auto_play: true }
        return undefined
      },
    )

    expect(externalJobs.value.map((j) => j.job_id)).toEqual(['failed-job'])

    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'skip_speech_job') return undefined
      if (cmd === 'get_speech_queue_state') return makeDto([])
      return undefined
    })

    await skipExternalJob('failed-job')

    expect(mocks.mockInvoke).toHaveBeenCalledWith('skip_speech_job', { jobId: 'failed-job' })
    expect(externalJobs.value).toEqual([])
    expect(busyIds.value.has('failed-job')).toBe(false)
  })

  it('cancels failed and queued external jobs and drops them from the active list', async () => {
    let queue: JobDto[] = [
      makeJob({ job_id: 'failed-job', status: 'failed', source: 'server' }),
      makeJob({ job_id: 'queued-job', status: 'queued', source: 'server' }),
    ]
    const { externalJobs, busyIds, cancelExternalJob } = await setupAndMount(
      async (cmd: string) => {
        if (cmd === 'list_incoming_texts') return []
        if (cmd === 'get_speech_queue_state') return makeDto(queue)
        if (cmd === 'get_incoming_settings') return { auto_play: true }
        return undefined
      },
    )

    expect(externalJobs.value.map((j) => j.job_id)).toEqual(['failed-job', 'queued-job'])

    mocks.mockInvoke.mockClear()
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'cancel_speech_job') return undefined
      if (cmd === 'get_speech_queue_state') return makeDto(queue)
      return undefined
    })

    queue = queue.slice(1)
    await cancelExternalJob('failed-job')

    expect(mocks.mockInvoke).toHaveBeenCalledWith('cancel_speech_job', { jobId: 'failed-job' })
    expect(externalJobs.value.map((j) => j.job_id)).toEqual(['queued-job'])
    expect(busyIds.value.has('failed-job')).toBe(false)

    queue = []
    await cancelExternalJob('queued-job')

    expect(mocks.mockInvoke).toHaveBeenCalledWith('cancel_speech_job', { jobId: 'queued-job' })
    expect(externalJobs.value).toEqual([])
    expect(busyIds.value.has('queued-job')).toBe(false)
  })

  it('cancelExternalJob surfaces a user-visible error and releases busy state on IPC failure', async () => {
    const { busyIds, cancelExternalJob } = await setupAndMount(async (cmd: string) => {
      if (cmd === 'list_incoming_texts') return []
      if (cmd === 'get_speech_queue_state') {
        return makeDto([makeJob({ job_id: 'failed-job', status: 'failed', source: 'server' })])
      }
      if (cmd === 'get_incoming_settings') return { auto_play: true }
      return undefined
    })

    let rejectCancel!: (e: unknown) => void
    mocks.mockInvoke.mockClear()
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'cancel_speech_job') {
        return new Promise((_resolve, reject) => { rejectCancel = reject })
      }
      return Promise.resolve(undefined)
    })

    const pending = cancelExternalJob('failed-job')
    await Promise.resolve()
    expect(busyIds.value.has('failed-job')).toBe(true)

    rejectCancel(new Error('backend down'))
    await pending

    expect(mocks.mockShowError).toHaveBeenCalledWith('Не удалось отклонить текст')
    expect(busyIds.value.has('failed-job')).toBe(false)
  })

  it('editExternalJob returns the original text of a failed job and drops it from the list', async () => {
    let queue: JobDto[] = [
      makeJob({ job_id: 'failed-job', status: 'failed', original_text: 'hello from server' }),
    ]
    const { externalJobs, busyIds, editInFlight, editExternalJob } = await setupAndMount(
      async (cmd: string) => {
        if (cmd === 'list_incoming_texts') return []
        if (cmd === 'get_speech_queue_state') return makeDto(queue)
        if (cmd === 'get_incoming_settings') return { auto_play: true }
        return undefined
      },
    )

    mocks.mockInvoke.mockClear()
    mocks.mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'cancel_speech_job') {
        queue = []
        return undefined
      }
      if (cmd === 'get_speech_queue_state') return makeDto(queue)
      return undefined
    })

    const text = await editExternalJob('failed-job')

    expect(text).toBe('hello from server')
    expect(mocks.mockInvoke).toHaveBeenCalledWith('cancel_speech_job', { jobId: 'failed-job' })
    expect(externalJobs.value).toEqual([])
    expect(editInFlight.value).toBe(false)
    expect(busyIds.value.has('failed-job')).toBe(false)
  })

  it('editExternalJob refuses non-failed and unknown jobs without cancelling them', async () => {
    const { busyIds, editInFlight, editExternalJob } = await setupAndMount(async (cmd: string) => {
      if (cmd === 'list_incoming_texts') return []
      if (cmd === 'get_speech_queue_state') {
        return makeDto([
          makeJob({ job_id: 'queued-job', status: 'queued', source: 'server' }),
          makeJob({ job_id: 'generating-job', status: 'generating', source: 'ocr' }),
          makeJob({ job_id: 'failed-job', status: 'failed', source: 'server' }),
        ])
      }
      if (cmd === 'get_incoming_settings') return { auto_play: true }
      return undefined
    })

    mocks.mockInvoke.mockClear()
    mocks.mockInvoke.mockImplementation(async () => undefined)

    for (const id of ['queued-job', 'generating-job', 'missing-job']) {
      expect(await editExternalJob(id)).toBeNull()
      expect(editInFlight.value).toBe(false)
      expect(busyIds.value.has(id)).toBe(false)
    }

    expect(mocks.mockInvoke.mock.calls.some(([cmd]) => cmd === 'cancel_speech_job')).toBe(false)
  })

  it('editExternalJob refuses a second take while one is already in flight', async () => {
    const { editInFlight, editExternalJob } = await setupAndMount(async (cmd: string) => {
      if (cmd === 'list_incoming_texts') return []
      if (cmd === 'get_speech_queue_state') {
        return makeDto([
          makeJob({ job_id: 'job-a', status: 'failed', original_text: 'first text' }),
          makeJob({ job_id: 'job-b', status: 'failed', original_text: 'second text' }),
        ])
      }
      if (cmd === 'get_incoming_settings') return { auto_play: true }
      return undefined
    })

    let resolveCancel!: (value: unknown) => void
    mocks.mockInvoke.mockClear()
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'cancel_speech_job') {
        return new Promise((resolve) => { resolveCancel = resolve })
      }
      if (cmd === 'get_speech_queue_state') return Promise.resolve(makeDto([]))
      return Promise.resolve(undefined)
    })

    const first = editExternalJob('job-a')
    await Promise.resolve()
    expect(editInFlight.value).toBe(true)

    expect(await editExternalJob('job-b')).toBeNull()

    const cancelCalls = mocks.mockInvoke.mock.calls.filter(([cmd]) => cmd === 'cancel_speech_job')
    expect(cancelCalls).toHaveLength(1)
    expect(cancelCalls[0]).toEqual(['cancel_speech_job', { jobId: 'job-a' }])

    resolveCancel(undefined)
    expect(await first).toBe('first text')
    expect(editInFlight.value).toBe(false)
  })

  it('editExternalJob returns null and reports the take error when the cancel fails', async () => {
    const { busyIds, editInFlight, editExternalJob } = await setupAndMount(async (cmd: string) => {
      if (cmd === 'list_incoming_texts') return []
      if (cmd === 'get_speech_queue_state') {
        return makeDto([makeJob({ job_id: 'failed-job', status: 'failed', source: 'server' })])
      }
      if (cmd === 'get_incoming_settings') return { auto_play: true }
      return undefined
    })

    mocks.mockInvoke.mockRejectedValueOnce(new Error('backend down'))

    expect(await editExternalJob('failed-job')).toBeNull()
    expect(mocks.mockShowError).toHaveBeenCalledWith('Не удалось взять текст для редактирования')
    expect(editInFlight.value).toBe(false)
    expect(busyIds.value.has('failed-job')).toBe(false)
  })

  it('unregisters all listeners on unmount', async () => {
    await setupAndMount()

    expect(mocks.unlistenFns.has('input-server-incoming-changed')).toBe(true)
    expect(mocks.unlistenFns.has('speech-queue-changed')).toBe(true)
    expect(mocks.unlistenFns.has('settings-changed')).toBe(true)

    capturedOnUnmountedCb?.()

    expect(mocks.unlistenFns.get('input-server-incoming-changed')).toHaveBeenCalled()
    expect(mocks.unlistenFns.get('speech-queue-changed')).toHaveBeenCalled()
    expect(mocks.unlistenFns.get('settings-changed')).toHaveBeenCalled()
  })

  it('ignores a stale snapshot that resolves after unmount', async () => {
    defaultInvoke()
    const { pendingItems, refreshPendingItems } = useIncomingTexts()

    let resolveList!: (value: unknown) => void
    mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
      if (cmd === 'list_incoming_texts') {
        return new Promise((resolve) => { resolveList = resolve })
      }
      return Promise.resolve(undefined)
    })

    const pending = refreshPendingItems()
    capturedOnUnmountedCb?.()
    resolveList([{ id: 'late', text: 'late' }])
    await pending

    expect(pendingItems.value).toEqual([])
  })

  it('defaults route to audio_only for an old backend returning only auto_play', async () => {
    const { settings } = await setupAndMount(async (cmd: string) => {
      if (cmd === 'list_incoming_texts') return []
      if (cmd === 'get_speech_queue_state') return { jobs: [] }
      if (cmd === 'get_incoming_settings') return { auto_play: false }
      return undefined
    })

    expect(settings.value).toEqual({ auto_play: false, route: 'audio_only' })
  })

  it('normalizes an invalid IPC route to audio_only without trusting the payload', async () => {
    for (const bad of [undefined, null, 42, 'twitch_only', 'no_twitch', 'AUDIO_TWITCH']) {
      const { settings } = await setupAndMount(async (cmd: string) => {
        if (cmd === 'list_incoming_texts') return []
        if (cmd === 'get_speech_queue_state') return { jobs: [] }
        if (cmd === 'get_incoming_settings') return { auto_play: true, route: bad }
        return undefined
      })

      expect(settings.value.route).toBe('audio_only')
    }
  })

  it('loads every valid incoming route from the backend', async () => {
    const routes = ['audio_only', 'audio_webview', 'audio_twitch', 'everywhere'] as const
    for (const route of routes) {
      const { settings } = await setupAndMount(async (cmd: string) => {
        if (cmd === 'list_incoming_texts') return []
        if (cmd === 'get_speech_queue_state') return { jobs: [] }
        if (cmd === 'get_incoming_settings') return { auto_play: true, route }
        return undefined
      })

      expect(settings.value.route).toBe(route)
    }
  })

  it('setRoute preserves auto_play and persists the complete snapshot', async () => {
    const { settings, setRoute } = await setupAndMount()

    mocks.mockInvoke.mockClear()
    await setRoute('everywhere')

    expect(mocks.mockInvoke).toHaveBeenCalledWith('save_incoming_settings', {
      settings: { auto_play: true, route: 'everywhere' },
    })
    expect(settings.value).toEqual({ auto_play: true, route: 'everywhere' })
  })

  it('setAutoPlay preserves the current route', async () => {
    const { settings, setAutoPlay } = await setupAndMount(async (cmd: string) => {
      if (cmd === 'list_incoming_texts') return []
      if (cmd === 'get_speech_queue_state') return { jobs: [] }
      if (cmd === 'get_incoming_settings') return { auto_play: true, route: 'audio_twitch' }
      return undefined
    })

    await setAutoPlay(false)

    expect(settings.value).toEqual({ auto_play: false, route: 'audio_twitch' })
  })

  it('reverts the complete snapshot when a route save is rejected', async () => {
    const { settings, setRoute } = await setupAndMount()

    mocks.mockInvoke.mockRejectedValueOnce(new Error('backend down'))

    await setRoute('everywhere')

    expect(settings.value).toEqual({ auto_play: true, route: 'audio_only' })
    expect(mocks.mockShowError).toHaveBeenCalled()
  })

  it('reverts the complete snapshot when an auto_play save is rejected', async () => {
    const { settings, setAutoPlay } = await setupAndMount(async (cmd: string) => {
      if (cmd === 'list_incoming_texts') return []
      if (cmd === 'get_speech_queue_state') return { jobs: [] }
      if (cmd === 'get_incoming_settings') return { auto_play: true, route: 'audio_webview' }
      return undefined
    })

    mocks.mockInvoke.mockRejectedValueOnce(new Error('backend down'))

    await setAutoPlay(false)

    expect(settings.value).toEqual({ auto_play: true, route: 'audio_webview' })
    expect(mocks.mockShowError).toHaveBeenCalled()
  })

  it('ignores a stale settings snapshot when a local route change happens during refresh', async () => {
    const { settings, setRoute, refreshSettings } = await setupAndMount()

    mocks.mockInvoke.mockClear()
    let resolveGet!: (value: unknown) => void
    mocks.mockInvoke.mockImplementationOnce((cmd: string) => {
      if (cmd === 'get_incoming_settings') {
        return new Promise((resolve) => { resolveGet = resolve })
      }
      return Promise.resolve(undefined)
    })

    const refresh = refreshSettings()
    await setRoute('everywhere')
    resolveGet({ auto_play: true, route: 'audio_only' })
    await refresh

    expect(settings.value).toEqual({ auto_play: true, route: 'everywhere' })
  })

  it('keeps the newer settings when overlapping refreshes resolve out of order', async () => {
    const { settings, refreshSettings } = await setupAndMount()

    const responses: Array<(value: unknown) => void> = []
    mocks.mockInvoke.mockClear()
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_incoming_settings') {
        return new Promise((resolve) => { responses.push(resolve) })
      }
      return Promise.resolve(undefined)
    })

    const older = refreshSettings()
    const newer = refreshSettings()
    expect(responses).toHaveLength(2)

    // Resolve the newer response first, then the older one: the older payload
    // must not overwrite the settings the newer refresh already applied.
    responses[1]({ auto_play: false, route: 'everywhere' })
    await newer
    expect(settings.value).toEqual({ auto_play: false, route: 'everywhere' })

    responses[0]({ auto_play: true, route: 'audio_only' })
    await older
    expect(settings.value).toEqual({ auto_play: false, route: 'everywhere' })
  })

  it('returns null for an edit of any other item while a take is in flight, without a second backend take', async () => {
    const { edit } = await setupAndMount()

    let resolveEdit!: (value: unknown) => void
    mocks.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'take_incoming_text_for_edit') {
        return new Promise((resolve) => { resolveEdit = resolve })
      }
      return Promise.resolve(undefined)
    })

    const first = edit('item-a')
    await Promise.resolve()

    // A different id is not per-item busy, but the destructive take is already
    // running: the call must be refused without invoking the backend.
    const second = await edit('item-b')
    expect(second).toBeNull()

    const takeCalls = mocks.mockInvoke.mock.calls.filter(
      ([cmd]) => cmd === 'take_incoming_text_for_edit',
    )
    expect(takeCalls).toHaveLength(1)

    // The refused call must not disturb the accepted request's result.
    resolveEdit({ id: 'item-a', text: 'first text', source: 'server' })
    expect(await first).toBe('first text')
  })

  it('releases the edit guard on rejection so a later edit is allowed', async () => {
    const { edit } = await setupAndMount()

    mocks.mockInvoke.mockRejectedValueOnce(new Error('backend down'))
    expect(await edit('item-a')).toBeNull()

    mocks.mockInvoke.mockResolvedValueOnce({ id: 'item-b', text: 'recovered text', source: 'server' })
    expect(await edit('item-b')).toBe('recovered text')

    const takeCalls = mocks.mockInvoke.mock.calls.filter(
      ([cmd]) => cmd === 'take_incoming_text_for_edit',
    )
    expect(takeCalls).toHaveLength(2)
  })

  describe('serialized incoming settings saves', () => {
    function deferred() {
      let resolve!: () => void
      let reject!: (e: unknown) => void
      const promise = new Promise<void>((res, rej) => { resolve = res; reject = rej })
      return { promise, resolve, reject }
    }

    it('writes rapid toggle+route changes in order with complete snapshots and ends on the latest intent', async () => {
      const { settings, setAutoPlay, setRoute } = await setupAndMount()

      const saves: Array<{ auto_play: boolean; route: string }> = []
      const first = deferred()
      const second = deferred()

      mocks.mockInvoke.mockClear()
      mocks.mockInvoke.mockImplementation(
        async (cmd: string, args?: { settings?: { auto_play: boolean; route: string } }) => {
          if (cmd === 'save_incoming_settings') {
            saves.push({ ...args!.settings! })
            return saves.length === 1 ? first.promise : second.promise
          }
          return undefined
        },
      )

      const p1 = setAutoPlay(false)
      const p2 = setRoute('everywhere')

      // The first write is dispatched immediately; the second is coalesced and
      // must not start while the first is in flight.
      await vi.waitFor(() => expect(saves).toHaveLength(1))
      expect(saves[0]).toEqual({ auto_play: false, route: 'audio_only' })

      first.resolve()
      await vi.waitFor(() => expect(saves).toHaveLength(2))
      expect(saves[1]).toEqual({ auto_play: false, route: 'everywhere' })

      second.resolve()
      await Promise.all([p1, p2])

      // Durable final state equals the latest user intent.
      expect(settings.value).toEqual({ auto_play: false, route: 'everywhere' })
    })

    it('does not roll back a newer intent when an older write fails', async () => {
      const { settings, setAutoPlay, setRoute } = await setupAndMount()

      const first = deferred()
      mocks.mockInvoke.mockClear()
      mocks.mockInvoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'save_incoming_settings') {
          const saveCalls = mocks.mockInvoke.mock.calls.filter(
            ([c]) => c === 'save_incoming_settings',
          ).length
          if (saveCalls === 1) return first.promise
          return undefined
        }
        return undefined
      })

      const p1 = setAutoPlay(false)
      await vi.waitFor(() => expect(mocks.mockInvoke).toHaveBeenCalledTimes(1))
      const p2 = setRoute('everywhere')

      // The older write fails while the newer intent is already queued.
      first.reject(new Error('backend down'))
      await vi.waitFor(() => expect(mocks.mockInvoke).toHaveBeenCalledTimes(2))

      await Promise.all([p1, p2])

      expect(settings.value).toEqual({ auto_play: false, route: 'everywhere' })
      expect(mocks.mockShowError).not.toHaveBeenCalled()
    })

    it('rolls back to the last persisted snapshot when the latest write fails', async () => {
      const { settings, setAutoPlay, setRoute } = await setupAndMount()

      mocks.mockInvoke.mockClear()
      mocks.mockInvoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'save_incoming_settings') {
          const saveCalls = mocks.mockInvoke.mock.calls.filter(
            ([c]) => c === 'save_incoming_settings',
          ).length
          if (saveCalls === 1) return undefined
          return Promise.reject(new Error('backend down'))
        }
        return undefined
      })

      await setAutoPlay(false)
      await setRoute('everywhere')

      expect(settings.value).toEqual({ auto_play: false, route: 'audio_only' })
      expect(mocks.mockShowError).toHaveBeenCalledWith('Не удалось сохранить настройки входящих')
    })
  })
})
