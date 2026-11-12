<script setup lang="ts">
import { ref, watch } from 'vue';
import { Cloud } from 'lucide-vue-next';
import ProviderCard from '../shared/ProviderCard.vue';
import InputWithToggle from '../shared/InputWithToggle.vue';
import VoiceSelector from './VoiceSelector.vue';
import { t } from '../../i18n';

interface Props {
  active?: boolean;
  expanded?: boolean;
  apiKey?: string;
  voice?: string;
  voices?: string[];
  useProxy?: boolean;
  loading?: boolean;
}

interface Emits {
  (e: 'select'): void;
  (e: 'toggle'): void;
  (e: 'save-api-key', key: string): void;
  (e: 'voice-change', voice: string): void;
  (e: 'toggle-proxy', enabled: boolean): void;
}

const props = withDefaults(defineProps<Props>(), {
  active: false,
  expanded: false,
  apiKey: '',
  voice: 'alloy',
  voices: () => ['alloy', 'echo', 'fable', 'onyx', 'nova', 'shimmer'],
  useProxy: false,
  loading: false,
});

const emit = defineEmits<Emits>();

const localApiKey = ref(props.apiKey);

watch(() => props.apiKey, (val) => {
  localApiKey.value = val;
});

function handleSaveApiKey() {
  if (!localApiKey.value.trim()) return;
  emit('save-api-key', localApiKey.value);
}

function handleVoiceChange(voice: string) {
  emit('voice-change', voice);
}

function handleProxyToggle(event: Event) {
  const target = event.target as HTMLInputElement;
  emit('toggle-proxy', target.checked);
}
</script>

<template>
  <ProviderCard
    title="OpenAI TTS"
    :icon="Cloud"
    :active="active"
    :expanded="expanded"
    @select="$emit('select')"
    @toggle="$emit('toggle')"
  >
    <div class="card-content-inner">
      <!-- API Key -->
      <div class="setting-group">
        <div class="openai-form-row">
          <label class="ui-label">{{ t('tts.api_key') }}</label>
          <InputWithToggle
            v-model="localApiKey"
            type="password"
            placeholder="sk-..."
            class="openai-input-wide"
            ui
          />
          <button @click="handleSaveApiKey" class="ui-button ui-button--primary save-settings-button">{{ t('common.save') }}</button>
        </div>
      </div>

      <!-- Voice -->
      <div class="setting-group">
        <VoiceSelector
          :voices="voices"
          :selected-voice-id="voice"
          :loading="loading"
          :label="t('tts.voice')"
          @voice-change="handleVoiceChange"
        />
      </div>

      <!-- Proxy -->
      <div class="setting-group">
        <div class="proxy-checkbox-container">
          <input
            id="openai-use-proxy"
            type="checkbox"
            :checked="useProxy"
            @change="handleProxyToggle"
            class="ui-choice-input"
          />
          <label for="openai-use-proxy" class="ui-choice-label">
            {{ t('tts.use_socks5') }}
          </label>
        </div>
      </div>
    </div>
  </ProviderCard>
</template>

<style scoped>
.card-content-inner {
  padding-top: 8px;
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr) auto;
  column-gap: var(--ui-row-label-gap-side);
  row-gap: var(--ui-row-gap);
}

.setting-group {
  grid-column: 1 / -1;
}

.card-content-inner > .setting-group:nth-child(-n + 2),
.card-content-inner .openai-form-row {
  display: grid;
  grid-template-columns: subgrid;
  grid-column: 1 / -1;
  margin-bottom: 0;
}

.card-content-inner :deep(.voice-selector) {
  display: grid;
  grid-template-columns: subgrid;
  grid-column: 1 / -1;
  gap: var(--ui-row-label-gap-side);
}

.card-content-inner :deep(.voice-select-wrapper) {
  width: 50%;
  min-width: min(100px, 100%);
}

.setting-group {
  margin-bottom: var(--ui-row-gap);
}

.setting-group:last-child {
  margin-bottom: 0;
}

/* OpenAI form row */
.openai-form-row {
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}

.openai-form-row .ui-label {
  min-width: 60px;
}

.openai-input-wide {
  flex: 1;
  min-width: 200px;
}

.save-settings-button {
  flex-shrink: 0;
}

/* Proxy checkbox container */
.proxy-checkbox-container {
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
