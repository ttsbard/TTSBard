<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Play, Square } from 'lucide-vue-next'
import { t } from '../i18n'
import type { VrchatSettingsDto } from '../composables/useAppSettings'

const saved = ref<VrchatSettingsDto | null>(null)
const host = ref('127.0.0.1')
const port = ref('9000')
const startOnBoot = ref(false)
const busy = ref(false)
const message = ref('')
const error = ref('')
const testText = ref('')
const sendingTest = ref(false)
let messageTimer: ReturnType<typeof setTimeout> | null = null

const active = computed(() => saved.value?.enabled === true)
const hostError = computed(() => {
  const value = host.value.trim()
  return !value || value.length > 253 || /\s|\/|\\|:\/\//.test(value)
    ? t('vrchat.error.host_invalid')
    : ''
})
const portError = computed(() => {
  const value = Number(port.value)
  return !/^\d+$/.test(port.value) || !Number.isInteger(value) || value < 1024 || value > 65535
    ? t('vrchat.error.port_invalid')
    : ''
})
const validEndpoint = computed(() => !hostError.value && !portError.value)

async function loadSettings() {
  busy.value = true
  try {
    const settings = await invoke<VrchatSettingsDto>('get_vrchat_settings')
    saved.value = settings
    host.value = settings.host
    port.value = String(settings.port)
    startOnBoot.value = settings.start_on_boot
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}

async function persist(enabled: boolean, useDraft: boolean) {
  if (busy.value || !saved.value || (useDraft && !validEndpoint.value)) return
  busy.value = true
  error.value = ''
  message.value = ''
  if (messageTimer) clearTimeout(messageTimer)
  const settings: VrchatSettingsDto = useDraft
    ? { enabled, start_on_boot: startOnBoot.value, host: host.value.trim(), port: Number(port.value) }
    : { ...saved.value, enabled, start_on_boot: startOnBoot.value }
  try {
    await invoke<string>('save_vrchat_settings', { settings })
    saved.value = settings
    message.value = t('vrchat.status.saved')
    messageTimer = setTimeout(() => { message.value = '' }, 3000)
  } catch (cause) {
    startOnBoot.value = saved.value.start_on_boot
    error.value = t('vrchat.error.save', { error: String(cause) })
  } finally {
    busy.value = false
  }
}

function saveStartOnBoot() {
  void persist(active.value, false)
}

async function sendTest() {
  const text = testText.value.trim()
  if (!active.value || !text || sendingTest.value) return
  sendingTest.value = true
  message.value = ''
  error.value = ''
  if (messageTimer) clearTimeout(messageTimer)
  try {
    await invoke<boolean>('send_vrchat_text', { text })
    message.value = t('vrchat.test.success')
    messageTimer = setTimeout(() => { message.value = '' }, 3000)
  } catch (cause) {
    error.value = t('vrchat.test.error', { error: String(cause) })
  } finally {
    sendingTest.value = false
  }
}

onMounted(() => { void loadSettings() })
onUnmounted(() => { if (messageTimer) clearTimeout(messageTimer) })
</script>

<template>
  <div class="vrchat-panel">
    <div v-if="error" class="message-box ui-status error" role="alert">{{ error }}</div>
    <div v-if="message" class="message-box ui-status success" role="status">{{ message }}</div>

    <section class="settings-section ui-section">
      <div class="section-header server-header">
        <h2 class="ui-section-title">{{ t('vrchat.connection') }}</h2>
        <div class="server-status">
          <span class="status-indicator ui-status" :class="active ? 'running' : 'disabled'">
            {{ active ? t('vrchat.status.active') : t('vrchat.status.disabled') }}
          </span>
          <button
            class="status-button start ui-icon-button ui-icon-button--accent"
            :disabled="busy || active || !validEndpoint"
            :title="t('vrchat.start')"
            :aria-label="t('vrchat.start')"
            @click="persist(true, true)"
          ><Play :size="18" /></button>
          <button
            class="status-button stop ui-icon-button ui-action--stop"
            :disabled="busy || !active"
            :title="t('vrchat.stop')"
            :aria-label="t('vrchat.stop')"
            @click="persist(false, false)"
          ><Square :size="18" /></button>
        </div>
      </div>
      <div class="ui-row">
        <label class="ui-choice-label">
          <input v-model="startOnBoot" type="checkbox" class="ui-choice-input"
            :disabled="busy || !saved" @change="saveStartOnBoot" />
          <span>{{ t('vrchat.start_on_boot') }}</span>
        </label>
      </div>
      <div class="ui-row port-setting-row">
        <label for="vrchat-host" class="ui-label">{{ t('vrchat.host') }}</label>
        <div class="endpoint-inputs ui-field-group">
          <input id="vrchat-host" v-model="host" class="ui-input endpoint-input endpoint-input-host"
            type="text" spellcheck="false" autocomplete="off" placeholder="127.0.0.1"
            :disabled="busy" :aria-invalid="hostError ? 'true' : undefined" />
          <input id="vrchat-port" v-model="port" class="ui-input endpoint-input endpoint-input-port"
            type="number" min="1024" max="65535" placeholder="9000"
            :aria-label="t('vrchat.port')" :title="t('vrchat.port')"
            :disabled="busy" :aria-invalid="portError ? 'true' : undefined" />
          <button class="save-button-inline ui-button ui-button--primary"
            :disabled="busy || !validEndpoint || !saved"
            @click="persist(active, true)">{{ t('common.save') }}</button>
        </div>
      </div>
      <div v-if="hostError" class="endpoint-error ui-status" role="alert">{{ hostError }}</div>
      <div v-if="portError" class="endpoint-error ui-status" role="alert">{{ portError }}</div>
    </section>

    <section class="settings-section ui-section">
      <h2 class="ui-section-title">{{ t('vrchat.test.title') }}</h2>
      <div class="test-row ui-field-group">
        <input v-model="testText" class="ui-input test-input" type="text"
          :placeholder="t('vrchat.test.placeholder')" :disabled="!active || sendingTest"
          @keydown.enter.prevent="sendTest" />
        <button class="ui-button ui-button--primary test-send-btn"
          :disabled="!active || sendingTest || !testText.trim()"
          @click="sendTest">{{ sendingTest ? t('vrchat.test.sending') : t('vrchat.test.send') }}</button>
      </div>
    </section>

    <section class="settings-section ui-section">
      <h2 class="ui-section-title">{{ t('vrchat.help.title') }}</h2>
      <p class="ui-description help-text">{{ t('vrchat.help.osc_enabled') }}</p>
    </section>
  </div>
</template>

<style scoped>
.vrchat-panel {
  max-width: 900px;
  margin: 0 auto;
}
h2 {
  margin-top: 0;
  margin-bottom: 1rem;
  color: var(--color-text-primary);
}
.settings-section {
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
}
.ui-row { flex-wrap: wrap; }
.ui-row:last-child { margin-bottom: 0; }
.ui-row label { color: var(--color-text-secondary); }
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
  overflow-wrap: break-word;
}
@keyframes slideDownFade {
  from { opacity: 0; transform: translateX(-50%) translateY(-20px); }
  to { opacity: 1; transform: translateX(-50%) translateY(0); }
}
.error {
  background: var(--danger-bg-weak, rgba(239, 68, 68, 0.1));
  color: var(--danger-text-weak, #ef4444);
  border: 1px solid var(--danger-border, rgba(239, 68, 68, 0.2));
}
.success {
  background: var(--color-success-bg, rgba(34, 197, 94, 0.1));
  color: var(--color-success-text, #22c55e);
  border: 1px solid var(--color-success-border, rgba(34, 197, 94, 0.2));
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
.server-header, .server-status, .endpoint-inputs, .test-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}
.server-header {
  justify-content: space-between;
  align-items: flex-start;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--color-border);
  margin-bottom: 1rem;
}
.server-header h2 { margin: 0; }
.server-status { margin-top: -2px; }
.status-indicator {
  display: flex;
  align-items: center;
  height: 28px;
  padding: 0.15rem 0.5rem;
  border-radius: 5px;
  border: 1px solid var(--color-border);
  background: var(--color-bg-field);
}
.status-indicator.running {
  color: var(--success-text-bright);
  background: var(--success-bg-weak);
  border-color: var(--success-shadow);
}
.status-indicator.disabled { color: var(--color-text-secondary); }
.port-setting-row { display: flex; align-items: center; flex-wrap: wrap; gap: var(--ui-row-label-gap-side); padding-top: var(--ui-row-gap); border-top: 1px solid var(--color-border); }
.endpoint-inputs { flex: 1 1 360px; min-width: 0; flex-wrap: wrap; }
.port-setting-row .endpoint-input { flex: 0 1 auto; font-family: var(--font-mono); min-width: 0; }
.port-setting-row .endpoint-input-host { flex: 1 1 190px; }
.endpoint-input-port { flex: 0 0 84px; width: 84px; max-width: 84px; }
.endpoint-error { color: var(--danger-text-weak, #ef4444); font-size: 0.85rem; margin-top: 0.25rem; padding-left: 70px; }
.test-input { flex: 1 1 240px; min-width: 140px; }
.help-text { margin: 0; line-height: 1.45; }
@media (max-width: 600px) {
  .port-setting-row { flex-direction: column; align-items: flex-start; gap: var(--ui-row-label-gap-stack); }
  .endpoint-inputs { width: 100%; }
  .port-setting-row .endpoint-input-host { flex: 0 1 auto; width: 100%; max-width: 100%; }
  .port-setting-row .endpoint-input-port { flex: 0 1 auto; width: 100px; max-width: 100px; }
  .endpoint-error { padding-left: 0; }
}
</style>
