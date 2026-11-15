<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { Network, Loader2, Check, X } from 'lucide-vue-next';
import { useAudioSettings } from '../../composables/useAppSettings';
import { debugError } from '../../utils/debug';
import { t } from '../../i18n';
import { presentCommandError } from '../../ipc/commandError';
import type { VbanSettingsDto, VbanStatusDto } from '../../types/settings';

/** Poll cadence for the local production sender status. */
const STATUS_POLL_MS = 2000;

/** How long a successful save or test result stays visible. */
const SUCCESS_STATUS_MS = 3000;

/** Contract defaults for a snapshot without the section (old configs). */
const DEFAULT_VBAN: VbanSettingsDto = {
  enabled: false,
  address: '',
  port: 6980,
  stream_name: 'TTSBard',
  volume: 100,
};

/** Editable form shape: the port stays raw text so invalid typing is visible. */
interface VbanDraft {
  enabled: boolean;
  address: string;
  port: string;
  stream_name: string;
  volume: number;
}

const props = defineProps<{ active: boolean }>();

const emit = defineEmits<{
  (e: 'dirty-change', dirty: boolean): void;
  (e: 'status-change', status: VbanStatusDto): void;
}>();

const audioSettingsFromComposable = useAudioSettings();

function fromSettings(section: VbanSettingsDto | undefined): VbanDraft {
  const s = section ?? DEFAULT_VBAN;
  return {
    enabled: s.enabled,
    address: s.address,
    port: String(s.port),
    stream_name: s.stream_name,
    volume: s.volume,
  };
}

const draft = ref<VbanDraft>(fromSettings(undefined));
const saved = ref<VbanDraft>(fromSettings(undefined));

const isSaving = ref(false);
const isTesting = ref(false);
const isUpdatingControls = ref(false);
const isToggling = ref(false);
const controlError = ref('');
let pendingVolume: number | undefined;
let confirmedVolume = DEFAULT_VBAN.volume;
const saveStatus = ref<'idle' | 'saved' | 'error'>('idle');
const saveError = ref('');
const testResult = ref<'' | 'success' | 'error'>('');
const testMessage = ref('');
const attemptedTest = ref(false);
let savedStatusTimer: ReturnType<typeof setTimeout> | undefined;
let testStatusTimer: ReturnType<typeof setTimeout> | undefined;
let actionGeneration = 0;
let disposed = false;

// ---------------------------------------------------------------------------
// Backend status polling (production sends from this PC only)
// ---------------------------------------------------------------------------

const backendStatus = ref<VbanStatusDto>({ state: 'disabled', error: null });
let statusTimer: ReturnType<typeof setInterval> | undefined;
let statusInFlight = false;

/** A malformed result keeps the last known status instead of blanking the line. */
function isVbanStatus(value: unknown): value is VbanStatusDto {
  if (!value || typeof value !== 'object') return false;
  const candidate = value as Partial<VbanStatusDto>;
  return (candidate.state === 'disabled' || candidate.state === 'idle'
    || candidate.state === 'sending' || candidate.state === 'error')
    && (candidate.error === null || typeof candidate.error === 'string');
}

async function refreshStatus() {
  if (disposed || statusInFlight) return;
  statusInFlight = true;
  try {
    const status = await invoke<VbanStatusDto>('get_vban_status');
    if (!disposed && isVbanStatus(status)) {
      backendStatus.value = status;
      // The Audio panel tab owns the production indicator; the body keeps
      // only the diagnostic error text.
      emit('status-change', status);
    }
  } catch (error) {
    if (!disposed) debugError('[AudioVbanTab] Failed to read VBAN status:', error);
  } finally {
    statusInFlight = false;
  }
}

function startPolling() {
  if (statusTimer === undefined && !disposed) {
    statusTimer = setInterval(() => { void refreshStatus(); }, STATUS_POLL_MS);
  }
}

function stopPolling() {
  if (statusTimer !== undefined) {
    clearInterval(statusTimer);
    statusTimer = undefined;
  }
}

onMounted(() => {
  // The production status is shown on the Audio panel tab, so it must stay
  // current even while another audio tab is visible.
  if (disposed) return;
  void refreshStatus();
  startPolling();
});

watch(() => props.active, (active) => {
  // Refresh immediately when the tab becomes visible; polling itself is
  // independent of the visible tab.
  if (active && !disposed) void refreshStatus();
});

onUnmounted(() => {
  disposed = true;
  stopPolling();
  actionGeneration++;
  if (savedStatusTimer !== undefined) clearTimeout(savedStatusTimer);
  stopTestStatusTimer();
});

