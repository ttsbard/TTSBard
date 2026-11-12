<script setup lang="ts">
import { computed } from 'vue';
import { Cpu, Loader2 } from 'lucide-vue-next';
import ProviderCard from '../shared/ProviderCard.vue';
import type { TtsProviderInfoDto } from '../../types/settings';
import { t } from '../../i18n';
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
}

const emit = defineEmits<Emits>();

interface PiperStatus {
  kind: 'none' | 'loading' | 'ready' | 'error';
  text: string;
  title?: string;
}

interface PiperRow {
  provider: TtsProviderInfoDto;
  status: PiperStatus;
}

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
</script>

<template>
  <ProviderCard
    :title="t('tts.piper.title')"
    :icon="Cpu"
    :active="isActive"
    :selectable="false"
    :collapsible="false"
  >
    <div class="card-content-inner">
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
    </div>
  </ProviderCard>
</template>

<style scoped>
.card-content-inner {
  padding-top: 8px;
  min-width: 0;
}
</style>
