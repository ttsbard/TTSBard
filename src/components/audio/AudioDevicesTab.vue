<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { Loader, Volume2, VolumeX, Mic, Info, Play } from 'lucide-vue-next';
import { useAudioSettings, useAppSettings } from '../../composables/useAppSettings';
import { debugLog, debugError } from '../../utils/debug';
import { t } from '../../i18n';
import { presentCommandError } from '../../ipc/commandError';

interface DeviceInfo {
  id: string;
  name: string;
  is_default: boolean;
}

const audioSettingsFromComposable = useAudioSettings();

const outputDevices = ref<DeviceInfo[]>([]);
const virtualMicDevices = ref<DeviceInfo[]>([]);
const audioSettings = ref({
  speaker_device: null as string | null,
  speaker_enabled: true,
  speaker_volume: 80,
  virtual_mic_device: null as string | null,
  virtual_mic_enabled: true,
  virtual_mic_volume: 100,
});

const isLoading = ref(false);
const isRefreshing = ref(false);
const isTestingSpeaker = ref(false);
const isTestingVirtualMic = ref(false);
const errorMessage = ref('');
const isDataLoaded = ref(false);
const { reload: reloadSettings } = useAppSettings();
const outputFormat = computed(() => audioSettingsFromComposable.value?.output_format ?? '');
const isSettingFormat = ref(false);
const isFormatPending = ref(false);
let pendingTimer: ReturnType<typeof setInterval> | undefined;
let disposed = false;
let pendingRequest = false;

async function refreshFormatPending() {
  if (disposed || pendingRequest) return;
  pendingRequest = true;
  const formatAtRequest = outputFormat.value;
  try {
    const pending = await invoke<boolean>('get_audio_output_format_pending');
    if (!disposed && outputFormat.value === formatAtRequest) isFormatPending.value = pending;
  } catch (error) {
    if (!disposed) debugError('Failed to read output format status:', error);
  } finally {
    pendingRequest = false;
  }
}

async function setOutputFormat(format: string) {
  if (isSettingFormat.value || (format !== 'default' && format !== 'i32')) return;
  isSettingFormat.value = true;
  errorMessage.value = '';
  try {
    await invoke('set_audio_output_format', { format });
    await reloadSettings();
    await refreshFormatPending();
  } catch (error) {
    if (!disposed) errorMessage.value = presentCommandError(error, t('audio.error.output_format'));
  } finally {
    if (!disposed) isSettingFormat.value = false;
  }
}

watch(outputFormat, () => { isFormatPending.value = false; void refreshFormatPending(); });
onUnmounted(() => {
  disposed = true;
  if (pendingTimer !== undefined) clearInterval(pendingTimer);
});

const selectedVirtualMicDevice = ref<string | null>(null);

async function loadDevices(force = false) {
  if (isDataLoaded.value && !force) {
    return;
  }

  try {
    const [outputs, virtuals] = await Promise.all([
      invoke<DeviceInfo[]>('get_output_devices'),
      invoke<DeviceInfo[]>('get_virtual_mic_devices'),
    ]);
    outputDevices.value = outputs;
    virtualMicDevices.value = virtuals;
    isDataLoaded.value = true;
  } catch (error) {
    debugError('Failed to load devices:', error);
    errorMessage.value = presentCommandError(error, t('audio.error.load_devices'));
  }
}

async function refreshData() {
  if (isRefreshing.value) return;
  isRefreshing.value = true;
  errorMessage.value = '';
  try {
    await loadDevices(true);
    await refreshFormatPending();
  } finally {
    isRefreshing.value = false;
  }
}

defineExpose({ refreshData, isRefreshing });

async function setSpeakerDevice(deviceId: string | null) {
  try {
    await invoke('set_speaker_device', { deviceId });
    audioSettings.value.speaker_device = deviceId;
  } catch (error) {
    debugError('Failed to set speaker device:', error);
    errorMessage.value = presentCommandError(error, t('audio.error.set_speaker_device'));
  }
}

async function setSpeakerEnabled(enabled: boolean) {
  try {
    await invoke('set_speaker_enabled', { enabled });
    audioSettings.value.speaker_enabled = enabled;
  } catch (error) {
    debugError('Failed to set speaker enabled:', error);
    errorMessage.value = presentCommandError(error, t('audio.error.set_speaker_enabled'));
  }
}

async function setSpeakerVolume(volume: number) {
  try {
    await invoke('set_speaker_volume', { volume });
    audioSettings.value.speaker_volume = volume;
  } catch (error) {
    debugError('Failed to set speaker volume:', error);
    errorMessage.value = presentCommandError(error, t('audio.error.set_speaker_volume'));
  }
}

