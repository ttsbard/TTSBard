<script setup lang="ts">
import { ref, computed, watch, onMounted, nextTick } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open as openDirectoryDialog } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import MarkdownIt from 'markdown-it';
import {
  AlertTriangle,
  Check,
  ExternalLink,
  FolderOpen,
  FolderCog,
  Loader2,
  RotateCcw
} from 'lucide-vue-next';
import { useGeneralSettings, useWindowsSettings, useLoggingSettings } from '../../composables/useAppSettings';
import { useAppUpdates } from '../../composables/useAppUpdates';
import { useUpdateInstall } from '../../composables/useUpdateInstall';
import { presentCommandError } from '../../ipc/commandError';
import {
  UPDATE_CHECK_ERROR_LOCALE_KEY,
  UPDATES_MAX_INTERVAL_DAYS,
  UPDATES_MIN_INTERVAL_DAYS
} from '../../ipc/appUpdates';
import { availableLanguages, locale, setLanguage, t } from '../../i18n';
import {
  getOutputControlsPlacement,
  setOutputControlsPlacement,
  type OutputControlsPlacement,
} from '../../composables/useMainWindowOutputControls';
import { saveStartCompactToStorage } from '../../composables/compactModeState';
import { APP_VERSION } from '../../version';
import DataTransferModal from './DataTransferModal.vue';

const DEFAULT_CONFIG_DIR_DISPLAY = '%APPDATA%\\ttsbard';
const DEFAULT_DATA_DIR_DISPLAY = '%LOCALAPPDATA%\\ttsbard';

const showPlaybackOnStart = ref(false);
const startCompact = ref(false);
const hideOnMinimize = ref(false);
const hideOnMinimizeSaving = ref(false);
const hideExtraWindowButtons = ref(false);
const hideExtraWindowButtonsSaving = ref(false);
const folderOpening = ref(false);
const languageSaving = ref(false);
const languageError = ref<string | null>(null);
const languageSaved = ref(false);
const pendingLanguage = ref<string | null>(null);

interface DataInfo {
  path: string;
  is_default: boolean;
}

const dataInfo = ref<DataInfo | null>(null);
const transferTarget = ref<string | null | undefined>(undefined);
const localDataOpening = ref(false);
const dataFolderSelecting = ref(false);

const dataPathDisplay = computed(() =>
  dataInfo.value?.is_default ? DEFAULT_DATA_DIR_DISPLAY : (dataInfo.value?.path ?? '…')
);
const dataPathTooltip = computed(() => dataInfo.value?.path ?? '');

// Get settings from composables
const generalSettings = useGeneralSettings();
const windowsSettings = useWindowsSettings();
const loggingSettings = useLoggingSettings();

// Local state for immediate UI feedback
const localLoggingEnabled = ref(false);

const loggingLevels = computed(() => [
  { value: 'error', label: t('general.logging.level.error') },
  { value: 'warn', label: t('general.logging.level.warn') },
  { value: 'info', label: t('general.logging.level.info') },
  { value: 'debug', label: t('general.logging.level.debug') },
  { value: 'trace', label: t('general.logging.level.trace') }
]);

// Computed properties
const excludeFromCapture = computed(() => windowsSettings.value?.global.exclude_from_capture ?? false);
const loggingEnabled = computed(() => localLoggingEnabled.value);
const loggingLevel = computed(() => loggingSettings.value?.level ?? 'info');
const selectedLanguage = computed(() => pendingLanguage.value ?? locale.value);

// Emit error message event for parent to display
type MessageSeverity = 'error' | 'success' | 'warning' | 'info';

const emit = defineEmits<{
  (e: 'show-message', message: string, severity?: MessageSeverity): void;
}>();

function showMessage(message: string, severity: MessageSeverity) {
  emit('show-message', message, severity);
}

async function onLanguageChange(event: Event) {
  const nextLanguage = (event.target as HTMLSelectElement).value;
  if (nextLanguage === selectedLanguage.value || languageSaving.value) return;

  languageSaving.value = true;
  languageError.value = null;
  languageSaved.value = false;
  try {
    await setLanguage(nextLanguage);
    pendingLanguage.value = nextLanguage;
    languageSaved.value = true;
    showMessage(t('settings.language.saved.restart'), 'warning');
  } catch (e) {
    languageError.value = presentCommandError(e, t('settings.language.save_failed'));
  } finally {
    languageSaving.value = false;
  }
}

