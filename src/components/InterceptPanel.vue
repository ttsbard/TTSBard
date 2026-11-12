<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { createAsyncCleanupScope } from '../utils/asyncCleanup'
import { resolveInterceptKey, formatInterceptKey } from '../utils/interceptKeys'
import { normalizeCommandError } from '../ipc/commandError'
import { Crosshair, Trash2, Keyboard, Check, X, TriangleAlert } from 'lucide-vue-next'
import { t } from '../i18n'

const props = withDefaults(defineProps<{ active?: boolean }>(), { active: true })

interface InterceptBindingDto {
  key: string
  action: string
}

interface InterceptSettingsDto {
  enabled: boolean
  allow_any_key: boolean
  bindings: InterceptBindingDto[]
}

const isLoading = ref(false)
const settings = ref<InterceptSettingsDto | null>(null)
const recordingKey = ref(false)
const recordingBusy = ref(false)
let recordingGeneration = 0
let recordingOperation: Promise<void> = Promise.resolve()
const keyOccupied = ref(false)
let occupiedTimeoutId: ReturnType<typeof setTimeout> | null = null

function clearOccupied() {
  if (occupiedTimeoutId !== null) clearTimeout(occupiedTimeoutId)
  occupiedTimeoutId = null
  keyOccupied.value = false
}
const recordingKeyFor = ref<string | null>(null)
const savingAllowAnyKey = ref(false)
const newBindingAction = ref<string>('show_main_window')
const errorMessage = ref<string | null>(null)
const messageState = ref<'error' | 'success' | 'warning' | null>(null)
let messageTimeoutId: ReturnType<typeof setTimeout> | null = null
const listenerScope = createAsyncCleanupScope()

const ACTION_IDS = [
  'show_main_window',
  'show_soundpanel_window',
  'show_playback_control_window',
  'playback_pause',
  'playback_stop',
  'playback_repeat',
  'ocr_capture',
] as const

const ACTIONS = computed<{ value: string; label: string }[]>(() =>
  ACTION_IDS.map((id) => ({ value: id, label: t(`intercept.actions.${id}`) })),
)

async function loadSettings() {
  try {
    isLoading.value = true
    settings.value = await invoke<InterceptSettingsDto>('get_intercept_settings')
  } catch (e) {
    showMessage(t('intercept.error.load', { detail: (e as Error).message }), 'error')
  } finally {
    isLoading.value = false
  }
}

async function toggleEnabled() {
  if (!settings.value) return
  try {
    const newVal = !settings.value.enabled
    await invoke('set_intercept_enabled', { enabled: newVal })
  } catch (e) {
    showMessage(t('intercept.error.generic', { detail: (e as Error).message }), 'error')
  }
}

async function toggleAllowAnyKey(event: Event) {
  const input = event.target as HTMLInputElement
  if (!settings.value || !props.active || isLoading.value || savingAllowAnyKey.value || recordingBusy.value || recordingKey.value || recordingKeyFor.value) {
    input.checked = settings.value?.allow_any_key ?? false
    return
  }
  const next = !settings.value.allow_any_key
  input.checked = settings.value.allow_any_key
  savingAllowAnyKey.value = true
  try {
    await invoke('set_intercept_allow_any_key', { allowAnyKey: next })
    settings.value.allow_any_key = next
    await loadSettings()
  } catch (e) {
    showMessage(t('intercept.error.generic', { detail: normalizeCommandError(e).message }), 'error')
  } finally {
    savingAllowAnyKey.value = false
  }
}

async function restoreRecording(unregistered: boolean) {
  let failure: unknown
  try {
    await invoke('set_hotkey_recording', { recording: false })
  } catch (e) {
    failure = e
  }
  // A failed flag reset must not prevent restoring global registrations.
  if (unregistered) {
    try {
      await invoke('reregister_hotkeys_cmd')
    } catch (e) {
      failure ??= e
    }
  }
  if (failure && props.active && !listenerScope.disposed) {
    showMessage(t('intercept.error.generic', { detail: normalizeCommandError(failure).message }), 'error')
  }
}