// ---------------------------------------------------------------------------
// Validation (mirrors the backend rules from the ROADMAP-138 contract)
// ---------------------------------------------------------------------------

function isIPv4(value: string): boolean {
  // Mirrors Rust `Ipv4Addr::from_str`: 0-255 octets, leading zeroes rejected.
  if (!/^(0|[1-9]\d{0,2})\.(0|[1-9]\d{0,2})\.(0|[1-9]\d{0,2})\.(0|[1-9]\d{0,2})$/.test(value)) return false;
  return value.split('.').every((octet) => Number(octet) <= 255);
}

const addressValue = computed(() => draft.value.address.trim());
const parsedPort = computed(() => {
  const raw = draft.value.port.trim();
  if (!/^\d+$/.test(raw)) return null;
  return Number(raw);
});
const streamValue = computed(() => draft.value.stream_name.trim());
const volumeValue = computed(() => draft.value.volume);
const editingField = ref<'address' | 'port' | 'stream_name' | null>(null);

const addressFilledValid = computed(() => addressValue.value !== '' && isIPv4(addressValue.value));
const portValid = computed(() => parsedPort.value !== null && parsedPort.value >= 1 && parsedPort.value <= 65535);
const streamValid = computed(() => /^[\x20-\x7E]{1,16}$/.test(streamValue.value));
const volumeValid = computed(() => Number.isFinite(volumeValue.value) && volumeValue.value >= 0 && volumeValue.value <= 100);

const addressError = computed(() => {
  const value = addressValue.value;
  if (value === '') {
    if (draft.value.enabled || attemptedTest.value) return t('audio.vban.address.required');
    return '';
  }
  return isIPv4(value) ? '' : t('audio.vban.address.invalid');
});
const portError = computed(() => (portValid.value ? '' : t('audio.vban.port.invalid')));
const streamError = computed(() => (streamValid.value ? '' : t('audio.vban.stream_name.invalid')));
const visibleAddressError = computed(() => editingField.value === 'address' ? '' : addressError.value);
const visiblePortError = computed(() => editingField.value === 'port' ? '' : portError.value);
const visibleStreamError = computed(() => editingField.value === 'stream_name' ? '' : streamError.value);

/** The address blocks Save only when it is definitely invalid: an enabled
    VBAN needs a valid destination, while a disabled VBAN stores any string
    (empty included) and never parses the address. */
const addressBlocksSave = computed(() => draft.value.enabled && addressError.value !== '');

/** Everything the backend will validate before applying saved settings. */
const formValid = computed(() => !addressBlocksSave.value && portValid.value && streamValid.value && volumeValid.value);
/** A concrete send destination: testable regardless of the enabled toggle. */
const targetValid = computed(() => addressFilledValid.value && portValid.value && streamValid.value && volumeValid.value);

function toSettings(source: VbanDraft): VbanSettingsDto {
  const rawPort = source.port.trim();
  return {
    enabled: source.enabled,
    address: source.address.trim(),
    port: /^\d+$/.test(rawPort) ? Number(rawPort) : 0,
    stream_name: source.stream_name.trim(),
    volume: source.volume,
  };
}

function normalized(source: VbanDraft): string {
  const { address, port, stream_name } = toSettings(source);
  return JSON.stringify({ address, port, stream_name });
}

const isDirty = computed(() => normalized(draft.value) !== normalized(saved.value));

watch(isDirty, (dirty) => {
  emit('dirty-change', dirty);
}, { immediate: true });

// Keep the local form aligned with the shared settings cache unless the user
// has unsaved edits (persist-first saves update the cache asynchronously).
watch(audioSettingsFromComposable, (settings) => {
  if (!settings) return;
  const incoming = fromSettings(settings.vban);
  if (!isUpdatingControls.value) {
    draft.value.enabled = incoming.enabled;
    draft.value.volume = incoming.volume;
    confirmedVolume = incoming.volume;
  }
  if (!isDirty.value) {
    const { enabled, volume } = draft.value;
    draft.value = { ...incoming, enabled, volume };
    saved.value = incoming;
  }
}, { immediate: true });

// ---------------------------------------------------------------------------
// Form interaction
// ---------------------------------------------------------------------------

function onFormInput() {
  attemptedTest.value = false;
  if (saveStatus.value === 'saved' || saveStatus.value === 'error') {
    saveStatus.value = 'idle';
    saveError.value = '';
  }
  if (testResult.value !== '') {
    stopTestStatusTimer();
    testResult.value = '';
    testMessage.value = '';
  }
}

