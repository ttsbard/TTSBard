<script setup lang="ts">
import { watch } from 'vue'
import { useErrorHandler, ErrorLevel } from '../composables/useErrorHandler'
import { t } from '../i18n'

const { errors, removeError } = useErrorHandler()

// Автоматически скроллить к новым ошибкам
watch(errors, () => {
  if (errors.value.length > 0) {
    // Можно добавить sound notification или другие эффекты
  }
}, { deep: true })

function getErrorClass(level: ErrorLevel): string {
  switch (level) {
    case ErrorLevel.ERROR:
      return 'error-toast error'
    case ErrorLevel.WARNING:
      return 'error-toast warning'
    case ErrorLevel.INFO:
      return 'error-toast info'
    case ErrorLevel.SUCCESS:
      return 'error-toast success'
    default:
      return 'error-toast'
  }
}
</script>

<template>
  <Teleport to="body">
    <div class="error-toasts-container">
      <TransitionGroup name="toast">
        <div
          v-for="error in errors"
          :key="error.id"
          :class="getErrorClass(error.level)"
          @click="removeError(error.id)"
        >
          <span class="toast-message">{{ error.message }}</span>
          <button class="toast-close" :title="t('toast.close')" :aria-label="t('toast.close')" @click.stop="removeError(error.id)">×</button>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<style scoped>
.error-toasts-container {
  position: fixed;
  top: 20px;
  right: 20px;
  z-index: 9999;
  display: flex;
  flex-direction: column;
  gap: 10px;
  pointer-events: none;
}

.error-toast {
  pointer-events: auto;
  min-width: 300px;
  max-width: 500px;
  padding: 12px 16px;
  border-radius: 8px;
  background: var(--toast-bg);
  border: 1px solid var(--toast-border);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
  display: flex;
  align-items: center;
  gap: 12px;
  cursor: pointer;
  transition: all 0.2s ease;
}

.error-toast:hover {
  transform: translateX(-4px);
  box-shadow: 0 6px 16px rgba(0, 0, 0, 0.4);
}

.error-toast.error {
  border-left: 4px solid var(--toast-error-border);
  background: var(--toast-error-bg);
}

.error-toast.warning {
  border-left: 4px solid var(--toast-warning-border);
  background: var(--toast-warning-bg);
}

.error-toast.info {
  border-left: 4px solid var(--toast-success-border);
  background: var(--toast-success-bg);
}

.error-toast.success {
  border-left: 4px solid var(--toast-success-border);
  background: var(--toast-success-bg);
}

.toast-icon {
  font-size: 18px;
  flex-shrink: 0;
}

.toast-message {
  flex: 1;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  line-height: 1.4;
  word-break: break-word;
}

.toast-close {
  flex-shrink: 0;
  background: none;
  border: none;
  color: inherit;
  font-size: 20px;
  line-height: 1;
  cursor: pointer;
  padding: 0;
  width: 20px;
  height: 20px;
  opacity: 0.6;
  transition: opacity 0.2s;
}

.toast-close:hover {
  opacity: 1;
}

.toast-close:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 1px;
}

/* Animations */
.toast-enter-active,
.toast-leave-active {
  transition: all 0.3s ease;
}

.toast-enter-from {
  opacity: 0;
  transform: translateX(100%);
}

.toast-leave-to {
  opacity: 0;
  transform: translateX(100%) scale(0.8);
}

.toast-move {
  transition: transform 0.3s ease;
}
</style>
