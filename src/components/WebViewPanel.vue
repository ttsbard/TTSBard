<script setup lang="ts">
import { computed } from 'vue'
import { Copy, RotateCw, Play, Square, AlertTriangle, Globe, RefreshCw } from 'lucide-vue-next'
import { useWebView } from '../composables/useWebView'
import { t } from '../i18n'
import InputWithToggle from './shared/InputWithToggle.vue'
import PanelEasterEgg from './shared/PanelEasterEgg.vue'

const messageBoxClass = computed(() => (errorMessage.value ? errorMessageType.value : ''))

const {
  settings,
  errorMessage,
  errorMessageType,
  testMessage,
  loopbackUrl,
  lanUrl,
  serverStatus,
  externalDisplay,
  hasToken,
  isPortValid,
  isUpnpAvailable,
  upnpForwardOpen,
  upnpForwardFailureText,
  serverErrorText,
  startServer,
  stopServer,
  restartServer,
  operationPending,
  awaitingRestart,
  awaitingStart,
  saveStartOnBoot,
  saveSendOriginalText,
  saveServerSettings,
  copyLoopbackUrl,
  copyLanUrl,
  resolveLanIp,
  copyToken,
  regenerateAccessToken,
  saveUpnpEnabled,
  upnpPending,
  showExternalUrl,
  copyExternalUrl,
  openTemplateFolder,
  sendTest,
  reloadTemplates,
} = useWebView()

// Управление блокируется не только по runtime-статусу, но и на всю
// пользовательскую операцию: нажатая кнопка держит роль до фактического
// завершения, промежуточный stopped перезапуска не открывает запуск.
const serverBusy = computed(() => serverStatus.value.state === 'starting')
const controlsLocked = computed(() => operationPending.value || serverBusy.value)
// Поля адреса недоступны работающему/запускающемуся серверу и на время любой
// операции: промежуточный stopped перезапуска не открывает правку.
const fieldsLocked = computed(
  () => serverStatus.value.state === 'running' || controlsLocked.value,
)
// Ветка перезапуска/стопа держится и на промежуточном stopped перезапуска.
const showRunControls = computed(
  () => serverStatus.value.state === 'running' || serverBusy.value || awaitingRestart.value,
)
const statusText = computed(() => {
  if (awaitingRestart.value) return t('webview.status.restarting')
  if (awaitingStart.value) return t('webview.status.starting')
  switch (serverStatus.value.state) {
    case 'running':
      return t('webview.status.running')
    case 'starting':
      return t('webview.status.starting')
    case 'error':
      return t('webview.status.error')
    default:
      return t('webview.status.stopped')
  }
})
</script>

