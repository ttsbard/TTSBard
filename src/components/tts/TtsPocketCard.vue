<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { Loader2, Play, RefreshCw, Sparkles, Waves } from 'lucide-vue-next';
import ProviderCard from '../shared/ProviderCard.vue';
import { t } from '../../i18n';
import { presentCommandError } from '../../ipc/commandError';
import { submitSpeech } from '../../ipc/speech';
import { createAsyncCleanupScope } from '../../utils/asyncCleanup';
import { debugWarn } from '../../utils/debug';
import {
  canTestPocketModel,
  isPocketReferenceModel,
  nextPocketFocusId,
  pickPocketTestPhrase,
  pocketLanguageLabel,
  pocketLanguageView,
  pocketSelectedReference,
  pocketSelectedVoice,
  pocketStatusHintKey,
  pocketStatusKind,
  pocketStatusLabelKey,
  replacePocketModel,
  type PocketModelDto,
  type PocketReadiness,
  type PocketReferenceCheckDto,
  type PocketVoiceDto,
} from './pocketTts';

interface Props {
  activeModelId?: string | null;
  speakerDevice?: string | null;
  speakerVolume?: number;
}

const props = withDefaults(defineProps<Props>(), {
  activeModelId: null,
  speakerDevice: null,
  speakerVolume: 100,
});

interface Emits {
  (e: 'activate', id: string): void;
}

const emit = defineEmits<Emits>();

const LIST_POCKET_MODELS_COMMAND = 'list_pocket_models';
const REFRESH_POCKET_MODELS_COMMAND = 'refresh_pocket_models';

interface Feedback {
  modelId: string;
  kind: 'success' | 'error';
  text: string;
}

const expanded = ref(false);
const listenerScope = createAsyncCleanupScope();
const models = ref<PocketModelDto[]>([]);
const loading = ref(false);
const listError = ref<string | null>(null);
const focusId = ref<string | null>(null);
const selectedModelId = ref<string | null>(props.activeModelId);
const activatingId = ref<string | null>(null);
const pendingActivationId = ref<string | null>(null);
const feedback = ref<Feedback | null>(null);
const voiceBusyFor = ref<string | null>(null);
const previewingFor = ref<string | null>(null);
const testingFor = ref<string | null>(null);
const lastTestPhrase = ref<string | null>(null);

let snapshotGeneration = 0;
let activationGeneration = 0;
let voiceGeneration = 0;
let previewGeneration = 0;
let testGeneration = 0;

const isPocketActive = computed(() => props.activeModelId !== null);

const focusModel = computed(() => {
  const id = focusId.value;
  return id ? models.value.find(model => model.id === id) ?? null : null;
});

const focusReference = computed(() => {
  const model = focusModel.value;
  return model ? pocketSelectedReference(model) : null;
});

const focusVoice = computed(() => {
  const model = focusModel.value;
  return model ? pocketSelectedVoice(model) : null;
});

const focusLanguage = computed(() => {
  const model = focusModel.value;
  if (!model) return null;
  return {
    view: pocketLanguageView(model.language),
    label: pocketLanguageLabel(pocketLanguageView(model.language), t),
  };
});

const currentFeedback = computed<Feedback | null>(() => {
  const modelId = focusModel.value?.id;
  if (!modelId || feedback.value?.modelId !== modelId) return null;
  return feedback.value;
});

const currentHintKey = computed<string | null>(() => {
  const model = focusModel.value;
  return model ? pocketStatusHintKey(model.readiness) : null;
});

const isVoiceBusy = computed(() => !!focusModel.value && voiceBusyFor.value === focusModel.value.id);
const isPreviewing = computed(() => !!focusModel.value && previewingFor.value === focusModel.value.id);
const isTesting = computed(() => !!focusModel.value && testingFor.value === focusModel.value.id);
const canTest = computed(() => canTestPocketModel(focusModel.value));
const isFocusActive = computed(() => !!focusModel.value && focusModel.value.id === props.activeModelId);

const activeVoiceId = computed(() => focusVoice.value?.voice_id ?? null);

const referenceButtonLabel = computed(() =>
  focusReference.value ? t('tts.pocket.replace') : t('tts.pocket.choose_wav'),
);

function languageCode(language: string): string {
  return pocketLanguageView(language).code;
}

function languageTitle(language: string): string {
  return pocketLanguageLabel(pocketLanguageView(language), t);
}

function statusLabel(readiness: PocketReadiness): string {
  return t(pocketStatusLabelKey(readiness));
}

