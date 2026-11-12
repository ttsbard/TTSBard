<script setup lang="ts">
import { ref, watch } from 'vue';
import { Cloud, Plus, Trash2, Loader2 } from 'lucide-vue-next';
import { confirm } from '@tauri-apps/plugin-dialog';
import ProviderCard from '../shared/ProviderCard.vue';
import InputWithToggle from '../shared/InputWithToggle.vue';
import FishAudioModelPicker from './FishAudioModelPicker.vue';
import type { FishAudioConnectionSettingsInput, VoiceModel } from '../../types/settings';
import { t } from '../../i18n';

interface Props {
  active?: boolean;
  expanded?: boolean;
  apiKey?: string;
  referenceId?: string;
  voices?: VoiceModel[];
  format?: string;
  temperature?: number;
  sampleRate?: number;
  useProxy?: boolean;
  onSaveAll?: (data: FishAudioConnectionSettingsInput) => Promise<void>;
}

interface Emits {
  (e: 'select'): void;
  (e: 'toggle'): void;
  (e: 'select-voice', voiceId: string): void;
  (e: 'add-voice', model: VoiceModel): void;
  (e: 'remove-voice', voiceId: string): void;
  (e: 'toggle-proxy', enabled: boolean): void;
}

const props = withDefaults(defineProps<Props>(), {
  active: false,
  expanded: false,
  apiKey: '',
  referenceId: '',
  voices: () => [],
  format: 'mp3',
  temperature: 0.7,
  sampleRate: 44100,
  useProxy: false,
});

const emit = defineEmits<Emits>();

const showModelPicker = ref(false);
// Локальное состояние для ввода API ключа
const localApiKey = ref(props.apiKey);
// Локальное состояние для аудио настроек
const localFormat = ref(props.format);
const localTemperature = ref(props.temperature);
const localSampleRate = ref(props.sampleRate);
const isSaving = ref(false);

// Синхронизация при изменении пропов извне
watch(
  [() => props.apiKey, () => props.format, () => props.temperature, () => props.sampleRate],
  ([apiKey, format, temperature, sampleRate]) => {
    localApiKey.value = apiKey;
    localFormat.value = format;
    localTemperature.value = temperature;
    localSampleRate.value = sampleRate;
  }
);

const audioFormats = [
  { value: 'mp3', label: 'MP3' },
  { value: 'wav', label: 'WAV' },
  { value: 'pcm', label: 'PCM' },
  { value: 'opus', label: 'Opus' },
];

const sampleRates = [
  { value: 8000, label: '8000 Hz' },
  { value: 16000, label: '16000 Hz' },
  { value: 24000, label: '24000 Hz' },
  { value: 32000, label: '32000 Hz' },
  { value: 44100, label: '44100 Hz' },
  { value: 48000, label: '48000 Hz' },
];

function handleOpenModelPicker() {
  showModelPicker.value = true;
}

function handleSelectModel(model: VoiceModel) {
  emit('add-voice', model);
  emit('select-voice', model.id);
  showModelPicker.value = false;
}

async function handleSaveAll() {
  if (!localApiKey.value.trim()) {
    return;
  }

  isSaving.value = true;
  try {
    await props.onSaveAll?.({
      apiKey: localApiKey.value,
      format: localFormat.value,
      temperature: localTemperature.value,
      sampleRate: localSampleRate.value,
    });
  } finally {
    isSaving.value = false;
  }
}

async function handleRemoveVoice(voiceId: string, voiceTitle: string, event: Event) {
  event.stopPropagation();

  const confirmed = await confirm(t('tts.remove_voice.message', { name: voiceTitle }), {
    title: t('tts.remove_voice.title'),
    kind: 'warning'
  });

  if (!confirmed) return;

  emit('remove-voice', voiceId);
}

function handleProxyToggle(event: Event) {
  const target = event.target as HTMLInputElement;
  emit('toggle-proxy', target.checked);
}
</script>

