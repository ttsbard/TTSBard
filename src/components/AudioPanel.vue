<script setup lang="ts">
import { computed, ref } from 'vue';
import { Volume2, Sliders, Network, RefreshCw, Loader2 } from 'lucide-vue-next';
import { t } from '../i18n';
import { useAudioSettings } from '../composables/useAppSettings';
import { useVbanHealth, type VbanHealth } from '../composables/useVbanHealth';
import AudioDevicesTab from './audio/AudioDevicesTab.vue';
import AudioEffectsTab from './audio/AudioEffectsTab.vue';
import AudioVbanTab from './audio/AudioVbanTab.vue';

const activeTab = ref<'devices' | 'effects_dsp' | 'vban'>('devices');
const effectsDirty = ref(false);
const vbanDirty = ref(false);
const devicesTab = ref<InstanceType<typeof AudioDevicesTab> | null>(null);

const audioSettings = useAudioSettings();
const savedVbanEnabled = computed(() => audioSettings.value?.vban?.enabled === true);

// Receiver availability verdict from the global monitor (App owns the
// listener); this panel only renders it. Local send diagnostics stay in the
// VBAN tab body and are never mixed into this indicator.
const { health } = useVbanHealth();

/**
 * Availability text for the title/accessible name: it describes the ping
 * service, never a confirmed audio playback on the receiver.
 */
function vbanHealthText(value: VbanHealth): string {
  switch (value.state) {
    case 'alive': return t('audio.vban.health.alive');
    case 'failed':
      if (value.reason === 'no_reply') return t('audio.vban.health.failed.no_reply');
      if (value.reason === 'local_socket') return t('audio.vban.health.failed.local_socket');
      return t('audio.vban.health.failed');
    case 'unknown':
    default: return t('audio.vban.health.unknown');
  }
}

const vbanHealthLabel = computed(() => vbanHealthText(health.value));

/** Keep the availability in the tab's accessible name; the dot is visual. */
const vbanTabAriaLabel = computed(() => {
  const base = vbanDirty.value ? t('audio.tabs.vban.dirty') : t('audio.tabs.vban');
  return savedVbanEnabled.value ? `${base} — ${vbanHealthLabel.value}` : base;
});

function onEffectsDirty(dirty: boolean) {
  effectsDirty.value = dirty;
}

function onVbanDirty(dirty: boolean) {
  vbanDirty.value = dirty;
}
</script>

<template>
  <div class="audio-panel" :class="{ 'audio-panel--effects': activeTab === 'effects_dsp' }">
    <div class="audio-panel-inner">
      <div class="audio-header">
        <div class="audio-tabs ui-tabs" role="tablist">
          <button
            class="ui-tab"
            role="tab"
            :aria-selected="activeTab === 'devices'"
            @click="activeTab = 'devices'"
          >
            <Volume2 :size="18" />
            <span>{{ t('audio.tabs.devices') }}</span>
          </button>
          <button
            class="ui-tab"
            role="tab"
            :aria-selected="activeTab === 'effects_dsp'"
            :aria-label="effectsDirty ? t('audio.tabs.effects_dsp.dirty') : t('audio.tabs.effects_dsp')"
            @click="activeTab = 'effects_dsp'"
          >
            <Sliders :size="18" />
            <span>{{ t('audio.tabs.effects_dsp') }}</span>
            <span v-if="effectsDirty" class="dirty-dot" aria-hidden="true">*</span>
          </button>
          <button
            class="ui-tab"
            role="tab"
            :aria-selected="activeTab === 'vban'"
            :aria-label="vbanTabAriaLabel"
            :title="savedVbanEnabled ? vbanHealthLabel : undefined"
            @click="activeTab = 'vban'"
          >
            <Network :size="18" />
            <span class="vban-tab-label">
              <span>{{ t('audio.tabs.vban') }}</span>
              <span
                v-if="savedVbanEnabled"
                class="vban-tab-dot"
                :class="`vban-tab-dot--${health.state}`"
                aria-hidden="true"
              ></span>
            </span>
            <span v-if="vbanDirty" class="dirty-dot" aria-hidden="true">*</span>
          </button>
        </div>
        <button
          v-if="activeTab === 'devices'"
          type="button"
          class="ui-icon-button refresh-btn"
          :disabled="!devicesTab || devicesTab.isRefreshing"
          :title="t('audio.refresh_devices')"
          :aria-label="t('audio.refresh_devices')"
          @click="devicesTab?.refreshData()"
        >
          <Loader2 v-if="devicesTab?.isRefreshing" :size="18" class="ui-spin" />
          <RefreshCw v-else :size="18" />
        </button>
      </div>

      <AudioDevicesTab v-if="activeTab === 'devices'" ref="devicesTab" />

      <AudioEffectsTab
        v-show="activeTab === 'effects_dsp'"
        @dirty-change="onEffectsDirty"
      />

      <AudioVbanTab
        v-show="activeTab === 'vban'"
        :active="activeTab === 'vban'"
        @dirty-change="onVbanDirty"
      />
    </div>
  </div>
