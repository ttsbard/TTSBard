<script setup lang="ts">
import { computed, ref, onBeforeUnmount } from 'vue';
import { Bot, Plus, Trash2, Loader2, RefreshCw, X } from 'lucide-vue-next';
import { confirm } from '@tauri-apps/plugin-dialog';
import ProviderCard from '../shared/ProviderCard.vue';
import PanelEasterEgg from '../shared/PanelEasterEgg.vue';
import TelegramConnectionStatus from './TelegramConnectionStatus.vue';
import type { VoiceCode } from '../../types/settings';
import type { CurrentVoice, Limits } from '../../composables/useTelegramAuth';
import { parseLimitsResetTimestamp, formatLimitCounter } from '../../utils/sileroLimits';
import { useModalFocus } from '../../composables/useModalFocus';
import { t } from '../../i18n';

interface Props {
  active?: boolean;
  expanded?: boolean;
  connected?: boolean;
  telegramStatus?: {
    first_name?: string;
    last_name?: string;
    username?: string;
  } | null;
  currentProxyStatus?: {
    mode: string;
    proxy_url: string | null;
  } | null;
  errorMessage?: string | null;
  reconnecting?: boolean;
  proxyMode?: string;
  proxyModes?: Array<{ value: string; label: string }>;
  currentVoice?: CurrentVoice | null;
  savedVoices?: VoiceCode[];
  voiceLoading?: boolean;
  voiceError?: string | null;
  limits?: Limits | null;
  limitsLoading?: boolean;
  limitsError?: string | null;
  synthesisResponseTimeoutMs?: number;
}

interface Emits {
  (e: 'select'): void;
  (e: 'toggle'): void;
  (e: 'connect'): void;
  (e: 'disconnect'): void;
  (e: 'reconnect'): void;
  (e: 'proxy-mode-change', mode: string): void;
  (e: 'refresh-voice'): void;
  (e: 'add-voice', data: { code: string; description?: string }, callback: (success: boolean, error?: string) => void): void;
  (e: 'remove-voice', id: string): void;
  (e: 'select-voice', id: string): void;
  (e: 'refresh-limits'): void;
  (e: 'timeout-change', timeoutMs: number): void;
}

const props = withDefaults(defineProps<Props>(), {
  active: false,
  expanded: false,
  connected: false,
  reconnecting: false,
  proxyMode: 'none',
  proxyModes: () => [
    { value: 'none', label: '' },
    { value: 'socks5', label: 'SOCKS5' },
    { value: 'mtproxy', label: 'MTProxy' }
  ],
  currentVoice: null,
  savedVoices: () => [],
  voiceLoading: false,
  voiceError: null,
});

const emit = defineEmits<Emits>();

const hasError = computed(() => props.errorMessage !== null);

const limitsVoicesFormatted = computed(() => {
  if (!props.limits) return null
  return formatLimitCounter(props.limits.voices)
})

const limitsResetFormatted = computed(() => {
  if (!props.limits?.reset_timestamp) return null
  const parsed = parseLimitsResetTimestamp(props.limits.reset_timestamp)
  return parsed.formatted
})

const limitsTooltip = computed(() => {
  if (!props.limits?.reset_timestamp) return ''
  return t('tts.silero.limits.reset_tooltip', { timestamp: props.limits.reset_timestamp })
})

const showAddVoiceDialog = ref(false);
const voiceCodeInput = ref('');
const voiceDescriptionInput = ref('');
const isAddingVoice = ref(false);
const addVoiceError = ref<string | null>(null);
const duplicateError = ref<string | null>(null);
const voiceInput = ref<HTMLInputElement | null>(null);
const voiceDialogRef = ref<HTMLElement | null>(null);

// Increments every time the dialog is opened, closed or unmounted. A pending
// add-voice callback only mutates form state while its epoch is still current,
// so a stale response from a previous open cannot corrupt a fresh form.
let addVoiceEpoch = 0;

