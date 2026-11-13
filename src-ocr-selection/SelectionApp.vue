<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref } from 'vue'
import { useInterfaceFontSync } from '../src/composables/useInterfaceFontSync';
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { t } from '../src/i18n'
import { createAsyncCleanupScope } from '../src/utils/asyncCleanup'
import { debugError } from '../src/utils/debug'
import { createFrameSession } from './frameSession'
import type { LoadedFrame, PreviewDto } from './frameSession'
import { isWithinBlurIgnoreWindow, selectionFromDrag } from './selection'
import type { PhysicalPoint } from './selection'

useInterfaceFontSync();

interface CssPoint {
  x: number
  y: number
}

type StatusState = 'disabled' | 'starting' | 'ready' | 'selectingArea' | 'recognizing' | 'error'

const STATUS_STATES: readonly StatusState[] = [
  'disabled',
  'starting',
  'ready',
  'selectingArea',
  'recognizing',
  'error',
]

const OCR_STATUS_CHANGED_EVENT = 'ocr-status-changed'
const OCR_SELECTION_REJECTED_EVENT = 'ocr-selection-rejected'

function readStatusState(raw: unknown): StatusState | null {
  if (typeof raw !== 'object' || raw === null) return null
  const state = (raw as { state?: unknown }).state
  return typeof state === 'string' && (STATUS_STATES as readonly string[]).includes(state)
    ? (state as StatusState)
    : null
}

function readSessionId(raw: unknown): string | null {
  if (typeof raw !== 'object' || raw === null) return null
  const sessionId = (raw as { sessionId?: unknown }).sessionId
  return typeof sessionId === 'string' && sessionId.length > 0 ? sessionId : null
}

const frame = ref<LoadedFrame | null>(null)
const dragging = ref(false)
const dragStart = ref<CssPoint | null>(null)
const dragEnd = ref<CssPoint | null>(null)
const tooSmallHint = ref(false)
const acknowledgedId = ref<string | null>(null)

const rootEl = ref<HTMLElement | null>(null)
const imgEl = ref<HTMLImageElement | null>(null)

let blurIgnoreArmedAt: number | null = null
let rejectedTimer: number | null = null
let cancelPendingId: string | null = null
let submitPendingId: string | null = null

const scope = createAsyncCleanupScope()

const canSelect = computed(
  () => frame.value !== null && acknowledgedId.value === frame.value.sessionId,
)

const rectStyle = computed(() => {
  const start = dragStart.value
  const end = dragEnd.value
  if (!start || !end) return null
  return {
    left: `${Math.min(start.x, end.x)}px`,
    top: `${Math.min(start.y, end.y)}px`,
    width: `${Math.abs(end.x - start.x)}px`,
    height: `${Math.abs(end.y - start.y)}px`,
  }
})

async function fetchPreview(): Promise<PreviewDto | null> {
  return invoke<PreviewDto | null>('ocr_selection_preview')
}

async function decodePreview(preview: PreviewDto): Promise<LoadedFrame> {
  const src = `data:image/png;base64,${preview.pngBase64}`
  const image = new Image()
  image.src = src
  await image.decode()
  if (image.naturalWidth !== preview.width || image.naturalHeight !== preview.height) {
    throw new Error(`preview dimensions mismatch for ${preview.sessionId}`)
  }
  return {
    sessionId: preview.sessionId,
    width: preview.width,
    height: preview.height,
    src,
    release: () => {},
  }
}

function displayFrame(next: LoadedFrame | null): void {
  const previousId = frame.value?.sessionId ?? null
  const nextId = next?.sessionId ?? null
  if (previousId !== nextId) {
    clearSessionPresentation()
  }
  frame.value = next
}

async function rendered(): Promise<void> {
  await nextTick()
  const element = imgEl.value
  if (!element) return
  if (typeof element.decode === 'function') {
    await element.decode()
  }
}

async function ready(sessionId: string): Promise<boolean> {
  const accepted = await invoke<boolean>('ocr_selection_ready', { sessionId })
  if (
    accepted &&
    !scope.disposed &&
    helper.sessionId === sessionId &&
    frame.value?.sessionId === sessionId
  ) {
    acknowledgedId.value = sessionId
  }
  return accepted
}