async function openAppFolder() {
  if (folderOpening.value) return;
  folderOpening.value = true;

  try {
    await invoke('open_app_folder');
  } catch (e) {
    showMessage(presentCommandError(e, t('general.error.open_folder')), 'error');
  } finally {
    folderOpening.value = false;
  }
}

async function refreshDataInfo() {
  try {
    dataInfo.value = await invoke<DataInfo>('storage_get_data_info');
  } catch (e) {
    // Информационная панель: молча оставляем прочерк, ошибка видна при действии.
    console.warn('Failed to load data dir info', e);
  }
}

async function changeDataDir() {
  if (dataFolderSelecting.value || transferTarget.value !== undefined) return;
  dataFolderSelecting.value = true;
  try {
    const selected = await openDirectoryDialog({ directory: true, multiple: false });
    if (!selected || typeof selected !== 'string') return;
    transferTarget.value = selected;
  } catch (e) {
    showMessage(presentCommandError(e, t('dataTransfer.error')), 'error');
  } finally {
    dataFolderSelecting.value = false;
  }
}

function resetDataDir() {
  transferTarget.value = null;
}

function handleTransferSuccess(result: { restartRequired: boolean; path: string; isDefault: boolean }) {
  dataInfo.value = { path: result.path, is_default: result.isDefault };
  transferTarget.value = undefined;
  if (result.restartRequired) {
    showMessage(t('dataTransfer.success.restart'), 'warning');
  } else {
    showMessage(t('dataTransfer.success'), 'success');
  }
}

async function openLocalDataFolder() {
  if (localDataOpening.value) return;
  localDataOpening.value = true;

  try {
    await invoke('open_local_data_folder');
  } catch (e) {
    showMessage(presentCommandError(e, t('general.error.open_folder')), 'error');
  } finally {
    localDataOpening.value = false;
  }
}

// ==================== Updates (ROADMAP-141) ====================

// App-level update state: the same snapshot that drives the banner under the
// titlebar. Background checks arrive as events even while the card is closed.
const appUpdates = useAppUpdates();

const updatesStatus = computed(() => appUpdates.status.value);
const updatesChecking = computed(() => appUpdates.checking.value);
const updatesCommandError = computed(() => appUpdates.commandError.value);
// Derived from the snapshot: the last completed attempt (any initiator)
// finished without a failure and found nothing newer. Stays until the next
// completed attempt, even while a new check is in flight.
const updatesUpToDate = computed(() => appUpdates.upToDate.value);
const currentVersion = computed(() => updatesStatus.value?.current_version ?? APP_VERSION);
// Snapshot fallbacks mirror the backend defaults for a missing snapshot.
const startupCheckEnabled = computed(() => updatesStatus.value?.startup_check_enabled ?? true);
const periodicCheckEnabled = computed(() => updatesStatus.value?.periodic_check_enabled ?? false);
const checkIntervalDays = computed(() => updatesStatus.value?.check_interval_days ?? 1);

// Any failed check (startup, periodic or manual) shows the same calm plate;
// the stable code stays in the snapshot and the backend log. The last found
// release keeps rendering below: both are shown together.
const updatesAttemptError = computed(() =>
  updatesStatus.value?.last_attempt_error ? t(UPDATE_CHECK_ERROR_LOCALE_KEY) : null
);
// Time of the last completed attempt, kept apart from the last success time:
// a failed attempt never overwrites the success stamp.
const lastAttemptDisplay = computed(() => {
  const status = updatesStatus.value;
  if (!status?.last_attempt_at) return '';
  if (!status.last_attempt_error && status.last_attempt_at === status.last_check_at) return '';
  return formatCheckTime(status.last_attempt_at);
});

// The card announces a release only while it is newer than the running
// version; an installed version removes the block again.
const updatesRelease = computed(() =>
  updatesStatus.value?.update_available ? updatesStatus.value.latest : null
);
// The «Обновить» button appears only for a supported NSIS installation with
// updater artifacts (backend `install_available`); the progress modal itself
// is hosted by App.vue via useUpdateInstall.
const updatesInstallAvailable = computed(() => updatesStatus.value?.install_available === true);
const updateInstall = useUpdateInstall();
const releaseMarkdown = new MarkdownIt({ html: false });
releaseMarkdown.renderer.rules.image = () => '';
releaseMarkdown.renderer.rules.link_open = () => '';
releaseMarkdown.renderer.rules.link_close = () => '';
const releaseNotesHtml = computed(() => releaseMarkdown.render(updatesRelease.value?.notes ?? ''));