function startRecordingKey(): Promise<void> {
  if (!settings.value || !props.active || listenerScope.disposed || isLoading.value || savingAllowAnyKey.value || recordingBusy.value || recordingKey.value || recordingKeyFor.value) return Promise.resolve()
  const generation = ++recordingGeneration
  recordingBusy.value = true
  clearOccupied()
  recordingKeyFor.value = null
  errorMessage.value = null
  recordingOperation = (async () => {
    let unregistered = false
    let started = false
    try {
      await invoke('set_hotkey_recording', { recording: true })
      if (!props.active || listenerScope.disposed || generation !== recordingGeneration) return
      // Even a partial unregister failure needs registration restoration.
      unregistered = true
      await invoke('unregister_hotkeys')
      if (!props.active || listenerScope.disposed || generation !== recordingGeneration) return
      recordingKey.value = true
      document.addEventListener('keydown', handleKeyDown, true)
      started = true
    } catch (e) {
      if (props.active && !listenerScope.disposed && generation === recordingGeneration) {
        showMessage(t('intercept.error.generic', { detail: normalizeCommandError(e).message }), 'error')
      }
    } finally {
      if (!started) await restoreRecording(unregistered)
      recordingBusy.value = false
    }
  })()
  return recordingOperation
}

function cancelRecordingKey(clearSelection = true): Promise<void> {
  recordingGeneration += 1
  const wasRecording = recordingKey.value
  clearOccupied()
  recordingKey.value = false
  if (clearSelection) recordingKeyFor.value = null
  document.removeEventListener('keydown', handleKeyDown, true)
  // An in-flight start owns its own rollback; wait before permitting a new one.
  if (!wasRecording) return recordingOperation
  recordingBusy.value = true
  recordingOperation = restoreRecording(true).finally(() => { recordingBusy.value = false })
  return recordingOperation
}

watch(() => props.active, (active) => {
  if (!active) {
    void cancelRecordingKey()
    recordingKeyFor.value = null
    newBindingAction.value = 'show_main_window'
  }
}, { flush: 'sync' })

function handleKeyDown(e: KeyboardEvent) {
  if (!recordingKey.value || !props.active) return

  e.preventDefault()
  e.stopImmediatePropagation()
  if (e.repeat) return

  const allowAnyKey = settings.value?.allow_any_key ?? false

  if (e.key === 'Escape' && !allowAnyKey) {
    void cancelRecordingKey()
    return
  }

  const canonicalName = resolveInterceptKey(e, allowAnyKey)
  if (canonicalName === null) {
    if (!allowAnyKey) {
      showMessage(t('intercept.error.numpad_only'), 'warning')
    }
    return
  }

  if (settings.value?.bindings.some((binding) => binding.key === canonicalName)) {
    clearOccupied()
    keyOccupied.value = true
    occupiedTimeoutId = setTimeout(clearOccupied, 2000)
    return
  }

  clearOccupied()
  recordingKeyFor.value = canonicalName
  void cancelRecordingKey(false)
}

async function saveBinding() {
  if (!recordingKeyFor.value || !settings.value || recordingBusy.value) return
  const key = recordingKeyFor.value
  const action = newBindingAction.value
  try {
    await invoke('set_intercept_binding', { key, action })
    await loadSettings()
  } catch (e) {
    showMessage(t('intercept.error.generic', { detail: (e as Error).message }), 'error')
  }
  recordingKeyFor.value = null
  newBindingAction.value = 'show_main_window'
}

async function updateBindingAction(binding: InterceptBindingDto, action: string) {
  try {
    await invoke('set_intercept_binding', { key: binding.key, action })
    await loadSettings()
  } catch (e) {
    showMessage(t('intercept.error.generic', { detail: (e as Error).message }), 'error')
  }
}

async function removeBinding(key: string) {
  try {
    await invoke('clear_intercept_binding', { key })
    await loadSettings()
  } catch (e) {
    showMessage(t('intercept.error.generic', { detail: (e as Error).message }), 'error')
  }
}

function showMessage(msg: string, type: 'error' | 'success' | 'warning') {
  errorMessage.value = msg
  messageState.value = type
  if (messageTimeoutId !== null) clearTimeout(messageTimeoutId)
  messageTimeoutId = setTimeout(() => {
    errorMessage.value = null
    messageState.value = null
    messageTimeoutId = null
  }, 3000)
}

onMounted(async () => {
  await loadSettings()
  // Payload приходит как весь AppEvent enum ({"InterceptionChanged": <bool>}),
  // а НЕ как чистый bool. Извлекаем реальное значение, иначе toggle «залипает»
  // во включённом виде (объект всегда truthy).
  await listenerScope.track(
    listen<unknown>('interception-changed', (event) => {
      if (!settings.value) return
      const payload = event.payload as { InterceptionChanged?: boolean } | boolean | null
      if (typeof payload === 'boolean') {
        settings.value.enabled = payload
      } else if (payload && typeof payload.InterceptionChanged === 'boolean') {
        settings.value.enabled = payload.InterceptionChanged
      }
    }),
  )
})