async function setVirtualMicDevice(deviceId: string | null) {
  try {
    await invoke('set_virtual_mic_device', { deviceId });
    selectedVirtualMicDevice.value = deviceId;
    audioSettings.value.virtual_mic_device = deviceId;
  } catch (error) {
    debugError('Failed to set virtual mic device:', error);
    errorMessage.value = presentCommandError(error, t('audio.error.set_virtual_mic_device'));
  }
}

async function enableVirtualMic() {
  try {
    let deviceId = selectedVirtualMicDevice.value;

    if (!deviceId && virtualMicDevices.value.length > 0) {
      deviceId = virtualMicDevices.value[0].id;
    }

    if (!deviceId) {
      errorMessage.value = t('audio.error.no_virtual_mic');
      return;
    }

    await invoke('set_virtual_mic_device', { deviceId });
    selectedVirtualMicDevice.value = deviceId;
    audioSettings.value.virtual_mic_device = deviceId;
    await setVirtualMicEnabled(true);
  } catch (error) {
    debugError('Failed to enable virtual mic:', error);
    errorMessage.value = presentCommandError(error, t('audio.error.set_virtual_mic_device'));
  }
}

// ROADMAP-142: the switch keeps the selected device and volume, so enabling
// restores the output with its previous hardware settings.
async function disableVirtualMic() {
  await setVirtualMicEnabled(false);
}

async function setVirtualMicEnabled(enabled: boolean) {
  try {
    await invoke('set_virtual_mic_enabled', { enabled });
    audioSettings.value.virtual_mic_enabled = enabled;
  } catch (error) {
    debugError('Failed to set virtual mic enabled:', error);
    errorMessage.value = presentCommandError(error, t('audio.error.set_virtual_mic_enabled'));
  }
}

async function setVirtualMicVolume(volume: number) {
  try {
    await invoke('set_virtual_mic_volume', { volume });
    audioSettings.value.virtual_mic_volume = volume;
  } catch (error) {
    debugError('Failed to set virtual mic volume:', error);
    errorMessage.value = presentCommandError(error, t('audio.error.set_virtual_mic_volume'));
  }
}

async function testSpeaker() {
  if (isTestingSpeaker.value) return;

  isTestingSpeaker.value = true;
  try {
    await invoke('test_audio_device', {
      deviceId: audioSettings.value.speaker_device,
      volume: audioSettings.value.speaker_volume,
    });
  } catch (error) {
    debugError('Failed to test speaker:', error);
    errorMessage.value = presentCommandError(error, t('audio.error.test_speaker'));
  } finally {
    isTestingSpeaker.value = false;
  }
}

async function testVirtualMic() {
  if (isTestingVirtualMic.value) return;

  isTestingVirtualMic.value = true;
  try {
    await invoke('test_audio_device', {
      deviceId: audioSettings.value.virtual_mic_device,
      volume: audioSettings.value.virtual_mic_volume,
    });
  } catch (error) {
    debugError('Failed to test virtual mic:', error);
    errorMessage.value = presentCommandError(error, t('audio.error.test_virtual_mic'));
  } finally {
    isTestingVirtualMic.value = false;
  }
}

function getDeviceDisplayName(device: DeviceInfo): string {
  if (device.is_default) {
    return `${device.name}${t('audio.default_device.suffix')}`;
  }
  return device.name;
}

onMounted(async () => {
  pendingTimer = setInterval(() => { void refreshFormatPending(); }, 1000);
  isLoading.value = true;
  try {
    await loadDevices();
    await refreshFormatPending();
  } finally {
    isLoading.value = false;
  }
});

watch(audioSettingsFromComposable, (newSettings) => {
  if (!newSettings) return;

  debugLog('[AudioDevicesTab] Settings updated from composable');

  if (selectedVirtualMicDevice.value === null && newSettings.virtual_mic_device) {
    selectedVirtualMicDevice.value = newSettings.virtual_mic_device;
  }

  audioSettings.value = {
    speaker_device: newSettings.speaker_device || null,
    speaker_enabled: newSettings.speaker_enabled,
    speaker_volume: newSettings.speaker_volume,
    virtual_mic_device: audioSettings.value.virtual_mic_device ?? newSettings.virtual_mic_device ?? null,
    virtual_mic_enabled: newSettings.virtual_mic_enabled,
    virtual_mic_volume: newSettings.virtual_mic_volume,
  };
}, { immediate: true });
</script>