const intervalOptions = computed(() => {
  const options: Array<{ value: number; label: string }> = [];
  for (let days = UPDATES_MIN_INTERVAL_DAYS; days <= UPDATES_MAX_INTERVAL_DAYS; days++) {
    options.push({ value: days, label: t('general.updates.interval.days', days) });
  }
  return options;
});

function formatCheckTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const day = String(date.getDate()).padStart(2, '0');
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const year = String(date.getFullYear()).padStart(4, '0');
  const hours = String(date.getHours()).padStart(2, '0');
  const minutes = String(date.getMinutes()).padStart(2, '0');
  return `${day}.${month}.${year} ${hours}:${minutes}`;
}

const lastCheckDisplay = computed(() => {
  const iso = updatesStatus.value?.last_check_at;
  return iso ? formatCheckTime(iso) : t('general.updates.last_check.never');
});

const updatesCardEl = ref<HTMLElement | null>(null);

function onStartupCheckChange(event: Event) {
  const input = event.target as HTMLInputElement;
  void appUpdates.setStartupCheck(input.checked).then(() => {
    // A failed save sends no snapshot, so Vue never re-renders the bound
    // `:checked`; snap the native input back to the persisted state so it
    // never shows an unsaved value.
    input.checked = startupCheckEnabled.value;
  });
}

function onPeriodicCheckChange(event: Event) {
  const input = event.target as HTMLInputElement;
  void appUpdates.setPeriodicCheck(input.checked).then(() => {
    input.checked = periodicCheckEnabled.value;
  });
}

function onIntervalChange(event: Event) {
  const select = event.target as HTMLSelectElement;
  const days = Number(select.value);
  if (!Number.isInteger(days)) return;
  void appUpdates.setIntervalDays(days).then(() => {
    select.value = String(checkIntervalDays.value);
  });
}

async function openReleasePage() {
  const url = updatesRelease.value?.url;
  if (!url) return;
  try {
    await openUrl(url);
  } catch (e) {
    showMessage(presentCommandError(e, t('general.updates.errors.generic')), 'error');
  }
}

/**
 * Scroll to the card from the update banner.
 */
async function revealUpdates() {
  await nextTick();
  updatesCardEl.value?.scrollIntoView({ behavior: 'smooth', block: 'start' });
}

defineExpose({ revealUpdates });

// ROADMAP-143: where the shared audio output controls live (titlebar or
// under the editor). A local UI preference persisted like the editor tabs
// visibility; a failed write must not break the UI — the placement is saved
// persist-first, so on failure the previous choice stays selected and the
// user is told nothing was stored (it would silently revert on relaunch).
const outputControlsPlacement = getOutputControlsPlacement();
const placementChoices = computed<Array<{ value: OutputControlsPlacement; label: string }>>(() => [
  { value: 'titlebar', label: t('general.outputs.placement.titlebar') },
  { value: 'editor', label: t('general.outputs.placement.editor') },
  { value: 'hidden', label: t('general.outputs.placement.hidden') },
]);
function onOutputPlacementChange(event: Event) {
  const select = event.target as HTMLSelectElement;
  const value = select.value;
  if (value !== 'titlebar' && value !== 'editor' && value !== 'hidden') return;
  if (!setOutputControlsPlacement(value)) {
    // Keep the persisted selection visible instead of the rejected one.
    select.value = outputControlsPlacement.value;
    showMessage(t('general.outputs.placement.save_failed'), 'error');
  }
}

onMounted(refreshDataInfo);

async function toggleExcludeFromCapture() {
  try {
    const newValue = !(windowsSettings.value?.global.exclude_from_capture ?? false);
    await invoke('set_global_exclude_from_capture', { value: newValue });
    showMessage(t('general.saved.restart'), 'warning');
  } catch (e) {
    showMessage(presentCommandError(e, t('general.error.capture')), 'error');
  }
}

async function setLoggingEnabled(value: boolean) {
  const previousValue = localLoggingEnabled.value;
  localLoggingEnabled.value = value;

  try {
    await invoke('save_logging_settings', {
      enabled: value,
      level: loggingSettings.value?.level ?? 'info'
    });
    showMessage(t('general.saved.restart'), 'warning');
  } catch (e) {
    // Rollback to previous value on error
    localLoggingEnabled.value = previousValue;
    showMessage(presentCommandError(e, t('general.error.logging')), 'error');
  }
}

