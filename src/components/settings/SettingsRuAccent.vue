<script setup lang="ts">
import { computed, watch, ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Play, Loader2, Check, RefreshCw } from 'lucide-vue-next'
import { useEditorSettings } from '../../composables/useAppSettings'
import { useRuAccentRuntime } from '../../composables/useRuAccentRuntime'
import type { HomographAccentorPackDto } from '../../types/settings'
import { t } from '../../i18n'
import { presentCommandError } from '../../ipc/commandError'

const editorSettings = useEditorSettings()

type MessageSeverity = 'error' | 'success' | 'warning' | 'info'

const emit = defineEmits<{
  (e: 'show-message', message: string, severity?: MessageSeverity): void
}>()

function emitSaved() {
  emit('show-message', t('settings.editor.saved'), 'success')
}

function emitError(key: string, error: unknown) {
  emit('show-message', presentCommandError(error, t(key)), 'error')
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

const accentorInstalled = computed(() => {
  const id = selectedPackId.value
  return !!id && accentorPacks.value.some((pack) => pack.id === id)
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

/** Refresh the on-disk package list after a package download completes. */
async function refreshFromDisk() {
  await loadAccentorPacks()
}

defineExpose({ refreshFromDisk })
</script>

<template>
  <section class="settings-section accentor-section">
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
    <div v-if="selectedPackId" class="accentor-state-row">
      <span
        v-if="accentorInstalled"
        class="accentor-state-badge"
      >
        {{ t('settings.models.ruaccent.installed') }}
      </span>
      <span
        class="setting-hint ui-hint accentor-status"
        role="status"
        aria-live="polite"
      >
        <span>{{ t('settings.editor.accentor.status_prefix') }}</span>
        <span>{{ accentorStatusText }}</span>
      </span>
    </div>
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
</template>

<style scoped>
.settings-section {
  padding: var(--ui-section-padding-y) var(--ui-section-padding-x);
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

.editor-row {
  display: block;
  margin-bottom: var(--ui-row-gap);
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
  overflow-wrap: anywhere;
}

.accentor-select-row {
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

.accentor-state-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--ui-hint-gap);
  margin-bottom: var(--ui-row-gap);
}

.accentor-state-badge {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border: 1px solid var(--success-border-weak, rgba(74, 222, 128, 0.4));
  border-radius: 999px;
  background: var(--success-bg-weak);
  color: var(--success-text-bright);
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  line-height: var(--ui-line-height-control);
}

.accentor-status {
  display: flex;
  gap: var(--ui-hint-gap);
  margin: 0;
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

@media (prefers-reduced-motion: reduce) {
  .accentor-spin {
    animation: none;
  }
}

@keyframes accentor-spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

@media (max-width: 620px) {
  .accentor-select-row {
    flex-wrap: wrap;
  }

  .accentor-select-row label {
    flex: 1 1 100%;
  }

  .accentor-select {
    flex: 1 1 180px;
  }
}
</style>
