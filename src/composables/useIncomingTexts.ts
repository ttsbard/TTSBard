import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  isSpeechQueueStateDto,
  type JobDto,
  type JobStatus,
} from '../../src-playback/speechQueue'
import { useErrorHandler } from './useErrorHandler'
import { debugError } from '../utils/debug'
import { createAsyncCleanupScope } from '../utils/asyncCleanup'
import { normalizeCommandError } from '../ipc/commandError'
import { t } from '../i18n'
import { sanitizeIncomingRoute, type IncomingRoute } from '../components/editor/incomingRoute'

export type { IncomingRoute } from '../components/editor/incomingRoute'

/** Source-neutral Incoming policy persisted under the top-level `incoming` section. */
export interface IncomingSettings {
  auto_play: boolean
  route: IncomingRoute
}

function sanitizeIncomingSettings(value: unknown): IncomingSettings {
  const record = value && typeof value === 'object' ? (value as Record<string, unknown>) : {}
  return {
    auto_play: typeof record.auto_play === 'boolean' ? record.auto_play : true,
    route: sanitizeIncomingRoute(record.route),
  }
}

export type IncomingTextSource = 'server' | 'ocr'

const VALID_INCOMING_SOURCES: ReadonlySet<string> = new Set(['server', 'ocr'])

export interface IncomingTextItem {
  id: string
  text: string
  source: IncomingTextSource
}

function isIncomingTextSource(value: unknown): value is IncomingTextSource {
  return typeof value === 'string' && VALID_INCOMING_SOURCES.has(value)
}

export function isIncomingTextItem(value: unknown): value is IncomingTextItem {
  if (!value || typeof value !== 'object') return false
  const item = value as Record<string, unknown>
  return (
    typeof item.id === 'string' &&
    typeof item.text === 'string' &&
    isIncomingTextSource(item.source)
  )
}

export function isIncomingTextList(payload: unknown): payload is IncomingTextItem[] {
  return Array.isArray(payload) && payload.every(isIncomingTextItem)
}

const UNKNOWN_ITEM_CODE = 'input_server.unknown_item'

function unknownItemError(): string {
  return t('editor.incoming.error.already_processed')
}

/**
 * Speech-queue statuses that are still "active" and therefore projected into
 * the incoming tab. Completed/cancelled jobs are terminal and are excluded.
 */
export const ACTIVE_EXTERNAL_JOB_STATUSES: ReadonlySet<JobStatus> = new Set([
  'queued',
  'generating',
  'ready',
  'playing',
  'failed',
])

export function isActiveExternalJob(job: JobDto): boolean {
  const isIncomingProducer = job.source === 'server' || job.source === 'ocr'
  return isIncomingProducer && ACTIVE_EXTERNAL_JOB_STATUSES.has(job.status)
}

/** Validates a `speech-queue-changed` payload and returns only active server/OCR jobs. */
export function selectActiveExternalJobs(payload: unknown): JobDto[] {
  if (!isSpeechQueueStateDto(payload)) return []
  return payload.jobs.filter(isActiveExternalJob)
}

export function computeIncomingCount(
  pendingItems: IncomingTextItem[],
  externalJobs: JobDto[],
): number {
  return pendingItems.length + externalJobs.length
}

const INCOMING_CHANGED_EVENT = 'input-server-incoming-changed'
const QUEUE_CHANGED_EVENT = 'speech-queue-changed'
const SETTINGS_CHANGED_EVENT = 'settings-changed'

