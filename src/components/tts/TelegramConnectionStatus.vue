<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { LogOut, RefreshCw, Settings } from 'lucide-vue-next';

import { t } from '../../i18n';

interface TelegramStatus {
  first_name?: string;
  last_name?: string;
  username?: string;
}

interface ProxyStatus {
  mode: string;
  proxy_url: string | null;
}

interface Props {
  connected: boolean;
  statusMessage?: string | null;
  userPhone?: string | null;
  reconnecting?: boolean;
  telegramStatus?: TelegramStatus | null;
  currentProxyStatus?: ProxyStatus | null;
  errorMessage?: string | null;
  proxyMode?: string;
  proxyModes?: Array<{ value: string; label: string }>;
  synthesisResponseTimeoutMs?: number;
}

interface Emits {
  (e: 'connect'): void;
  (e: 'disconnect'): void;
  (e: 'reconnect'): void;
  (e: 'proxy-mode-change', mode: string): void;
  (e: 'timeout-change', timeoutMs: number): void;
}

const props = withDefaults(defineProps<Props>(), {
  reconnecting: false,
  proxyMode: 'none',
  proxyModes: () => [
    { value: 'none', label: '' },
    { value: 'socks5', label: 'SOCKS5' },
    { value: 'mtproxy', label: 'MTProxy' }
  ],
  synthesisResponseTimeoutMs: 60000,
});

const emit = defineEmits<Emits>();

const proxyModeLabel = computed(() => {
  if (props.currentProxyStatus) {
    const mode = props.currentProxyStatus.mode;
    if (mode === 'none') return '';
    if (mode === 'socks5') return 'SOCKS5';
    if (mode === 'mtproxy') return 'MTProxy';
  }
  return '';
});

const proxyModeOptions = computed(() =>
  props.proxyModes.map((mode) => ({
    ...mode,
    label: mode.value === 'none' ? t('tts.proxy.none') : mode.label,
  })),
);


const showTimeoutSettings = ref(false);
const timeoutSeconds = ref(Math.max(1, Math.round((props.synthesisResponseTimeoutMs ?? 60000) / 1000)));

watch(() => props.synthesisResponseTimeoutMs, (newMs) => {
  if (typeof newMs === 'number') {
    timeoutSeconds.value = Math.max(1, Math.round(newMs / 1000));
  }
});

function handleTimeoutChange(event: Event) {
  const target = event.target as HTMLInputElement;
  const raw = parseInt(target.value, 10);
  if (isNaN(raw) || raw < 1) {
    const fallback = Math.max(1, Math.round((props.synthesisResponseTimeoutMs ?? 60000) / 1000));
    timeoutSeconds.value = fallback;
    target.value = String(fallback);
    return;
  }
  timeoutSeconds.value = raw;
  emit('timeout-change', raw * 1000);
}

function handleProxyChange(event: Event) {
  const target = event.target as HTMLSelectElement;
  emit('proxy-mode-change', target.value);
}
</script>

