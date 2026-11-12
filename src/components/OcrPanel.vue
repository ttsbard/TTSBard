<script setup lang="ts">
import { computed } from 'vue'
import { AlertTriangle, Info, RefreshCw, Download, SlidersHorizontal } from 'lucide-vue-next'
import { useOcr } from '../composables/useOcr'
import type { MonitorInfoDto } from '../composables/useOcr'
import { t } from '../i18n'

const props = defineProps<{ active?: boolean }>()

const emit = defineEmits<{
  (e: 'open-models'): void
}>()

const {
  settings,
  status,
  packs,
  monitors,
  monitorListPending,
  monitorListError,
  message,
  messageType,
  savePending,
  rescanPending,
  statusLabel,
  statusErrorMessage,
  saveSettings,
  rescanPacks,
  refreshMonitors,
  runtimeHoldsModel,
  runtimeModelLabel,
  selectedMonitorMissing,
} = useOcr(() => props.active === true)

const statusClass = computed(() => {
  switch (status.value.state) {
    case 'ready':
      return 'ready'
    case 'starting':
    case 'selectingArea':
    case 'recognizing':
      return 'busy'
    case 'error':
      return 'error'
    default:
      return 'disabled'
  }
})

const savedModelId = computed(() => settings.value.model_id)

const selectedPackMissing = computed(
  () => savedModelId.value !== null && !packs.value.some((pack) => pack.id === savedModelId.value),
)

const controlsDisabled = computed(() => savePending.value)

const noPacks = computed(() => packs.value.length === 0)

const showRuntimeError = computed(() => status.value.state === 'error')

const captureTargetValue = computed(() => {
  const target = settings.value.capture_target
  if (target.type === 'all') return 'all'
  if (target.type === 'primary') return 'primary'
  return `monitor:${target.devicePath ?? ''}`
})

const selectedMonitorLabel = computed(() => {
  const target = settings.value.capture_target
  return monitors.value.find((monitor) => monitor.devicePath === target.devicePath)?.label
    ?? t('ocr.capture_target_selected_monitor')
})

const captureTargetSelectDisabled = computed(
  () => controlsDisabled.value || monitorListPending.value,
)

const selectableMonitors = computed(() =>
  monitorListPending.value || monitorListError.value ? [] : monitors.value,
)

const needsSelectedMonitorOption = computed(() => {
  const target = settings.value.capture_target
  return target.type === 'monitor'
    && !selectableMonitors.value.some((monitor) => monitor.devicePath === target.devicePath)
})

function monitorOptionValue(monitor: MonitorInfoDto): string {
  return `monitor:${monitor.devicePath}`
}

function formatMonitorLabel(monitor: MonitorInfoDto): string {
  const primary = monitor.isPrimary ? ` ${t('ocr.monitor_primary_marker')}` : ''
  return `${monitor.label} — ${monitor.geometry.width}×${monitor.geometry.height}${primary}`
}

function formatPackLanguages(languages: string[]): string {
  return languages.map((language) => language.toUpperCase()).join(', ')
}

function onModelChange(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  settings.value.model_id = value === '' ? null : value
  void saveSettings()
}

function openModelManagement() {
  emit('open-models')
}

function onCaptureTargetChange(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  if (value === 'all') {
    settings.value.capture_target = { type: 'all' }
  } else if (value === 'primary') {
    settings.value.capture_target = { type: 'primary' }
  } else if (value.startsWith('monitor:')) {
    settings.value.capture_target = { type: 'monitor', devicePath: value.slice('monitor:'.length) }
  }
  void saveSettings()
}
</script>