<template>
  <div class="webview-panel">
    <!-- Error/Info Message Display -->
    <div v-if="errorMessage" class="message-box ui-status" :class="messageBoxClass">
      {{ errorMessage }}
    </div>

    <section class="settings-section ui-section easter-egg-host">
      <PanelEasterEgg kind="tail" />
      <div class="section-header server-header">
        <h2 class="ui-section-title">{{ t('webview.server') }}</h2>
        <div class="server-status">
          <span class="status-indicator ui-status" :class="{ running: serverStatus.state === 'running', error: serverStatus.state === 'error' && !controlsLocked }">
            {{ statusText }}
          </span>
          <template v-if="showRunControls">
            <button @click="restartServer" class="status-button restart ui-icon-button ui-icon-button--accent" :disabled="controlsLocked" :class="{ disabled: controlsLocked }" :title="t('webview.restart')" :aria-label="t('webview.restart')">
              <RotateCw :size="18" />
            </button>
            <button @click="stopServer" class="status-button stop ui-icon-button ui-action--stop" :disabled="controlsLocked" :class="{ disabled: controlsLocked }" :title="t('webview.stop')" :aria-label="t('webview.stop')">
              <Square :size="18" />
            </button>
          </template>
          <template v-else>
            <button @click="startServer" class="status-button start ui-icon-button ui-icon-button--accent" :disabled="!isPortValid || operationPending" :class="{ disabled: !isPortValid || operationPending }" :title="t('webview.start')" :aria-label="t('webview.start')">
              <Play :size="18" />
            </button>
            <button @click="stopServer" class="status-button stop ui-icon-button ui-action--stop" :class="{ disabled: serverStatus.state !== 'error' || operationPending }" :title="t('webview.stop')" :aria-label="t('webview.stop')" :disabled="serverStatus.state !== 'error' || operationPending">
              <Square :size="18" />
            </button>
          </template>
        </div>
      </div>

      <div v-if="serverErrorText" class="status-error-banner ui-status">
        <AlertTriangle :size="14" />
        <span>{{ serverErrorText }}</span>
      </div>

      <div class="ui-row">
        <label class="ui-choice-label">
          <input type="checkbox" v-model="settings.start_on_boot" @change="saveStartOnBoot" class="ui-choice-input" />
          <span>{{ t('webview.start_on_boot') }}</span>
        </label>
      </div>

      <div class="ui-row">
        <label class="ui-choice-label">
          <input type="checkbox" v-model="settings.send_original_text" @change="saveSendOriginalText" class="ui-choice-input" />
          <span>{{ t('webview.send_original_text') }}</span>
        </label>
      </div>

      <div class="ui-row address-row">
        <label class="ui-label">{{ t('webview.port') }}</label>
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
            placeholder="10100"
          />
          <button @click="saveServerSettings" class="save-button-inline ui-button ui-button--primary" :disabled="fieldsLocked">{{ t('common.save') }}</button>
        </div>
        <span v-if="!isPortValid" class="error-text ui-status">{{ t('webview.port_error') }}</span>
      </div>
    </section>

    <section class="settings-section ui-section connection-section">
      <h2 class="ui-section-title">{{ t('webview.connection') }}</h2>

      <div class="connection-grid">
        <div class="connection-row">
          <label class="row-label ui-label ui-label--secondary">{{ t('webview.connection.local_label') }}</label>
          <div class="url-display url-display-full ui-composite">
            <input class="ui-input url-code url-code-wide ui-composite-field" type="text" readonly :value="loopbackUrl" :aria-label="t('webview.connection.local_label')" :title="loopbackUrl" />
            <button @click="copyLoopbackUrl" class="ui-icon-button ui-icon-button--adjacent ui-composite-action" :title="t('webview.copy_url')" :aria-label="t('webview.copy_url')">
              <Copy :size="18" />
            </button>
          </div>
        </div>

        <div class="connection-row">
          <label class="row-label ui-label ui-label--secondary">{{ t('webview.connection.lan_label') }}</label>
          <div class="url-display url-display-full ui-composite">
            <input class="ui-input url-code url-code-wide ui-composite-field" type="text" readonly :value="lanUrl || t('webview.lan_url_unavailable')" :aria-label="t('webview.connection.lan_label')" :title="lanUrl || t('webview.lan_url_unavailable')" />
            <button v-if="lanUrl" @click="copyLanUrl" class="ui-icon-button ui-icon-button--adjacent ui-composite-action" :title="t('webview.copy_lan_url')" :aria-label="t('webview.copy_lan_url')">
              <Copy :size="18" />
            </button>
            <button v-else @click="resolveLanIp" class="ui-icon-button ui-icon-button--adjacent ui-composite-action" :title="t('webview.lan_retry')" :aria-label="t('webview.lan_retry')">
              <RefreshCw :size="18" />
            </button>
          </div>
        </div>
      </div>
    </section>

    <section class="settings-section ui-section">
      <h2 class="ui-section-title">{{ t('webview.templates.title') }}</h2>
      <div class="ui-row">
        <button @click="openTemplateFolder" class="ui-button">
          {{ t('webview.templates.open_folder') }}
        </button>
        <button @click="reloadTemplates" class="ui-button">
          {{ t('webview.templates.reload') }}
        </button>
      </div>
      <span class="setting-warning ui-hint"><AlertTriangle :size="14" /> {{ t('webview.templates.hint') }}</span>
    </section>

    <section class="settings-section ui-section">
      <h2 class="ui-section-title">{{ t('webview.test.title') }}</h2>
      <div class="ui-row">
        <input
          type="text"
          v-model="testMessage"
          :placeholder="t('webview.test.placeholder')"
          class="ui-input test-input"
          @keyup.enter="sendTest"
        />
        <button @click="sendTest" class="test-button ui-button ui-button--primary" :disabled="serverStatus.state !== 'running' || !testMessage">
          {{ t('webview.test.send') }}
        </button>
      </div>
    </section>

    <section class="settings-section ui-section" :class="{ 'section-disabled': !isUpnpAvailable }">
      <h2 class="ui-section-title">{{ t('webview.external.title') }}</h2>

      <!-- Warning for local address -->
      <div v-if="!isUpnpAvailable" class="external-access-warning ui-status">
        <AlertTriangle :size="14" />
        <span>{{ t('webview.external.local_only_warning') }}</span>
      </div>

      <!-- External URL display (shows full URL with token if available) -->
      <div class="ui-row" v-if="hasToken">
        <div class="url-display url-display-full ui-composite">
          <input class="ui-input url-code url-code-wide ui-composite-field" type="text" readonly :value="externalDisplay" :aria-label="t('webview.external.title')" :title="externalDisplay" />
          <button @click="copyExternalUrl" class="ui-icon-button ui-icon-button--adjacent ui-composite-action" :title="t('webview.external.copy_url')" :aria-label="t('webview.external.copy_url')" :disabled="!isUpnpAvailable || !externalDisplay">
            <Copy :size="18" />
          </button>
          <button @click="showExternalUrl" class="ui-icon-button ui-icon-button--adjacent ui-composite-action" :title="t('webview.external.refresh_ip')" :aria-label="t('webview.external.refresh_ip')" :disabled="!isUpnpAvailable">
            <Globe :size="18" />
          </button>
        </div>
      </div>

      <!-- Token access -->
      <div class="token-row">
        <label class="ui-label ui-label--secondary">{{ t('webview.token.label') }}</label>
        <div class="token-field ui-composite">
          <InputWithToggle
            :model-value="settings.access_token ?? ''"
            :label="t('webview.token.label')"
            type="password"
            readonly
            ui
            :placeholder="t('webview.token.not_generated')"
            :disabled="!hasToken"
            class="token-field-input ui-composite-field"
          />
          <button @click="copyToken" class="ui-icon-button ui-icon-button--adjacent ui-composite-action" :title="t('webview.token.copy')" :aria-label="t('webview.token.copy')" :disabled="!hasToken || !isUpnpAvailable">
            <Copy :size="18" />
          </button>
        </div>
        <button @click="regenerateAccessToken" class="ui-icon-button ui-icon-button--adjacent danger-button ui-action--danger" :title="t('webview.token.regenerate')" :aria-label="t('webview.token.regenerate')" :disabled="!isUpnpAvailable">
          <RotateCw :size="18" />
        </button>
      </div>

      <!-- UPnP status -->
      <div class="ui-row">
        <label class="ui-choice-label" :class="{ disabled: !isUpnpAvailable }" :title="t('webview.upnp.tooltip')">
          <input type="checkbox" v-model="settings.upnp_enabled" @change="saveUpnpEnabled" :disabled="!isUpnpAvailable || upnpPending" class="ui-choice-input" />
          <span>{{ t('webview.upnp.enable') }}</span>
        </label>
      </div>

      <!-- Actual runtime UPnP status: mapping is a fact, not a preference. -->
      <div v-if="upnpForwardOpen" class="upnp-status upnp-status-open ui-status">
        {{ t('webview.upnp.status.open') }}
      </div>
      <div v-else-if="upnpForwardFailureText" class="upnp-status upnp-status-failed ui-status">
        {{ upnpForwardFailureText }}
      </div>
    </section>
  </div>
