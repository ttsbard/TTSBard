<script setup lang="ts">
import { computed } from 'vue'
import { Copy, AlertTriangle, Play, Square, Info, RefreshCw, RotateCw } from 'lucide-vue-next'
import { useInputServer } from '../composables/useInputServer'
import { t } from '../i18n'
import InputWithToggle from './shared/InputWithToggle.vue'

const {
  settings,
  status,
  loading,
  message,
  messageType,
  testText,
  testResult,
  testError,
  testPending,
  operationPending,
  awaitingRestart,
  awaitingStart,
  isPortValid,
  isRunning,
  isStartingOrRunning,
  statusLabel,
  statusError,
  overlayUrl,
  lanUrl,
  accessToken,
  tokenAvailable,
  regeneratePending,
  saveSettings,
  startInputServer,
  stopInputServer,
  restartInputServer,
  sendTest,
  copyOverlayUrl,
  copyLanUrl,
  copyToken,
  regenerateToken,
} = useInputServer()

const messageBoxClass = computed(() => (message.value ? messageType.value : ''))

// Управление блокируется не только по runtime-статусу, но и на всю
// пользовательскую операцию: промежуточный stopped перезапуска не открывает
// запуск, нажатая кнопка держит роль до фактического завершения.
const serverBusy = computed(() => status.value.state === 'starting')
const controlsLocked = computed(() => operationPending.value || serverBusy.value)
const showRunControls = computed(
  () => isStartingOrRunning.value || awaitingRestart.value,
)
const fieldsLocked = computed(() => isStartingOrRunning.value || controlsLocked.value)
const statusText = computed(() => {
  if (awaitingRestart.value) return t('input_server.status.restarting')
  if (awaitingStart.value) return t('input_server.status.starting')
  return statusLabel.value
})
</script>