async function onLoggingLevelChange(event: Event) {
  const target = event.target as HTMLSelectElement;
  const newLevel = target.value;
  try {
    await invoke('save_logging_settings', {
      enabled: localLoggingEnabled.value,
      level: newLevel
    });
    showMessage(t('general.level_saved.restart'), 'warning');
  } catch (e) {
    showMessage(presentCommandError(e, t('general.error.logging_level')), 'error');
  }
}

async function toggleStartCompact() {
  try {
    const newValue = !startCompact.value;
    startCompact.value = newValue;
    await invoke('set_start_compact', { value: newValue });
    saveStartCompactToStorage(newValue);
  } catch (e) {
    startCompact.value = !startCompact.value;
    saveStartCompactToStorage(startCompact.value);
    showMessage(presentCommandError(e, t('general.error.save')), 'error');
  }
}

async function toggleShowPlaybackOnStart() {
  try {
    const newValue = !showPlaybackOnStart.value;
    showPlaybackOnStart.value = newValue;
    await invoke('set_show_playback_on_start', { value: newValue });
  } catch (e) {
    showPlaybackOnStart.value = !showPlaybackOnStart.value;
    showMessage(presentCommandError(e, t('general.error.save')), 'error');
  }
}

async function toggleHideOnMinimize() {
  if (hideOnMinimizeSaving.value) return;
  const previousValue = hideOnMinimize.value;
  const newValue = !previousValue;
  hideOnMinimize.value = newValue;
  hideOnMinimizeSaving.value = true;
  try {
    await invoke('set_hide_on_minimize', { value: newValue });
  } catch (e) {
    hideOnMinimize.value = previousValue;
    showMessage(presentCommandError(e, t('general.error.save')), 'error');
  } finally {
    hideOnMinimizeSaving.value = false;
  }
}

async function toggleHideExtraWindowButtons() {
  if (hideExtraWindowButtonsSaving.value) return;
  const previousValue = hideExtraWindowButtons.value;
  const newValue = !previousValue;
  hideExtraWindowButtons.value = newValue;
  hideExtraWindowButtonsSaving.value = true;
  try {
    await invoke('set_hide_extra_window_buttons', { value: newValue });
  } catch (e) {
    hideExtraWindowButtons.value = previousValue;
    showMessage(presentCommandError(e, t('general.error.save')), 'error');
  } finally {
    hideExtraWindowButtonsSaving.value = false;
  }
}

// Watch for settings changes from composables
watch(generalSettings, (newSettings) => {
  if (!newSettings) return;
  showPlaybackOnStart.value = newSettings.show_playback_on_start ?? false;
  startCompact.value = newSettings.start_compact ?? false;
  hideOnMinimize.value = newSettings.hide_on_minimize ?? false;
}, { immediate: true });

watch(windowsSettings, (newSettings) => {
  if (!newSettings) return;
  if (hideExtraWindowButtonsSaving.value) return;
  hideExtraWindowButtons.value = newSettings.main?.hide_extra_window_buttons ?? false;
}, { immediate: true });

watch(loggingSettings, (newSettings) => {
  if (!newSettings) return;
  // Sync local state with composable
  localLoggingEnabled.value = newSettings.enabled;
}, { immediate: true });
</script>

