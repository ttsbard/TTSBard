<script setup lang="ts">
import { computed, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { Cpu, Loader2, RefreshCw } from 'lucide-vue-next';
import ProviderCard from '../shared/ProviderCard.vue';
import type { TtsProviderInfoDto } from '../../types/settings';
import { t } from '../../i18n';
import { presentCommandError } from '../../ipc/commandError';
import { getPiperProviderUiStatus } from '../ttsProviderSelection';
import './local-model-list.css';

interface Props {
  providers: TtsProviderInfoDto[];
  activeId?: string | null;
  loading?: Record<string, boolean>;
  errors?: Record<string, string | null>;
}

const props = withDefaults(defineProps<Props>(), {
  activeId: null,
  loading: () => ({}),
  errors: () => ({}),
});

interface Emits {
  (e: 'select', id: string): void;
  (e: 'refreshed', providers: TtsProviderInfoDto[]): void;
}

const emit = defineEmits<Emits>();

const REFRESH_PIPER_MODELS_COMMAND = 'refresh_piper_models';

interface PiperStatus {
  kind: 'none' | 'loading' | 'ready' | 'error';
  text: string;
  title?: string;
}

interface PiperRow {
  provider: TtsProviderInfoDto;
  status: PiperStatus;
}

const refreshing = ref(false);
const refreshError = ref<string | null>(null);

const isActive = computed(() => props.activeId !== null);

function rowStatus(provider: TtsProviderInfoDto): PiperStatus {
  const ui = getPiperProviderUiStatus(
    provider,
    !!props.loading[provider.id],
    props.errors[provider.id],
  );
  if (ui.kind === 'loading') return { kind: 'loading', text: t('tts.model.loading') };
  if (ui.kind === 'ready') return { kind: 'ready', text: t('tts.model.ready') };
  if (ui.kind === 'error') {
    return { kind: 'error', text: t('tts.model.load_error'), title: ui.label };
  }
  return { kind: 'none', text: '' };
}

const rows = computed<PiperRow[]>(() =>
  props.providers.map(provider => ({ provider, status: rowStatus(provider) })),
);

const failedRows = computed(() => rows.value.filter(row => row.status.kind === 'error'));

async function refreshModels(): Promise<void> {
  if (refreshing.value) return;

  refreshing.value = true;
  refreshError.value = null;
  try {
    const result = await invoke<TtsProviderInfoDto[]>(REFRESH_PIPER_MODELS_COMMAND);
    emit('refreshed', Array.isArray(result) ? result : []);
  } catch (error) {
    refreshError.value = presentCommandError(error, t('tts.piper.error.refresh'));
  } finally {
    refreshing.value = false;
  }
}
</script>

<template>
  <ProviderCard
    :title="t('tts.piper.title')"
    :icon="Cpu"
    :active="isActive"
    :selectable="false"
    :collapsible="false"
  >
    <template #header-actions>
      <button
        type="button"
        class="ui-icon-button piper-refresh-button"
        :title="t('tts.piper.refresh')"
        :aria-label="t('tts.piper.refresh')"
        :disabled="refreshing"
        @click="refreshModels"
      >
        <RefreshCw :size="18" :class="{ 'ui-spin': refreshing }" aria-hidden="true" />
      </button>
    </template>

    <div class="card-content-inner" :aria-busy="refreshing">
      <div v-if="refreshError" class="local-model-message local-model-message--error" role="alert">
        <span class="local-model-message-text">{{ refreshError }}</span>
        <button type="button" class="ui-button ui-button--compact" @click="refreshModels">
          {{ t('tts.piper.retry') }}
        </button>
      </div>

      <p v-if="refreshing && providers.length === 0" class="ui-hint">{{ t('tts.piper.loading') }}</p>

      <div v-else-if="providers.length === 0" class="piper-empty">
        <p class="ui-hint">{{ t('tts.piper.empty') }}</p>
        <p class="ui-hint">{{ t('tts.piper.empty_hint') }}</p>
      </div>

      <template v-else>
        <div class="local-model-list" role="radiogroup" :aria-label="t('tts.piper.title')">
          <label
            v-for="row in rows"
            :key="row.provider.id"
            class="local-model-row"
            :class="{ 'local-model-row--active': row.provider.id === activeId }"
          >
            <input
              class="local-model-radio"
              type="radio"
              name="piper-provider"
              :checked="row.provider.id === activeId"
              :disabled="!!loading[row.provider.id]"
              :aria-busy="!!loading[row.provider.id]"
              @change="emit('select', row.provider.id)"
            />
            <span class="local-model-name ui-choice-label">{{ row.provider.display_name }}</span>
            <Loader2
              v-if="row.status.kind === 'loading'"
              :size="14"
              class="ui-spin"
              aria-hidden="true"
            />
            <span
              v-if="row.status.kind !== 'none'"
              class="local-model-status"
              :class="`local-model-status--${row.status.kind}`"
              :title="row.status.title"
            >{{ row.status.text }}</span>
          </label>
        </div>

        <div
          v-for="row in failedRows"
          :key="`error-${row.provider.id}`"
          class="local-model-message local-model-message--error"
          role="alert"
        >
          <span class="local-model-message-text">{{ row.status.title ?? row.status.text }}</span>
        </div>
      </template>
    </div>
  </ProviderCard>
</template>

<style scoped>
.card-content-inner {
  padding-top: 8px;
  min-width: 0;
}

.piper-empty {
  padding: 4px 0 2px;
}

.piper-empty .ui-hint:first-child {
  margin-top: 0;
}
</style>