<template>
  <div class="input-server-panel">
    <div v-if="message" class="message-box ui-status" :class="messageBoxClass">
      {{ message }}
    </div>

    <section class="settings-section ui-section">
      <div class="section-header server-header">
        <h2 class="ui-section-title">{{ t('input_server.server') }}</h2>
        <div class="server-status">
          <span
            class="status-indicator ui-status"
            :class="{
              running: isRunning,
              starting: status.state === 'starting',
              error: status.state === 'error' && !controlsLocked,
            }"
          >
            {{ statusText }}
          </span>
          <template v-if="showRunControls">
            <button
              class="status-button restart ui-icon-button ui-icon-button--accent"
              :disabled="controlsLocked"
              :class="{ disabled: controlsLocked }"
              :title="t('input_server.restart')"
              :aria-label="t('input_server.restart')"
              @click="restartInputServer"
            >
              <RotateCw :size="18" />
            </button>
            <button
              class="status-button stop ui-icon-button ui-action--stop"
              :disabled="controlsLocked"
              :class="{ disabled: controlsLocked }"
              :title="t('input_server.stop')"
              :aria-label="t('input_server.stop')"
              @click="stopInputServer"
            >
              <Square :size="18" />
            </button>
          </template>
          <template v-else>
            <button
              class="status-button start ui-icon-button ui-icon-button--accent"
              :class="{ disabled: !isPortValid }"
              :disabled="!isPortValid || operationPending"
              :title="t('input_server.start')"
              :aria-label="t('input_server.start')"
              @click="startInputServer"
            >
              <Play :size="18" />
            </button>
            <button
              class="status-button stop ui-icon-button ui-action--stop"
              :class="{ disabled: status.state !== 'error' || operationPending }"
              :title="t('input_server.stop')"
              :aria-label="t('input_server.stop')"
              :disabled="status.state !== 'error' || operationPending"
              @click="stopInputServer"
            >
              <Square :size="18" />
            </button>
          </template>
        </div>
      </div>

      <div v-if="statusError" class="status-error-banner ui-status">
        <AlertTriangle :size="14" />
        <span>{{ statusError }}</span>
      </div>

      <div class="ui-row">
        <label class="ui-choice-label">
          <input
            type="checkbox"
            v-model="settings.start_on_boot"
            :disabled="loading"
            @change="saveSettings"
            class="ui-choice-input"
          />
          <span>{{ t('input_server.start_on_boot') }}</span>
        </label>
      </div>

      <div class="ui-row port-setting-row">
        <label class="ui-label">{{ t('input_server.port') }}</label>
        <div class="address-inputs ui-field-group">
          <input
            type="number"
            v-model.number="settings.port"
            min="1024"
            max="65535"
            step="1"
            inputmode="numeric"
            class="ui-input address-port"
            :aria-invalid="!isPortValid ? 'true' : undefined"
            :disabled="fieldsLocked"
            placeholder="10101"
          />
          <button class="save-button-inline ui-button ui-button--primary" :disabled="loading || operationPending" @click="saveSettings">
            {{ t('common.save') }}
          </button>
        </div>
        <span v-if="!isPortValid" class="error-text ui-status">{{ t('input_server.port_error') }}</span>
      </div>
    </section>

    <div class="info-callout ui-status">
      <Info :size="16" class="info-icon" />
      <span>
        {{ t('input_server.info_callout') }}
      </span>
    </div>

    <section class="settings-section ui-section">
      <h2 class="ui-section-title">{{ t('input_server.connection') }}</h2>

      <h3 class="subsection-heading ui-group-title">{{ t('input_server.web_form') }}</h3>

      <div class="connection-grid">
        <div class="connection-row">
          <label class="row-label ui-label ui-label--secondary">{{ t('input_server.local_url_label') }}</label>
          <div class="url-display url-display-full ui-composite">
            <input class="ui-input url-code url-code-wide ui-composite-field" type="text" readonly :value="overlayUrl" :aria-label="t('input_server.local_url_label')" :title="overlayUrl" />
            <button
              class="ui-icon-button ui-icon-button--adjacent ui-composite-action"
              :title="t('input_server.copy_overlay')"
              :aria-label="t('input_server.copy_overlay')"
              @click="copyOverlayUrl"
            >
              <Copy :size="18" />
            </button>
          </div>
        </div>

        <div class="connection-row">
          <label class="row-label ui-label ui-label--secondary">{{ t('input_server.lan_url_label') }}</label>
          <div class="url-display url-display-full ui-composite">
            <input class="ui-input url-code url-code-wide ui-composite-field" type="text" readonly :value="lanUrl ?? t('input_server.lan_url_unavailable')" :aria-label="t('input_server.lan_url_label')" :title="lanUrl ?? t('input_server.lan_url_unavailable')" />
            <button
              class="ui-icon-button ui-icon-button--adjacent ui-composite-action"
              :title="t('input_server.copy_lan_url')"
              :aria-label="t('input_server.copy_lan_url')"
              :disabled="!lanUrl"
              :class="{ disabled: !lanUrl }"
              @click="copyLanUrl"
            >
              <Copy :size="18" />
            </button>
          </div>
        </div>

        <div class="connection-row connection-token-row">
          <label class="row-label ui-label ui-label--secondary">{{ t('input_server.token.label') }}</label>
          <div class="token-field-group ui-composite">
            <InputWithToggle
              :model-value="accessToken ?? ''"
              :label="t('input_server.token.label')"
              type="password"
              readonly
              ui
              :placeholder="t('input_server.token.not_generated')"
              :disabled="!tokenAvailable"
              class="token-field ui-composite-field"
            />
            <button
              class="ui-icon-button ui-icon-button--adjacent ui-composite-action"
              :title="t('input_server.token.copy')"
              :aria-label="t('input_server.token.copy')"
              :disabled="!tokenAvailable || regeneratePending"
              :class="{ disabled: !tokenAvailable || regeneratePending }"
              @click="copyToken"
            >
              <Copy :size="18" />
            </button>
          </div>
          <button
            class="ui-icon-button ui-icon-button--adjacent danger-button ui-action--danger"
            :title="t('input_server.token.regenerate')"
            :aria-label="t('input_server.token.regenerate')"
            :disabled="regeneratePending"
            :class="{ disabled: regeneratePending }"
            @click="regenerateToken"
          >
            <RefreshCw :size="18" />
          </button>
        </div>
      </div>

      <div class="subsection-divider"></div>

      <h3 class="subsection-heading ui-group-title">{{ t('input_server.speech_api') }}</h3>
      <p class="api-summary">
        <span>POST <code class="inline-code">&lt;url&gt;/v1/speech</code></span>
        <span>JSON <code class="inline-code">{"text":"реплика"}</code></span>
      </p>
    </section>

    <section class="settings-section ui-section">
      <h2 class="ui-section-title">{{ t('input_server.test.title') }}</h2>
      <div class="ui-row">
        <input
          type="text"
          v-model="testText"
          :placeholder="t('input_server.test.placeholder')"
          class="ui-input test-input"
          @keyup.enter="sendTest"
        />
        <button
          class="test-button ui-button ui-button--primary"
          :disabled="!testText.trim() || testPending"
          @click="sendTest"
        >
          {{ testPending ? t('input_server.test.sending') : t('input_server.test.send') }}
        </button>
      </div>
      <div v-if="testResult" class="test-result ui-status" :class="testResult.status">
        <template v-if="testResult.status === 'queued'">{{ t('input_server.test.queued') }}</template>
        <template v-else>{{ t('input_server.test.pending_review') }}</template>
      </div>
      <div v-else-if="testError" class="test-result error ui-status">{{ testError }}</div>
    </section>
  </div>