<template>
  <div class="ocr-panel">
    <div
      v-if="message"
      class="message-box ui-notice ui-notice--panel"
      :class="messageType ? [messageType, `ui-notice--${messageType}`] : null"
    >
      {{ message }}
    </div>

    <section class="settings-section ui-section">
      <div class="section-header server-header">
        <h2 class="ui-section-title">OCR</h2>
        <span class="status-indicator" :class="statusClass">
          {{ status.state === 'ready' ? t('ocr.status.loaded') : statusLabel }}
        </span>
      </div>

      <div v-if="showRuntimeError" class="callout runtime-error-box">
        <AlertTriangle :size="15" class="callout-icon" />
        <span class="callout-text">
          {{ statusErrorMessage ?? t('ocr.runtime_error_fallback') }}
        </span>
      </div>

      <div class="ocr-row enable-row">
        <label class="ui-choice-label checkbox-label">
          <input
            type="checkbox"
            v-model="settings.enabled"
            :disabled="controlsDisabled || (noPacks && !runtimeHoldsModel)"
            @change="saveSettings"
          />
          <span>{{ t('ocr.enable') }}</span>
        </label>
        <p class="setting-hint ui-hint">
          {{ t('ocr.enable_hint') }}
        </p>
      </div>

      <div class="model-field">
        <label for="ocr-model" class="model-label ui-label">{{ t('ocr.model_label') }}</label>
        <div class="model-row">
          <select
            id="ocr-model"
            class="ui-select model-select"
            :value="noPacks && !runtimeHoldsModel ? '' : (savedModelId ?? '')"
            :disabled="controlsDisabled || noPacks"
            @change="onModelChange"
          >
            <option v-if="noPacks && !runtimeHoldsModel" value="" disabled>
              {{ t('ocr.no_models_option') }}
            </option>
            <template v-else-if="!noPacks">
              <option value="" disabled>{{ t('ocr.select_model_option') }}</option>
              <option v-for="pack in packs" :key="pack.id" :value="pack.id">
                {{ pack.display_name }} ({{ formatPackLanguages(pack.languages) }})
              </option>
            </template>
            <option v-if="runtimeHoldsModel" :value="savedModelId ?? ''" disabled>
              {{ t('ocr.model_in_memory', { name: runtimeModelLabel }) }}
            </option>
            <option v-else-if="selectedPackMissing" :value="savedModelId ?? ''" disabled>
              {{ t('ocr.model_not_installed', { id: savedModelId ?? '' }) }}
            </option>
          </select>
          <button
            class="ui-icon-button ui-icon-button--adjacent refresh-button"
            :disabled="rescanPending || controlsDisabled"
            @click="rescanPacks"
            :title="t('ocr.refresh_models')"
            :aria-label="t('ocr.refresh_models')"
          >
            <RefreshCw :size="18" :class="{ 'button-spinner': rescanPending }" />
          </button>
        </div>
        <p v-if="noPacks" class="path-hint" role="status" aria-live="polite">
          {{ t('ocr.no_models_hint') }}
          <code>%APPDATA%\ttsbard\models\ocr</code>
        </p>
        <p
          v-else-if="selectedPackMissing && !runtimeHoldsModel"
          class="path-hint"
          role="status"
          aria-live="polite"
        >
          {{ t('ocr.model_missing_hint', { model: savedModelId ?? '' }) }}
          <code>%APPDATA%\ttsbard\models\ocr</code>
        </p>
        <div class="model-management">
          <button
            v-if="noPacks"
            type="button"
            class="ui-button ui-button--compact ui-button--leading-icon"
            @click="openModelManagement"
          >
            <Download :size="16" />
            <span>{{ t('ocr.download_model') }}</span>
          </button>
          <button
            v-else
            type="button"
            class="model-manage-link"
            @click="openModelManagement"
          >
            <SlidersHorizontal :size="15" />
            <span>{{ t('ocr.manage_models') }}</span>
          </button>
        </div>
      </div>

      <div class="model-field capture-target-field">
        <label for="ocr-capture-target" class="model-label ui-label">
          {{ t('ocr.capture_target_label') }}
        </label>
        <div class="model-row">
          <select
            id="ocr-capture-target"
            class="ui-select model-select"
            :value="captureTargetValue"
            :disabled="captureTargetSelectDisabled"
            @change="onCaptureTargetChange"
          >
            <option value="all">{{ t('ocr.capture_target_all') }}</option>
            <option value="primary">{{ t('ocr.capture_target_primary') }}</option>
            <option
              v-for="monitor in selectableMonitors"
              :key="monitor.devicePath"
              :value="monitorOptionValue(monitor)"
            >
              {{ formatMonitorLabel(monitor) }}
            </option>
            <option v-if="needsSelectedMonitorOption" :value="captureTargetValue" disabled>
              {{ selectedMonitorMissing
                ? t('ocr.capture_target_monitor_unavailable', { name: selectedMonitorLabel })
                : selectedMonitorLabel }}
            </option>
          </select>
          <button
            class="ui-icon-button ui-icon-button--adjacent refresh-button"
            :disabled="monitorListPending || controlsDisabled"
            @click="refreshMonitors"
            :title="t('ocr.refresh_monitors')"
            :aria-label="t('ocr.refresh_monitors')"
          >
            <RefreshCw :size="18" :class="{ 'button-spinner': monitorListPending }" />
          </button>
        </div>
        <p
          v-if="monitorListPending"
          class="path-hint"
          role="status"
          aria-live="polite"
        >
          {{ t('ocr.monitor_list_loading') }}
        </p>
        <p v-else-if="monitorListError" class="path-hint" role="alert">
          {{ t('ocr.monitor_list_error') }}
        </p>
        <p
          v-else-if="selectedMonitorMissing"
          class="path-hint"
          role="status"
          aria-live="polite"
        >
          {{ t('ocr.capture_target_missing_hint') }}
        </p>
        <p class="capture-target-hint ui-hint">
          {{ t('ocr.capture_target_hint') }}
        </p>
      </div>
    </section>

    <div class="info-callout">
      <Info :size="16" class="info-icon" />
      <span>{{ t('ocr.info_to_incoming') }}</span>
    </div>
  </div>