</template>

<style>
/* Global block: legacy style owner for the not-yet-migrated DSP surfaces
   (DspSettings, EffectsSettings, Eq/Compressor/Limiter) and Hotkeys/Intercept
   fallbacks. AudioDevicesTab no longer consumes these names; consumers move
   off stage by stage. */
.setting-section {
  padding: 12px 16px;
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
}

.section-header {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--color-border);
}

.section-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.section-title {
  flex: 1;
  font-weight: 600;
  font-size: 1.1rem;
  color: var(--color-text-primary);
}

.toggle-buttons {
  display: flex;
  gap: 4px;
}

.toggle-btn {
  padding: 6px 12px;
  border: 1px solid var(--color-border);
  background: var(--color-bg-field);
  color: var(--color-text-secondary);
  border-radius: 8px;
  cursor: pointer;
  font-size: 12px;
  transition: all 0.2s;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-family: inherit;
}

.toggle-btn:hover {
  background: var(--color-bg-field-hover);
}

.toggle-btn.active {
  background: var(--btn-accent-bg);
  border-color: var(--color-accent);
  color: var(--color-text-primary);
}

.setting-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
  flex-wrap: nowrap;
  min-width: 0;
  overflow: hidden;
}

.setting-row:last-child {
  margin-bottom: 0;
}

.setting-row.disabled {
  opacity: 0.5;
  pointer-events: none;
}

.setting-row label {
  min-width: 100px;
  font-size: 14px;
  color: var(--color-text-secondary);
  font-weight: 500;
}

.setting-row select {
  flex: 1;
  padding: 8px 12px;
  border: 1px solid var(--color-border-strong);
  border-radius: 10px;
  background: var(--input-bg-strong);
  color: var(--color-text-primary);
  font-size: 14px;
  cursor: pointer;
  transition: all 0.15s ease;
}

.setting-row select:hover {
  background: var(--color-bg-field-hover);
  border-color: var(--color-border-strong);
}

.setting-row select:disabled {
  background: var(--color-border-weak);
  cursor: not-allowed;
}

.setting-row select:focus {
  outline: none;
  border-color: var(--color-accent);
  box-shadow: 0 0 0 3px var(--color-accent-glow);
}

.volume-control {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 12px;
}

.volume-control input[type="range"] {
  flex: 1;
  height: 6px;
  -webkit-appearance: none;
  background: var(--range-bg);
  border-radius: 3px;
  outline: none;
}

.volume-control input[type="range"]::-webkit-slider-thumb {
  -webkit-appearance: none;
  width: 16px;
  height: 16px;
  background: var(--color-accent);
  border-radius: 50%;
  cursor: pointer;
}

.volume-value {
  min-width: 45px;
  text-align: right;
  font-size: 14px;
  color: var(--color-text-secondary);
  font-weight: 500;
}

@keyframes spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}
</style>

<style scoped>
.audio-panel {
  max-width: 900px;
  margin: 0 auto;
  min-height: 100%;
  display: flex;
  flex-direction: column;
}

.audio-panel-inner {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}

.audio-panel--effects {
  height: 100%;
  min-height: 0;
}

/* Keep tabs in normal flow: cards must not scroll beneath a transparent band. */
.audio-header {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  margin-bottom: var(--ui-section-gap);
  padding-bottom: 8px;
  border-bottom: 1px solid var(--color-border);
}

.audio-panel:not(.audio-panel--effects) .audio-header {
  margin-bottom: 8px;
}

.audio-tabs {
  flex: 1;
  min-width: 0;
  margin-bottom: 0;
  padding-bottom: 0;
  border-bottom: none;
}

.refresh-btn {
  flex-shrink: 0;
}

.dirty-dot {
  color: var(--warning-text-bright);
  font-size: 18px;
  font-weight: 700;
  line-height: 1;
  margin-left: 2px;
  flex-shrink: 0;
}

/* Receiver availability in the VBAN tab; shown only for a saved enabled
   output. Grey unknown / green alive / red failed; local send diagnostics are
   not part of this indicator. */
.vban-tab-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.vban-tab-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
  background: var(--color-text-muted);
}

.vban-tab-dot--unknown {
  background: var(--color-text-muted);
}

.vban-tab-dot--alive {
  background: var(--color-success);
}

.vban-tab-dot--failed {
  background: var(--color-danger);
}

</style>