onUnmounted(() => {
  listenerScope.dispose()
  void cancelRecordingKey()
  clearOccupied()
  if (messageTimeoutId !== null) clearTimeout(messageTimeoutId)
  document.removeEventListener('keydown', handleKeyDown, true)
})
</script>

<template>
  <div class="intercept-panel">
    <div v-if="errorMessage" class="message-box" :class="messageState">
      {{ errorMessage }}
    </div>

    <div class="setting-section ui-section">
      <!-- Toggle -->
      <div class="toggle-row">
        <div class="toggle-label ui-section-title">
          <Crosshair :size="18" />
          <span>{{ t('intercept.title') }}</span>
        </div>
        <label class="toggle-switch">
          <input
            type="checkbox"
            :checked="settings?.enabled ?? false"
            @change="toggleEnabled"
          />
          <span class="toggle-slider" />
        </label>
      </div>

      <p class="hint-text ui-description">
        {{ t('intercept.description') }}
      </p>

      <p class="hint-text ui-description">
        {{ settings?.allow_any_key ? t('intercept.hint_unrestricted') : t('intercept.hint') }}
      </p>

      <div class="unrestricted-row">
        <label class="ui-choice-label">
          <input
            type="checkbox"
            class="ui-choice-input"
            :checked="settings?.allow_any_key ?? false"
      :disabled="!settings || isLoading || savingAllowAnyKey || recordingBusy || recordingKey || !!recordingKeyFor"
            @change="toggleAllowAnyKey"
          />
          <span>{{ t('intercept.allow_any_key') }}</span>
        </label>
      </div>
      <p class="unrestricted-warning ui-hint ui-hint--choice">
        {{ t('intercept.allow_any_key_warning') }}
      </p>

      <!-- Bindings list -->
      <div class="bindings-section">
        <div class="bindings-header">
          <span class="section-title ui-group-title">{{ t('intercept.bindings') }}</span>
          <div class="record-actions">
          <button
            v-if="!recordingKey && !recordingKeyFor"
            @click="startRecordingKey"
            :disabled="!settings || !active || isLoading || savingAllowAnyKey || recordingBusy"
            class="record-btn ui-icon-button"
            :title="t('intercept.record')"
            :aria-label="t('intercept.record')"
          >
            <Keyboard :size="18" />
          </button>
          <button
            v-if="recordingKey"
            class="record-btn recording ui-icon-button"
            disabled
            aria-live="polite"
          >
            <TriangleAlert v-if="keyOccupied" :size="18" />
            <Keyboard v-else :size="18" />
            <span>{{ keyOccupied ? t('intercept.key_occupied') : t('hotkeys.action.press') }}</span>
          </button>
          <button
            v-if="recordingKey || recordingBusy"
            @click="cancelRecordingKey()"
            class="ui-icon-button"
            :title="t('common.cancel')"
            :aria-label="t('common.cancel')"
          >
            <X :size="18" />
          </button>
          </div>
        </div>

        <!-- New binding confirmation -->
        <div v-if="recordingKeyFor" class="new-binding-row">
          <span class="key-badge">{{ formatInterceptKey(recordingKeyFor) }}</span>
          <span class="arrow">→</span>
          <select v-model="newBindingAction" class="ui-select action-select">
            <option v-for="a in ACTIONS" :key="a.value" :value="a.value">
              {{ a.label }}
            </option>
          </select>
          <button @click="saveBinding" :disabled="recordingBusy" class="ui-icon-button" :title="t('common.add')" :aria-label="t('common.add')">
            <Check :size="18" />
          </button>
          <button @click="(recordingKeyFor = null, newBindingAction = 'show_main_window')" class="ui-icon-button" :title="t('common.cancel')" :aria-label="t('common.cancel')">
            <X :size="18" />
          </button>
        </div>

        <div v-if="settings && settings.bindings.length === 0 && !recordingKeyFor" class="empty-hint ui-hint">
          {{ settings.allow_any_key ? t('intercept.empty_hint_unrestricted') : t('intercept.empty_hint') }}
        </div>

        <div v-if="settings?.bindings.length" class="ui-menu ui-menu--embedded">
        <div v-for="binding in settings?.bindings ?? []" :key="binding.key" class="binding-row">
          <span class="key-badge">{{ formatInterceptKey(binding.key) }}</span>
          <span class="arrow">→</span>
          <select
            :value="binding.action"
            @change="updateBindingAction(binding, ($event.target as HTMLSelectElement).value)"
            class="ui-select action-select"
          >
            <option v-for="a in ACTIONS" :key="a.value" :value="a.value">
              {{ a.label }}
            </option>
          </select>
          <button
            @click="removeBinding(binding.key)"
            class="remove-btn ui-icon-button ui-icon-button--adjacent ui-action--danger"
            :title="t('intercept.clear_binding')"
            :aria-label="t('intercept.clear_binding')"
          >
            <Trash2 :size="18" />
          </button>
        </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.intercept-panel {
  max-width: 900px;
  margin: 0 auto;
}

