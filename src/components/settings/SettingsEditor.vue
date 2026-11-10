<script setup lang="ts">
import { computed, watch, ref, onMounted, onUnmounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { Play, Loader2, Check, RefreshCw } from 'lucide-vue-next';
import { useEditorSettings } from '../../composables/useAppSettings';
import { useRuAccentRuntime } from '../../composables/useRuAccentRuntime';
import type { HomographAccentorPackDto, QuickEditorMode } from '../../types/settings';
import { normalizeTypingTimeout } from '../../utils/validateTypingTimeout';
import { t } from '../../i18n';
import { presentCommandError } from '../../ipc/commandError';
import EditorFontSettings from './EditorFontSettings.vue';

const editorSettings = useEditorSettings();

type MessageSeverity = 'error' | 'success' | 'warning' | 'info';

const emit = defineEmits<{
  (e: 'show-message', message: string, severity?: MessageSeverity): void;
}>();

function emitSaved() {
  emit('show-message', t('settings.editor.saved'), 'success');
}

function emitError(key: string, error: unknown) {
  emit('show-message', presentCommandError(error, t(key)), 'error');
}

const quickEditorMode = computed<QuickEditorMode>(() => editorSettings.value?.quick ?? 'disabled');

const spellcheckEnabled = computed(() => editorSettings.value?.spellcheck_enabled ?? true)

const keepTextAfterSend = computed(() => editorSettings.value?.keep_text_after_send ?? false)

const autocompleteEnabled = ref(editorSettings.value?.autocomplete_enabled ?? true)

const autocompletePending = ref(false)

const confirmedAutocompleteEnabled = ref(editorSettings.value?.autocomplete_enabled ?? true)

const numberConversionEnabled = ref(editorSettings.value?.number_conversion_enabled ?? true)

const numberConversionPending = ref(false)

const confirmedNumberConversionEnabled = ref(editorSettings.value?.number_conversion_enabled ?? true)

const typingTimeoutInput = ref(editorSettings.value?.typing_idle_timeout_ms ?? 800)

watch(() => editorSettings.value?.typing_idle_timeout_ms, (newVal) => {
  if (newVal !== undefined) {
    typingTimeoutInput.value = newVal
  }
})

onUnmounted(() => {
  typingTimeoutInput.value = editorSettings.value?.typing_idle_timeout_ms ?? 800
})

const quickModeOptions = computed<{ value: QuickEditorMode; label: string }[]>(() => [
  { value: 'disabled', label: t('settings.editor.quick.options.disabled') },
  { value: 'collapse', label: t('settings.editor.quick.options.collapse') },
  { value: 'return_focus', label: t('settings.editor.quick.options.return_focus') },
])

async function onTypingTimeoutChange() {
  const raw = typingTimeoutInput.value
  const normalized = normalizeTypingTimeout(raw)
  if (normalized === null) {
    typingTimeoutInput.value = editorSettings.value?.typing_idle_timeout_ms ?? 800
    return
  }
  typingTimeoutInput.value = normalized
  try {
    await invoke('set_editor_typing_idle_timeout_ms', { ms: normalized })
    emitSaved()
  } catch (e) {
    emitError('settings.editor.error.typing_timeout', e)
    typingTimeoutInput.value = editorSettings.value?.typing_idle_timeout_ms ?? 800
  }
}

async function setQuickMode(mode: QuickEditorMode) {
  try {
    await invoke('set_editor_quick', { value: mode });
    emitSaved();
  } catch (e) {
    emitError('settings.editor.error.quick_mode', e);
  }
}

async function toggleSpellcheck() {
  try {
    const newValue = !(editorSettings.value?.spellcheck_enabled ?? true)
    await invoke('set_editor_spellcheck_enabled', { value: newValue })
    emitSaved()
  } catch (e) {
    emitError('settings.editor.error.spellcheck', e)
  }
}

async function toggleKeepText() {
  try {
    const newValue = !(editorSettings.value?.keep_text_after_send ?? false)
    await invoke('set_editor_keep_text', { enabled: newValue })
    emitSaved()
  } catch (e) {
    emitError('settings.editor.error.keep_text', e)
  }
}

async function saveAutocompleteEnabled() {
  if (autocompletePending.value) return
  autocompletePending.value = true
  const requested = autocompleteEnabled.value
  try {
    await invoke('set_editor_autocomplete_enabled', { enabled: requested })
    confirmedAutocompleteEnabled.value = requested
    emitSaved()
  } catch (e) {
    autocompleteEnabled.value = confirmedAutocompleteEnabled.value
    emitError('settings.editor.error.autocomplete', e)
  } finally {
    autocompletePending.value = false
  }
}

async function saveNumberConversionEnabled() {
  if (numberConversionPending.value) return
  numberConversionPending.value = true
  const requested = numberConversionEnabled.value
  try {
    await invoke('set_editor_number_conversion_enabled', { enabled: requested })
    confirmedNumberConversionEnabled.value = requested
    emitSaved()
  } catch (e) {
    numberConversionEnabled.value = confirmedNumberConversionEnabled.value
    emitError('settings.editor.error.number_conversion', e)
  } finally {
    numberConversionPending.value = false
  }
}

const homographAccentor = computed(() => editorSettings.value?.homograph_accentor)
const accentorEnabled = computed(() => homographAccentor.value?.enabled ?? false)
const accentorLoadOnStart = computed(() => homographAccentor.value?.load_on_start ?? false)

function formatAccentorPackLabel(pack: HomographAccentorPackDto): string {
  return pack.display_name.trim()
}

const { statusFor, load: loadRuAccentModel, refreshPacks, rescanPacks, packLabelFor } = useRuAccentRuntime()

const accentorPacks = ref<HomographAccentorPackDto[]>([])
const selectedPackId = ref<string>('')
const accentorRescanPending = ref(false)

function syncAccentorFromSettings() {
  const persistedId = homographAccentor.value?.accentor_pack_id ?? ''
  if (selectedPackId.value !== persistedId) {
    selectedPackId.value = persistedId
  }
}

watch(homographAccentor, () => syncAccentorFromSettings(), { immediate: true })

const accentorRuntimeStatus = statusFor(selectedPackId)
const accentorReady = computed(() => accentorRuntimeStatus.value === 'ready')

const noPacks = computed(() => accentorPacks.value.length === 0)

const accentorInMemory = computed(() => {
  const id = selectedPackId.value
  if (!id) return false
  if (accentorPacks.value.some((pack) => pack.id === id)) return false
  const status = accentorRuntimeStatus.value
  return status === 'loading' || status === 'ready'
})

const accentorSelectedMissing = computed(() => {
  const id = selectedPackId.value
  if (!id) return false
  return !accentorPacks.value.some((pack) => pack.id === id)
})

const accentorStatusText = computed(() => {
  switch (accentorRuntimeStatus.value) {
    case 'not_loaded':
      return t('settings.editor.accentor.status.not_loaded')
    case 'loading':
      return t('settings.editor.accentor.status.loading')
    case 'ready':
      return t('settings.editor.accentor.status.ready')
    case 'failed':
      return t('settings.editor.accentor.status.failed')
    default:
      return ''
  }
})

const accentorLoadTitle = computed(() => {
  switch (accentorRuntimeStatus.value) {
    case 'loading':
      return t('settings.editor.accentor.load.loading')
    case 'ready':
      return t('settings.editor.accentor.load.ready')
    case 'failed':
      return t('settings.editor.accentor.load.retry')
    default:
      return t('settings.editor.accentor.load.default')
  }
})

async function loadAccentorPacks() {
  try {
    accentorPacks.value = await refreshPacks()
  } catch (e) {
    emitError('settings.editor.error.accentor_load_models', e)
  }
}

async function rescanAccentorPacks() {
  if (accentorRescanPending.value) return
  accentorRescanPending.value = true
  try {
    accentorPacks.value = await rescanPacks()
  } catch (e) {
    emitError('settings.editor.error.accentor_rescan', e)
  } finally {
    accentorRescanPending.value = false
  }
}

onMounted(async () => {
  await loadAccentorPacks()
  syncAccentorFromSettings()
})

async function saveAccentor(enabled: boolean, packId: string | null) {
  try {
    await invoke('set_editor_homograph_accentor', { enabled, accentorPackId: packId })
    emitSaved()
    await loadAccentorPacks()
  } catch (e) {
    emitError('settings.editor.error.accentor_save', e)
    syncAccentorFromSettings()
  }
}

async function toggleAccentor() {
  const next = !accentorEnabled.value
  let packId: string | null = selectedPackId.value || null
  if (next && !packId && accentorPacks.value.length > 0) {
    packId = accentorPacks.value[0].id
    selectedPackId.value = packId
  }
  await saveAccentor(next, packId)
}

function onPackSelect(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  selectedPackId.value = value
  void saveAccentor(accentorEnabled.value, value || null)
}

async function loadSelectedModel() {
  if (
    !selectedPackId.value ||
    accentorSelectedMissing.value ||
    accentorRuntimeStatus.value === 'loading' ||
    accentorRuntimeStatus.value === 'ready'
  ) {
    return
  }
  await loadRuAccentModel(selectedPackId.value)
}

async function toggleLoadOnStart() {
  try {
    const next = !(homographAccentor.value?.load_on_start ?? false)
    await invoke('set_editor_homograph_accentor_load_on_start', { loadOnStart: next })
    emitSaved()
  } catch (e) {
    emitError('settings.editor.error.accentor_autoload', e)
  }
}

watch(editorSettings, (newSettings) => {
  if (!newSettings) return;
  if (autocompletePending.value) return;
  const persisted = newSettings.autocomplete_enabled ?? true;
  autocompleteEnabled.value = persisted;
  confirmedAutocompleteEnabled.value = persisted;
}, { immediate: true });

watch(editorSettings, (newSettings) => {
  if (!newSettings) return;
  if (numberConversionPending.value) return;
  const persisted = newSettings.number_conversion_enabled ?? true;
  numberConversionEnabled.value = persisted;
  confirmedNumberConversionEnabled.value = persisted;
}, { immediate: true });

onUnmounted(() => {
  numberConversionEnabled.value = editorSettings.value?.number_conversion_enabled ?? true;
  confirmedNumberConversionEnabled.value = editorSettings.value?.number_conversion_enabled ?? true;
});
</script>

<template>
  <div class="settings-editor">
    <EditorFontSettings />

    <section class="settings-section">
      <div class="editor-row">
        <label class="ui-choice-label setting-label checkbox-label">
          <input
            v-model="numberConversionEnabled"
            type="checkbox"
            class="ui-choice-input"
            :disabled="numberConversionPending"
            @change="saveNumberConversionEnabled"
          />
          <span>{{ t('settings.editor.number_conversion') }}</span>
        </label>
        <span class="setting-hint ui-hint ui-hint--choice">
          {{ t('settings.editor.number_conversion.hint') }}
        </span>
      </div>
    </section>

    <section class="settings-section">
      <div class="card-header">
        <h3 class="card-title ui-group-title">{{ t('settings.editor.quick.title') }}</h3>
        <p class="card-desc">{{ t('settings.editor.quick.desc') }}</p>
      </div>
      <div class="editor-row" v-for="opt in quickModeOptions" :key="opt.value">
        <label class="ui-choice-label setting-label radio-label">
          <input
            type="radio"
            :value="opt.value"
            :checked="quickEditorMode === opt.value"
            class="ui-choice-input"
            @change="setQuickMode(opt.value)"
          />
          <span>{{ opt.label }}</span>
        </label>
        <span v-if="opt.value === 'return_focus'" class="setting-hint ui-hint ui-hint--choice">
          {{ t('settings.editor.quick.return_focus.hint') }}
        </span>
      </div>
      <div class="editor-row">
        <label class="ui-choice-label setting-label checkbox-label">
          <input
            :checked="keepTextAfterSend"
            type="checkbox"
            class="ui-choice-input"
            @change="toggleKeepText"
          />
          <span>{{ t('settings.editor.keep_text_after_send') }}</span>
        </label>
        <span class="setting-hint ui-hint ui-hint--choice">
          {{ t('settings.editor.keep_text_after_send.hint') }}
        </span>
      </div>
    </section>

    <section class="settings-section">
      <div class="card-header">
        <h3 class="card-title ui-group-title">{{ t('settings.editor.proofing.title') }}</h3>
      </div>
      <div class="editor-row">
        <label class="ui-choice-label setting-label checkbox-label">
          <input
            :checked="spellcheckEnabled"
            type="checkbox"
            class="ui-choice-input"
            @change="toggleSpellcheck"
          />
          <span>{{ t('settings.editor.spellcheck') }}</span>
        </label>
        <span class="setting-hint ui-hint ui-hint--choice">
          {{ t('settings.editor.spellcheck.hint') }}
        </span>
      </div>
      <div class="editor-row">
        <label class="ui-choice-label setting-label checkbox-label">
          <input
            v-model="autocompleteEnabled"
            type="checkbox"
            class="ui-choice-input"
            :disabled="autocompletePending"
            @change="saveAutocompleteEnabled"
          />
          <span>{{ t('settings.editor.autocomplete_enabled') }}</span>
        </label>
        <span class="setting-hint ui-hint ui-hint--choice">
          {{ t('settings.editor.autocomplete_enabled.hint') }}
        </span>
      </div>
    </section>

    <section class="settings-section">
      <div class="card-header">
        <h3 class="card-title ui-group-title">{{ t('settings.editor.typing.title') }}</h3>
        <p class="card-desc">{{ t('settings.editor.typing.desc') }}</p>
      </div>
      <div class="editor-row typing-row">
        <label class="ui-label setting-label">{{ t('settings.editor.typing.label') }}</label>
        <input
          type="number"
          v-model="typingTimeoutInput"
          @change="onTypingTimeoutChange"
          class="ui-input number-input"
          :min="200"
          :max="5000"
          :step="100"
        />
        <span class="setting-hint ui-hint typing-hint">
          {{ t('settings.editor.typing.hint') }}
        </span>
      </div>
    </section>

    <section class="settings-section">
      <div class="card-header">
        <h3 class="card-title ui-group-title">{{ t('settings.editor.accentor.title') }}</h3>
        <p class="card-desc">
          {{ t('settings.editor.accentor.desc') }}
        </p>
      </div>

      <div class="editor-row accentor-select-row">
        <label class="ui-label setting-label" for="accentor-select">{{ t('settings.editor.accentor.model_label') }}</label>
        <select
          id="accentor-select"
          class="ui-select accentor-select"
          :value="noPacks && !accentorInMemory ? '' : selectedPackId"
          :disabled="noPacks || accentorRuntimeStatus === 'loading'"
          @change="onPackSelect"
        >
          <option v-if="noPacks && !accentorInMemory" value="" disabled>{{ t('settings.editor.accentor.options.no_models') }}</option>
          <template v-else-if="!noPacks">
            <option value="" disabled>{{ t('settings.editor.accentor.options.choose') }}</option>
            <option v-for="pack in accentorPacks" :key="pack.id" :value="pack.id">
              {{ formatAccentorPackLabel(pack) }}
            </option>
          </template>
          <option v-if="accentorInMemory" :value="selectedPackId" disabled>
            {{ t('settings.editor.accentor.in_memory', { name: packLabelFor(selectedPackId) }) }}
          </option>
        </select>
        <button
          type="button"
          class="ui-icon-button ui-icon-button--adjacent accentor-load-btn"
          :disabled="
            !selectedPackId ||
            accentorSelectedMissing ||
            accentorRuntimeStatus === 'loading' ||
            accentorRuntimeStatus === 'ready'
          "
          :title="accentorLoadTitle"
          :aria-label="accentorLoadTitle"
          @click="loadSelectedModel"
        >
          <Loader2 v-if="accentorRuntimeStatus === 'loading'" :size="18" class="accentor-spin" />
          <Check v-else-if="accentorRuntimeStatus === 'ready'" :size="18" />
          <RefreshCw v-else-if="accentorRuntimeStatus === 'failed'" :size="18" />
          <Play v-else :size="18" />
        </button>
        <button
          type="button"
          class="ui-icon-button ui-icon-button--adjacent accentor-rescan-btn"
          :disabled="accentorRescanPending"
          @click="rescanAccentorPacks"
          :title="t('settings.editor.accentor.rescan')"
          :aria-label="t('settings.editor.accentor.rescan')"
        >
          <RefreshCw :size="18" :class="{ 'accentor-spin': accentorRescanPending }" />
        </button>
      </div>
      <span
        v-if="selectedPackId"
        class="setting-hint ui-hint accentor-status"
        role="status"
        aria-live="polite"
      >
        <span>{{ t('settings.editor.accentor.status_prefix') }}</span>
        <span>{{ accentorStatusText }}</span>
      </span>
      <p v-if="noPacks" class="setting-hint ui-hint accentor-empty" role="status" aria-live="polite">
        {{ t('settings.editor.accentor.no_models.empty') }}
        <code>%APPDATA%\ttsbard\models\ruaccent</code>
      </p>
      <div class="editor-row accentor-load-on-start-row">
        <label class="ui-choice-label setting-label checkbox-label">
          <input
            :checked="accentorLoadOnStart"
            :disabled="noPacks || !selectedPackId || accentorRuntimeStatus === 'loading'"
            type="checkbox"
            class="ui-choice-input"
            @change="toggleLoadOnStart"
          />
          <span>{{ t('settings.editor.accentor.load_on_start') }}</span>
        </label>
        <span class="setting-hint ui-hint ui-hint--choice">
          {{ t('settings.editor.accentor.load_on_start.hint') }}
        </span>
      </div>
      <div class="editor-row accentor-enable-row">
        <label class="ui-choice-label setting-label checkbox-label">
          <input
            :checked="accentorEnabled"
            :disabled="!accentorReady"
            type="checkbox"
            class="ui-choice-input"
            @change="toggleAccentor"
          />
          <span>{{ t('settings.editor.accentor.enabled') }}</span>
        </label>
        <span class="setting-hint ui-hint ui-hint--choice">
          {{ t('settings.editor.accentor.enabled.hint') }}
        </span>
      </div>
    </section>
  </div>
</template>

<style scoped>
.settings-editor {
  display: flex;
  flex-direction: column;
  gap: var(--ui-section-gap);
}

.settings-section {
  padding: 12px 16px;
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
}

.card-header {
  margin-bottom: 0.75rem;
}

.card-title {
  margin: 0 0 0.25rem;
}

.card-desc {
  margin: 0;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--color-text-muted);
  line-height: 1.4;
}

