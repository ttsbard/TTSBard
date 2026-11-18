<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { X } from 'lucide-vue-next'
import { t } from '../../i18n'
import { useUpdateInstall } from '../../composables/useUpdateInstall'
import { formatTransferBytes } from '../../composables/useDataTransfer'
import { updateInstallStageLocaleKey, type UpdateInstallStage } from '../../ipc/appUpdates'

/**
 * Progress modal for the in-app update install (ROADMAP-141, stage 2).
 *
 * Rendered by `App.vue` while the install context is active, so the progress
 * stays visible regardless of settings navigation. Closing the window during
 * the download cancels it (the same action as the «Отмена» button); once
 * the download finished and the signature was verified the pipeline is
 * committed — the modal becomes non-cancellable and the app exits into the
 * installer. A failed download or signature keeps the app running and offers
 * a retry.
 */

const install = useUpdateInstall()

const dialogRef = ref<HTMLElement | null>(null)
const titleId = 'update-install-title'

const stage = computed(() => install.stage.value)
const canCancel = computed(() => install.canCancel.value)
const failed = computed(() => stage.value === 'failed')
const indeterminate = computed(() => install.percent.value === null)
const percent = computed(() => install.percent.value ?? 0)

const sizeText = computed(() =>
  t('general.updates.install.size', {
    done: formatTransferBytes(install.downloaded.value),
    total: formatTransferBytes(install.total.value ?? 0),
  }),
)

const stageText = computed(() => {
  const current = stage.value
  return current ? t(updateInstallStageLocaleKey(current as UpdateInstallStage)) : ''
})

let previouslyFocused: HTMLElement | null = null

function appRoot(): HTMLElement | null {
  return document.getElementById('app')
}

function applyInert() {
  appRoot()?.setAttribute('inert', '')
}

function removeInert() {
  appRoot()?.removeAttribute('inert')
}

function focusableElements(): HTMLElement[] {
  const dialog = dialogRef.value
  if (!dialog) return []
  const selector = [
    'button:not([disabled])',
    'a[href]',
    'input:not([disabled])',
    'select:not([disabled])',
    'textarea:not([disabled])',
    '[tabindex]:not([tabindex="-1"])',
  ].join(',')
  return Array.from(dialog.querySelectorAll<HTMLElement>(selector)).filter(
    (el) => el.offsetParent !== null,
  )
}

function focusInitial() {
  const first = focusableElements()[0]
  if (first) first.focus()
  else dialogRef.value?.focus()
}

function trapFocus(event: KeyboardEvent) {
  const dialog = dialogRef.value
  if (!dialog) return
  const focusable = focusableElements()
  if (focusable.length === 0) {
    event.preventDefault()
    dialog.focus()
    return
  }
  const first = focusable[0]
  const last = focusable[focusable.length - 1]
  const active = document.activeElement as HTMLElement | null
  const outside = !active || !dialog.contains(active)
  if (event.shiftKey) {
    if (outside || active === first || active === dialog) {
      event.preventDefault()
      last.focus()
    }
  } else if (outside || active === last || active === dialog) {
    event.preventDefault()
    first.focus()
  }
}

/**
 * Explicit close actions cancel the download or dismiss a failed attempt.
 */
async function requestClose() {
  if (canCancel.value) {
    await install.cancel()
    return
  }
  if (failed.value) {
    install.reset()
  }
}

function dismissFailure() {
  if (failed.value) install.reset()
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopImmediatePropagation()
    dismissFailure()
    return
  }
  if (event.key === 'Tab') {
    trapFocus(event)
  }
}

async function retry() {
  install.reset()
  await install.start()
}

onMounted(() => {
  previouslyFocused = document.activeElement as HTMLElement | null
  applyInert()
  document.addEventListener('keydown', onKeydown, true)
  void nextTick(focusInitial)
})

onUnmounted(() => {
  document.removeEventListener('keydown', onKeydown, true)
  removeInert()
  previouslyFocused?.focus?.()
})
</script>

<template>
  <Teleport to="body">
    <div class="update-install-overlay" @click.self="dismissFailure">
      <div
        ref="dialogRef"
        class="update-install-dialog"
        role="dialog"
        aria-modal="true"
        :aria-labelledby="titleId"
        tabindex="-1"
        @keydown.stop
      >
        <header class="ui-modal-header">
          <h2 :id="titleId" class="ui-section-title">
            {{ t('general.updates.install.title', { version: install.version.value ?? '' }) }}
          </h2>
          <button
            v-if="canCancel || failed"
            type="button"
            class="ui-icon-button"
            :title="t('common.close')"
            :aria-label="t('common.close')"
            @click="requestClose"
          >
            <X :size="18" aria-hidden="true" />
          </button>
        </header>

        <div class="ui-modal-body update-install-body">
          <div
            v-if="!failed"
            class="update-install-progress"
            role="progressbar"
            :aria-label="stageText"
            aria-valuemin="0"
            aria-valuemax="100"
            :aria-valuenow="stage === 'downloading' && !indeterminate ? percent : undefined"
            :aria-valuetext="stageText"
          >
            <div
              class="update-install-bar"
              :class="{ indeterminate }"
              :style="stage === 'downloading' && !indeterminate ? { width: `${percent}%` } : undefined"
            ></div>
          </div>
          <p v-if="!failed" class="ui-metadata">
            {{ stage === 'launching' ? t('general.updates.install.wait') : stageText }}
          </p>
          <div v-if="stage === 'downloading' && !indeterminate" class="update-install-stats ui-metadata">
            <span>{{ sizeText }}</span>
            <span>{{ percent }}%</span>
          </div>

          <div v-if="failed" class="update-install-error" role="alert">
            {{ install.errorText.value }}
          </div>

          <div v-if="failed || canCancel" class="update-install-actions">
            <template v-if="failed">
              <button type="button" class="ui-button" @click="requestClose">
                {{ t('common.close') }}
              </button>
              <button type="button" class="ui-button ui-button--primary" @click="retry">
                {{ t('general.updates.install.retry') }}
              </button>
            </template>
            <button
              v-else-if="canCancel"
              type="button"
              class="ui-button"
              @click="requestClose"
            >
              {{ t('common.cancel') }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.update-install-overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background: var(--modal-overlay);
  backdrop-filter: blur(4px);
}

.update-install-dialog {
  background: var(--color-bg-panel-strong);
  border: 1px solid var(--color-border-strong);
  border-radius: 12px;
  max-width: 420px;
  width: 100%;
  max-height: 90vh;
  overflow-y: auto;
  box-shadow: var(--shadow-soft);
  outline: none;
}

.update-install-dialog .ui-icon-button {
  flex-shrink: 0;
}

.update-install-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-bottom: 16px;
}

.update-install-progress {
  height: 8px;
  border-radius: 999px;
  background: var(--color-bg-field-hover);
  overflow: hidden;
}

.update-install-bar {
  height: 100%;
  border-radius: 999px;
  background: var(--color-accent);
  transition: width 0.15s ease;
}

.update-install-bar.indeterminate {
  width: 40%;
  animation: update-install-indeterminate 1.2s ease-in-out infinite;
}

@keyframes update-install-indeterminate {
  0% {
    transform: translateX(-100%);
  }
  100% {
    transform: translateX(350%);
  }
}

.update-install-stats {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 4px 12px;
}

.update-install-error {
  padding: 8px 12px;
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border-strong);
  border-left: 4px solid var(--status-disconnected);
  border-radius: 8px;
  color: var(--danger-text-weak);
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  line-height: 1.4;
  overflow-wrap: anywhere;
}

.update-install-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 4px;
}
</style>