function statusHint(readiness: PocketReadiness): string | null {
  const key = pocketStatusHintKey(readiness);
  return key ? t(key) : null;
}

function setFeedback(modelId: string, kind: Feedback['kind'], text: string) {
  feedback.value = { modelId, kind, text };
}

function clearFeedback(modelId: string) {
  if (feedback.value?.modelId === modelId) feedback.value = null;
}

function stopReferencePreview() {
  if (previewingFor.value === null) return;
  previewGeneration += 1;
  previewingFor.value = null;
  invoke('stop_preview').catch(() => {});
}

function focusModelById(id: string | null) {
  if (focusId.value !== id) {
    stopReferencePreview();
    pendingActivationId.value = null;
  }
  focusId.value = id;
}

function applySnapshot(result: PocketModelDto[]) {
  models.value = Array.isArray(result) ? result : [];
  const activeId = props.activeModelId;
  if (!activatingId.value) {
    selectedModelId.value = activeId && models.value.some(model => model.id === activeId)
      ? activeId
      : null;
  }
  // A manual refresh must not pull the details away from a model the user is
  // configuring while another pocket model is active. Fall back to the active
  // model only when the current focus disappeared from the snapshot.
  const focusStillExists = !!focusId.value && models.value.some(model => model.id === focusId.value);
  const nextFocus = focusStillExists
    ? focusId.value
    : nextPocketFocusId(models.value, activeId, null);
  if (nextFocus !== focusId.value) {
    focusModelById(nextFocus);
  } else if (!nextFocus) {
    focusModelById(null);
  }
}

async function loadModels(mode: 'list' | 'refresh'): Promise<void> {
  const token = ++snapshotGeneration;
  loading.value = true;
  listError.value = null;
  try {
    const result = mode === 'refresh'
      ? await invoke<PocketModelDto[]>(REFRESH_POCKET_MODELS_COMMAND)
      : await invoke<PocketModelDto[]>(LIST_POCKET_MODELS_COMMAND);
    if (token !== snapshotGeneration) return;
    applySnapshot(result);
  } catch (error) {
    if (token !== snapshotGeneration) return;
    listError.value = presentCommandError(error, t('tts.pocket.error.refresh'));
  } finally {
    if (token === snapshotGeneration) loading.value = false;
  }
}

function refreshModels() {
  void loadModels('refresh');
}

async function activateModelById(
  id: string,
  options: { keepSuccessMessage?: boolean } = {},
): Promise<boolean> {
  const model = models.value.find(entry => entry.id === id);
  if (!model || activatingId.value !== null) return false;

  focusModelById(id);
  const token = ++activationGeneration;
  selectedModelId.value = id;
  activatingId.value = id;
  if (!options.keepSuccessMessage) clearFeedback(id);

  try {
    await invoke('select_tts_provider_by_id', { id });
    if (token !== activationGeneration) return false;
    selectModelAfterSuccess(id, options.keepSuccessMessage === true);
    return true;
  } catch (error) {
    if (token !== activationGeneration) return false;
    selectedModelId.value = props.activeModelId && models.value.some(entry => entry.id === props.activeModelId)
      ? props.activeModelId
      : null;
    pendingActivationId.value = id;
    const hintKey = pocketStatusHintKey(model.readiness);
    setFeedback(
      id,
      'error',
      hintKey ? t(hintKey) : presentCommandError(error, t('tts.pocket.error.select_model')),
    );
    return false;
  } finally {
    if (token === activationGeneration) activatingId.value = null;
  }
}

function selectModelAfterSuccess(id: string, keepSuccessMessage: boolean) {
  pendingActivationId.value = null;
  selectedModelId.value = id;
  emit('activate', id);
  if (!keepSuccessMessage) {
    setFeedback(id, 'success', t('tts.pocket.status.selected'));
  }
}

function handleHeaderSelect() {
  if (isPocketActive.value) return;
  const target = focusModel.value ?? models.value[0];
  if (target) void activateModelById(target.id);
}

function handleModelChange(model: PocketModelDto) {
  void activateModelById(model.id);
}

function handleModelRowClick(model: PocketModelDto) {
  if (model.id !== selectedModelId.value) return;
  // A checked radio does not emit `change`. Clicking the active model's row
  // must still return the details to it after configuring another model.
  if (model.id === props.activeModelId) {
    focusModelById(model.id);
    return;
  }
  void activateModelById(model.id);
}