<template>
  <div class="settings-general">
    <!-- Language -->
    <section class="settings-group ui-section">
      <h3 class="settings-group-title ui-group-title">{{ t('general.groups.language') }}</h3>
      <div class="general-row">
        <label class="sr-only" for="ui-language">{{ t('settings.language') }}</label>
        <select
          id="ui-language"
          class="ui-select level-select language-select"
          :value="selectedLanguage"
          :disabled="languageSaving"
          @change="onLanguageChange"
        >
          <option v-for="language in availableLanguages" :key="language.locale" :value="language.locale">
            {{ language.name }}
          </option>
        </select>
        <span class="setting-hint ui-hint language-hint">{{ t('settings.language.restart_hint') }}</span>
        <span v-if="languageSaved" class="setting-warning"><AlertTriangle :size="14" /> {{ t('settings.language.saved.restart') }}</span>
        <span v-if="languageError" class="setting-warning">{{ languageError }}</span>
      </div>
    </section>

    <!-- Window behavior -->
    <section class="settings-group ui-section">
      <h3 class="settings-group-title ui-group-title">{{ t('general.groups.window') }}</h3>

      <div class="general-row">
        <label class="ui-choice-label setting-label">
          <input
            :checked="showPlaybackOnStart"
            @change="toggleShowPlaybackOnStart"
            type="checkbox"
            class="ui-choice-input"
          />
          <span>{{ t('general.show_playback.label') }}</span>
        </label>
        <span class="setting-hint ui-hint">{{ t('general.show_playback.hint') }}</span>
      </div>

      <div class="general-row">
        <label class="ui-choice-label setting-label">
          <input
            :checked="startCompact"
            @change="toggleStartCompact"
            type="checkbox"
            class="ui-choice-input"
          />
          <span>{{ t('general.start_compact.label') }}</span>
        </label>
        <span class="setting-hint ui-hint">{{ t('general.start_compact.hint') }}</span>
        <span class="setting-hint ui-hint">{{ t('general.start_compact.view_hint') }}</span>
      </div>

      <div class="general-row">
        <label class="ui-choice-label setting-label">
          <input
            :checked="hideOnMinimize"
            :disabled="hideOnMinimizeSaving"
            @change="toggleHideOnMinimize"
            type="checkbox"
            class="ui-choice-input"
          />
          <span>{{ t('general.hide_on_minimize.label') }}</span>
        </label>
        <span class="setting-hint ui-hint">{{ t('general.hide_on_minimize.hint') }}</span>
      </div>

      <div class="general-row">
        <label class="ui-choice-label setting-label">
          <input
            :checked="excludeFromCapture"
            type="checkbox"
            class="ui-choice-input"
            @change="toggleExcludeFromCapture"
          />
          <span>{{ t('general.exclude_capture.label') }}</span>
        </label>
        <span class="setting-hint ui-hint">{{ t('general.exclude_capture.hint') }}</span>
        <span class="setting-warning"><AlertTriangle :size="14" /> {{ t('general.restart_required') }}</span>
      </div>
    </section>

    <!-- Additional window buttons -->
    <section class="settings-group ui-section">
      <h3 class="settings-group-title ui-group-title">{{ t('general.groups.window_buttons') }}</h3>

      <div class="general-row">
        <label class="ui-choice-label setting-label">
          <input
            :checked="hideExtraWindowButtons"
            :disabled="hideExtraWindowButtonsSaving || !windowsSettings"
            @change="toggleHideExtraWindowButtons"
            type="checkbox"
            class="ui-choice-input"
          />
          <span>{{ t('general.hide_extra_window_buttons.label') }}</span>
        </label>
        <span class="setting-hint ui-hint ui-hint--choice">{{ t('general.hide_extra_window_buttons.hint') }}</span>
      </div>
    </section>

    <!-- Audio outputs placement (ROADMAP-143) -->
    <section class="settings-group ui-section">
      <h3 class="settings-group-title ui-group-title">{{ t('general.groups.audio_outputs') }}</h3>
      <div class="general-row output-placement-row">
        <label class="ui-label" for="output-controls-placement">{{ t('general.outputs.placement') }}</label>
        <select
          id="output-controls-placement"
          class="ui-select level-select output-placement-select"
          :value="outputControlsPlacement"
          @change="onOutputPlacementChange"
        >
          <option v-for="choice in placementChoices" :key="choice.value" :value="choice.value">
            {{ choice.label }}
          </option>
        </select>
      </div>
    </section>

    <!-- Folders -->
    <section class="settings-group ui-section">
      <h3 class="settings-group-title ui-group-title">{{ t('general.folders.title') }}</h3>

      <div class="folder-row">
        <div class="folder-info">
          <span class="folder-name ui-label ui-label--secondary">{{ t('general.folders.configuration') }}</span>
          <span class="folder-path ui-metadata">{{ DEFAULT_CONFIG_DIR_DISPLAY }}</span>
        </div>
        <div class="folder-actions">
          <button
            type="button"
            class="ui-icon-button"
            :disabled="folderOpening"
            :title="t('general.folders.open')"
            :aria-label="t('general.folders.open')"
            @click="openAppFolder"
          >
            <FolderOpen :size="18" />
          </button>
        </div>
      </div>

      <div class="folder-row">
        <div class="folder-info">
          <span class="folder-name ui-label ui-label--secondary">{{ t('general.folders.program_data') }}</span>
          <span class="folder-path ui-metadata" :title="dataPathTooltip">{{ dataPathDisplay }}</span>
        </div>
        <div class="folder-actions">
          <button
            type="button"
            class="ui-icon-button"
            :disabled="localDataOpening"
            :title="t('general.folders.open')"
            :aria-label="t('general.folders.open')"
            @click="openLocalDataFolder"
          >
            <FolderOpen :size="18" />
          </button>
          <button
            type="button"
            class="ui-icon-button"
            :title="t('general.folders.change')"
            :aria-label="t('general.folders.change')"
            @click="changeDataDir"
          >
            <FolderCog :size="18" />
          </button>
          <button
            type="button"
            class="ui-icon-button"
            :disabled="dataInfo?.is_default !== false"
            :title="t('general.folders.reset')"
            :aria-label="t('general.folders.reset')"
            @click="resetDataDir"
          >
            <RotateCcw :size="18" />
          </button>
        </div>
      </div>
    </section>

    <!-- Diagnostics -->
    <section class="settings-group ui-section">
      <h3 class="settings-group-title ui-group-title">{{ t('general.groups.diagnostics') }}</h3>

      <div class="general-row logging-controls-row">
        <label class="ui-choice-label setting-label">
          <input
            :checked="loggingEnabled"
            @change="(e) => setLoggingEnabled((e.target as HTMLInputElement).checked)"
            type="checkbox"
            class="ui-choice-input"
          />
          <span>{{ t('general.logging.enabled') }}</span>
        </label>

        <div v-if="loggingEnabled" class="logging-level-control">
          <label class="ui-label ui-label--secondary">{{ t('general.logging.level.label') }}</label>
          <select
            :value="loggingLevel"
            @change="onLoggingLevelChange"
            class="ui-select level-select"
          >
            <option v-for="level in loggingLevels" :key="level.value" :value="level.value">
              {{ level.label }}
            </option>
          </select>
        </div>
      </div>

      <span class="setting-warning logging-warning">
        <AlertTriangle :size="14" />
        {{ t('general.restart_required') }}
      </span>
    </section>

    <!-- Updates (ROADMAP-141): current version moved here from the sidebar -->
    <section ref="updatesCardEl" class="settings-group ui-section updates-section">
      <h3 class="settings-group-title ui-group-title">{{ t('general.groups.updates') }}</h3>

      <div class="updates-version-row">
        <div class="updates-current-version">
          <span class="ui-label ui-label--secondary">{{ t('general.updates.current_version') }}</span>
          <span class="ui-metadata updates-version">{{ currentVersion }}</span>
        </div>
        <button
          type="button"
          class="ui-button ui-button--primary updates-check-button"
          :disabled="updatesChecking"
          @click="appUpdates.checkNow()"
        >
          <Loader2 v-if="updatesChecking" :size="16" class="ui-spin" />
          {{ updatesChecking ? t('general.updates.checking') : t('general.updates.check_now') }}
        </button>
      </div>

      <dl class="updates-check-times">
        <div>
          <dt class="ui-label ui-label--secondary">{{ t('general.updates.last_check.label') }}</dt>
          <dd class="ui-metadata updates-last-check">{{ lastCheckDisplay }}</dd>
        </div>
        <div v-if="lastAttemptDisplay">
          <dt class="ui-label ui-label--secondary">{{ t('general.updates.last_attempt.label') }}</dt>
          <dd class="ui-metadata updates-last-attempt">{{ lastAttemptDisplay }}</dd>
        </div>
      </dl>

      <!-- Calm persistent plate for the failed attempt, inside the card; a
           found release keeps rendering below next to it. -->
      <div v-if="updatesAttemptError" class="setting-warning updates-error-plate" role="status">
        <AlertTriangle :size="14" />
        <span class="updates-error-text">{{ updatesAttemptError }}</span>
      </div>
      <span v-else-if="updatesUpToDate" class="updates-ok">
        <Check :size="14" /> {{ t('general.updates.up_to_date') }}
      </span>
      <div v-if="updatesRelease" class="updates-release">
        <div class="updates-release-head">
          <span class="updates-release-title">{{ t('general.updates.available', { version: updatesRelease.version }) }}</span>
          <button
            type="button"
            class="ui-button ui-button--compact ui-button--leading-icon"
            @click="openReleasePage"
          >
            <ExternalLink :size="16" />
            {{ t('general.updates.open_release') }}
          </button>
        </div>
        <div v-if="releaseNotesHtml" class="updates-notes" tabindex="0" v-html="releaseNotesHtml"></div>
        <!-- In-app install (ROADMAP-141, stage 2): only for a supported NSIS
             installation with an available updater artifact (see
             `install_available`); standalone builds keep the GitHub link. -->
        <div v-if="updatesInstallAvailable" class="updates-install-row">
          <button
            type="button"
            class="ui-button ui-button--primary updates-install-button"
            :disabled="updateInstall.active.value"
            @click="updateInstall.start()"
          >
            {{ t('general.updates.install.label') }}
          </button>
        </div>
      </div>

      <!-- Two independent automatic checks (ROADMAP-141, stage 2): the
           startup switch and the periodic schedule. The interval selector is
           enabled only while the schedule is on and keeps its value when it
           is turned off. -->
      <div class="updates-auto">
        <label class="ui-choice-label setting-label">
          <input
            :checked="startupCheckEnabled"
            @change="onStartupCheckChange"
            type="checkbox"
            class="ui-choice-input"
          />
          <span>{{ t('general.updates.startup_check.label') }}</span>
        </label>
      </div>

      <div class="updates-interval">
        <label class="ui-choice-label setting-label">
          <input
            :checked="periodicCheckEnabled"
            @change="onPeriodicCheckChange"
            type="checkbox"
            class="ui-choice-input"
          />
          <span>{{ t('general.updates.periodic_check.label') }}</span>
        </label>
        <div class="updates-interval-field">
          <label class="ui-label ui-label--secondary" for="updates-interval">
            {{ t('general.updates.interval.label') }}
          </label>
          <select
            id="updates-interval"
            class="ui-select updates-interval-select"
            :value="checkIntervalDays"
            :disabled="!periodicCheckEnabled"
            @change="onIntervalChange"
          >
            <option v-for="option in intervalOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </div>
      </div>

      <span v-if="updatesCommandError" class="setting-warning updates-error">
        <AlertTriangle :size="14" /> {{ updatesCommandError }}
      </span>
    </section>

    <DataTransferModal
      v-if="transferTarget !== undefined"
      :target-path="transferTarget"
      @close="transferTarget = undefined"
      @success="handleTransferSuccess"
    />
  </div>