.message-box {
  position: fixed;
  top: 20px;
  left: calc(50% + 100px);
  transform: translateX(-50%);
  padding: 0.4rem 0.75rem;
  border-radius: 8px;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  z-index: 1000;
  box-shadow: var(--dialog-shadow);
  backdrop-filter: blur(10px);
  animation: slideDownFade 0.3s ease-out;
}

.message-box.error {
  background: var(--danger-bg);
  border: 1px solid var(--danger-border);
  color: var(--danger-text);
}

.message-box.success {
  background: var(--success-bg);
  border: 1px solid var(--success-border);
  color: var(--success-text);
}

.message-box.warning {
  background: var(--warning-bg);
  border: 1px solid var(--warning-border);
  color: var(--warning-text-bright);
}

.setting-section {
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
}

.toggle-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--color-border);
  margin-bottom: 1rem;
}

.toggle-label {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  color: var(--color-text-primary);
}

.toggle-switch {
  position: relative;
  display: inline-block;
  width: 44px;
  height: 24px;
}

.toggle-switch input {
  opacity: 0;
  width: 0;
  height: 0;
}

.toggle-slider {
  position: absolute;
  cursor: pointer;
  inset: 0;
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border);
  border-radius: 24px;
  transition: 0.25s;
}

.toggle-slider::before {
  content: '';
  position: absolute;
  height: 18px;
  width: 18px;
  left: 2px;
  bottom: 2px;
  background: var(--color-text-secondary);
  border-radius: 50%;
  transition: 0.25s;
}

.toggle-switch input:checked + .toggle-slider {
  background: var(--color-accent);
  border-color: var(--color-accent);
}

.toggle-switch input:checked + .toggle-slider::before {
  transform: translateX(20px);
  background: var(--color-text-white);
}

.hint-text {
  margin: 0 0 var(--ui-row-gap);
}

.unrestricted-row {
  margin-bottom: var(--ui-hint-gap);
}

.unrestricted-warning {
  margin-top: 0;
  margin-bottom: var(--ui-row-gap);
}

.bindings-section {
  margin-top: 8px;
}

.bindings-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}

.section-title {
  color: var(--color-text-primary);
}

.record-btn {
  gap: var(--ui-field-group-gap);
}

.record-actions {
  display: flex;
  align-items: center;
  gap: var(--ui-field-group-gap);
}

.record-btn.recording {
  width: auto;
  padding: 0 var(--ui-control-padding-x);
  animation: pulse 1s infinite;
  background: var(--warning-bg);
  border-color: var(--warning-border);
}

.record-btn.recording:disabled {
  opacity: 1;
}

.new-binding-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  padding: var(--ui-menu-item-padding-y) calc(var(--ui-menu-item-padding-x) + var(--ui-menu-padding));
  background: var(--color-bg-field);
  border: 1px solid var(--color-accent);
  border-radius: 8px;
}

.empty-hint {
  padding: 12px 0;
}

.binding-row {
  display: flex;
  align-items: center;
  gap: var(--ui-field-group-gap);
  padding: var(--ui-menu-item-padding-y) var(--ui-menu-item-padding-x);
}

.key-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: var(--ui-control-min-height);
  box-sizing: border-box;
  flex-shrink: 0;
  padding: 0.25rem 0.6rem;
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 6px;
  font-family: var(--font-mono);
  font-size: 0.85rem;
  color: var(--color-text-primary);
  min-width: 80px;
  text-align: center;
}

.arrow {
  color: var(--color-text-muted);
  font-size: 0.9rem;
}

.action-select {
  flex: 1;
  min-width: 0;
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.7; }
}
</style>