async function chooseReference() {
  const model = focusModel.value;
  if (!model || voiceBusyFor.value !== null) return;
  voiceBusyFor.value = model.id;

  let picked: string | string[] | null = null;
  try {
    picked = await open({
      filters: [{ name: t('tts.pocket.dialog_filter'), extensions: ['wav'] }],
      multiple: false,
    });
  } catch {
    setFeedback(model.id, 'error', t('tts.pocket.error.dialog'));
    return;
  } finally {
    voiceBusyFor.value = null;
  }

  const path = Array.isArray(picked) ? picked[0] : picked;
  if (!path) return;

  await applyReference(model.id, path);
}

async function applyReference(modelId: string, path: string): Promise<void> {
  const token = ++voiceGeneration;
  voiceBusyFor.value = modelId;
  clearFeedback(modelId);

  try {
    const check = await invoke<PocketReferenceCheckDto>('validate_pocket_reference', { path });
    if (token !== voiceGeneration) return;

    if (check.status !== 'ready') {
      setFeedback(
        modelId,
        'error',
        check.status === 'reference_missing'
          ? t('tts.pocket.error.reference_missing')
          : t('tts.pocket.error.reference_invalid'),
      );
      return;
    }

    const updated = await invoke<PocketModelDto>('set_pocket_voice', {
      modelId,
      voice: { kind: 'reference_wav', path },
    });
    if (!updated || updated.id !== modelId) {
      throw new Error('Pocket voice response does not match the requested model');
    }
    models.value = replacePocketModel(models.value, updated);

    if (token !== voiceGeneration) return;
    setFeedback(modelId, 'success', t('tts.pocket.reference_saved'));

    if (focusModel.value?.id === modelId && pendingActivationId.value === modelId) {
      pendingActivationId.value = null;
      await activateModelById(modelId, { keepSuccessMessage: true });
    }
  } catch (error) {
    if (token !== voiceGeneration) return;
    setFeedback(modelId, 'error', presentCommandError(error, t('tts.pocket.error.save_reference')));
  } finally {
    if (token === voiceGeneration) voiceBusyFor.value = null;
  }
}

async function selectReadyVoice(voice: PocketVoiceDto) {
  const model = focusModel.value;
  if (!model || !voice.available || voiceBusyFor.value !== null) return;

  const modelId = model.id;
  const token = ++voiceGeneration;
  voiceBusyFor.value = modelId;
  clearFeedback(modelId);

  try {
    const updated = await invoke<PocketModelDto>('set_pocket_voice', {
      modelId,
      voice: { kind: 'voice_state', voice_id: voice.id },
    });
    if (!updated || updated.id !== modelId) {
      throw new Error('Pocket voice response does not match the requested model');
    }
    models.value = replacePocketModel(models.value, updated);

    if (token !== voiceGeneration) return;
    setFeedback(modelId, 'success', t('tts.pocket.voice_saved'));

    if (focusModel.value?.id === modelId && pendingActivationId.value === modelId) {
      pendingActivationId.value = null;
      await activateModelById(modelId, { keepSuccessMessage: true });
    }
  } catch (error) {
    if (token !== voiceGeneration) return;
    setFeedback(modelId, 'error', presentCommandError(error, t('tts.pocket.error.select_voice')));
  } finally {
    if (token === voiceGeneration) voiceBusyFor.value = null;
  }
}

async function playReference() {
  const model = focusModel.value;
  const reference = focusReference.value;
  if (!model || !reference || previewingFor.value !== null) return;

  const token = ++previewGeneration;
  previewingFor.value = model.id;
  clearFeedback(model.id);

  try {
    await invoke('preview_audio_file', {
      filePath: reference.path,
      speakerDevice: props.speakerDevice,
      speakerVolume: props.speakerVolume,
      voiceTransformEnabled: false,
      pitch: 0,
      speed: 0,
      volume: 100,
      enhanceEnabled: false,
      enhanceAttenDb: 12,
      dspSettings: null,
    });
  } catch (error) {
    if (token !== previewGeneration) return;
    setFeedback(model.id, 'error', presentCommandError(error, t('tts.pocket.error.play_reference')));
    void loadModels('list');
  } finally {
    if (token === previewGeneration) previewingFor.value = null;
  }
}