</template>

<style scoped>
.ocr-panel {
  max-width: 900px;
  margin: 0 auto;
}

h2 {
  margin-top: 0;
  margin-bottom: 1rem;
}

.settings-section {
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
}

.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

.server-header {
  padding-top: 0;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--color-border);
  margin-bottom: 1rem;
  align-items: flex-start;
  gap: 0.75rem;
  flex-wrap: wrap;
}

.server-header h2 {
  margin: 0;
}

.status-indicator {
  display: inline-flex;
  align-items: center;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--color-text-secondary);
  padding: 0.15rem 0.5rem;
  background: var(--color-bg-field);
  border-radius: 5px;
  border: 1px solid var(--color-border);
  height: 28px;
  white-space: nowrap;
}

.status-indicator.ready {
  color: var(--success-text-bright);
  background: var(--success-bg-weak);
  border-color: var(--success-shadow);
}

.status-indicator.busy {
  color: var(--info-text-bright);
  background: var(--info-bg-weak);
  border-color: var(--info-border);
}

.status-indicator.error {
  color: var(--danger-text-bright);
  background: var(--danger-bg-weak);
  border-color: var(--danger-border);
}

.ocr-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: var(--ui-row-gap);
  flex-wrap: wrap;
}

.enable-row {
  display: block;
}

.checkbox-label {
  min-width: auto !important;
}

.setting-hint {
  display: block;
  margin-left: 2.4rem;
}

.model-field {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  column-gap: 0.75rem;
  row-gap: 0.5rem;
  margin-bottom: 1rem;
}

.settings-section > :last-child {
  margin-bottom: 0;
}

.model-label {
  flex: 0 0 5em;
  min-width: 0;
  color: var(--color-text-secondary);
}

.model-row {
  display: flex;
  align-items: stretch;
  gap: 0.5rem;
  flex: 1 1 240px;
  min-width: 0;
}

.model-select {
  flex: 1 1 auto;
  min-width: 0;
}

.capture-target-hint {
  flex: 0 0 100%;
  min-width: 0;
  overflow-wrap: break-word;
  margin-bottom: 0;
}

.model-management {
  display: flex;
  flex: 0 0 100%;
  align-items: center;
  min-width: 0;
}

.model-manage-link {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 32px;
  padding: 4px 0;
  border: none;
  background: transparent;
  color: var(--color-accent);
  font: inherit;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-label);
  line-height: var(--ui-line-height-control);
  text-decoration: underline;
  text-underline-offset: 2px;
  cursor: pointer;
}

.model-manage-link:hover {
  color: var(--color-accent-hover, var(--color-accent));
}

.model-manage-link:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
  border-radius: 4px;
}

.refresh-button {
  flex: 0 0 auto;
}

.callout {
  display: flex;
  align-items: flex-start;
  gap: 0.5rem;
  padding: 0.6rem 0.75rem;
  margin-bottom: 1rem;
  border-radius: 8px;
  font-size: 0.85rem;
  line-height: 1.4;
}

.callout-icon {
  flex-shrink: 0;
  margin-top: 1px;
}

.callout-text {
  min-width: 0;
  overflow-wrap: break-word;
  word-break: break-word;
}

.runtime-error-box {
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border);
  color: var(--danger-text-bright);
  align-items: center;
}

.path-hint {
  display: block;
  margin: 0;
  flex: 0 0 100%;
  font-size: 0.85rem;
  color: var(--color-text-muted);
  line-height: 1.4;
  overflow-wrap: break-word;
  min-width: 0;
}

.path-hint code {
  display: table;
  margin-top: 0.25rem;
  background: var(--btn-neutral-bg);
  padding: 0.15rem 0.35rem;
  border-radius: 4px;
  font-family: var(--font-mono);
  font-size: 0.85em;
  overflow-wrap: anywhere;
  max-width: 100%;
}

.button-spinner {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

.info-callout {
  display: flex;
  align-items: flex-start;
  gap: 0.6rem;
  padding: 0.75rem 1rem;
  margin-bottom: 1.5rem;
  background: var(--info-bg-weak);
  border: 1px solid var(--info-border);
  border-left: 4px solid var(--color-accent);
  border-radius: 8px;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--info-text-bright);
  line-height: 1.5;
}

.info-icon {
  flex-shrink: 0;
  margin-top: 1px;
  color: var(--info-text-bright);
}
</style>