</template>

<style scoped>
.settings-general {
  display: flex;
  flex-direction: column;
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

/* Card skin stays local; section padding/rhythm come from ui-section. */
.settings-group {
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
}

.settings-group:last-of-type {
  margin-bottom: 0;
}

.settings-group-title {
  margin: 0 0 0.5rem;
}

.general-row {
  display: block;
  margin-bottom: var(--ui-row-gap);
}

.general-row:last-child {
  margin-bottom: 0;
}

.output-placement-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--ui-row-gap) var(--ui-row-label-gap-side);
}

.setting-label {
  color: var(--color-text-primary);
  user-select: none;
}

.setting-hint {
  display: block;
  margin-left: 2.4rem;
}

.language-hint {
  margin-left: 0;
}

.language-select {
  width: min(100%, 180px);
}

.setting-warning {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  margin-top: 0.5rem;
  margin-left: 2.4rem;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--warning-text-bright);
}

.logging-controls-row {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--ui-field-group-gap);
}

.logging-level-control {
  display: flex;
  align-items: center;
  gap: var(--ui-row-label-gap-side);
}

.setting-warning.logging-warning {
  margin-left: 0;
}

.level-select {
  min-width: 140px;
}

.logging-level-control .level-select {
  width: 180px;
  flex: 0 0 auto;
}