async function testSynthesis() {
  const model = focusModel.value;
  if (!model || !canTest.value || testingFor.value !== null || activatingId.value !== null) return;

  const token = ++testGeneration;
  testingFor.value = model.id;
  clearFeedback(model.id);

  if (props.activeModelId !== model.id) {
    const activated = await activateModelById(model.id, { keepSuccessMessage: true });
    if (!activated || token !== testGeneration) {
      if (token === testGeneration) testingFor.value = null;
      return;
    }
  }

  const phrase = pickPocketTestPhrase(model.language, lastTestPhrase.value);
  lastTestPhrase.value = phrase;

  try {
    await submitSpeech(phrase);
    if (token !== testGeneration) return;
    setFeedback(model.id, 'success', t('tts.pocket.test_started', { phrase }));
  } catch (error) {
    if (token !== testGeneration) return;
    setFeedback(model.id, 'error', presentCommandError(error, t('tts.pocket.error.test_synthesis')));
    if (isPocketReadinessError(error)) void loadModels('list');
  } finally {
    if (token === testGeneration) testingFor.value = null;
  }
}

function isPocketReadinessError(error: unknown): boolean {
  if (!error || typeof error !== 'object' || !('code' in error)) return false;
  return ['pocket.reference_missing', 'pocket.reference_invalid', 'pocket.package_invalid']
    .includes(String(error.code));
}

watch(
  () => props.activeModelId,
  (id) => {
    if (id) {
      selectedModelId.value = id;
      if (models.value.some(model => model.id === id)) {
        focusModelById(id);
      }
    } else {
      selectedModelId.value = null;
    }
  },
  { immediate: true },
);

watch(expanded, (isOpen) => {
  if (isOpen) void loadModels('list');
});

onMounted(() => {
  void loadModels('list');
  void listenerScope.track(listen('pocket-models-changed', () => {
    if (!listenerScope.disposed) void loadModels('list');
  })).catch(error => debugWarn('[TtsPocketCard] Failed to subscribe to model changes:', error));
});

onBeforeUnmount(() => {
  listenerScope.dispose();
  previewGeneration += 1;
  if (previewingFor.value !== null) {
    previewingFor.value = null;
    invoke('stop_preview').catch(() => {});
  }
});
</script>

