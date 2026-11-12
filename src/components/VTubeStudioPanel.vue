<script setup lang="ts">
import { Play, RefreshCw, RotateCw, Square } from 'lucide-vue-next'
import { useVTubeStudio, SAVED_HOTKEY_TYPE } from '../composables/useVTubeStudio'
import { t } from '../i18n'
import PanelEasterEgg from './shared/PanelEasterEgg.vue'

const {
  settings,
  errorMessage,
  errorMessageType,
  portError,
  hostError,
  currentStatus,
  busy,
  typingTimeout,
  typingRepeats,
  typingTimeoutError,
  typingRepeatsError,
  canTestAction,
  canLoadHotkeys,
  canEditTypingAction,
  canSubmitTypingAction,
  typingMode,
  eventName,
  startHotkeyId,
  stopHotkeyId,
  itemFileName,
  savedTypingAction,
  hotkeys,
  hotkeysLoading,
  hotkeysError,
  sceneItems,
  sceneItemsLoading,
  sceneItemsError,
  itemStatusWarning,
  selectedSceneItem,
  canLoadSceneItems,
  save,
  saveTypingAction,
  loadHotkeys,
  refreshItemAction,
  testAction,
  startVTubeStudio,
  stopVTubeStudio,
  restartVTubeStudio,
  saveStartOnBoot,
} = useVTubeStudio()

</script>