<template>
  <ProviderCard
    title="Fish Audio"
    :icon="Cloud"
    :active="active"
    :expanded="expanded"
    @select="$emit('select')"
    @toggle="$emit('toggle')"
  >
    <div class="card-content-inner">
      <!-- API Key -->
      <div class="setting-group">
        <div class="form-row">
          <label class="ui-label">{{ t('tts.api_key') }}</label>
          <InputWithToggle
            :model-value="localApiKey"
            @update:model-value="localApiKey = $event"
            type="password"
            :placeholder="t('tts.api_key_placeholder')"
            class="input-wide"
            ui
          />
        </div>
      </div>

      <!-- Audio Settings -->
      <div class="setting-group">
        <!-- Format and Sample Rate in one row -->
        <div class="audio-settings-row">
          <div class="audio-setting">
            <label class="ui-label">{{ t('tts.format') }}</label>
            <select
              :value="localFormat"
              @change="localFormat = ($event.target as HTMLSelectElement).value"
              class="ui-select setting-select"
            >
              <option v-for="f in audioFormats" :key="f.value" :value="f.value">
                {{ f.label }}
              </option>
            </select>
          </div>

          <div class="audio-setting">
            <label class="ui-label">{{ t('tts.sample_rate') }}</label>
            <select
              :value="localSampleRate"
              @change="localSampleRate = Number(($event.target as HTMLSelectElement).value)"
              class="ui-select setting-select"
            >
              <option v-for="sr in sampleRates" :key="sr.value" :value="sr.value">
                {{ sr.label }}
              </option>
            </select>
          </div>
        </div>

        <!-- Temperature in separate row -->
        <div class="audio-settings-row">
          <div class="audio-setting">
            <label class="ui-label">{{ t('tts.temperature') }} <span class="temperature-value">{{ localTemperature }}</span></label>
            <input
              type="range"
              :value="localTemperature"
              @input="localTemperature = Number(($event.target as HTMLInputElement).value)"
              min="0"
              max="1"
              step="0.1"
              class="temperature-slider"
            />
          </div>
        </div>

        <!-- Save Button -->
        <div class="button-row">
          <button
            @click="handleSaveAll"
            :disabled="isSaving"
            class="ui-button ui-button--primary save-button-inline"
          >
            <Loader2 v-if="isSaving" :size="16" class="ui-spin" />
            {{ isSaving ? t('tts.saving') : t('common.save') }}
          </button>
        </div>
      </div>

      <!-- Proxy -->
      <div class="setting-group">
        <div class="proxy-checkbox-container">
          <input
            id="fish-use-proxy"
            type="checkbox"
            :checked="useProxy"
            @change="handleProxyToggle"
            class="ui-choice-input"
          />
          <label for="fish-use-proxy" class="ui-choice-label">
            {{ t('tts.use_socks5') }}
          </label>
        </div>
      </div>

      <!-- Voice Management -->
      <div class="setting-group">
        <div class="voice-header">
          <label class="ui-label">{{ t('tts.voices') }}</label>
          <button @click="handleOpenModelPicker" class="ui-icon-button" :title="t('tts.add_voice')" :aria-label="t('tts.add_voice')">
            <Plus :size="18" />
          </button>
        </div>

        <div v-if="voices.length > 0" class="voice-list ui-menu ui-menu--embedded">
          <div
            v-for="voice in voices"
            :key="voice.id"
            :class="['voice-item', { 'ui-menu-item--selected': referenceId === voice.id }]"
          >
            <button class="voice-info ui-menu-item" @click="$emit('select-voice', voice.id)">
              <div class="voice-title">{{ voice.title }}</div>
              <div class="voice-details">
                <span v-if="voice.languages.length" class="voice-languages">
                  {{ voice.languages.join(', ') }}
                </span>
                <span v-if="voice.description" class="voice-description">{{ voice.description }}</span>
              </div>
            </button>

            <button
              @click="handleRemoveVoice(voice.id, voice.title, $event)"
              class="ui-icon-button ui-action--danger remove-button"
              :title="t('tts.delete')"
              :aria-label="t('tts.delete')"
            >
              <Trash2 :size="14" />
            </button>
          </div>
        </div>
        <div v-else class="empty-voices">
          {{ t('tts.no_voices_added') }}
        </div>
      </div>
    </div>
  </ProviderCard>

  <!-- Model Picker Modal -->
  <FishAudioModelPicker
    v-if="showModelPicker"
    :api-key="apiKey"
    @select="handleSelectModel"
    @close="showModelPicker = false"
  />