</template>

<style scoped>
.easter-egg-host {
  position: relative;
}

.webview-panel {
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

.settings-section.section-disabled {
  opacity: 0.7;
}

.settings-section.section-disabled .external-access-warning {
  opacity: 1;
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

.upnp-status {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.4rem 0.75rem;
  margin-bottom: 0;
  border-radius: 8px;
  line-height: 1.35;
}

.upnp-status-open {
  background: var(--success-bg-weak);
  border: 1px solid var(--success-shadow);
  color: var(--success-text-bright);
}

.upnp-status-failed {
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border);
  color: var(--danger-text-bright);
}

.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

/* Server header with status */
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

.status-indicator.error {
  color: var(--danger-text-bright);
  background: var(--danger-bg-weak);
  border-color: var(--danger-border);
}

:global([data-theme='light'] .webview-panel .status-indicator.error),
:global([data-theme='light'] .webview-panel .message-box.error) {
  color: var(--ui-action-danger-hover-text);
}

.status-button.disabled:not(.stop) {
  background: var(--btn-disabled-bg);
  cursor: not-allowed;
  opacity: 0.6;
}

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

.ui-choice-label.disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.ui-choice-label.disabled .ui-choice-input {
  cursor: not-allowed;
}

/* Address inputs group (bind address + port): the bind select stays wider
   than the port by purpose. Geometry and states come from ui-select/ui-input. */
.address-row {
  padding-top: var(--ui-row-gap);
  border-top: 1px solid var(--color-border);
}

.address-inputs {
  flex: 1 1 360px;
  min-width: 0;
}

.address-inputs .address-port {
  flex: 0 0 84px;
  width: 84px;
  max-width: 84px;
}

/* URL display: one composite field — readonly value plus adjacent actions. */
.url-display {
  flex: 0 1 auto;
  width: 318px;
  max-width: 100%;
}

.url-display-full {
  flex: 1;
  width: 100%;
}

/* URL typography comes from ui-input; sizing and monospace stay local.
   Joined borders and corners come from ui-composite. */
.url-code {
  flex: 1;
  min-width: 0;
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

/* Connection rows share the label column (max-content) and the URL composite
   (minmax(0, 1fr)) so each label stays beside its URL on one row. The label may
   wrap inside its own column at enlarged text, but the field stays next to it
   rather than dropping below. */
.connection-grid {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  column-gap: var(--ui-row-label-gap-side);
  row-gap: var(--ui-row-gap);
  align-items: center;
}

.connection-row {
  display: contents;
}

.connection-row .url-display {
  grid-column: 2;
}

/* Token row: grid keeps the token field shrinkable next to regenerate. */
.token-row {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  column-gap: var(--ui-row-label-gap-side);
  row-gap: var(--ui-row-label-gap-stack);
  margin-bottom: var(--ui-row-gap);
}

.token-row .ui-label {
  min-width: 60px;
}

.token-field {
  width: auto;
}

.token-field-input {
  flex: 1;
  min-width: 0;
}

/* InputWithToggle keeps mono text here; shared roles own its field and joins. */
.token-field :deep(.input-with-toggle-input.ui-input) {
  font-family: var(--font-mono);
}

@media (max-width: 440px) {
  .token-row {
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .token-row > label {
    grid-column: 1 / -1;
  }
}

.test-input {
  flex: 1;
}

.setting-warning {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  margin-top: 0.5rem;
  color: var(--warning-text-bright);
}

.error-text {
  color: var(--danger-text-weak);
}
</style>
