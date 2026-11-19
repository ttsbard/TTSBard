<script setup lang="ts">
import { computed, ref, watch, onMounted, onUnmounted } from 'vue'
import { ChevronDown, Minus, X } from 'lucide-vue-next'
import { t } from '../i18n'
import {
  useIntegrationStatusSlots,
  type StatusService,
} from '../composables/useIntegrationStatusSlots'

const emit = defineEmits<{
  'return-compact': []
  'focus-editor': []
  minimize: []
}>()

const { errorSlots } = useIntegrationStatusSlots()

const activeErrorService = ref<StatusService | null>(null)
let triggerElement: HTMLElement | null = null

const activeError = computed(() =>
  errorSlots.value.find((err) => err.service === activeErrorService.value),
)

// When errors change, if currently active error resolved, close popover immediately
watch(errorSlots, (currentErrors) => {
  if (activeErrorService.value) {
    const stillActive = currentErrors.some((e) => e.service === activeErrorService.value)
    if (!stillActive) {
      closePopover()
    }
  }
}, { flush: 'post' })

function toggleErrorPopover(service: StatusService, event?: MouseEvent) {
  if (activeErrorService.value === service) {
    closePopover()
  } else {
    activeErrorService.value = service
    if (event?.currentTarget instanceof HTMLElement) {
      triggerElement = event.currentTarget
    }
  }
}

function closePopover() {
  activeErrorService.value = null
  if (triggerElement && document.contains(triggerElement)) {
    triggerElement.focus()
  } else {
    emit('focus-editor')
  }
  triggerElement = null
}

function handleEscClose() {
  closePopover()
}

function handlePointerDown(event: PointerEvent) {
  if (!activeErrorService.value) return
  const target = event.target as HTMLElement | null
  if (!target) return
  if (target.closest('.mono-error-popover') || target.closest('.mono-error-btn')) {
    return
  }
  closePopover()
}

function handleWindowKeyDown(event: KeyboardEvent) {
  if (event.key === 'Escape' && activeErrorService.value) {
    event.stopPropagation()
    event.preventDefault()
    closePopover()
  }
}

onMounted(() => {
  window.addEventListener('pointerdown', handlePointerDown)
  window.addEventListener('keydown', handleWindowKeyDown, true)
})

onUnmounted(() => {
  window.removeEventListener('pointerdown', handlePointerDown)
  window.removeEventListener('keydown', handleWindowKeyDown, true)
})
</script>

<template>
  <div class="mono-mode-bar" data-tauri-drag-region>
    <!-- Left slot: error icons in Task 004 -->
    <div class="mono-left-slot" data-tauri-drag-region>
      <slot name="errors">
        <div v-if="errorSlots.length > 0" class="mono-errors-list" data-tauri-drag-region="false">
          <button
            v-for="err in errorSlots"
            :key="err.service"
            type="button"
            class="mono-error-btn"
            :class="{ 'is-active': activeErrorService === err.service }"
            data-tauri-drag-region="false"
            :title="err.label"
            :aria-label="err.label"
            :aria-expanded="activeErrorService === err.service"
            aria-haspopup="dialog"
            @click.stop="toggleErrorPopover(err.service, $event)"
          >
            <component :is="err.icon" :size="12" />
          </button>
        </div>
      </slot>
    </div>

    <!-- Center handle: visual grab affordance -->
    <div class="mono-handle-area" data-tauri-drag-region>
      <div class="mono-handle-bar" data-tauri-drag-region />
    </div>

    <!-- Right buttons: return to compact mode and minimize -->
    <div class="mono-right-slot" data-tauri-drag-region>
      <div class="mono-output-slot" data-tauri-drag-region="false">
        <slot name="outputs" />
      </div>
      <button
        type="button"
        class="mono-return-btn"
        data-tauri-drag-region="false"
        @click="emit('return-compact')"
        :title="t('shell.mono.return')"
        :aria-label="t('shell.mono.return')"
      >
        <ChevronDown :size="14" />
      </button>
      <button
        type="button"
        class="mono-minimize-btn"
        data-tauri-drag-region="false"
        @click.stop="emit('minimize')"
        :title="t('shell.minimize')"
        :aria-label="t('shell.minimize')"
      >
        <Minus :size="14" />
      </button>
    </div>

    <!-- Floating Popover for Active Error -->
    <div
      v-if="activeError"
      class="mono-error-popover"
      role="dialog"
      :aria-label="t('shell.mono.error_details')"
      data-tauri-drag-region="false"
      tabindex="-1"
      @keydown.esc.stop.prevent="handleEscClose"
    >
      <div class="mono-error-popover-header" data-tauri-drag-region="false">
        <span class="mono-error-title" data-tauri-drag-region="false">{{ activeError.serviceName }}</span>
        <button
          type="button"
          class="mono-error-close-btn"
          data-tauri-drag-region="false"
          :title="t('shell.mono.close_error')"
          :aria-label="t('shell.mono.close_error')"
          @click.stop="closePopover"
        >
          <X :size="12" />
        </button>
      </div>
      <div class="mono-error-popover-body" data-tauri-drag-region="false">
        <p class="mono-error-reason" data-tauri-drag-region="false">{{ activeError.errorReason }}</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mono-mode-bar {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 18px;
  min-height: 18px;
  max-height: 20px;
  width: 100%;
  padding: 0 4px;
  background: transparent;
  user-select: none;
  flex-shrink: 0;
  box-sizing: border-box;
  z-index: 10;
}