async function reportFailed(sessionId: string): Promise<void> {
  await invoke<void>('ocr_selection_failed', { sessionId })
}

const helper = createFrameSession({
  fetchPreview,
  decode: decodePreview,
  display: displayFrame,
  rendered,
  ready,
  failed: reportFailed,
})

function resetDrag(): void {
  dragging.value = false
  dragStart.value = null
  dragEnd.value = null
}

function clearSessionPresentation(): void {
  resetDrag()
  tooSmallHint.value = false
  acknowledgedId.value = null
  blurIgnoreArmedAt = null
  if (rejectedTimer !== null) {
    window.clearTimeout(rejectedTimer)
    rejectedTimer = null
  }
}

function showTooSmallHint(): void {
  tooSmallHint.value = true
  blurIgnoreArmedAt = Date.now()
  if (rejectedTimer !== null) {
    window.clearTimeout(rejectedTimer)
  }
  rejectedTimer = window.setTimeout(() => {
    tooSmallHint.value = false
    rejectedTimer = null
  }, 2000)
}

function cancelSession(sessionId: string): void {
  resetDrag()
  helper.clear()
  cancelPendingId = sessionId
  invoke('ocr_selection_cancel', { sessionId })
    .catch(() => {})
    .finally(() => {
      if (cancelPendingId === sessionId) cancelPendingId = null
    })
}

function cancelCurrent(): void {
  const sessionId = helper.sessionId
  if (sessionId === null) return
  if (submitPendingId === sessionId || cancelPendingId === sessionId) return
  cancelSession(sessionId)
}

function toPhysicalPoint(point: CssPoint): PhysicalPoint {
  const current = frame.value
  const root = rootEl.value
  if (!current || !root) {
    throw new Error('selection without a current frame')
  }
  const rect = root.getBoundingClientRect()
  return {
    x: Math.round(point.x * (rect.width > 0 ? current.width / rect.width : 1)),
    y: Math.round(point.y * (rect.height > 0 ? current.height / rect.height : 1)),
  }
}

function onMouseDown(event: MouseEvent): void {
  if (event.button !== 0) return
  if (!canSelect.value) return
  if (!helper.ready) return
  const sessionId = helper.sessionId
  if (sessionId === null) return
  if (submitPendingId === sessionId || cancelPendingId === sessionId) return
  dragging.value = true
  dragStart.value = { x: event.clientX, y: event.clientY }
  dragEnd.value = { x: event.clientX, y: event.clientY }
}

function onMouseMove(event: MouseEvent): void {
  if (!dragging.value) return
  dragEnd.value = { x: event.clientX, y: event.clientY }
}

function onMouseUp(event: MouseEvent): void {
  if (!dragging.value) return
  dragEnd.value = { x: event.clientX, y: event.clientY }
  const start = dragStart.value
  const end = dragEnd.value
  resetDrag()
  if (!start || !end) return
  if (!canSelect.value) return
  if (!helper.ready) return
  const current = frame.value
  const sessionId = helper.sessionId
  if (!current || sessionId === null) return
  if (sessionId !== current.sessionId) return
  const selection = selectionFromDrag(toPhysicalPoint(start), toPhysicalPoint(end))
  submitPendingId = sessionId
  invoke('ocr_selection_submit', {
    sessionId,
    x1: selection.x1,
    y1: selection.y1,
    x2: selection.x2,
    y2: selection.y2,
  })
    .catch((error) => {
      // A rejected submit means the backend could not take this selection
      // (hide failed, runtime unavailable, IPC error). Never swallow it
      // silently: log it and drop the local presentation guard so the overlay
      // cannot keep offering a drag into a session that is no longer backed.
      debugError('[SelectionApp] Failed to submit OCR selection:', error)
      clearSessionPresentation()
    })
    .finally(() => {
      if (submitPendingId === sessionId) submitPendingId = null
    })
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    cancelCurrent()
  }
}

function onWindowBlur(): void {
  if (isWithinBlurIgnoreWindow(blurIgnoreArmedAt, Date.now())) return
  if (!helper.ready) return
  const sessionId = helper.sessionId
  if (sessionId === null) return
  if (submitPendingId === sessionId || cancelPendingId === sessionId) return
  cancelSession(sessionId)
}