</template>

<style scoped>
.input-server-panel {
  max-width: 900px;
  margin: 0 auto;
}

h2 {
  margin-top: 0;
  margin-bottom: 1rem;
  color: var(--color-text-primary);
}

.message-box {
  position: fixed;
  top: 20px;
  left: calc(50% + 100px);
  transform: translateX(-50%);
  padding: 0.4rem 0.75rem;
  border-radius: 8px;
  z-index: 1000;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.3);
  backdrop-filter: blur(10px);
  animation: slideDownFade 0.3s ease-out;
  white-space: nowrap;
}

.message-box.success {
  background: var(--success-bg);
  border: 1px solid var(--success-shadow);
  color: var(--success-text);
}

.message-box.info {
  background: var(--info-bg);
  border: 1px solid var(--info-border);
  color: var(--info-text);
}

.message-box.warning {
  background: var(--warning-bg);
  border: 1px solid var(--warning-border);
  color: var(--warning-text);
}

.message-box.error {
  background: var(--danger-bg);
  border: 1px solid var(--danger-border);
  border-left: 4px solid var(--danger-gradient-start);
  color: var(--danger-text);
}

@keyframes slideDownFade {
  from {
    opacity: 0;
    transform: translateX(-50%) translateY(-20px);
  }
  to {
    opacity: 1;
    transform: translateX(-50%) translateY(0);
  }
}

/* Decorative card surface stays local; padding and rhythm come from ui-section. */
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
}

.server-header h2 {
  margin: 0;
}

.server-status {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-top: -2px;
}

/* Connection badge: local colors and compact surface, shared typography. */
.status-indicator {
  padding: 0.15rem 0.5rem;
  background: var(--color-bg-field);
  border-radius: 5px;
  border: 1px solid var(--color-border);
  height: 28px;
  display: flex;
  align-items: center;
}

.status-indicator.running {
  color: var(--success-text-bright);
  background: var(--success-bg-weak);
  border-color: var(--success-shadow);
}

.status-indicator.starting {
  color: var(--info-text);
  background: var(--info-bg-weak);
  border-color: var(--info-border);
}

.status-indicator.error {
  color: var(--danger-text-bright);
  background: var(--danger-bg-weak);
  border-color: var(--danger-border);
}

:global([data-theme='light'] .input-server-panel .status-indicator.error),
:global([data-theme='light'] .input-server-panel .message-box.error) {
  color: var(--ui-action-danger-hover-text);
}

.status-button.disabled:not(.stop) {
  background: var(--btn-disabled-bg);
  cursor: not-allowed;
  opacity: 0.6;
}

.external-access-warning {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.6rem 0.75rem;
  margin-bottom: 1rem;
  background: var(--warning-bg-weak);
  border: 1px solid var(--warning-border);
  border-radius: 8px;
  color: var(--warning-text-bright);
  line-height: 1.4;
}

/* Persistent-баннер отказа запуска: красная тема, общая типографика ui-status. */
.status-error-banner {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.6rem 0.75rem;
  margin-bottom: 1rem;
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border);
  border-radius: 8px;
  color: var(--danger-text-bright);
  line-height: 1.4;
}

.status-error-banner svg { flex-shrink: 0; }
:global([data-theme='light'] .status-error-banner) { color: var(--ui-action-danger-hover-text); }

/* Rows use ui-row; only wrap behavior and the last-row reset stay local.
   The legacy .setting-row name is gone so global AudioPanel rules cannot
   reach this panel. */