function onPortInput(event: Event) {
  draft.value.port = (event.target as HTMLInputElement).value;
  onFormInput();
}

function onVolumeInput(event: Event) {
  if (isSaving.value || isTesting.value) return;
  const value = Number((event.target as HTMLInputElement).value);
  draft.value.volume = Number.isFinite(value) ? Math.min(100, Math.max(0, Math.round(value))) : 0;
  onFormInput();
  pendingVolume = draft.value.volume;
  void updateVolume();
}

// Serialize slider writes and coalesce intermediate positions. A late reply
// cannot overwrite a newer slider position or apply a network draft.
async function updateVolume() {
  if (isUpdatingControls.value) return;
  isUpdatingControls.value = true;
  controlError.value = '';
  try {
    while (pendingVolume !== undefined && !disposed) {
      const volume = pendingVolume;
      pendingVolume = undefined;
      try {
        await invoke('set_vban_volume', { volume });
        confirmedVolume = volume;
        controlError.value = '';
      } catch (error) {
        if (disposed) return;
        controlError.value = presentCommandError(error, t('audio.vban.error.volume'));
        if (pendingVolume === undefined) draft.value.volume = confirmedVolume;
      }
    }
  } finally {
    isUpdatingControls.value = false;
  }
}

const canEnable = computed(() => {
  const connection = toSettings(saved.value);
  return isIPv4(connection.address) && connection.port >= 1 && connection.port <= 65535
    && /^[\x20-\x7E]{1,16}$/.test(connection.stream_name);
});

async function onEnabledChange(event: Event) {
  const enabled = (event.target as HTMLInputElement).checked;
  const previous = draft.value.enabled;
  draft.value.enabled = enabled;
  controlError.value = '';
  isToggling.value = true;
  isUpdatingControls.value = true;
  try {
    await invoke('set_vban_enabled', { enabled });
    if (!disposed) void refreshStatus();
  } catch (error) {
    if (disposed) return;
    draft.value.enabled = previous;
    controlError.value = presentCommandError(error, t('audio.vban.error.enable'));
  } finally {
    isUpdatingControls.value = false;
    isToggling.value = false;
  }
}

// ---------------------------------------------------------------------------
// Save and test actions
// ---------------------------------------------------------------------------

const canSave = computed(() => isDirty.value && formValid.value && !isSaving.value && !isTesting.value && !isUpdatingControls.value);
const canTest = computed(() => targetValid.value && !isSaving.value && !isTesting.value && !isUpdatingControls.value);

function stopSavedStatusTimer() {
  if (savedStatusTimer !== undefined) {
    clearTimeout(savedStatusTimer);
    savedStatusTimer = undefined;
  }
}

function stopTestStatusTimer() {
  if (testStatusTimer !== undefined) {
    clearTimeout(testStatusTimer);
    testStatusTimer = undefined;
  }
}

async function save() {
  if (!canSave.value) return;
  const gen = ++actionGeneration;
  isSaving.value = true;
  saveStatus.value = 'idle';
  saveError.value = '';
  stopTestStatusTimer();
  testResult.value = '';
  testMessage.value = '';
  // Snapshot of the values actually sent: the IPC response confirms these,
  // not whatever the user typed while the request was in flight. The editable
  // draft keeps later edits, so they stay dirty and can be saved afterwards.
  const snapshot = { ...draft.value };
  const payload = toSettings(draft.value);
  try {
    await invoke('set_vban_connection_settings', { settings: payload });
    if (disposed || gen !== actionGeneration) return;
    saved.value = snapshot;
    saveStatus.value = 'saved';
    stopSavedStatusTimer();
    savedStatusTimer = setTimeout(() => {
      savedStatusTimer = undefined;
      if (saveStatus.value === 'saved') saveStatus.value = 'idle';
    }, SUCCESS_STATUS_MS);
    void refreshStatus();
  } catch (error) {
    if (disposed || gen !== actionGeneration) return;
    saveStatus.value = 'error';
    saveError.value = presentCommandError(error, t('audio.vban.error.save'));
  } finally {
    if (!disposed && gen === actionGeneration) isSaving.value = false;
  }
}