<template>
  <ProviderCard
    :title="t('tts.pocket.title')"
    :icon="Waves"
    :active="isPocketActive"
    :expanded="expanded"
    @select="handleHeaderSelect"
    @toggle="expanded = !expanded"
  >
    <div class="card-content-inner" :aria-busy="loading">
      <div class="pocket-toolbar">
        <div class="card-subtitle">{{ t('tts.pocket.desc') }}</div>
        <button
          type="button"
          class="ui-icon-button pocket-refresh-button"
          :title="t('tts.pocket.refresh')"
          :aria-label="t('tts.pocket.refresh')"
          :disabled="loading"
          @click="refreshModels"
        >
          <RefreshCw :size="18" :class="{ 'ui-spin': loading }" aria-hidden="true" />
        </button>
      </div>

      <div v-if="listError" class="pocket-message pocket-message--error" role="alert">
        <span class="pocket-message-text">{{ listError }}</span>
        <button type="button" class="ui-button ui-button--compact" @click="refreshModels">
          {{ t('tts.pocket.retry') }}
        </button>
      </div>

      <p v-if="loading && models.length === 0" class="ui-hint">{{ t('tts.pocket.loading') }}</p>

      <div v-else-if="models.length === 0" class="pocket-empty">
        <p class="ui-hint">{{ t('tts.pocket.empty') }}</p>
        <p class="ui-hint">{{ t('tts.pocket.empty_hint') }}</p>
      </div>

      <template v-else>
        <div class="pocket-model-list" role="radiogroup" :aria-label="t('tts.pocket.models')">
          <label
            v-for="model in models"
            :key="model.id"
            class="pocket-model-row"
            :class="{ 'pocket-model-row--active': model.id === props.activeModelId }"
            @click="handleModelRowClick(model)"
          >
            <input
              class="ui-choice-input pocket-model-radio"
              type="radio"
              name="pocket-model"
              :value="model.id"
              :checked="model.id === selectedModelId"
              :aria-busy="activatingId === model.id"
              :disabled="activatingId !== null"
              @change="handleModelChange(model)"
            />
            <span class="pocket-model-name ui-choice-label">{{ model.display_name }}</span>
            <span
              class="pocket-model-language"
              :title="languageTitle(model.language)"
            >{{ languageCode(model.language) }}</span>
            <span
              class="pocket-model-status"
              :class="`pocket-status--${pocketStatusKind(model.readiness)}`"
              :title="statusHint(model.readiness) || undefined"
            >{{ statusLabel(model.readiness) }}</span>
          </label>
        </div>

        <div v-if="focusModel" class="pocket-details">
          <div class="pocket-details-header">
            <span class="ui-group-title pocket-details-title">{{ focusModel.display_name }}</span>
            <span
              class="pocket-model-language"
              :title="focusLanguage?.label"
            >{{ focusLanguage?.view.code }}</span>
            <span
              class="pocket-model-status"
              :class="`pocket-status--${pocketStatusKind(focusModel.readiness)}`"
            >{{ statusLabel(focusModel.readiness) }}</span>
          </div>

          <template v-if="isPocketReferenceModel(focusModel)">
            <div class="pocket-field">
              <span class="ui-label pocket-field-label">{{ t('tts.pocket.reference') }}</span>
              <div v-if="focusReference" class="pocket-reference-row">
                <span
                  class="pocket-reference-name ui-label--secondary"
                  :title="focusReference.path"
                >{{ focusReference.file_name }}</span>
                <button
                  type="button"
                  class="ui-button ui-button--compact pocket-replace-button"
                  :disabled="isVoiceBusy"
                  @click="chooseReference"
                >
                  <Loader2 v-if="isVoiceBusy" :size="16" class="ui-spin" aria-hidden="true" />
                  {{ referenceButtonLabel }}
                </button>
                <button
                  type="button"
                  class="ui-button ui-button--compact ui-button--leading-icon pocket-listen-button"
                  :disabled="isVoiceBusy || isPreviewing"
                  @click="playReference"
                >
                  <Play :size="16" aria-hidden="true" />
                  {{ isPreviewing ? t('tts.pocket.playing') : t('tts.pocket.listen') }}
                </button>
              </div>
              <div v-else class="pocket-reference-row">
                <button
                  type="button"
                  class="ui-button ui-button--primary pocket-choose-button"
                  :disabled="isVoiceBusy"
                  @click="chooseReference"
                >
                  <Loader2 v-if="isVoiceBusy" :size="16" class="ui-spin" aria-hidden="true" />
                  {{ t('tts.pocket.choose_wav') }}
                </button>
              </div>
              <p class="ui-hint">{{ t('tts.pocket.reference_hint') }}</p>
            </div>
          </template>

          <template v-else>
            <div class="pocket-field">
              <span class="ui-label pocket-field-label">{{ t('tts.pocket.voice') }}</span>
              <div v-if="focusModel.voices.length > 0" class="pocket-voice-list">
                <label
                  v-for="voice in focusModel.voices"
                  :key="voice.id"
                  class="pocket-voice-row"
                  :class="{ 'pocket-voice-row--unavailable': !voice.available }"
                >
                  <input
                    v-if="voice.available"
                    class="ui-choice-input"
                    type="radio"
                    :name="`pocket-voice-${focusModel.id}`"
                    :value="voice.id"
                    :checked="activeVoiceId === voice.id"
                    :disabled="isVoiceBusy"
                    @change="selectReadyVoice(voice)"
                  />
                  <span class="pocket-voice-name ui-choice-label">{{ voice.display_name }}</span>
                  <span
                    v-if="voice.available"
                    class="pocket-model-status pocket-status--ready"
                  >{{ t('tts.pocket.status.ready') }}</span>
                  <span
                    v-else
                    class="pocket-model-status pocket-status--error"
                  >{{ t('tts.pocket.voice_unavailable') }}</span>
                </label>
              </div>
              <p v-else class="ui-hint">{{ t('tts.pocket.no_voices') }}</p>
              <p class="ui-hint">{{ t('tts.pocket.voice_hint') }}</p>
            </div>
          </template>

          <p
            v-if="currentHintKey && currentFeedback?.kind !== 'error'"
            class="ui-hint"
          >{{ t(currentHintKey) }}</p>

          <div
            v-if="currentFeedback"
            class="pocket-message"
            :class="`pocket-message--${currentFeedback.kind}`"
            :role="currentFeedback.kind === 'error' ? 'alert' : 'status'"
          >
            <span class="pocket-message-text">{{ currentFeedback.text }}</span>
          </div>

          <div class="pocket-test-row">
            <button
              type="button"
              class="ui-button ui-button--primary ui-button--leading-icon pocket-test-button"
              :disabled="!canTest || isTesting || activatingId !== null"
              @click="testSynthesis"
            >
              <Loader2 v-if="isTesting" :size="16" class="ui-spin" aria-hidden="true" />
              <Sparkles v-else :size="16" aria-hidden="true" />
              {{ t('tts.pocket.test_synthesis') }}
            </button>
            <p v-if="!isFocusActive && focusModel.readiness === 'ready'" class="ui-hint pocket-test-hint">
              {{ t('tts.pocket.test_select_hint') }}
            </p>
            <p v-else-if="focusModel.readiness !== 'ready'" class="ui-hint pocket-test-hint">
              {{ t('tts.pocket.test_not_ready_hint') }}
            </p>
          </div>
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