.mono-left-slot {
  display: flex;
  align-items: center;
  min-width: 30px;
  height: 100%;
}

.mono-errors-list {
  display: flex;
  align-items: center;
  gap: 2px;
}

.mono-error-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 16px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--status-disconnected, #ef4444);
  cursor: pointer;
  border-radius: 3px;
  transition: background 0.15s ease, color 0.15s ease;
}

.mono-error-btn:hover {
  background: var(--sidebar-btn-hover-bg, rgba(255, 255, 255, 0.1));
}

.mono-error-btn.is-active {
  background: rgba(239, 68, 68, 0.2);
}

.mono-error-btn:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: -1px;
}

.mono-handle-area {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: 1;
  height: 100%;
}

.mono-handle-bar {
  width: 32px;
  height: 3px;
  border-radius: 2px;
  background: var(--color-border-strong, rgba(255, 255, 255, 0.2));
  opacity: 0.6;
  transition: opacity 0.15s ease;
}

.mono-mode-bar:hover .mono-handle-bar {
  opacity: 0.9;
}

.mono-output-slot {
  display: flex;
  align-items: center;
  height: 100%;
  box-sizing: border-box;
  padding-block: 2px;
}

.mono-right-slot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  min-width: 30px;
  height: 100%;
}

.mono-return-btn,
.mono-minimize-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 18px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--color-text-muted, #888);
  cursor: pointer;
  border-radius: 3px;
  transition: background 0.15s ease, color 0.15s ease;
}

.mono-return-btn:hover,
.mono-minimize-btn:hover {
  background: var(--sidebar-btn-hover-bg, rgba(255, 255, 255, 0.1));
  color: var(--color-text-primary, #fff);
}

.mono-return-btn:focus-visible,
.mono-minimize-btn:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: -1px;
}

/* Floating error popover */
.mono-error-popover {
  position: absolute;
  top: 20px;
  left: 4px;
  z-index: 100;
  max-width: min(280px, calc(100vw - 8px));
  width: max-content;
  box-sizing: border-box;
  padding: 6px 8px;
  border-radius: 6px;
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border-strong, rgba(255, 255, 255, 0.15));
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  font-size: 11px;
  line-height: 1.35;
  user-select: text;
}

.mono-error-popover-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 4px;
  font-weight: 600;
  color: var(--color-text-primary, #fff);
}

.mono-error-title {
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.mono-error-close-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--color-text-muted, #888);
  cursor: pointer;
  border-radius: 2px;
}

.mono-error-close-btn:hover {
  color: var(--color-text-primary, #fff);
  background: var(--sidebar-btn-hover-bg, rgba(255, 255, 255, 0.1));
}

.mono-error-close-btn:focus-visible {
  outline: 2px solid var(--color-accent);
}

.mono-error-popover-body {
  word-break: break-word;
  overflow-wrap: anywhere;
  color: var(--color-text-secondary, #ccc);
}

.mono-error-reason {
  margin: 0;
  font-size: 11px;
  line-height: 1.35;
}
</style>
