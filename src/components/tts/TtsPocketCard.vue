<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { Cpu, Loader2, Play, RefreshCw, Square } from 'lucide-vue-next';
import ProviderCard from '../shared/ProviderCard.vue';
import { locale, t } from '../../i18n';
import { presentCommandError } from '../../ipc/commandError';
import { createAsyncCleanupScope } from '../../utils/asyncCleanup';
import { debugWarn } from '../../utils/debug';
import './local-model-list.css';
import {
  isPocketReferenceModel,
  nextPocketFocusId,
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

let snapshotGeneration = 0;
let activationGeneration = 0;
let voiceGeneration = 0;
let previewGeneration = 0;

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

const referenceDuration = computed(() => {
  const seconds = focusModel.value?.reference_duration_seconds;
  if (seconds == null) return null;
  return new Intl.NumberFormat(locale.value, { maximumFractionDigits: 2 }).format(seconds);
});

const referenceFileParts = computed(() => {
  const name = focusReference.value?.file_name ?? '';
  return { stem: name.slice(0, -4), extension: name.slice(-4) };
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

// When the details are showing a model that is not the active one, name it in
// the field label so it is obvious which package the controls configure.
const focusedModelName = computed<string | null>(() => {
  const model = focusModel.value;
  if (!model || models.value.length <= 1 || model.id === props.activeModelId) return null;
  return model.display_name;
});

const isVoiceBusy = computed(() => !!focusModel.value && voiceBusyFor.value === focusModel.value.id);
const isPreviewing = computed(() => !!focusModel.value && previewingFor.value === focusModel.value.id);

const activeVoiceId = computed(() => focusVoice.value?.voice_id ?? null);


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
    pendingActivationId.value = null;
    selectedModelId.value = id;
    emit('activate', id);
    return true;
  } catch (error) {
    if (token !== activationGeneration) return false;
    selectedModelId.value = props.activeModelId && models.value.some(entry => entry.id === props.activeModelId)
      ? props.activeModelId
      : null;
    pendingActivationId.value = id;
    // A missing reference is the normal starting state of a reference package,
    // not an activation error. Keep `pendingActivationId` so the model activates
    // right after a valid WAV is chosen, but let the calm readiness hint render
    // instead of a red error box.
    if (model.readiness === 'needs_reference') {
      clearFeedback(id);
    } else {
      const hintKey = pocketStatusHintKey(model.readiness);
      setFeedback(
        id,
        'error',
        hintKey ? t(hintKey) : presentCommandError(error, t('tts.pocket.error.select_model')),
      );
    }
    return false;
  } finally {
    if (token === activationGeneration) activatingId.value = null;
  }
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
  if (previewingFor.value === model.id) stopReferencePreview();

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
          : check.error_code === 'pocket.reference_too_long'
            ? t('tts.pocket.error.reference_too_long')
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

async function normalizeReference(): Promise<void> {
  const model = focusModel.value;
  if (!model || voiceBusyFor.value !== null) return;

  const modelId = model.id;
  const token = ++voiceGeneration;
  voiceBusyFor.value = modelId;
  clearFeedback(modelId);

  try {
    const updated = await invoke<PocketModelDto>('normalize_pocket_reference', { modelId });
    if (!updated || updated.id !== modelId) {
      throw new Error('Pocket reference normalization response does not match the requested model');
    }
    models.value = replacePocketModel(models.value, updated);

    if (token !== voiceGeneration) return;
    setFeedback(modelId, 'success', t('tts.pocket.normalized'));

    if (focusModel.value?.id === modelId && pendingActivationId.value === modelId) {
      pendingActivationId.value = null;
      await activateModelById(modelId, { keepSuccessMessage: true });
    }
  } catch (error) {
    if (token !== voiceGeneration) return;
    setFeedback(modelId, 'error', presentCommandError(error, t('tts.pocket.error.normalize')));
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

onMounted(() => {
  void loadModels('list');
  void listenerScope.track(listen('pocket-models-changed', () => {
    if (!listenerScope.disposed) void loadModels('list');
  })).catch(error => debugWarn('[TtsPocketCard] Failed to subscribe to model changes:', error));
});

onBeforeUnmount(() => {
  listenerScope.dispose();
  stopReferencePreview();
});
</script>

<template>
  <ProviderCard
    :title="t('tts.pocket.title')"
    :icon="Cpu"
    :active="isPocketActive"
    :selectable="false"
    :collapsible="false"
  >
    <template #header-actions>
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
    </template>

    <div class="card-content-inner" :aria-busy="loading">
      <div v-if="listError" class="local-model-message local-model-message--error" role="alert">
        <span class="local-model-message-text">{{ listError }}</span>
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
        <div class="local-model-list" role="radiogroup" :aria-label="t('tts.pocket.models')">
          <label
            v-for="model in models"
            :key="model.id"
            class="local-model-row"
            :class="{
              'local-model-row--active': model.id === props.activeModelId,
              'local-model-row--focused': model.id === focusId && model.id !== props.activeModelId,
            }"
            @click="handleModelRowClick(model)"
          >
            <input
              class="local-model-radio"
              type="radio"
              name="pocket-model"
              :value="model.id"
              :checked="model.id === selectedModelId"
              :aria-busy="activatingId === model.id"
              :disabled="activatingId !== null"
              @change="handleModelChange(model)"
            />
            <span class="local-model-name ui-choice-label">{{ model.display_name }}</span>
            <span
              class="pocket-model-language"
              :title="languageTitle(model.language)"
            >{{ languageCode(model.language) }}</span>
            <span
              class="local-model-status"
              :class="`local-model-status--${pocketStatusKind(model.readiness)}`"
              :title="statusHint(model.readiness) || undefined"
            >{{ statusLabel(model.readiness) }}</span>
          </label>
        </div>

        <div v-if="focusModel" class="pocket-details">
          <template v-if="isPocketReferenceModel(focusModel)">
            <div class="pocket-field">
              <span class="ui-label pocket-field-label">{{ t('tts.pocket.reference') }}<template v-if="focusedModelName"> — {{ focusedModelName }}</template></span>
              <div v-if="focusReference" class="pocket-reference-row">
                <div class="pocket-reference-file">
                  <span class="pocket-reference-name ui-label--secondary" :title="focusReference.file_name">
                    <span class="pocket-reference-stem">{{ referenceFileParts.stem }}</span>{{ referenceFileParts.extension }}
                  </span>
                  <span v-if="referenceDuration" class="pocket-reference-duration">
                    {{ t('tts.pocket.reference_duration', { seconds: referenceDuration }) }}
                  </span>
                </div>
                <div class="pocket-reference-actions">
                  <button
                    type="button"
                    class="ui-icon-button pocket-listen-button"
                    :title="t('tts.pocket.listen')"
                    :aria-label="t('tts.pocket.listen')"
                    :disabled="isVoiceBusy || previewingFor !== null"
                    @click="playReference"
                  >
                    <Play :size="18" aria-hidden="true" />
                  </button>
                  <button
                    type="button"
                    class="ui-icon-button pocket-stop-button"
                    :title="t('tts.pocket.stop_reference')"
                    :aria-label="t('tts.pocket.stop_reference')"
                    :disabled="!isPreviewing"
                    @click="stopReferencePreview"
                  >
                    <Square :size="18" aria-hidden="true" />
                  </button>
                  <button
                    type="button"
                    class="ui-button ui-button--compact pocket-replace-button"
                    :disabled="isVoiceBusy"
                    @click="chooseReference"
                  >
                    <Loader2 v-if="isVoiceBusy" :size="16" class="ui-spin" aria-hidden="true" />
                    {{ t('tts.pocket.replace') }}
                  </button>
                  <button
                    type="button"
                    class="ui-button ui-button--compact pocket-normalize-button"
                    :title="t('tts.pocket.normalize_tooltip')"
                    :disabled="isVoiceBusy"
                    @click="normalizeReference"
                  >
                    <Loader2 v-if="isVoiceBusy" :size="16" class="ui-spin" aria-hidden="true" />
                    {{ t('tts.pocket.normalize') }}
                  </button>
                </div>
              </div>
              <div v-else class="pocket-reference-row">
                <button
                  type="button"
                  class="ui-button ui-button--compact pocket-choose-button"
                  :disabled="isVoiceBusy"
                  @click="chooseReference"
                >
                  <Loader2 v-if="isVoiceBusy" :size="16" class="ui-spin" aria-hidden="true" />
                  {{ t('tts.pocket.choose_wav') }}
                </button>
                <span class="pocket-reference-duration">{{ t('tts.pocket.reference_limit') }}</span>
              </div>
            </div>
          </template>

          <template v-else>
            <div class="pocket-field">
              <span class="ui-label pocket-field-label">{{ t('tts.pocket.voice') }}<template v-if="focusedModelName"> — {{ focusedModelName }}</template></span>
              <div v-if="focusModel.voices.length > 0" class="pocket-voice-list">
                <label
                  v-for="voice in focusModel.voices"
                  :key="voice.id"
                  class="pocket-voice-row"
                  :class="{ 'pocket-voice-row--unavailable': !voice.available }"
                >
                  <input
                    v-if="voice.available"
                    class="local-model-radio"
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
                    class="local-model-status local-model-status--ready"
                  >{{ t('tts.pocket.status.ready') }}</span>
                  <span
                    v-else
                    class="local-model-status local-model-status--error"
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
            class="local-model-message"
            :class="`local-model-message--${currentFeedback.kind}`"
            :role="currentFeedback.kind === 'error' ? 'alert' : 'status'"
          >
            <span class="local-model-message-text">{{ currentFeedback.text }}</span>
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

.pocket-empty {
  padding: 4px 0 2px;
}

.pocket-empty .ui-hint:first-child {
  margin-top: 0;
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

.pocket-details {
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--color-border);
  min-width: 0;
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
}

.pocket-reference-row {
  display: flex;
  align-items: center;
  gap: var(--ui-field-group-gap);
  flex-wrap: wrap;
  min-width: 0;
}

.pocket-reference-file,
.pocket-reference-name,
.pocket-reference-actions {
  display: flex;
  align-items: center;
  min-width: 0;
}

.pocket-reference-file {
  flex: 1 1 128px;
  gap: var(--ui-field-group-gap);
}

.pocket-reference-name {
  font-size: var(--ui-text-size-control);
  font-weight: var(--ui-text-weight-control);
  white-space: nowrap;
}

.pocket-reference-stem {
  overflow: hidden;
  text-overflow: ellipsis;
}

.pocket-reference-duration {
  flex-shrink: 0;
  white-space: nowrap;
  color: var(--color-text-secondary);
  font-size: var(--ui-text-size-hint);
}

.pocket-reference-actions {
  flex-wrap: wrap;
  gap: var(--ui-field-group-gap);
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

@media (max-width: 520px) {
  .local-model-status {
    white-space: normal;
  }
}
</style>