useModalFocus({
  active: showAddVoiceDialog,
  container: voiceDialogRef,
  initialFocus: () => voiceInput.value,
  closeOnEscape: true,
  onEscape: handleCloseAddVoiceDialog,
})

onBeforeUnmount(() => {
  addVoiceEpoch++
})

function handleOpenAddVoiceDialog() {
  addVoiceEpoch++
  showAddVoiceDialog.value = true;
  voiceCodeInput.value = '';
  voiceDescriptionInput.value = '';
  isAddingVoice.value = false;
  addVoiceError.value = null;
  duplicateError.value = null;
}

function handleCloseAddVoiceDialog() {
  addVoiceEpoch++
  showAddVoiceDialog.value = false;
  voiceCodeInput.value = '';
  voiceDescriptionInput.value = '';
  isAddingVoice.value = false;
  addVoiceError.value = null;
  duplicateError.value = null;
}

async function handleAddVoice() {
  if (isAddingVoice.value || !showAddVoiceDialog.value) return;

  const code = voiceCodeInput.value.trim().toLowerCase();
  if (!code) return;

  // Проверка на дубликаты
  const duplicate = props.savedVoices?.find(v => v.id.toLowerCase() === code);
  if (duplicate) {
    duplicateError.value = t('tts.silero.add_voice.duplicate', { code });
    return;
  }

  isAddingVoice.value = true;
  addVoiceError.value = null;
  duplicateError.value = null;

  const description = voiceDescriptionInput.value.trim() || undefined;
  const epoch = addVoiceEpoch;

  emit('add-voice', { code, description }, (success: boolean, error?: string) => {
    if (epoch !== addVoiceEpoch) return;
    isAddingVoice.value = false;
    if (success) {
      // Успешно добавлено - закрываем диалог
      showAddVoiceDialog.value = false;
      voiceCodeInput.value = '';
      voiceDescriptionInput.value = '';
      addVoiceError.value = null;
      duplicateError.value = null;
    } else {
      // Ошибка - показываем и не закрываем диалог
      addVoiceError.value = error || t('tts.silero.add_voice.error');
    }
  });
}

async function handleRemoveVoice(voiceId: string) {
  const confirmed = await confirm(t('tts.remove_voice.message', { name: voiceId }), {
    title: t('tts.remove_voice.title'),
    kind: 'warning'
  });

  if (!confirmed) return;

  emit('remove-voice', voiceId);
}

function handleSelectVoice(voiceId: string) {
  emit('select-voice', voiceId);
}
</script>