.settings-editor .editor-row {
  display: block;
  margin-bottom: var(--ui-row-gap);
}

.settings-editor .editor-row:last-child,
.setting-row:last-child {
  margin-bottom: 0;
}

.setting-label {
  color: var(--color-text-primary);
  user-select: none;
}

.setting-hint {
  display: block;
}

.setting-hint code {
  background: var(--btn-neutral-bg);
  padding: 0.15rem 0.35rem;
  border-radius: 4px;
  font-family: var(--font-mono);
  font-size: 0.85em;
}

.settings-editor .typing-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem;
}

.typing-row label {
  min-width: 110px;
  color: var(--color-text-secondary);
}

.typing-row .number-input {
  width: 88px;
}

.typing-hint {
  width: 100%;
}

.accentor-status {
  display: flex;
  gap: var(--ui-hint-gap);
  margin-bottom: var(--ui-row-gap);
}

.settings-editor .accentor-select-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
}

.accentor-select-row label {
  flex-shrink: 0;
  color: var(--color-text-secondary);
}

.accentor-select {
  flex: 1 1 auto;
  min-width: 0;
}

.accentor-enable-row {
  margin-top: 0.75rem;
}

.accentor-load-on-start-row {
  margin-top: 0.25rem;
}



.accentor-spin {
  animation: accentor-spin 1s linear infinite;
}

@keyframes accentor-spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

</style>