export function useIncomingTexts() {
  const pendingItems = ref<IncomingTextItem[]>([])
  const externalJobs = ref<JobDto[]>([])
  const settings = ref<IncomingSettings>({ auto_play: true, route: 'audio_only' })
  const busyIds = ref<ReadonlySet<string>>(new Set())
  const loadError = ref<string | null>(null)

  const { showError } = useErrorHandler()

  const autoPlay = computed(() => settings.value.auto_play)
  const count = computed(() => computeIncomingCount(pendingItems.value, externalJobs.value))

  const listenerScope = createAsyncCleanupScope()
  let disposed = false
  // Event-wins guards: an event arriving while a snapshot invoke is in flight
  // must win over the (possibly stale) snapshot result, mirroring
  // `runtimeStatusSource`'s `eventArrived` flag — one per channel.
  let pendingItemsEventArrived = false
  let externalJobsEventArrived = false
  // Settings guard: every refresh claims a monotonically increasing token, and
  // a result may apply only while it is the latest request. This makes the
  // event-triggered refresh win over any snapshot started before it, and stops
  // an older overlapping `get_incoming_settings` response from overwriting a
  // newer one that resolved first. Local optimistic writes and the serialized
  // save queue are guarded separately below.
  let settingsRefreshToken = 0
  let localSettingsWrites = 0
  // Serialized save queue: only one backend write is in flight at a time, and
  // newer intents coalesce into a single pending snapshot. This guarantees
  // writes cannot complete out of order and the durable state always ends on
  // the latest user intent.
  let saveInFlight: Promise<void> | null = null
  let pendingSave: IncomingSettings | null = null
  let lastPersistedSettings: IncomingSettings = { ...settings.value }
  let saveIdleWaiters: Array<() => void> = []
  // Global edit-in-flight guard: the backend take removes the item from the
  // inbox before returning it, so at most one take may be in flight at a time.
  // A later edit while one is running is refused (returns null) instead of
  // removing a second item and discarding the first take's result. Exposed as a
  // ref so the UI can disable every incoming Edit action while a take runs.
  const editInFlight = ref(false)

  function isBusy(id: string): boolean {
    return busyIds.value.has(id)
  }

  function markBusy(id: string) {
    const next = new Set(busyIds.value)
    next.add(id)
    busyIds.value = next
  }

  function markIdle(id: string) {
    const next = new Set(busyIds.value)
    next.delete(id)
    busyIds.value = next
  }

  async function refreshPendingItems(): Promise<void> {
    pendingItemsEventArrived = false
    try {
      const payload = await invoke<unknown>('list_incoming_texts')
      if (disposed || pendingItemsEventArrived) return
      if (isIncomingTextList(payload)) {
        pendingItems.value = payload
        loadError.value = null
      } else {
        loadError.value = t('editor.incoming.error.load')
      }
    } catch (e) {
      if (disposed) return
      debugError('[IncomingTexts] Failed to load pending items:', e)
      loadError.value = t('editor.incoming.error.load')
    }
  }

  async function refreshExternalJobs(): Promise<void> {
    externalJobsEventArrived = false
    try {
      const payload = await invoke<unknown>('get_speech_queue_state')
      if (disposed || externalJobsEventArrived) return
      externalJobs.value = selectActiveExternalJobs(payload)
    } catch (e) {
      if (disposed) return
      debugError('[IncomingTexts] Failed to load speech queue:', e)
    }
  }

  async function refreshSettings(): Promise<void> {
    const token = ++settingsRefreshToken
    const writesAtStart = localSettingsWrites
    try {
      const payload = await invoke<unknown>('get_incoming_settings')
      if (disposed || token !== settingsRefreshToken) return
      if (writesAtStart !== localSettingsWrites) return
      if (saveInFlight || pendingSave) return
      settings.value = sanitizeIncomingSettings(payload)
      lastPersistedSettings = settings.value
    } catch (e) {
      if (disposed) return
      debugError('[IncomingTexts] Failed to load incoming settings:', e)
    }
  }

  function enqueueSave(snapshot: IncomingSettings): Promise<void> {
    pendingSave = snapshot
    localSettingsWrites++
    pumpSave()
    return waitForSaveIdle()
  }

  function pumpSave(): void {
    if (saveInFlight) return
    const next = pendingSave
    if (!next) return
    pendingSave = null
    saveInFlight = runSave(next)
  }

  async function runSave(snapshot: IncomingSettings): Promise<void> {
    try {
      await invoke('save_incoming_settings', { settings: snapshot })
      if (!disposed) lastPersistedSettings = snapshot
    } catch (e) {
      if (disposed) return
      // An older intent failing while a newer one is pending must not roll the
      // newer intent back; only the latest failure is rolled back and surfaced.
      if (!pendingSave) {
        settings.value = { ...lastPersistedSettings }
        debugError('[IncomingTexts] Failed to save incoming settings:', e)
        showError(t('editor.incoming.error.save'))
      } else {
        debugError('[IncomingTexts] Failed to save incoming settings:', e)
      }
    } finally {
      saveInFlight = null
      if (!disposed && pendingSave) {
        pumpSave()
      } else {
        pendingSave = null
        const waiters = saveIdleWaiters
        saveIdleWaiters = []
        waiters.forEach((resolve) => resolve())
      }
    }
  }

  function waitForSaveIdle(): Promise<void> {
    if (!saveInFlight && !pendingSave) return Promise.resolve()
    return new Promise((resolve) => { saveIdleWaiters.push(resolve) })
  }

  async function saveSettings(next: IncomingSettings): Promise<void> {
    if (disposed) return
    settings.value = next
    await enqueueSave(next)
  }

  async function setAutoPlay(value: boolean): Promise<void> {
    if (value === settings.value.auto_play) return
    await saveSettings({ auto_play: value, route: settings.value.route })
  }

  async function setRoute(route: IncomingRoute): Promise<void> {
    if (route === settings.value.route) return
    await saveSettings({ auto_play: settings.value.auto_play, route })
  }

  async function approve(id: string): Promise<void> {
    if (isBusy(id)) return
    markBusy(id)
    try {
      await invoke('approve_incoming_text', { incomingId: id })
    } catch (e) {
      debugError('[IncomingTexts] Failed to approve item:', e)
      showError(
        normalizeCommandError(e).code === UNKNOWN_ITEM_CODE
          ? unknownItemError()
          : t('editor.incoming.error.approve'),
      )
    } finally {
      markIdle(id)
    }
  }

  /** Takes the item for editing and returns its text, or null on failure. */
  async function edit(id: string): Promise<string | null> {
    if (isBusy(id)) return null
    // A destructive take is already in flight: refusing keeps that take the
    // only one, so its item is the only one removed from the inbox.
    if (editInFlight.value) return null
    editInFlight.value = true
    markBusy(id)
    try {
      const item = await invoke<unknown>('take_incoming_text_for_edit', {
        incomingId: id,
      })
      if (disposed) return null
      // The take payload is the one untrusted IPC surface here: a contract
      // drift must never leak undefined/untrusted text into a tab. Treat a
      // malformed payload exactly like a command failure for the user.
    if (!isIncomingTextItem(item)) {
      debugError('[IncomingTexts] Malformed take-for-edit payload')
        showError(t('editor.incoming.error.take_edit'))
        return null
      }
      return item.text
    } catch (e) {
      if (disposed) return null
      debugError('[IncomingTexts] Failed to take item for edit:', e)
      showError(
        normalizeCommandError(e).code === UNKNOWN_ITEM_CODE
          ? unknownItemError()
          : t('editor.incoming.error.take_edit'),
      )
      return null
    } finally {
      editInFlight.value = false
      markIdle(id)
    }
  }

  async function discard(id: string): Promise<void> {
    if (isBusy(id)) return
    markBusy(id)
    try {
      await invoke('discard_incoming_text', { incomingId: id })
    } catch (e) {
      debugError('[IncomingTexts] Failed to discard item:', e)
      showError(
        normalizeCommandError(e).code === UNKNOWN_ITEM_CODE
          ? unknownItemError()
          : t('editor.incoming.error.discard'),
      )
    } finally {
      markIdle(id)
    }
  }

  async function skipExternalJob(id: string): Promise<void> {
    if (isBusy(id)) return
    markBusy(id)
    try {
      await invoke('skip_speech_job', { jobId: id })
      if (disposed) return
      await refreshExternalJobs()
    } catch (e) {
      if (disposed) return
      debugError('[IncomingTexts] Failed to skip external job:', e)
      showError(t('editor.incoming.error.skip'))
    } finally {
      markIdle(id)
    }
  }

  /** Cancels any active external job (queued/generating/ready/failed). */
  async function cancelExternalJob(id: string): Promise<void> {
    if (isBusy(id)) return
    markBusy(id)
    try {
      await invoke('cancel_speech_job', { jobId: id })
      if (disposed) return
      await refreshExternalJobs()
    } catch (e) {
      if (disposed) return
      debugError('[IncomingTexts] Failed to cancel external job:', e)
      showError(t('editor.incoming.error.discard'))
    } finally {
      markIdle(id)
    }
  }

  /**
   * Takes a failed external job back for editing and returns its original text.
   * The cancel removes the job's inbox entry before returning the text, so at
   * most one such take may be in flight at a time.
   */
  async function editExternalJob(id: string): Promise<string | null> {
    if (isBusy(id)) return null
    if (editInFlight.value) return null
    const job = externalJobs.value.find((j) => j.job_id === id)
    if (!job || job.status !== 'failed') return null
    editInFlight.value = true
    markBusy(id)
    try {
      await invoke('cancel_speech_job', { jobId: id })
      if (disposed) return null
      await refreshExternalJobs()
      if (disposed) return null
      return job.original_text
    } catch (e) {
      if (disposed) return null
      debugError('[IncomingTexts] Failed to edit external job:', e)
      showError(t('editor.incoming.error.take_edit'))
      return null
    } finally {
      editInFlight.value = false
      markIdle(id)
    }
  }

  onMounted(async () => {
    await listenerScope.track(
      listen<unknown>(INCOMING_CHANGED_EVENT, (event) => {
        pendingItemsEventArrived = true
        if (isIncomingTextList(event.payload)) {
          pendingItems.value = event.payload
          loadError.value = null
        }
      }),
    )
    await listenerScope.track(
      listen<unknown>(QUEUE_CHANGED_EVENT, (event) => {
        externalJobsEventArrived = true
        if (isSpeechQueueStateDto(event.payload)) {
          externalJobs.value = event.payload.jobs.filter(isActiveExternalJob)
        }
      }),
    )
    await listenerScope.track(
      listen(SETTINGS_CHANGED_EVENT, () => {
        // The triggered refresh claims the next token, so any snapshot started
        // before the event is already stale and cannot apply.
        void refreshSettings()
      }),
    )
    // Listeners first, then snapshots: either ordering observes the latest state.
    await refreshPendingItems()
    await refreshExternalJobs()
    await refreshSettings()
  })

  onUnmounted(() => {
    disposed = true
    listenerScope.dispose()
  })

  return {
    pendingItems,
    externalJobs,
    settings,
    autoPlay,
    busyIds,
    editInFlight,
    loadError,
    count,
    isBusy,
    approve,
    edit,
    discard,
    skipExternalJob,
    cancelExternalJob,
    editExternalJob,
    setAutoPlay,
    setRoute,
    refreshPendingItems,
    refreshExternalJobs,
    refreshSettings,
  }
}