<template>
  <div>
    <div v-if="errorMessage" class="error-box">
      {{ errorMessage }}
      <button @click="errorMessage = ''" class="close-btn" :aria-label="t('audio.close')" :title="t('audio.close')">&times;</button>
    </div>

    <div v-if="isLoading" class="loading">
      {{ t('audio.loading') }}
    </div>

    <div v-else class="audio-settings">
      <div class="devices-card ui-section">
        <div class="devices-header">
          <Volume2 class="section-icon" :size="20" />
          <span class="ui-section-title section-title">{{ t('audio.speaker.title') }}</span>
          <div class="toggle-buttons">
            <button
              @click="setSpeakerEnabled(true)"
              :class="{ active: audioSettings.speaker_enabled }"
              class="toggle-btn"
            >
              <Volume2 :size="14" /> {{ t('audio.on') }}
            </button>
            <button
              @click="setSpeakerEnabled(false)"
              :class="{ active: !audioSettings.speaker_enabled }"
              class="toggle-btn"
            >
              <VolumeX :size="14" /> {{ t('audio.off') }}
            </button>
          </div>
        </div>

        <div class="devices-row" :class="{ disabled: !audioSettings.speaker_enabled }">
          <label class="ui-label">{{ t('audio.device') }}</label>
          <div class="input-with-action">
            <select
              class="ui-select"
              :disabled="!audioSettings.speaker_enabled"
              @change="setSpeakerDevice(($event.target as HTMLSelectElement).value || null)"
            >
              <option value="">{{ t('audio.device.default') }}</option>
              <option
                v-for="device in outputDevices"
                :key="device.id"
                :value="device.id"
                :selected="audioSettings.speaker_device === device.id"
              >
                {{ getDeviceDisplayName(device) }}
              </option>
            </select>
            <button
              @click="testSpeaker"
              :disabled="!audioSettings.speaker_enabled || isTestingSpeaker || isSettingFormat"
              class="ui-icon-button ui-icon-button--adjacent test-btn"
              :title="t('audio.test_playback')"
              :aria-label="t('audio.test_playback')"
            >
              <Loader v-if="isTestingSpeaker" :size="16" class="spinner" />
              <Play v-else :size="18" />
            </button>
          </div>
        </div>

        <div class="devices-row" :class="{ disabled: !audioSettings.speaker_enabled }">
          <label class="ui-label">{{ t('audio.volume') }}</label>
          <div class="volume-control">
            <input
              type="range"
              min="0"
              max="100"
              :value="audioSettings.speaker_volume"
              @input="setSpeakerVolume(($event.target as HTMLInputElement).valueAsNumber)"
              :disabled="!audioSettings.speaker_enabled"
            />
            <span class="volume-value">{{ audioSettings.speaker_volume }}%</span>
          </div>
        </div>
      </div>

      <div class="devices-card ui-section">
        <div class="devices-header">
          <Mic class="section-icon" :size="20" />
          <span class="ui-section-title section-title">{{ t('audio.mic.title') }}</span>
          <div class="toggle-buttons">
            <button
              @click="enableVirtualMic()"
              :class="{ active: audioSettings.virtual_mic_enabled }"
              class="toggle-btn"
            >
              <Mic :size="14" /> {{ t('audio.on') }}
            </button>
            <button
              @click="disableVirtualMic()"
              :class="{ active: !audioSettings.virtual_mic_enabled }"
              class="toggle-btn"
            >
              <Mic :size="14" /> {{ t('audio.off') }}
            </button>
          </div>
        </div>

        <div class="devices-row" :class="{ disabled: !audioSettings.virtual_mic_enabled }">
          <label class="ui-label">{{ t('audio.device') }}</label>
          <div class="input-with-action">
            <select
              class="ui-select"
              :disabled="!audioSettings.virtual_mic_enabled"
              @change="setVirtualMicDevice(($event.target as HTMLSelectElement).value || null)"
            >
              <option value="">{{ t('audio.mic.none') }}</option>
              <option
                v-for="device in virtualMicDevices"
                :key="device.id"
                :value="device.id"
                :selected="selectedVirtualMicDevice === device.id"
              >
                {{ device.name }}
              </option>
            </select>
            <button
              @click="testVirtualMic"
              :disabled="!audioSettings.virtual_mic_enabled || !audioSettings.virtual_mic_device || isTestingVirtualMic || isSettingFormat"
              class="ui-icon-button ui-icon-button--adjacent test-btn"
              :title="t('audio.test_playback')"
              :aria-label="t('audio.test_playback')"
            >
              <Loader v-if="isTestingVirtualMic" :size="16" class="spinner" />
              <Play v-else :size="18" />
            </button>
          </div>
        </div>

        <div class="devices-row" :class="{ disabled: !audioSettings.virtual_mic_enabled || !audioSettings.virtual_mic_device }">
          <label class="ui-label">{{ t('audio.volume') }}</label>
          <div class="volume-control">
            <input
              type="range"
              min="0"
              max="100"
              :value="audioSettings.virtual_mic_volume"
              @input="setVirtualMicVolume(($event.target as HTMLInputElement).valueAsNumber)"
              :disabled="!audioSettings.virtual_mic_enabled || !audioSettings.virtual_mic_device"
            />
            <span class="volume-value">{{ audioSettings.virtual_mic_volume }}%</span>
          </div>
        </div>

        <div v-if="virtualMicDevices.length === 0" class="info-box">
          <Info :size="16" /> {{ t('audio.mic.not_found_info') }}
        </div>
      </div>
      <div class="devices-card ui-section output-format-section">
        <div class="devices-row output-format-row">
          <label class="ui-label" for="audio-output-format">{{ t('audio.output_format.label') }}</label>
          <div class="input-with-action">
            <select id="audio-output-format" class="ui-select" :value="outputFormat"
              :disabled="!audioSettingsFromComposable || isSettingFormat || isRefreshing || isTestingSpeaker || isTestingVirtualMic"
              @change="setOutputFormat(($event.target as HTMLSelectElement).value)">
              <option v-if="!outputFormat" value="" disabled>{{ t('audio.loading') }}</option>
              <option value="default">{{ t('audio.output_format.default') }}</option>
              <option value="i32">{{ t('audio.output_format.i32') }}</option>
            </select>
          </div>
        </div>
        <p class="format-hint">{{ t('audio.output_format.hint') }}</p>
        <p v-if="isFormatPending" class="format-hint" role="status">{{ t('audio.output_format.pending') }}</p>
      </div>
    </div>


  </div>