<template>
  <div class="vtube-panel">
    <div v-if="errorMessage" class="message-box ui-status" :class="errorMessageType">
      {{ errorMessage }}
    </div>

    <section class="settings-section ui-section easter-egg-host">
      <PanelEasterEgg kind="cursor" />
      <div class="section-header server-header">
        <h2 class="ui-section-title">{{ t('vtube.connection') }}</h2>
        <div class="server-status">
          <span class="status-indicator ui-status" :class="{
            running: currentStatus === 'Connected',
            connecting: currentStatus === 'Connecting',
            error: currentStatus === 'Error'
          }">
            {{ currentStatus === 'Connected' ? t('vtube.status.connected') :
               currentStatus === 'Connecting' ? t('vtube.status.connecting') :
               currentStatus === 'Error' ? t('vtube.status.error') :
               t('vtube.status.disconnected') }}
          </span>
          <template v-if="currentStatus === 'Connected'">
            <button @click="restartVTubeStudio" class="status-button refresh ui-icon-button ui-icon-button--accent" :disabled="busy" :class="{ disabled: busy }" :title="t('vtube.restart')" :aria-label="t('vtube.restart')">
              <RotateCw :size="18" />
            </button>
            <button @click="stopVTubeStudio" class="status-button stop ui-icon-button ui-action--stop" :disabled="busy" :class="{ disabled: busy }" :title="t('vtube.disconnect')" :aria-label="t('vtube.disconnect')">
              <Square :size="18" />
            </button>
          </template>
          <template v-else>
            <button @click="startVTubeStudio" class="status-button start ui-icon-button ui-icon-button--accent" :disabled="busy || currentStatus === 'Connecting'" :class="{ disabled: busy || currentStatus === 'Connecting' }" :title="t('vtube.connect')" :aria-label="t('vtube.connect')">
              <Play :size="18" />
            </button>
            <button class="status-button stop disabled ui-icon-button ui-action--stop" :title="t('vtube.disconnect')" :aria-label="t('vtube.disconnect')" disabled>
              <Square :size="18" />
            </button>
          </template>
        </div>
      </div>

      <div class="ui-row">
        <label class="ui-choice-label">
          <input type="checkbox" v-model="settings.start_on_boot" @change="saveStartOnBoot" class="ui-choice-input" />
          <span>{{ t('vtube.start_on_boot') }}</span>
        </label>
      </div>

      <div class="ui-row port-setting-row">
        <label for="vtube-host" class="ui-label">{{ t('vtube.host') }}</label>
        <div class="endpoint-inputs ui-field-group">
        <input
          id="vtube-host"
          type="text"
          v-model="settings.host"
          class="ui-input endpoint-input endpoint-input-host"
          :aria-invalid="hostError ? 'true' : undefined"
          :disabled="busy"
          spellcheck="false"
          autocomplete="off"
          placeholder="127.0.0.1"
        />
        <input
          type="number"
          :aria-label="t('vtube.port')"
          :title="t('vtube.port')"
          v-model.number="settings.port"
          class="ui-input endpoint-input endpoint-input-port"
          :aria-invalid="portError ? 'true' : undefined"
          :disabled="busy"
          :min="1024"
          :max="65535"
          placeholder="8001"
        />
        <button @click="save" class="save-button-inline ui-button ui-button--primary" :disabled="busy" :class="{ disabled: busy }">
          {{ t('common.save') }}
        </button>
        </div>
      </div>
      <div v-if="hostError" class="port-error endpoint-error ui-status">{{ hostError }}</div>
      <div v-if="portError" class="port-error endpoint-error ui-status">{{ portError }}</div>
    </section>

    <section class="settings-section ui-section typing-action-section">
      <h2 class="ui-section-title">{{ t('vtube.action.title') }}</h2>
      <p class="ui-label current-action-summary">
        <span class="ui-label ui-label--secondary">{{ t('vtube.action.current') }}</span>
        {{ savedTypingAction.outputMode === 'Event' ? t('vtube.action.mode.event') : savedTypingAction.outputMode === 'Hotkeys' ? t('vtube.action.mode.hotkeys') : t('vtube.action.mode.item') }} ·
        <template v-if="savedTypingAction.outputMode === 'Event'">{{ savedTypingAction.parameterName || t('vtube.not_set_parameter') }}</template>
        <template v-else-if="savedTypingAction.outputMode === 'Hotkeys'">{{ t('vtube.info.start') }}: {{ savedTypingAction.startHotkeyName || savedTypingAction.startHotkeyId || t('vtube.not_set') }} · {{ t('vtube.info.stop') }}: {{ savedTypingAction.stopHotkeyName || savedTypingAction.stopHotkeyId || t('vtube.not_set') }}</template>
        <template v-else>{{ savedTypingAction.itemFileName || t('vtube.not_set') }}</template>
      </p>

      <div class="ui-row typing-action-row typing-mode-row">
        <label class="ui-label typing-mode-label"><span>{{ t('vtube.action.mode_label') }}</span><span class="typing-mode-label-sizer" aria-hidden="true">{{ t('vtube.action.param_label') }}</span></label>
        <select v-model="typingMode" class="ui-select typing-mode-select" :disabled="busy || !canEditTypingAction">
          <option value="Event">{{ t('vtube.action.mode.event') }}</option>
          <option value="Hotkeys">{{ t('vtube.action.mode.hotkeys') }}</option>
          <option value="Item">{{ t('vtube.action.mode.item') }}</option>
        </select>
        <button v-if="typingMode === 'Hotkeys'" class="ui-icon-button ui-icon-button--adjacent" @click="loadHotkeys()" :disabled="!canLoadHotkeys" :title="t('vtube.action.load_hotkeys')" :aria-label="t('vtube.action.load_hotkeys')">
          <RefreshCw :size="18" :class="{ 'ui-spin': hotkeysLoading }" />
        </button>
        <button v-else-if="typingMode === 'Item'" class="ui-icon-button ui-icon-button--adjacent" @click="refreshItemAction()" :disabled="!canLoadSceneItems" :title="t('vtube.action.refresh_items')" :aria-label="t('vtube.action.refresh_items')">
          <RefreshCw :size="18" :class="{ 'ui-spin': sceneItemsLoading }" />
        </button>
      </div>

      <template v-if="typingMode === 'Event'">
        <div class="ui-row typing-action-row">
          <label class="ui-label">{{ t('vtube.action.param_label') }}</label>
          <input
            type="text"
            v-model="eventName"
            class="ui-input typing-param-input"
            :disabled="busy || !canEditTypingAction"
            placeholder="TTSBardTyping"
          />
        </div>
      </template>

      <template v-else-if="typingMode === 'Hotkeys'">

        <p class="info-hint ui-hint">{{ t('vtube.action.hotkeys_hint_compact') }}</p>

        <div v-if="hotkeysError" class="hotkey-status error ui-status">{{ hotkeysError }}</div>

        <div class="ui-row typing-action-row">
          <label class="ui-label typing-mode-label"><span>{{ t('vtube.action.start_label') }}</span><span class="typing-mode-label-sizer" aria-hidden="true">{{ t('vtube.action.stop_label') }}</span></label>
          <select v-model="startHotkeyId" class="ui-select typing-target-select" :disabled="busy || !canEditTypingAction">
            <option value="" disabled>{{ t('vtube.select_placeholder') }}</option>
            <option v-for="h in hotkeys" :key="h.hotkeyID" :value="h.hotkeyID">
              {{ h.name }}<template v-if="h.type !== SAVED_HOTKEY_TYPE"> ({{ h.type }})</template>
            </option>
          </select>
        </div>

        <div class="ui-row typing-action-row">
          <label class="ui-label typing-mode-label"><span>{{ t('vtube.action.stop_label') }}</span><span class="typing-mode-label-sizer" aria-hidden="true">{{ t('vtube.action.start_label') }}</span></label>
          <select v-model="stopHotkeyId" class="ui-select typing-target-select" :disabled="busy || !canEditTypingAction">
            <option value="" disabled>{{ t('vtube.select_placeholder') }}</option>
            <option v-for="h in hotkeys" :key="h.hotkeyID" :value="h.hotkeyID">
              {{ h.name }}<template v-if="h.type !== SAVED_HOTKEY_TYPE"> ({{ h.type }})</template>
            </option>
          </select>
        </div>
      </template>

      <template v-else>

        <p class="info-hint ui-hint item-selection-hint">{{ t('vtube.action.item_hint_compact') }}</p>

        <div v-if="sceneItemsError" class="hotkey-status error ui-status">{{ sceneItemsError }}</div>

        <div class="ui-row typing-action-row item-selection-row">
          <label class="ui-label typing-mode-label"><span>{{ t('vtube.action.item_label') }}</span><span class="typing-mode-label-sizer" aria-hidden="true">{{ t('vtube.action.param_label') }}</span></label>
          <select v-model="itemFileName" class="ui-select item-select typing-target-select" :disabled="busy || !canEditTypingAction">
            <option value="" disabled>{{ t('vtube.select_placeholder') }}</option>
            <option v-if="itemFileName && !selectedSceneItem" :value="itemFileName">
              {{ t('vtube.action.item_missing', { name: itemFileName }) }}
            </option>
            <option v-for="item in sceneItems" :key="`${item.fileName}:${item.itemType}`" :value="item.fileName">
              {{ item.fileName }} · {{ item.itemType }}<template v-if="item.duplicateCount > 1"> · {{ t('vtube.action.item_copies', { count: item.duplicateCount }) }}</template>
            </option>
          </select>
        </div>
        <div v-if="itemFileName && selectedSceneItem && selectedSceneItem.duplicateCount > 1" class="item-metadata">
          <span class="duplicate-warning">
            {{ t('vtube.action.duplicate_warning') }}
          </span>
        </div>
      </template>

      <div v-if="itemStatusWarning" class="item-warning ui-status" role="status">
        {{ itemStatusWarning }}
      </div>

      <div class="ui-row button-row">
        <button
          @click="saveTypingAction()"
          class="save-button-inline ui-button ui-button--primary"
          :disabled="!canSubmitTypingAction"
          :class="{ disabled: !canSubmitTypingAction }"
          :title="t('vtube.action.save_title')"
          :aria-label="t('common.save')"
        >
          {{ t('common.save') }}
        </button>
      </div>
    </section>

    <section class="settings-section ui-section">
      <h2 class="ui-section-title">{{ t('vtube.test.title') }}</h2>
      <p class="info-hint ui-description test-description">
        {{ t('vtube.test.hint_title') }} {{ t('vtube.test.hint_repeats') }}
      </p>
      <div class="ui-row test-parameters-row">
        <label class="ui-label">{{ t('vtube.test.timeout_label') }}</label>
        <input
          type="number"
          v-model.number="typingTimeout"
          class="ui-input"
          :aria-invalid="typingTimeoutError ? 'true' : undefined"
          :min="100"
          :max="5000"
        />
        <label class="ui-label">{{ t('vtube.test.repeats_label') }}</label>
        <input
          type="number"
          v-model.number="typingRepeats"
          class="ui-input"
          :aria-invalid="typingRepeatsError ? 'true' : undefined"
          :min="1"
          :max="10"
        />
        <button
          @click="testAction()"
          class="save-button-inline ui-button ui-button--primary"
          :disabled="!canTestAction"
          :class="{ disabled: !canTestAction }"
          :title="t('vtube.test.run_title')"
          :aria-label="t('vtube.test.run')"
        >
          {{ t('vtube.test.run') }}
        </button>
      </div>
      <div v-if="typingTimeoutError" class="test-error ui-status">{{ typingTimeoutError }}</div>
      <div v-if="typingRepeatsError" class="test-error ui-status">{{ typingRepeatsError }}</div>

    </section>

    <section class="settings-section help-section ui-section">
      <h2 class="ui-section-title">{{ t('vtube.help.title') }}</h2>
      <p class="help-text ui-description">{{ t('vtube.help.plugin_api') }}</p>
    </section>
  </div>
