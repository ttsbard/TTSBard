<script setup lang="ts">
import { watch } from 'vue'
import { CircleAlert, CircleCheck, Info, TriangleAlert } from 'lucide-vue-next'
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
  // ErrorLevel values already match the shared ui-notice state modifiers.
  return `error-toast ui-notice ui-notice--${level}`
}

function getLevelIcon(level: ErrorLevel) {
  switch (level) {
    case ErrorLevel.ERROR:
      return CircleAlert
    case ErrorLevel.WARNING:
      return TriangleAlert
    case ErrorLevel.SUCCESS:
      return CircleCheck
    case ErrorLevel.INFO:
    default:
      return Info
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
          <component :is="getLevelIcon(error.level)" class="toast-icon" :size="16" aria-hidden="true" />
          <span class="toast-message">{{ error.message }}</span>
          <button class="toast-close" :title="t('toast.close')" :aria-label="t('toast.close')" @click.stop="removeError(error.id)">×</button>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<style scoped>
/* A definite width lets the shared max-width: min(480px, 100%) resolve
   against the viewport instead of a shrink-to-fit container, so a narrow
   window never clips the stack; the items stay anchored to the right edge. */
.error-toasts-container {
  position: fixed;
  top: 20px;
  right: 20px;
  left: 20px;
  z-index: 9999;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 10px;
  pointer-events: none;
}

/* The stack keeps its top-right placement; surface, compact form and the
   four state color pairs come from the shared ui-notice classes. The shared
   ui-notice-in keyframes animate the same transform/opacity that the
   TransitionGroup enter/leave classes own, so the local animation is disabled:
   otherwise the enter would play the shared slide from the top instead of the
   stack's slide from the right. */
.error-toast {
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: 12px;
  cursor: pointer;
  transition: all 0.2s ease;
  animation: none;
}

.error-toast:hover {
  transform: translateX(-4px);
}

.toast-icon {
  flex-shrink: 0;
}

.toast-message {
  flex: 1;
  min-width: 0;
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

/* The state fills are light, so the accent-colored ring has too little
   contrast on top of them; the inherited dark text color stays visible. */
.toast-close:focus-visible {
  opacity: 1;
  outline: 2px solid currentColor;
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
