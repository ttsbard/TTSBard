/**
 * Composable owning the in-app update install state (ROADMAP-141, stage 2).
 *
 * Created once in `App.vue` (next to the update status context) and provided
 * to the tree: the «Обновить» button in the «Обновления» card starts the
 * backend pipeline, and the progress modal in `App.vue` renders the state.
 * The backend pushes `update-install-progress` events; the pipeline runs
 * detached, so this context survives navigation and never cancels a download
 * by itself — only `cancel()` does.
 */

import {
  computed,
  getCurrentScope,
  inject,
  onScopeDispose,
  provide,
  ref,
  type ComputedRef,
  type InjectionKey,
  type Ref,
} from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { createAsyncCleanupScope } from '../utils/asyncCleanup'
import { debugError } from '../utils/debug'
import { t } from '../i18n'
import {
  CANCEL_UPDATE_INSTALL_COMMAND,
  INSTALL_UPDATE_COMMAND,
  parseUpdateInstallProgress,
  UPDATE_INSTALL_PROGRESS_EVENT,
  updateInstallErrorLocaleKey,
  type UpdateInstallErrorCode,
  type UpdateInstallStage,
} from '../ipc/appUpdates'

export interface UpdateInstallContext {
  /** Current pipeline stage; `null` when nothing is running. */
  stage: Ref<UpdateInstallStage | null>
  /** Release version being installed. */
  version: Ref<string | null>
  /** Bytes received so far during `downloading`. */
  downloaded: Ref<number>
  /** Total download size when the server announced one. */
  total: Ref<number | null>
  /** Localized failure text for the `failed` stage. */
  errorText: Ref<string | null>
  /** A pipeline is running (any stage, including `failed` until dismissed). */
  active: ComputedRef<boolean>
  /** Cancel is still possible: `checking` or `downloading`. */
  canCancel: ComputedRef<boolean>
  /** 0..100 when the total size is known; `null` otherwise. */
  percent: ComputedRef<number | null>
  /** Start the install pipeline; a rejection maps to a localized error. */
  start(): Promise<void>
  /** Cancel the running download; `true` when the backend cancelled it. */
  cancel(): Promise<boolean>
  /** Dismiss a finished/failed run and return to idle. */
  reset(): void
}

export function createUpdateInstall(): UpdateInstallContext {
  const stage = ref<UpdateInstallStage | null>(null)
  const version = ref<string | null>(null)
  const downloaded = ref(0)
  const total = ref<number | null>(null)
  const errorText = ref<string | null>(null)
  const listenerScope = createAsyncCleanupScope()

  const active = computed(() => stage.value !== null)
  const canCancel = computed(
    () => stage.value === 'checking' || stage.value === 'downloading',
  )
  const percent = computed(() => {
    if (stage.value !== 'downloading' || total.value === null || total.value <= 0) {
      return null
    }
    return Math.min(100, Math.round((downloaded.value / total.value) * 100))
  })

  function applyProgress(payload: unknown): void {
    const progress = parseUpdateInstallProgress(payload)
    if (!progress) {
      debugError('[useUpdateInstall] Ignoring a malformed install progress payload')
      return
    }
    downloaded.value = progress.downloaded
    total.value = progress.total
    version.value = progress.version ?? version.value
    if (progress.stage === 'cancelled') {
      // The modal closes on its own when the stage leaves the screen; here we
      // only return the context to idle.
      reset()
      return
    }
    stage.value = progress.stage
    if (progress.stage === 'failed') {
      errorText.value = t(
        updateInstallErrorLocaleKey(progress.error_code as UpdateInstallErrorCode | null),
      )
    }
  }

  function reset(): void {
    stage.value = null
    version.value = null
    downloaded.value = 0
    total.value = null
    errorText.value = null
  }

  async function start(): Promise<void> {
    if (stage.value !== null) return
    errorText.value = null
    try {
      applyProgress(await invoke<unknown>(INSTALL_UPDATE_COMMAND))
    } catch (e) {
      // Command rejections carry stable code strings from the backend
      // (`InstallErrorKind::as_str` or the transient `busy`).
      const code = typeof e === 'string' ? e : null
      stage.value = 'failed'
      errorText.value = t(updateInstallErrorLocaleKey(code))
      debugError('[useUpdateInstall] Failed to start the install:', e)
    }
  }

  async function cancel(): Promise<boolean> {
    if (!canCancel.value) return false
    try {
      const cancelled = await invoke<unknown>(CANCEL_UPDATE_INSTALL_COMMAND)
      if (cancelled) {
        // The backend aborted the download; the `cancelled` event (already
        // consumed or about to arrive) is mirrored by an immediate reset so
        // the modal does not linger on a stale stage.
        reset()
        return true
      }
      // Nothing cancellable anymore: the pipeline moved past the download.
      return false
    } catch (e) {
      debugError('[useUpdateInstall] Failed to cancel the install:', e)
      return false
    }
  }

  listenerScope
    .track(listen<unknown>(UPDATE_INSTALL_PROGRESS_EVENT, (event) => applyProgress(event.payload)))
    .catch((e) => {
      debugError('[useUpdateInstall] Failed to subscribe to install progress events:', e)
    })

  if (getCurrentScope()) {
    onScopeDispose(() => {
      listenerScope.dispose()
    })
  }

  if (import.meta.env.DEV && import.meta.hot) {
    import.meta.hot.dispose(() => {
      listenerScope.dispose()
    })
  }

  return {
    stage,
    version,
    downloaded,
    total,
    errorText,
    active,
    canCancel,
    percent,
    start,
    cancel,
    reset,
  }
}

export const UPDATE_INSTALL_KEY: InjectionKey<UpdateInstallContext> = Symbol('update-install')

/** Owner call: create the context once and provide it to the tree. */
export function provideUpdateInstall(): UpdateInstallContext {
  const context = createUpdateInstall()
  provide(UPDATE_INSTALL_KEY, context)
  return context
}

/** Consumer call for the settings card and the progress modal. */
export function useUpdateInstall(): UpdateInstallContext {
  const context = inject<UpdateInstallContext>(UPDATE_INSTALL_KEY)
  if (!context) {
    throw new Error('useUpdateInstall must be used within a component that provides update install')
  }
  return context
}
