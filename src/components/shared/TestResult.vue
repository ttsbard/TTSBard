<script setup lang="ts">
import { Check, X } from 'lucide-vue-next';
import { t } from '../../i18n';

export interface TestResult {
  success: boolean;
  latency_ms: number | null;
  mode: string;
  error: string | null;
}

interface Props {
  result: TestResult | null;
  successMessage?: string;
  animate?: boolean;
}

defineProps<Props>();
</script>

<template>
  <Transition name="fade" :css="animate !== false">
    <div v-if="result" class="test-result" :class="{ success: result.success, error: !result.success }">
      <Check v-if="result.success" :size="16" />
      <X v-else :size="16" />
      <span v-if="result.success">
        <template v-if="successMessage">{{ successMessage }}</template>
        <template v-else-if="result.latency_ms">{{ t('common.connection_success_latency', { latency_ms: result.latency_ms }) }}</template>
        <template v-else>{{ t('common.connection_success') }}</template>
      </span>
      <span v-else>{{ result.error || t('common.connection_failed') }}</span>
    </div>
  </Transition>
</template>

<style scoped>
.test-result {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-radius: 8px;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  line-height: 1.4;
}

.test-result.success {
  background: var(--success-bg-weak);
  border: 1px solid var(--success-border, rgba(74, 222, 128, 0.24));
  color: var(--success-text-bright);
}

.test-result.error {
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border-strong);
  color: var(--danger-text-bright);
}

.test-result span {
  display: flex;
  align-items: center;
  gap: 6px;
}

/* Fade transition */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