async function runTest() {
  if (!canTest.value) return;
  const gen = ++actionGeneration;
  attemptedTest.value = false;
  isTesting.value = true;
  stopTestStatusTimer();
  testResult.value = '';
  testMessage.value = '';
  saveStatus.value = 'idle';
  saveError.value = '';
  // Snapshot: the test always sends the form as shown when the click happened,
  // even if the user keeps editing or the cache reloads during the send.
  const payload = toSettings(draft.value);
  try {
    await invoke('test_vban_output', { settings: payload });
    if (disposed || gen !== actionGeneration) return;
    testResult.value = 'success';
    testMessage.value = t('audio.vban.test.success');
    // A successful test is transient feedback, like the saved status; an error
    // below stays visible until the form changes or another action runs.
    testStatusTimer = setTimeout(() => {
      testStatusTimer = undefined;
      if (disposed) return;
      if (testResult.value === 'success') {
        testResult.value = '';
        testMessage.value = '';
      }
    }, SUCCESS_STATUS_MS);
  } catch (error) {
    if (disposed || gen !== actionGeneration) return;
    testResult.value = 'error';
    testMessage.value = presentCommandError(error, t('audio.vban.error.test'));
  } finally {
    if (!disposed && gen === actionGeneration) isTesting.value = false;
  }
}

// ---------------------------------------------------------------------------
// Derived display state
// ---------------------------------------------------------------------------

const isLoadingSettings = computed(() => audioSettingsFromComposable.value === undefined);

const actionMessage = computed(() => {
  if (isSaving.value) return { text: t('audio.save.saving'), kind: 'progress' };
  if (isTesting.value) return { text: t('audio.vban.test.sending'), kind: 'progress' };
  if (saveStatus.value === 'error') return { text: saveError.value, kind: 'error' };
  if (saveStatus.value === 'saved') return { text: t('audio.save.saved'), kind: 'success' };
  if (testResult.value !== '') return { text: testMessage.value, kind: testResult.value };
  return null;
});
</script>

<template>
  <div>
    <div v-if="isLoadingSettings" class="loading">
      {{ t('audio.vban.loading') }}
    </div>

    <div v-else class="vban-settings">
      <div class="vban-card ui-section">
        <div class="vban-header">
          <Network class="section-icon" :size="20" />
          <span class="ui-section-title section-title">{{ t('audio.vban.title') }}</span>
          <label class="toggle-switch">
            <input
              type="checkbox"
              :checked="draft.enabled"
              :disabled="isSaving || isTesting || isUpdatingControls || (!draft.enabled && !canEnable)"
              :aria-label="t('audio.vban.enable')"
              @change="onEnabledChange"
            />
            <span class="toggle-slider"></span>
          </label>
        </div>

        <p
          v-if="backendStatus.state === 'error' && backendStatus.error"
          class="vban-status-error"
          role="status"
          aria-live="polite"
        >
          {{ backendStatus.error }}
        </p>

        <div class="vban-live-controls">
          <div class="vban-row vban-row--volume">
            <label class="ui-label" for="vban-volume">{{ t('audio.vban.volume') }}</label>
            <div class="vban-field-stack">
              <div class="volume-control">
                <input id="vban-volume" type="range" min="0" max="100"
                  :value="draft.volume" :disabled="isSaving || isTesting || isToggling"
                  @input="onVolumeInput" />
                <span class="volume-value">{{ draft.volume }}%</span>
              </div>
            </div>
          </div>
          <p v-if="controlError" class="ui-hint field-error" role="status">{{ controlError }}</p>
        </div>

        <h3 class="ui-group-title vban-connection-title">{{ t('audio.vban.connection') }}</h3>
        <div class="vban-form">
          <div class="vban-pair">
            <label class="ui-label" for="vban-address">{{ t('audio.vban.address') }}</label>
            <div class="vban-field-stack">
              <input
                id="vban-address"
                type="text"
                class="ui-input"
                v-model="draft.address"
                :aria-invalid="visibleAddressError ? 'true' : 'false'"
                :aria-describedby="visibleAddressError ? 'vban-address-error' : undefined"
                autocomplete="off"
                spellcheck="false"
                @focus="editingField = 'address'"
                @blur="editingField = null"
                @input="onFormInput"
              />
            </div>
            <label class="ui-label" for="vban-port">{{ t('audio.vban.port') }}</label>
            <div class="vban-field-stack">
              <input
                id="vban-port"
                type="number"
                inputmode="numeric"
                min="1"
                max="65535"
                step="1"
                class="ui-input"
                :value="draft.port"
                :aria-invalid="visiblePortError ? 'true' : 'false'"
                :aria-describedby="visiblePortError ? 'vban-port-error' : undefined"
                autocomplete="off"
                spellcheck="false"
                @focus="editingField = 'port'"
                @blur="editingField = null"
                @input="onPortInput"
              />
            </div>
          </div>

          <div class="vban-row">
            <label class="ui-label" for="vban-stream-name">{{ t('audio.vban.stream_name') }}</label>
            <div class="vban-field-stack">
              <input
                id="vban-stream-name"
                type="text"
                class="ui-input"
                v-model="draft.stream_name"
                maxlength="16"
                :aria-invalid="visibleStreamError ? 'true' : 'false'"
                :aria-describedby="visibleStreamError ? 'vban-stream-error' : undefined"
                autocomplete="off"
                spellcheck="false"
                @focus="editingField = 'stream_name'"
                @blur="editingField = null"
                @input="onFormInput"
              />
            </div>
          </div>

        </div>

        <div v-if="visibleAddressError || visiblePortError || visibleStreamError" class="vban-validation" role="status" aria-live="polite">
          <p v-if="visibleAddressError" id="vban-address-error" class="ui-hint field-error">{{ visibleAddressError }}</p>
          <p v-if="visiblePortError" id="vban-port-error" class="ui-hint field-error">{{ visiblePortError }}</p>
          <p v-if="visibleStreamError" id="vban-stream-error" class="ui-hint field-error">{{ visibleStreamError }}</p>
        </div>

        <div class="save-section">
          <div class="save-status-area" role="status" aria-live="polite" aria-atomic="true">
            <span
              v-if="actionMessage"
              class="save-status"
              :class="`save-status--${actionMessage.kind}`"
            >
              <Loader2 v-if="actionMessage.kind === 'progress'" :size="16" class="ui-spin" aria-hidden="true" />
              <Check v-else-if="actionMessage.kind === 'success'" :size="16" aria-hidden="true" />
              <X v-else :size="16" aria-hidden="true" />
              <span>{{ actionMessage.text }}</span>
            </span>
          </div>
          <button
            type="button"
            class="ui-button test-btn"
            :disabled="!canTest"
            @click="runTest"
          >
            <span>{{ t('audio.vban.test') }}</span>
          </button>
          <button
            type="button"
            class="ui-button ui-button--primary save-btn"
            :disabled="!canSave"
            @click="save"
          >
            <span v-if="isSaving">{{ t('audio.save.saving') }}</span>
            <span v-else>{{ t('common.save') }}</span>
          </button>
        </div>
      </div>

      <section class="ui-section vban-help" aria-labelledby="vban-help-title">
        <h2 id="vban-help-title" class="ui-section-title">{{ t('audio.vban.help.title') }}</h2>
        <p id="vban-help-panel" class="ui-description">{{ t('audio.vban.help.text') }}</p>
      </section>
    </div>
  </div>