</template>

<style scoped>
.easter-egg-host {
  position: relative;
}

.vtube-panel {
  max-width: 900px;
  margin: 0 auto;
}

h2 {
  margin-top: 0;
  margin-bottom: 1rem;
  color: var(--color-text-primary);
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

.status-indicator.connecting {
  color: var(--success-text-bright);
  background: var(--success-bg-weak);
  border-color: var(--success-border);
}

.status-indicator.error {
  color: var(--danger-text-weak);
  background: var(--danger-bg-weak);
  border-color: var(--danger-border);
}

.status-button.disabled:not(.stop) {
  background: var(--btn-disabled-bg);
  cursor: not-allowed;
  opacity: 0.6;
}

.message-box {
  position: fixed;
  top: 20px;
  left: calc(50% + 100px);
  transform: translateX(-50%);
  max-width: 460px;
  padding: 0.4rem 0.75rem;
  border-radius: 8px;
  z-index: 1000;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.3);
  backdrop-filter: blur(10px);
  animation: slideDownFade 0.3s ease-out;
  word-wrap: break-word;
  overflow-wrap: break-word;
}

.message-box.success {
  background: var(--success-bg);
  border: 1px solid var(--success-border, rgba(74, 222, 128, 0.4));
  color: var(--success-text);
}