</template>

<style scoped>
.format-hint { margin: var(--ui-hint-gap) 0 0; font-size: var(--ui-text-size-hint); font-weight: var(--ui-text-weight-hint); line-height: 1.5; color: var(--color-text-secondary); overflow-wrap: anywhere; }
.output-format-section { min-width: 0; }
.output-format-row { flex-wrap: wrap; gap: 8px; }
.output-format-row .input-with-action { flex: 1 1 220px; }
.output-format-row .ui-select { width: 100%; }

.error-box {
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border-strong);
  border-radius: 12px;
  padding: 12px;
  margin-bottom: 16px;
  color: var(--danger-text-weak);
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.close-btn {
  background: none;
  border: none;
  font-size: 20px;
  cursor: pointer;
  color: inherit;
}

.loading {
  text-align: center;
  padding: 40px;
  color: var(--color-text-secondary);
}

.audio-settings {
  display: flex;
  flex-direction: column;
}

/* Card skin stays local; padding/rhythm come from ui-section. */
.devices-card {
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
}

/* Header row with icon + title + segmented toggles. */
.devices-header {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--color-border);
}

.devices-header .section-title {
  flex: 1;
}

/* Segmented On/Off: maps to the agreed audio preset role (0.8rem/500,
   padding 6x14, radius 8); no separate 12px scale step is kept. */
.devices-header .toggle-btn {
  padding: 6px 14px;
  border: 1px solid var(--color-border);
  background: var(--color-bg-field);
  color: var(--color-text-secondary);
  border-radius: 8px;
  cursor: pointer;
  font-size: 0.8rem;
  font-weight: 500;
  transition: all 0.2s;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-family: inherit;
}

.devices-header .toggle-btn:hover {
  background: var(--color-bg-field-hover);
}

.devices-header .toggle-btn.active {
  background: var(--btn-accent-bg);
  border-color: var(--color-accent);
  color: var(--color-text-primary);
}

.devices-header .toggle-buttons {
  display: flex;
  gap: 4px;
}

/* Local row: leaves the global .setting-row cascade (overflow:hidden and
   14px select typography) owned by AudioPanel for the DSP surfaces. */
.devices-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: var(--ui-row-gap);
  min-width: 0;
}

.devices-row:last-child {
  margin-bottom: 0;
}

.devices-row.disabled {
  opacity: 0.5;
  pointer-events: none;
}

.devices-row > .ui-label {
  min-width: 100px;
  color: var(--color-text-secondary);
}

.input-with-action {
  display: flex;
  gap: 8px;
  flex: 1;
  align-items: center;
  min-width: 0;
  overflow: hidden;
}

.input-with-action select {
  flex: 1;
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.test-btn {
  flex-shrink: 0;
}

.test-btn .spinner {
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

.info-box {
  background: var(--info-bg-weak);
  border: 1px solid var(--info-border);
  border-radius: 8px;
  padding: 12px;
  margin-top: 12px;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--color-info);
  display: flex;
  align-items: center;
  gap: 8px;
}

.panel-footer {
  display: flex;
  justify-content: center;
  margin-top: 1.5rem;
}

</style>