</template>

<style scoped>
.loading {
  text-align: center;
  padding: 40px;
  color: var(--color-text-secondary);
}

.vban-settings {
  display: flex;
  flex-direction: column;
}

/* Card skin stays local; padding/rhythm come from ui-section. */
.vban-card {
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
  /* Query container for the form: its content box is the form width. */
  container-type: inline-size;
  container-name: vban-form;
}

/* Header row with icon + title + enable toggle (agreed audio-block toggle). */
.vban-header {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--color-border);
}

.vban-header .section-title {
  flex: 1;
}

.section-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

/* 44 × 24 capsule, 18 × 18 thumb — agreed audio-block toggle geometry. */
.toggle-switch {
  position: relative;
  display: inline-block;
  width: 44px;
  height: 24px;
  flex: 0 0 44px;
}

.toggle-switch input {
  opacity: 0;
  width: 0;
  height: 0;
}

.toggle-slider {
  position: absolute;
  cursor: pointer;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border);
  transition: 0.25s;
  border-radius: 24px;
}

.toggle-slider:before {
  position: absolute;
  content: "";
  height: 18px;
  width: 18px;
  left: 2px;
  bottom: 2px;
  background: var(--color-text-secondary);
  transition: 0.25s;
  border-radius: 50%;
}

.toggle-switch input:checked + .toggle-slider {
  background: var(--color-accent);
  border-color: var(--color-accent);
}

.toggle-switch input:checked + .toggle-slider:before {
  transform: translateX(20px);
  background: var(--color-text-white);
}