.ui-row {
  flex-wrap: wrap;
}

.ui-row:last-child {
  margin-bottom: 0;
}

.ui-row label {
  color: var(--color-text-secondary);
}

.ui-row .ui-choice-label {
  min-width: 0;
}

.ui-choice-label .ui-choice-input:disabled {
  cursor: not-allowed;
}

/* Port keeps its narrow by-purpose width; geometry and states come from
   ui-input. */
.port-setting-row {
  padding-top: var(--ui-row-gap);
  border-top: 1px solid var(--color-border);
}

.address-inputs {
  min-width: 0;
}

.address-inputs .address-port {
  flex: 0 0 84px;
  width: 84px;
  min-width: 84px;
  max-width: 84px;
}

.error-text {
  color: var(--danger-text-weak);
  width: 100%;
}

/* URL display: one composite field — readonly value plus adjacent actions. */
.url-display {
  flex: 0;
  width: auto;
}

.url-display-full {
  flex: 1;
  width: 100%;
}

/* Native readonly inputs keep selection and caret scrolling for long URLs.
   Joined borders and corners come from ui-composite. */
.url-code {
  font-family: var(--font-mono);
}

.url-code.ui-input:hover:not(:disabled) {
  background: var(--color-bg-field);
}

.url-code-wide {
  flex: 1;
  width: auto;
  min-width: 0;
}

.subsection-divider {
  border-top: 1px solid var(--color-border);
  margin: var(--ui-row-gap) 0;
}

.subsection-heading {
  margin: 0 0 0.75rem;
  color: var(--color-text-primary);
}

/* Shared label column aligns connection fields; column gap stays 12 px. */
.connection-grid {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr) auto;
  column-gap: var(--ui-row-label-gap-side);
  row-gap: var(--ui-row-gap);
  align-items: center;
}

.connection-row {
  display: contents;
}

.connection-row .url-display {
  grid-column: 2 / -1;
}

.connection-token-row .token-field-group {
  grid-column: 2;
  min-width: 0;
}

.token-field {
  flex: 1;
  min-width: 0;
}

/* InputWithToggle keeps mono text here; shared roles own its field and joins. */
.token-field :deep(.input-with-toggle-input.ui-input) {
  font-family: var(--font-mono);
}

@media (max-width: 440px) {
  .connection-grid {
    display: block;
  }
  .connection-row {
    display: grid;
    gap: var(--ui-row-label-gap-stack);
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: stretch;
    margin-bottom: 1rem;
  }
  .connection-row:last-child {
    margin-bottom: 0;
  }
  .connection-row .row-label {
    grid-column: 1 / -1;
  }
  .connection-row .url-display {
    width: 100%;
    grid-column: 1 / -1;
  }
  .connection-token-row .token-field-group {
    grid-column: 1;
  }
}

.inline-code {
  font-family: var(--font-mono);
  font-size: 0.85rem;
  color: inherit;
  background: var(--info-bg-weak);
  border-radius: 3px;
  padding: 0.1rem 0.3rem;
}

/* Text actions keep their local accent surfaces; typography and geometry
   come from ui-button. */
.test-input {
  flex: 1;
  min-width: 0;
}

.test-result {
  padding: 0.5rem 0.75rem;
  border-radius: 8px;
  line-height: 1.4;
}

.test-result.queued {
  background: var(--success-bg-weak);
  border: 1px solid var(--success-shadow);
  color: var(--success-text-bright);
}

.test-result.pending_review {
  background: var(--info-bg-weak);
  border: 1px solid var(--info-border);
  color: var(--info-text);
}

.test-result.error {
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border);
  color: var(--danger-text-bright);
}

.info-callout {
  display: flex;
  align-items: flex-start;
  gap: 0.6rem;
  padding: 0.75rem 1rem;
  margin-bottom: var(--ui-section-gap);
  background: var(--info-bg-weak);
  border: 1px solid var(--info-border);
  border-left: 4px solid var(--info-accent, var(--color-accent));
  border-radius: 10px;
  color: var(--info-text-bright);
  line-height: 1.5;
}

.info-icon {
  flex-shrink: 0;
  margin-top: 1px;
  color: var(--info-text-bright);
}

.api-summary {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem;
  margin: 0;
  font-size: 0.85rem;
  font-weight: 400;
  color: var(--color-text-secondary);
}
</style>