function onStatusChanged(raw: unknown): void {
  const state = readStatusState(raw)
  if (state === null) return
  helper.reconcile(state === 'selectingArea')
}

function onSelectionRejected(raw: unknown): void {
  const sessionId = readSessionId(raw)
  if (sessionId === null) return
  if (helper.sessionId !== sessionId) return
  showTooSmallHint()
}

window.addEventListener('keydown', onKeydown)
scope.add(() => window.removeEventListener('keydown', onKeydown))

async function boot(): Promise<void> {
  try {
    await Promise.all([
      scope.track(
        listen<unknown>(OCR_STATUS_CHANGED_EVENT, (event) => onStatusChanged(event.payload)),
      ),
      scope.track(
        listen<unknown>(OCR_SELECTION_REJECTED_EVENT, (event) => onSelectionRejected(event.payload)),
      ),
      scope.track(
        getCurrentWindow().onFocusChanged(({ payload: focused }) => {
          if (!focused) onWindowBlur()
        }),
      ),
    ])
  } catch {
    // A failed listener registration must not surface or block the initial probe.
  }
  if (scope.disposed) return
  helper.reconcile(true)
}

void boot()

onBeforeUnmount(() => {
  scope.dispose()
  helper.dispose()
  // `clearSessionPresentation()` owns the too-small `rejectedTimer` and the drag
  // state: clearing it here keeps a pending timeout from firing into an
  // unmounted overlay.
  clearSessionPresentation()
})
</script>

<template>
  <div
    ref="rootEl"
    class="ocr-selection"
    :class="{ selectable: canSelect }"
    @mousedown="onMouseDown"
    @mousemove="onMouseMove"
    @mouseup="onMouseUp"
  >
    <img
      v-if="frame"
      ref="imgEl"
      class="frame"
      :src="frame.src"
      alt=""
      draggable="false"
    />
    <div v-if="frame && !dragging" class="dim" aria-hidden="true"></div>
    <div
      v-if="frame && dragging && rectStyle"
      class="selection-rect"
      :style="rectStyle"
      aria-hidden="true"
    ></div>
    <div v-if="frame" class="hint">{{ t('ocr_selection.hint') }}</div>
    <div v-if="tooSmallHint" class="too-small">{{ t('ocr_selection.area_too_small') }}</div>
  </div>
</template>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html {
  --font-sans: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  font-size: var(--ui-font-size, 16px);
}

html,
body {
  margin: 0;
  padding: 0;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: #000000;
}

#app {
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: #000000;
}
</style>

<style scoped>
.ocr-selection {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: #000000;
  cursor: default;
  user-select: none;
}

.ocr-selection.selectable {
  cursor: crosshair;
}

.frame {
  display: block;
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: fill;
  pointer-events: none;
  user-select: none;
  -webkit-user-drag: none;
}

.dim {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  pointer-events: none;
  z-index: 1;
}

.selection-rect {
  position: absolute;
  border: 1px solid rgba(255, 255, 255, 0.8);
  background: transparent;
  box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.5);
  pointer-events: none;
  z-index: 1;
}

.hint {
  position: fixed;
  top: 12px;
  left: 50%;
  transform: translateX(-50%);
  padding: 6px 14px;
  border-radius: 6px;
  background: rgba(0, 0, 0, 0.6);
  border: 1px solid rgba(255, 255, 255, 0.25);
  color: #ffffff;
  font-family: var(--ui-font-family, var(--font-sans));
  font-size: 13px;
  line-height: 1.4;
  text-align: center;
  white-space: normal;
  overflow-wrap: break-word;
  max-width: calc(100vw - 24px);
  user-select: none;
  pointer-events: none;
  z-index: 2;
}

.too-small {
  position: fixed;
  top: 52px;
  left: 50%;
  transform: translateX(-50%);
  padding: 4px 12px;
  border-radius: 6px;
  background: rgba(0, 0, 0, 0.6);
  border: 1px solid rgba(255, 255, 255, 0.25);
  color: #ffd7d7;
  font-family: var(--ui-font-family, var(--font-sans));
  font-size: 13px;
  line-height: 1.4;
  text-align: center;
  white-space: normal;
  overflow-wrap: break-word;
  max-width: calc(100vw - 24px);
  user-select: none;
  pointer-events: none;
  z-index: 2;
}
</style>