.folder-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.5rem 1rem;
  flex-wrap: wrap;
  padding: 0.6rem 0;
}

.folder-row + .folder-row {
  border-top: 1px solid var(--color-border);
}

.folder-row:last-child {
  padding-bottom: 0;
}

.folder-info {
  flex: 1 1 160px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
}

.folder-path {
  color: var(--color-text-primary);
  flex: 0 1 auto;
  min-width: 0;
  font-family: var(--font-mono);
  overflow-wrap: anywhere;
  word-break: break-word;
}

.folder-actions {
  display: flex;
  justify-content: flex-end;
  margin-left: auto;
  align-items: center;
  gap: 0.5rem;
  flex: 0 0 auto;
  flex-wrap: wrap;
}

/* ==================== Updates (ROADMAP-141) ==================== */

.updates-version-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--ui-row-gap) var(--ui-row-label-gap-side);
}

.updates-current-version,
.updates-check-times > div {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 0.25rem;
}

.updates-check-times {
  display: grid;
  gap: 0.25rem;
  margin: var(--ui-row-gap) 0 0;
  overflow-wrap: anywhere;
}

.updates-check-times dd {
  margin: 0;
}

.updates-auto,
.updates-release {
  margin-top: var(--ui-row-gap);
}

.updates-version,
.updates-last-check,
.updates-last-attempt {
  color: var(--color-text-primary);
}