.message-box.error {
  background: var(--danger-bg);
  border: 1px solid var(--danger-border);
  border-left: 4px solid var(--status-disconnected);
  color: var(--danger-text);
}

.message-box.info {
  background: var(--info-bg);
  border: 1px solid var(--info-border);
  color: var(--info-text);
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

/* A long event name must not look truncated in the compact settings panel.
   If there is not enough room beside the label, the control moves to its own
   line instead of shrinking to an unreadable width. */
.current-action-summary {
  margin: 0 0 var(--ui-row-gap);
  padding-bottom: var(--ui-row-gap);
  border-bottom: 1px solid var(--color-border);
  color: var(--color-text-primary);
  overflow-wrap: anywhere;
}

.typing-action-section .typing-mode-row {
  display: flex;
  gap: var(--ui-row-label-gap-side);
}

.typing-mode-row .ui-label {
  white-space: nowrap;
}

.typing-mode-label {
  display: grid;
  flex-shrink: 0;
}

.typing-mode-label > span {
  grid-area: 1 / 1;
}

.typing-mode-label-sizer {
  visibility: hidden;
}

.typing-action-row .ui-input,
.typing-action-row .ui-select {
  flex: 1 1 180px;
  min-width: 180px;
}

.typing-action-row .typing-mode-select,
.typing-action-row .typing-param-input {
  flex: 0 0 180px;
  width: 180px;
  min-width: 180px;
  max-width: 180px;
}

.port-setting-row {
  flex-wrap: wrap;
  gap: var(--ui-row-label-gap-side);
  padding-top: var(--ui-row-gap);
  border-top: 1px solid var(--color-border);
}

.endpoint-inputs {
  flex: 1 1 360px;
  min-width: 0;
}

/* Host/port keep mono text; geometry and states come from ui-input. */
.port-setting-row .endpoint-input {
  flex: 0 1 auto;
  font-family: var(--font-mono);
  min-width: 0;
}

/* The host field uses the free space instead of a hard 190 px cap; the port
   stays narrow by purpose. */
.port-setting-row .endpoint-input-host {
  flex: 1 1 190px;
}

.port-setting-row .endpoint-input-port {
  flex: 0 0 84px;
  width: 84px;
  max-width: 84px;
}

.endpoint-error {
  padding-left: 70px;
}

.info-hint.test-description {
  margin: 0.25rem 0 var(--ui-row-gap);
}

.test-parameters-row {
  flex-wrap: wrap;
}

.test-parameters-row .ui-label {
  min-width: 50px;
}

.test-parameters-row .ui-input {
  flex: 1;
  min-width: 80px;
  max-width: 96px;
}


.ui-row.button-row {
  justify-content: flex-end;
  gap: 0.75rem;
  margin-top: 0.5rem;
  padding-top: 0.5rem;
  border-top: 1px solid var(--color-border);
}

/* Text actions keep their local accent surfaces; typography and geometry
   come from ui-button. */
.save-button-inline {
  gap: 0.4rem;
}

.icon-left {
  flex-shrink: 0;
}

.port-error {
  color: var(--danger-text-weak);
  margin-top: -0.5rem;
  margin-bottom: 1rem;
  padding-left: 82px;
}

.test-error {
  color: var(--danger-text-weak);
  margin-top: -0.5rem;
  margin-bottom: 1rem;
}

.hotkey-status {
  margin-bottom: 0.75rem;
  margin-top: -0.5rem;
  padding: 0.3rem 0.75rem;
  border-radius: 6px;
}

.hotkey-status.error {
  color: var(--danger-text-weak);
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border);
}