</template>

<style scoped>
.card-content-inner {
  padding-top: 8px;
}

.setting-group {
  margin-bottom: var(--ui-row-gap);
}

.setting-group:last-child {
  margin-bottom: 0;
}

.setting-group > .ui-label {
  display: block;
  margin-bottom: 8px;
}

.form-row {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}

.input-wide {
  flex: 1;
  min-width: 200px;
}

.button-row {
  display: flex;
  gap: 0.75rem;
  flex-wrap: wrap;
  align-items: center;
  justify-content: flex-end;
  margin-top: 0.5rem;
  padding-top: 0.5rem;
  border-top: 1px solid var(--color-border);
}

.save-button-inline {
  gap: 8px;
}

.voice-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}

.voice-header .ui-label {
  margin-bottom: 0;
}

.voice-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 300px;
  overflow-y: auto;
}

.voice-item {
  display: flex;
  align-items: center;
  padding-right: 4px;
  border-radius: var(--ui-radius-menu-item);
}

.voice-item:hover {
  background: var(--color-bg-field-hover);
}

.voice-info {
  flex: 1;
  min-width: 0;
}

.voice-title {
  color: inherit;
  margin-bottom: 2px;
}

.voice-item.ui-menu-item--selected .voice-info {
  color: inherit;
  font-weight: inherit;
}

.voice-details {
  display: flex;
  align-items: center;
  gap: 8px;
}

.voice-languages {
  font-size: var(--ui-text-size-hint);
  text-transform: uppercase;
  color: var(--color-text-muted);
  flex-shrink: 0;
}

.voice-description {
  font-size: var(--ui-text-size-hint);
  color: var(--color-text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.empty-voices {
  padding: 1rem;
  text-align: center;
  color: var(--color-text-secondary);
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  background: var(--color-bg-field);
  border-radius: 8px;
}

.audio-settings-row {
  display: flex;
  flex-direction: row;
  gap: 12px;
}

.audio-settings-row:not(:first-child) {
  margin-top: 12px;
}

.audio-setting {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 1;
}

.card-content-inner {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr) max-content minmax(0, 1fr);
  column-gap: var(--ui-row-label-gap-side);
}

.card-content-inner > .setting-group {
  grid-column: 1 / -1;
}

.card-content-inner > .setting-group:nth-child(-n + 2),
.card-content-inner > .setting-group:first-child .form-row,
.card-content-inner > .setting-group:nth-child(2) > .audio-settings-row:first-child {
  display: grid;
  grid-template-columns: subgrid;
  grid-column: 1 / -1;
  align-items: center;
}

.card-content-inner > .setting-group:first-child .input-wide {
  grid-column: 2 / -1;
  min-width: 0;
}

.card-content-inner > .setting-group:nth-child(2) > .audio-settings-row:first-child .audio-setting {
  display: contents;
}

.card-content-inner > .setting-group:nth-child(2) > .audio-settings-row:not(:first-child),
.card-content-inner > .setting-group:nth-child(2) > .button-row {
  grid-column: 1 / -1;
}

.setting-select {
  flex: 1;
}

.temperature-value {
  color: var(--color-text-primary);
  font-variant-numeric: tabular-nums;
}

.temperature-slider {
  flex: 1;
  cursor: pointer;
  accent-color: var(--color-accent);
}

.proxy-checkbox-container {
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