<template>
  <div class="telegram-connection-status">
    <!-- Error Banner -->
    <div v-if="errorMessage" class="silero-error-banner">
      <div class="error-banner-content">
        <div class="error-icon">⚠</div>
        <div class="error-text">
          <p class="error-title">{{ t('tts.silero.error.title') }}</p>
          <p class="error-message">{{ errorMessage }}</p>
        </div>
      </div>
      <button class="ui-button fix-button" @click="$emit('connect')">
        {{ t('tts.silero.error.fix') }}
      </button>
    </div>

    <!-- Connection Status -->
    <div class="telegram-status">
      <div v-if="connected" class="status-connected">
        <div class="status-indicator connected"></div>
        <div class="status-info">
          <p class="status-text ui-label">{{ t('tts.silero.connected') }}</p>
          <p v-if="telegramStatus" class="status-details">
            {{ telegramStatus.first_name }} {{ telegramStatus.last_name }}
            <span v-if="telegramStatus.username">@{{ telegramStatus.username }}</span>
          </p>
          <p v-if="proxyModeLabel" class="status-proxy">{{ t('tts.silero.connected_via', { mode: proxyModeLabel }) }}</p>
          <p v-if="currentProxyStatus?.proxy_url" class="status-details">
            {{ currentProxyStatus.proxy_url }}
          </p>
        </div>
        <button class="ui-icon-button ui-action--danger status-signout-button" @click="$emit('disconnect')" :title="t('tts.silero.sign_out')" :aria-label="t('tts.silero.sign_out')">
          <LogOut :size="18" />
        </button>
      </div>
      <div v-else class="status-disconnected">
        <div class="status-indicator disconnected"></div>
        <div class="status-info">
          <p class="status-text ui-label">{{ t('tts.silero.not_connected') }}</p>
          <p class="status-details">{{ t('tts.silero.not_connected_hint') }}</p>
        </div>
      </div>
    </div>

    <!-- Proxy Settings -->
    <div class="setting-group">
      <div class="proxy-settings-row">
        <div class="proxy-select-row">
          <div class="form-field">
            <label class="ui-label">{{ t('tts.proxy') }}</label>
            <select
              :value="proxyMode"
              @change="handleProxyChange"
              class="ui-select network-select"
            >
              <option
                v-for="mode in proxyModeOptions"
                :key="mode.value"
                :value="mode.value"
              >
                {{ mode.label }}
              </option>
            </select>
          </div>
        </div>
        <div class="proxy-actions-row">
          <button
            type="button"
            class="ui-icon-button timeout-gear-button"
            :aria-expanded="showTimeoutSettings"
            :title="t('tts.silero.timeout.toggle')"
            :aria-label="t('tts.silero.timeout.toggle')"
            @click="showTimeoutSettings = !showTimeoutSettings"
          >
            <Settings :size="18" />
          </button>
          <button
            v-if="connected"
            @click="$emit('reconnect')"
            :disabled="reconnecting"
            class="ui-button ui-button--primary ui-button--leading-icon ui-button--compact"
            :title="t('tts.silero.reconnect')"
          >
            <RefreshCw v-if="reconnecting" :size="18" class="spin-icon" />
            <RefreshCw v-else :size="18" />
            {{ reconnecting ? t('tts.silero.reconnecting') : t('tts.silero.reconnect') }}
          </button>
        </div>
      </div>

      <!-- Timeout Settings Drawer -->
      <div v-if="showTimeoutSettings" class="timeout-settings-row">
        <div class="timeout-field-wrap">
          <label class="ui-label" for="silero-timeout-input">
            {{ t('tts.silero.timeout.label') }}
          </label>
          <div class="timeout-input-group">
            <input
              id="silero-timeout-input"
              :value="timeoutSeconds"
              type="number"
              class="ui-input timeout-input"
              min="1"
              step="1"
              @change="handleTimeoutChange"
              @keydown.enter="($event.target as HTMLInputElement).blur()"
            />
            <span class="timeout-unit">{{ t('tts.silero.timeout.unit') }}</span>
          </div>
        </div>
        <p class="ui-hint timeout-hint">{{ t('tts.silero.timeout.hint') }}</p>
      </div>
    </div>

    <!-- Connect Button -->
    <div v-if="!connected" class="setting-group">
      <button
        class="ui-button ui-button--primary telegram-connect-button"
        @click="$emit('connect')"
      >
        {{ t('tts.silero.connect_telegram') }}
      </button>
    </div>

    <!-- Info Section -->
    <div v-if="!connected" class="telegram-info">
      <p class="info-title ui-label">{{ t('tts.silero.info.title') }}</p>
      <ul class="info-list">
        <li>{{ t('tts.silero.info.line1') }}</li>
        <li>{{ t('tts.silero.info.line2_prefix') }}<a href="https://my.telegram.org/apps" target="_blank" rel="noopener noreferrer">my.telegram.org</a></li>
        <li>{{ t('tts.silero.info.line3_prefix') }}<strong>@sileroBot</strong>{{ t('tts.silero.info.line3_suffix') }}</li>
        <li>{{ t('tts.silero.info.line4') }}</li>
      </ul>
    </div>
  </div>