<template>
  <ProviderCard
    class="silero-easter-egg-host"
    title="Silero Bot"
    :icon="Bot"
    :active="active"
    :expanded="expanded"
    :class="{ 'error-state': hasError }"
    @select="$emit('select')"
    @toggle="$emit('toggle')"
  >
    <PanelEasterEgg v-if="expanded" kind="cassette" />
    <TelegramConnectionStatus
      :connected="connected"
      :telegram-status="telegramStatus"
      :current-proxy-status="currentProxyStatus"
      :error-message="errorMessage"
      :reconnecting="reconnecting"
      :proxy-mode="proxyMode"
      :proxy-modes="proxyModes"
      :synthesis-response-timeout-ms="synthesisResponseTimeoutMs"
      @connect="$emit('connect')"
      @disconnect="$emit('disconnect')"
      @reconnect="$emit('reconnect')"
      @proxy-mode-change="$emit('proxy-mode-change', $event)"
      @timeout-change="$emit('timeout-change', $event)"
    />

    <div v-if="connected && limits" class="limits-row">
      <span class="limits-counters ui-label">
        <span class="ui-label ui-label--secondary">{{ t('tts.silero.limits.characters') }}</span>
        {{ limitsVoicesFormatted }}
        <template v-if="limitsResetFormatted">
          · <span :title="limitsTooltip">{{ t('tts.silero.limits.will_reset', { when: limitsResetFormatted }) }}</span>
        </template>
        <template v-else-if="limits.reset_timestamp">
          · <span :title="limitsTooltip">{{ t('tts.silero.limits.will_reset_dash') }}</span>
        </template>
        <span v-if="limitsError" class="limits-stale-cue" :title="t('tts.silero.limits.stale')">⚠</span>
      </span>
      <button
        class="ui-icon-button"
        :disabled="limitsLoading"
        :title="limitsError || t('tts.silero.limits.refresh')"
        :aria-label="limitsError || t('tts.silero.limits.refresh')"
        @click="$emit('refresh-limits')"
      >
        <Loader2 v-if="limitsLoading" :size="14" class="spinner" />
        <RefreshCw v-else :size="14" />
      </button>
    </div>

    <div v-else-if="connected && !limits && !limitsLoading" class="limits-row limits-row-unavailable">
      <span class="limits-counters ui-label"><span class="ui-label ui-label--secondary">{{ t('tts.silero.limits.characters') }}</span> —</span>
      <button
        class="ui-icon-button"
        :title="limitsError || t('tts.silero.limits.load')"
        :aria-label="limitsError || t('tts.silero.limits.load')"
        @click="$emit('refresh-limits')"
      >
        <RefreshCw :size="14" />
      </button>
    </div>

    <div v-else-if="connected && limitsLoading && !limits" class="limits-row limits-row-loading">
      <Loader2 :size="14" class="spinner" />
      <span class="limits-counters ui-label">{{ t('tts.silero.limits.loading') }}</span>
    </div>

    <!-- Voice Management Section (shown when connected) -->
    <div v-if="connected" class="voice-management-section">
      <!-- Saved Voices List -->
      <div class="saved-voices-section">
        <div class="voice-header">
          <label class="ui-label ui-label--secondary">{{ t('tts.voices') }}</label>
          <div class="voice-header-buttons">
            <button
              @click="$emit('refresh-voice')"
              :disabled="voiceLoading"
              class="ui-button ui-button--leading-icon ui-button--compact"
              :title="t('tts.silero.refresh_voice')"
            >
              <Loader2 v-if="voiceLoading" :size="16" class="spinner" />
              <RefreshCw v-else :size="16" />
              <span>{{ t('tts.silero.refresh_voice') }}</span>
            </button>
            <button @click="handleOpenAddVoiceDialog" class="ui-icon-button" :title="t('tts.add_voice')" :aria-label="t('tts.add_voice')">
              <Plus :size="18" />
            </button>
          </div>
        </div>

        <div v-if="savedVoices.length > 0" class="voice-list ui-menu ui-menu--embedded">
          <div
            v-for="voice in savedVoices"
            :key="voice.id"
            :class="['voice-item', { 'ui-menu-item--selected': currentVoice?.id === voice.id }]"
          >
            <button class="voice-info ui-menu-item" @click="handleSelectVoice(voice.id)">
              {{ voice.id }}{{ voice.description ? ` (${voice.description})` : '' }}
            </button>
            <button
              @click.stop="handleRemoveVoice(voice.id)"
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

    <!-- Add Voice Dialog -->
    <Teleport to="body">
      <div v-if="showAddVoiceDialog" class="dialog-overlay" @click.self="handleCloseAddVoiceDialog">
        <div
          ref="voiceDialogRef"
          class="dialog"
          role="dialog"
          aria-modal="true"
          aria-labelledby="silero-add-voice-title"
          tabindex="-1"
        >
          <div class="ui-modal-header">
            <h3 id="silero-add-voice-title" class="ui-section-title">{{ t('tts.add_voice') }}</h3>
            <button
              type="button"
              class="ui-icon-button dialog-close"
              :title="t('common.close')"
              :aria-label="t('common.close')"
              @click="handleCloseAddVoiceDialog"
            >
              <X :size="18" />
            </button>
          </div>
          <div class="ui-modal-body">
            <input
              v-model="voiceCodeInput"
              :placeholder="t('tts.silero.add_voice.code_placeholder')"
              @keyup.enter="handleAddVoice"
              class="ui-input voice-input"
              ref="voiceInput"
              :class="{ 'has-error': duplicateError || addVoiceError }"
            />
            <input
              v-model="voiceDescriptionInput"
              :placeholder="t('tts.silero.add_voice.desc_placeholder')"
              @keyup.enter="handleAddVoice"
              class="ui-input voice-input"
              :class="{ 'has-error': duplicateError || addVoiceError }"
            />
            <!-- Duplicate error -->
            <div v-if="duplicateError" class="dialog-error duplicate-error">
              {{ duplicateError }}
            </div>
            <!-- Bot error -->
            <div v-if="addVoiceError" class="dialog-error bot-error">
              {{ addVoiceError }}
            </div>
            <div class="dialog-buttons">
              <button
                @click="handleAddVoice"
                :disabled="!voiceCodeInput.trim() || isAddingVoice"
                class="ui-button ui-button--primary add-button-confirm"
              >
                <Loader2 v-if="isAddingVoice" :size="16" class="spinner" />
                {{ isAddingVoice ? t('tts.adding') : t('tts.add') }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </Teleport>
  </ProviderCard>
</template>

<style scoped>
.silero-easter-egg-host {
  position: relative;
}

.error-state {
  border-color: var(--card-error-border) !important;
  background: var(--card-error-bg) !important;
}

/* Voice Management Section */
.voice-management-section {
  margin-top: 16px;
  padding-top: 16px;
  border-top: 1px solid var(--color-border);
}

.voice-management-section {
  margin-bottom: 0;
}

.voice-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}