.typing-action-row .typing-target-select {
  flex: 0 0 280px;
  width: 280px;
  min-width: 280px;
  max-width: 280px;
}


.item-metadata {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem 1rem;
  margin: -0.35rem 0 1rem;
  color: var(--color-text-secondary);
  font-size: 0.85rem;
  font-weight: 400;
  overflow-wrap: anywhere;
}

.item-metadata code {
  color: var(--color-info);
}

.duplicate-warning {
  color: var(--danger-text-weak);
  font-weight: 600;
}

.item-warning {
  margin: 0 0 1rem;
  padding: 0.65rem 0.75rem;
  border: 1px solid var(--danger-border);
  border-radius: 8px;
  background: var(--danger-bg-weak);
  color: var(--danger-text-weak);
  line-height: 1.4;
  overflow-wrap: anywhere;
}

.info-hint {
  margin: 0.6rem 0 0;
}

.info-hint.item-selection-hint {
  margin-bottom: var(--ui-row-gap);
}

.info-hint em {
  font-style: normal;
  font-weight: 500;
  color: var(--color-text-secondary);
}

.info-hint code {
  font-family: var(--font-mono);
  background: var(--info-bg-weak);
  padding: 0.1rem 0.3rem;
  border-radius: 3px;
  font-size: 0.85rem;
}

.help-text {
  margin: 0.5rem 0;
}

.help-section > :last-child {
  margin-bottom: 0;
}

.help-text code {
  background: var(--info-bg-weak);
  padding: 0.2rem 0.4rem;
  border-radius: 4px;
  font-family: var(--font-mono);
  color: var(--color-info);
  border: 1px solid var(--info-border);
}

@media (max-width: 600px) {
  .typing-action-row label {
    flex: 0 1 auto;
  }

  .typing-action-row .typing-mode-select,
  .typing-action-row .typing-param-input,
  .typing-action-row .typing-target-select {
    flex: 0 1 auto;
    width: 100%;
    min-width: 0;
    max-width: 100%;
  }

  .ui-row {
    flex-direction: column;
    align-items: flex-start;
    gap: var(--ui-row-label-gap-stack);
  }

  .test-parameters-row {
    flex-direction: row;
    align-items: center;
    gap: var(--ui-row-label-gap-side);
  }

  .test-parameters-row .ui-input {
    width: auto;
    min-width: 80px;
    max-width: 96px;
  }

  .port-setting-row .endpoint-input-host {
    flex: 0 1 auto;
    width: 100%;
    max-width: 100%;
  }

  .endpoint-inputs {
    width: 100%;
  }

  .port-setting-row .endpoint-input-port {
    flex: 0 1 auto;
    width: 100px;
    max-width: 100px;
  }

  .endpoint-error {
    padding-left: 0;
  }
}
</style>