.updates-version {
  font-family: var(--font-mono);
}

.updates-check-button {
  margin-inline-start: auto;
}

.updates-interval {
  display: grid;
  gap: var(--ui-hint-gap);
}

.updates-interval-field {
  display: flex;
  align-items: center;
  gap: var(--ui-row-label-gap-side);
  flex-wrap: wrap;
  margin-inline-start: calc(var(--ui-choice-size) + 2 * var(--ui-choice-gap));
}

.updates-auto:has(+ *),
.updates-interval:has(+ *) {
  margin-bottom: 8px;
}

.updates-interval-select {
  max-width: 100%;
}

.setting-warning.updates-error {
  margin-left: 0;
  margin-top: 0;
}

/* Calm persistent plate for a failed check: a bordered box inside the card,
   not a popup, toast or bubble; the text wraps within the card width. */
.updates-error-plate {
  align-items: flex-start;
  margin-left: 0;
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  background: var(--color-bg-elevated);
}

.updates-error-plate svg {
  flex: 0 0 auto;
  margin-top: 0.15em;
}

.updates-error-plate .updates-error-text {
  overflow-wrap: anywhere;
}

/* Outcome of a finished manual check that found nothing newer; mirrors the
   error line above with the success color. */
.updates-ok {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  margin-top: var(--ui-hint-gap);
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--success-text-bright);
}

/* «Подробнее» from the update bubble scrolls the card below the bubble
   overlay (4rem clears its position under the titlebar at any interface
   font size). */
.updates-section {
  scroll-margin-top: 4rem;
}

.updates-release {
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
  padding-top: 0.75rem;
  border-top: 1px solid var(--color-border);
}

.updates-release-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem 1rem;
  flex-wrap: wrap;
}

.updates-release-title {
  font-size: 0.95rem;
  font-weight: 600;
  color: var(--color-text-primary);
}

.updates-notes {
  max-height: 16rem;
  overflow-y: auto;
  overscroll-behavior-y: contain;
  scrollbar-gutter: stable;
  padding: 0.6rem 0.75rem;
  border-radius: 8px;
  background: var(--color-bg-elevated);
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.updates-notes :deep(> :first-child) {
  margin-top: 0;
}

.updates-notes :deep(> :last-child) {
  margin-bottom: 0;
}

/* Install button: centered under the scrollable release notes, no caption. */
.updates-install-row {
  display: flex;
  justify-content: center;
}

.updates-install-button {
  min-width: 10rem;
}

/* The global `* { margin: 0; padding: 0 }` reset strips list indentation, so
   outside markers hang left of the content box and get clipped by this
   scroll container's edge. Restore the logical list padding (nested lists
   receive their own shift from the same rule). */
.updates-notes :deep(ul),
.updates-notes :deep(ol) {
  margin: 0.4em 0;
  padding-left: 1.5rem;
}

.updates-notes :deep(li) {
  margin: 0.15em 0;
}

.updates-notes :deep(p) {
  margin: 0.5em 0;
}

.updates-notes :deep(pre) {
  white-space: pre-wrap;
}

/* Explicit vertical spacing after headings (the first-child rule above
   removes the leading gap of a heading that opens the notes). */
.updates-notes :deep(:is(h1, h2, h3, h4, h5, h6)) {
  font-size: 1em;
  font-weight: 600;
  margin: 0.9em 0 0.4em;
}

@media (max-width: 520px) {
  .language-select {
    width: 100%;
  }

  .logging-controls-row {
    align-items: stretch;
    flex-direction: column;
    gap: var(--ui-field-group-gap);
  }

  .logging-level-control {
    flex-direction: column;
    align-items: stretch;
    gap: 0.4rem;
  }

  .logging-level-control .level-select {
    width: 100%;
    min-width: 0;
    flex: 1 1 100%;
  }

}
</style>