.pocket-toolbar {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.card-subtitle {
  flex: 1 1 220px;
  min-width: 0;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--color-text-secondary);
  line-height: var(--ui-line-height-multiline);
}

.pocket-empty {
  padding: 4px 0 2px;
}

.pocket-empty .ui-hint:first-child {
  margin-top: 0;
}

.pocket-model-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 4px;
  border: 1px solid var(--color-border);
  border-radius: var(--ui-radius);
  background: var(--color-bg-field);
  min-width: 0;
}

.pocket-model-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  min-width: 0;
  padding: 6px 4px;
  border-radius: var(--ui-radius-menu-item);
  cursor: pointer;
}

.pocket-model-row:hover {
  background: var(--color-bg-field-hover);
}

.pocket-model-row--active {
  background: var(--card-active-bg);
}

.pocket-model-radio {
  flex-shrink: 0;
  margin: 0;
}

.pocket-model-name {
  flex: 1 1 150px;
  min-width: 0;
  overflow-wrap: anywhere;
}

.pocket-model-language {
  flex: 0 0 auto;
  padding: 1px 6px;
  border: 1px solid var(--color-border-strong);
  border-radius: 6px;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-label);
  line-height: var(--ui-line-height-control);
  color: var(--color-text-secondary);
}

.pocket-model-status {
  flex: 0 0 auto;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  line-height: var(--ui-line-height-control);
  white-space: nowrap;
}

.pocket-status--ready {
  color: var(--color-success);
}

.pocket-status--warning {
  color: var(--color-warning);
}

.pocket-status--error {
  color: var(--color-danger);
}

.pocket-details {
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--color-border);
  min-width: 0;
}

.pocket-details-header {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 12px;
  min-width: 0;
}

.pocket-details-title {
  flex: 1 1 150px;
  min-width: 0;
  overflow-wrap: anywhere;
}

.pocket-field {
  min-width: 0;
}

.pocket-field + .pocket-field {
  margin-top: var(--ui-row-gap);
}

.pocket-field-label {
  display: block;
  margin-bottom: var(--ui-row-label-gap-stack);
  color: var(--color-text-secondary);
}

.pocket-reference-row {
  display: flex;
  align-items: center;
  gap: var(--ui-field-group-gap);
  flex-wrap: wrap;
  min-width: 0;
}

.pocket-reference-name {
  flex: 1 1 160px;
  min-width: 0;
  font-size: var(--ui-text-size-control);
  font-weight: var(--ui-text-weight-control);
  line-height: var(--ui-line-height-multiline);
  overflow-wrap: anywhere;
}

.pocket-voice-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.pocket-voice-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  min-width: 0;
  padding: 4px 0;
  cursor: pointer;
}

.pocket-voice-row--unavailable {
  cursor: default;
}

.pocket-voice-name {
  min-width: 0;
  overflow-wrap: anywhere;
}

.pocket-message {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 12px;
  padding: 8px 10px;
  border-radius: var(--ui-radius);
  border: 1px solid transparent;
  background: var(--color-bg-field);
  color: var(--color-text-primary);
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  line-height: var(--ui-line-height-multiline);
}

.pocket-message-text {
  flex: 1 1 180px;
  min-width: 0;
  overflow-wrap: anywhere;
}

.pocket-message--error {
  background: var(--danger-bg-weak);
  border-color: var(--danger-border-strong);
}

.pocket-message--success {
  background: var(--success-bg-weak);
  border-color: var(--success-border-weak);
}

.pocket-test-row {
  margin-top: var(--ui-row-gap);
}

.pocket-test-hint {
  margin-top: 6px;
}

@media (max-width: 520px) {
  .pocket-toolbar {
    align-items: center;
  }

  .card-subtitle {
    flex-basis: 100%;
  }

  .pocket-model-status {
    white-space: normal;
  }

  .pocket-reference-row > .ui-button,
  .pocket-test-button {
    flex: 1 1 100%;
  }
}
</style>