.voice-header-buttons {
  display: flex;
  gap: 8px;
  align-items: center;
}

/* Add Button */
.add-button {
  gap: 6px;
}

/* Voice List */
.voice-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 250px;
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
  overflow-wrap: anywhere;
}

.voice-item.ui-menu-item--selected .voice-info {
  color: inherit;
  font-weight: inherit;
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

/* Dialog */
.dialog-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: 20px;
  box-sizing: border-box;
}

.dialog {
  background: var(--color-bg-panel-strong);
  border-radius: 12px;
  padding: 0;
  width: 100%;
  max-width: 400px;
  max-height: 90vh;
  overflow-y: auto;
  box-sizing: border-box;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.3);
  outline: none;
}

.voice-input {
  width: 100%;
  margin-bottom: 8px;
}

.voice-input.has-error {
  border-color: var(--color-danger);
  box-shadow: 0 0 0 3px var(--status-disconnected-glow);
}

.voice-input.has-error:focus {
  border-color: var(--color-danger);
  box-shadow: 0 0 0 3px var(--status-disconnected-glow);
}

.dialog-error {
  padding: 10px 12px;
  border-radius: 6px;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  margin-bottom: 12px;
  line-height: 1.4;
}

.dialog-error.duplicate-error {
  background: var(--warning-bg-weak);
  color: var(--warning-text);
  border: 1px solid var(--warning-border);
}

.dialog-error.bot-error {
  background: var(--danger-bg-weak);
  color: var(--color-danger);
  border: 1px solid var(--danger-border);
}

.dialog-buttons {
  display: flex;
  gap: 12px;
  justify-content: flex-end;
}

.add-button-confirm {
  gap: 8px;
}

.spinner {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.limits-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  padding: 6px 0;
  gap: 8px;
  overflow: hidden;
}

.limits-counters {
  color: var(--color-text-primary);
  min-width: 0;
  overflow-wrap: anywhere;
}

.limits-stale-cue {
  color: var(--color-warning, #e67e22);
  cursor: help;
  margin-left: 4px;
}

.limits-row-unavailable,
.limits-row-loading,
.limits-row-stale {
  padding: 6px 0;
}

.limits-row-unavailable .limits-counters {
  min-width: 0;
  overflow-wrap: anywhere;
}

.limits-row-loading {
  gap: 8px;
}

.limits-row-loading .limits-counters {
  min-width: 0;
  overflow-wrap: anywhere;
}

</style>