</template>

<style scoped>
.telegram-connection-status {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* Error Banner */
.silero-error-banner {
  padding: 16px;
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border-strong);
  border-left: 4px solid var(--color-danger);
  border-radius: 10px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.error-banner-content {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  flex: 1;
}

.error-icon {
  font-size: 20px;
  line-height: 1;
  flex-shrink: 0;
}

.error-text {
  flex: 1;
}

.error-title {
  margin: 0 0 4px;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--danger-text-bright);
}

.error-message {
  margin: 0;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--danger-text-weak);
}

.fix-button {
  background: var(--danger-bg-hover);
  color: var(--color-text-white);
  border: none;
  white-space: nowrap;
}

.fix-button:hover:not(:disabled) {
  background: var(--danger-border-strong);
}

/* Telegram Status */
.telegram-status {
  padding: 16px;
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  margin-top: 8px;
}

.status-connected,
.status-disconnected {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.status-indicator {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  flex-shrink: 0;
}

.status-indicator.connected {
  background: var(--status-connected);
  box-shadow: 0 0 0 3px var(--status-connected-glow);
}

.status-indicator.disconnected {
  background: var(--status-disconnected);
  box-shadow: 0 0 0 3px var(--status-disconnected-glow);
}

.status-info {
  flex: 1;
}

.status-text {
  margin: 0;
  color: var(--color-text-primary);
}

.status-details {
  margin: 4px 0 0;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--color-text-secondary);
}

.status-proxy {
  margin: 2px 0 0;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--color-accent);
}

/* Setting group */
.setting-group {
  margin-top: 8px;
}

/* Proxy settings row */
.proxy-settings-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 8px;
}

.proxy-select-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.proxy-select-row .form-field {
  display: flex;
  align-items: center;
  gap: 10px;
}

.proxy-select-row .ui-label {
  min-width: fit-content;
  color: var(--color-text-secondary);
}

.network-select {
  width: fit-content;
  min-width: 100px;
  min-height: 32px;
  padding-top: 6px;
  padding-bottom: 6px;
}

.spin-icon {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

/* Connect button */
.telegram-connect-button {
  width: 100%;
}

/* Info section */
.telegram-info {
  padding: 16px;
  background: var(--info-bg-weak);
  border-left: 4px solid var(--color-accent);
  border-radius: 10px;
}

.info-title {
  margin: 0 0 8px;
  color: var(--color-text-primary);
}

.info-list {
  margin: 0;
  padding-left: 20px;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--color-text-secondary);
  line-height: 1.6;
}

.info-list li {
  margin-bottom: 4px;
}

.info-list li:last-child {
  margin-bottom: 0;
}

.info-list a {
  color: var(--color-info);
  text-decoration: none;
  font-weight: 500;
}

.info-list a:hover {
  text-decoration: underline;
}

.info-list strong {
  color: var(--color-text-primary);
}

.proxy-actions-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
}

.timeout-settings-row {
  margin-top: 4px;
  padding: 10px 12px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--ui-radius-control, 8px);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.timeout-field-wrap {
  display: flex;
  align-items: center;
  gap: 12px;
}

.timeout-input-group {
  display: flex;
  align-items: center;
  gap: 6px;
}

.timeout-input {
  width: 68px;
  min-height: 32px;
  padding-top: 6px;
  padding-bottom: 6px;
  text-align: right;
  padding-right: 8px;
}

.timeout-unit {
  font-size: var(--ui-text-size-control, 0.9rem);
  color: var(--color-text-secondary);
}

.timeout-hint {
  margin: 0;
  font-size: 0.85rem;
  color: var(--color-text-muted);
}

</style>
