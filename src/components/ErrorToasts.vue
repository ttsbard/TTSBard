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
  // The level values already match the shared ui-notice state modifiers.
  return `error-toast ui-notice ui-notice--${level}`
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

/* The stack stays in its existing top-right corner; surface, semantic accent
   and typography come from the shared ui-notice classes. */
.error-toast {
  pointer-events: auto;
  display: flex;
  align-items: center;
  /* Safe centering keeps short messages centered, but when a long message
     needs the shared max-height scrollport it aligns to the start instead of
     clipping the top of the text above the unreachable scroll origin. */
  align-items: safe center;
  gap: 12px;
  cursor: pointer;
  transition: all 0.2s ease;
}

.error-toast:hover {
  transform: translateX(-4px);
}

.toast-icon {
  font-size: 18px;
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
  /* The shared notice scrolls when long text reaches the viewport limit.
     Keep the focus ring and the button itself inside the scrollport when
     the browser scrolls it into view. */
  scroll-margin: 4px;
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