.toggle-switch input:focus-visible + .toggle-slider {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

/* Only the diagnostic backend text lives in the body: the production status
   itself is shown in the Audio panel tab. */
.vban-status-error {
  margin: 0 0 var(--ui-row-gap);
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  line-height: 1.5;
  color: var(--color-danger);
  overflow-wrap: anywhere;
}

.vban-form {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr) max-content 96px max-content minmax(0, 1fr);
  column-gap: var(--ui-field-group-gap);
  row-gap: var(--ui-row-gap);
  align-items: start;
  min-width: 0;
}

.vban-live-controls {
  padding-bottom: var(--ui-section-padding-y);
  margin-bottom: var(--ui-section-gap);
  border-bottom: 1px solid var(--color-border);
}

.vban-connection-title {
  margin: 0 0 var(--ui-row-gap);
}

/* Rows and the pair are wrappers: their children take the shared columns. */
.vban-row,
.vban-pair {
  display: contents;
}

.vban-row > .ui-label,
.vban-pair > .ui-label {
  color: var(--color-text-secondary);
  /* Label text sits on the first line of the 38 px field. 4 px padding plus
     the 8 px column gap makes the agreed 12 px label-to-field distance. */
  padding-top: calc(var(--ui-control-padding-y) + var(--ui-border-width));
  padding-right: 4px;
  white-space: nowrap;
}

.vban-field-stack {
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.vban-field-stack .ui-input {
  width: 100%;
}

.vban-row--volume {
  grid-column: 1 / -1;
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  column-gap: var(--ui-row-label-gap-side);
  align-items: center;
}

.field-error {
  color: var(--color-danger);
  overflow-wrap: anywhere;
}

.vban-validation {
  display: flex;
  flex-direction: column;
  gap: var(--ui-hint-gap);
  margin-top: var(--ui-hint-gap);
}

.vban-validation .field-error {
  margin: 0;
}

/* Slider row: no field frame, so the label centers on the slider line. */
.vban-row--volume > .ui-label {
  padding-top: 0;
  align-self: start;
  margin-top: 3px;
}

.vban-row--volume .volume-control {
  min-width: 0;
}

.vban-help > :last-child {
  margin-bottom: 0;
}

.vban-help {
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
}

.vban-help .ui-section-title {
  margin: 0 0 var(--ui-hint-gap);
}

.save-section {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: start;
  gap: var(--ui-field-group-gap);
  justify-content: flex-end;
  margin-top: var(--ui-row-gap);
  padding-top: var(--ui-section-padding-y);
  border-top: 1px solid var(--color-border);
  flex-shrink: 0;
  box-sizing: border-box;
}

.save-status-area {
  display: flex;
  justify-content: flex-start;
  align-items: center;
  min-height: var(--ui-control-min-height);
  min-width: 0;
}

.save-status {
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  max-width: 100%;
  padding: 8px 12px;
  border: 1px solid transparent;
  border-radius: var(--ui-radius);
  line-height: 1.4;
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.save-status > svg {
  flex-shrink: 0;
}

.save-status > span {
  min-width: 0;
  overflow-wrap: anywhere;
}

.save-status--progress {
  background: var(--color-bg-elevated);
  border-color: var(--color-border);
  color: var(--color-text-secondary);
}

.save-status--success {
  background: var(--success-bg-weak);
  border-color: var(--success-border);
  color: var(--success-text-bright);
}

.save-status--error {
  background: var(--danger-bg-weak);
  border-color: var(--danger-border-strong);
  color: var(--danger-text-bright);
}

[data-theme="light"] .save-status--success {
  color: var(--success-text);
}

[data-theme="light"] .save-status--error {
  color: var(--danger-text);
}

.test-btn,
.save-btn {
  white-space: nowrap;
  flex-shrink: 0;
}

@container vban-form (max-width: 465px) {
  .vban-form {
    grid-template-columns: minmax(0, 1fr);
    row-gap: var(--ui-row-label-gap-stack);
  }

  .vban-row > .ui-label,
  .vban-pair > .ui-label {
    padding-top: 0;
    padding-right: 0;
    white-space: normal;
  }

  .vban-row > :not(.ui-label),
  .vban-pair > .vban-field-stack {
    margin-bottom: var(--ui-row-label-gap-stack);
  }

  .vban-row:last-of-type > :not(.ui-label) {
    margin-bottom: 0;
  }

  .vban-row--volume {
    grid-template-columns: minmax(0, 1fr);
    row-gap: var(--ui-row-label-gap-stack);
  }

}

@container vban-form (max-width: 465px) {
  .save-section {
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 8px;
  }

  .save-status-area {
    grid-column: 1 / -1;
  }

  .test-btn {
    justify-self: end;
  }
}
</style>
