<script setup lang="ts">
import { ref, computed, onUnmounted, watch, type Component } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Keyboard, RotateCcw, AppWindow, Music, MonitorPlay, SquarePen, ScanLine } from 'lucide-vue-next'
import type { HotkeyDto } from '../types/settings'
import { useAppSettings } from '../composables/useAppSettings'
import { debugError } from '../utils/debug'
import { normalizeCommandError } from '../ipc/commandError'
import { t } from '../i18n'

const { settings, isLoading, reload } = useAppSettings()

const props = withDefaults(defineProps<{ active?: boolean }>(), { active: true })
let recordingGeneration = 0
let recordingStartPending = false

watch(() => props.active, active => {
  if (!active) {
    recordingGeneration++
    if (recordingFor.value) void cancelRecording()
  }
}, { flush: 'sync' })

const hotkeys = computed(() => settings.value?.hotkeys)

type HotkeyName = 'main_window' | 'sound_panel' | 'playback_control_window' | 'return_previous_window' | 'toggle_minimal_mode' | 'ocr_capture'

const EDITOR_HOTKEY_NAMES = ['edit_word', 'submit_continue', 'submit_keep_text', 'submit_keep_focus', 'next_spelling_error', 'previous_spelling_error', 'next_tab', 'previous_tab', 'cycle_route', 'toggle_typing', 'cycle_quick_mode', 'toggle_history', 'accent_homographs', 'approve_next_incoming', 'edit_next_incoming'] as const
type EditorHotkeyName = (typeof EDITOR_HOTKEY_NAMES)[number]

function isEditorHotkeyName(name: string): name is EditorHotkeyName {
  return (EDITOR_HOTKEY_NAMES as readonly string[]).includes(name)
}

type HotkeyMessageType = 'error' | 'success' | 'warning' | null

interface HotkeyRow {
  name: HotkeyName | EditorHotkeyName
  icon?: Component
}

const GLOBAL_HOTKEY_ROWS: HotkeyRow[] = [
  { name: 'main_window', icon: AppWindow },
  { name: 'sound_panel', icon: Music },
  { name: 'playback_control_window', icon: MonitorPlay },
  { name: 'ocr_capture', icon: ScanLine },
]

const MAIN_WINDOW_HOTKEY_ROWS: HotkeyRow[] = [
  { name: 'toggle_minimal_mode' },
  { name: 'return_previous_window' },
]

const EDITOR_HOTKEY_ROWS: HotkeyRow[] = [
  { name: 'edit_word' },
  { name: 'submit_continue' },
  { name: 'submit_keep_text' },
  { name: 'submit_keep_focus' },
  { name: 'next_spelling_error' },
  { name: 'previous_spelling_error' },
  { name: 'next_tab' },
  { name: 'previous_tab' },
  { name: 'cycle_route' },
  { name: 'toggle_typing' },
  { name: 'cycle_quick_mode' },
  { name: 'toggle_history' },
  { name: 'accent_homographs' },
]

const INCOMING_HOTKEY_ROWS: HotkeyRow[] = [
  { name: 'approve_next_incoming' },
  { name: 'edit_next_incoming' },
]

const recordingFor = ref<HotkeyName | EditorHotkeyName | null>(null)
const pendingToggle = ref<HotkeyName | EditorHotkeyName | null>(null)
const keyOccupied = ref(false)
let occupiedTimeoutId: ReturnType<typeof setTimeout> | null = null

function clearOccupied() {
  if (occupiedTimeoutId !== null) clearTimeout(occupiedTimeoutId)
  occupiedTimeoutId = null
  keyOccupied.value = false
}

function getBinding(name: HotkeyName | EditorHotkeyName): HotkeyDto | undefined {
  const all = hotkeys.value
  if (!all) return undefined
  return isEditorHotkeyName(name) ? all.editor[name] : all[name]
}

// A missing `enabled` comes from a legacy payload and means "enabled": the
// backend omits the default `true` on the wire.
function isHotkeyEnabled(binding: HotkeyDto | undefined | null): boolean {
  return binding?.enabled !== false
}

function formattedHotkey(name: HotkeyName | EditorHotkeyName): string {
  const binding = getBinding(name)
  return binding ? formatHotkey(binding) : ''
}

function hotkeyName(name: HotkeyName | EditorHotkeyName): string {
  return t(`hotkeys.name.${name}`)
}

function toggleTitle(name: HotkeyName | EditorHotkeyName): string {
  const state = isHotkeyEnabled(getBinding(name)) ? t('editor.typing.on') : t('editor.typing.off')
  return `${hotkeyName(name)}: ${state}`
}

function bindingOccupied(candidate: HotkeyDto): boolean {
  const all = hotkeys.value
  if (!all) return false
  const matches = (binding: HotkeyDto) => isHotkeyEnabled(binding)
    && binding.key.toUpperCase() === candidate.key.toUpperCase()
    && binding.modifiers.length === candidate.modifiers.length
    && binding.modifiers.every(modifier => candidate.modifiers.includes(modifier))
  return Object.entries(all).some(([name, binding]) =>
    name !== 'editor' && name !== recordingFor.value && matches(binding as HotkeyDto),
  ) || Object.entries(all.editor).some(([name, binding]) =>
    name !== recordingFor.value && matches(binding),
  )
}

const errorMessage = ref<string | null>(null)
const messageState = ref<HotkeyMessageType>(null)
const currentRecording = ref<{ modifiers: HotkeyDto['modifiers']; key: string } | null>(null)
let messageTimeoutId: ReturnType<typeof setTimeout> | null = null

// Start recording a hotkey
async function startRecording(name: HotkeyName) {
  if (!props.active || recordingStartPending) return
  const generation = recordingGeneration
  recordingStartPending = true
  try {
    // Устанавливаем флаг записи (блокирует выполнение хоткеев)
    await invoke('set_hotkey_recording', { recording: true })

    // Отключаем все глобальные хоткеи для надежности
    await invoke('unregister_hotkeys')

    if (!props.active || generation !== recordingGeneration) {
      await invoke('set_hotkey_recording', { recording: false })
      await invoke('reregister_hotkeys_cmd')
      return
    }

    recordingFor.value = name
    errorMessage.value = null
    currentRecording.value = { modifiers: [], key: '' }

    // Listen for keydown (to capture) and keyup (to finish)
    document.addEventListener('keydown', handleKeyDown)
    document.addEventListener('keyup', handleKeyUp)
  } catch (e) {
    showMessage(t('hotkeys.error_prefix', { detail: normalizeCommandError(e).message }), 'error')
    // Сбрасываем флаг при ошибке
    try {
      await invoke('set_hotkey_recording', { recording: false })
    } catch {}
  } finally {
    recordingStartPending = false
  }
}

// Start recording an editor-scoped hotkey (no global unregister/reregister)
async function startEditorRecording(name: EditorHotkeyName) {
  if (!props.active || recordingStartPending) return
  const generation = recordingGeneration
  recordingStartPending = true
  try {
    // Устанавливаем флаг записи (блокирует выполнение хоткеев)
    await invoke('set_hotkey_recording', { recording: true })

    if (!props.active || generation !== recordingGeneration) {
      await invoke('set_hotkey_recording', { recording: false })
      return
    }

    // Editor hotkeys не регистрируются глобально, поэтому unregister/reregister не нужны

    recordingFor.value = name
    errorMessage.value = null
    currentRecording.value = { modifiers: [], key: '' }

    // Listen for keydown (to capture) and keyup (to finish)
    document.addEventListener('keydown', handleKeyDown)
    document.addEventListener('keyup', handleKeyUp)
  } catch (e) {
    showMessage(t('hotkeys.error_prefix', { detail: normalizeCommandError(e).message }), 'error')
    // Сбрасываем флаг при ошибке
    try {
      await invoke('set_hotkey_recording', { recording: false })
    } catch {}
  } finally {
    recordingStartPending = false
  }
}

function startRecordingFor(name: HotkeyName | EditorHotkeyName) {
  if (isEditorHotkeyName(name)) {
    void startEditorRecording(name)
  } else {
    void startRecording(name)
  }
}

async function cancelRecording() {
  clearOccupied()
  const wasEditor = recordingFor.value !== null && isEditorHotkeyName(recordingFor.value)
  recordingFor.value = null
  currentRecording.value = null

  document.removeEventListener('keydown', handleKeyDown)
  document.removeEventListener('keyup', handleKeyUp)

  // Сбрасываем флаг записи и восстанавливаем хоткеи
  try {
    await invoke('set_hotkey_recording', { recording: false })
    // Для editor-хоткеев глобальные хоткеи не отключались — reregister не нужен
    if (!wasEditor) {
      await invoke('reregister_hotkeys_cmd')
    }
  } catch {
    // Игнорируем ошибки при восстановлении хоткеев при отмене записи
  }
}

function codeToKey(code: string): string {
  if (code === 'Space') return 'SPACE'
  if (code === 'Enter') return 'Enter'
  if (code === 'Tab') return 'Tab'
  if (/^F\d+$/.test(code)) return code
  if (/^Key[A-Z]$/.test(code)) return code[3]
  if (/^Digit\d$/.test(code)) return code[5]
  return ''
}

function handleKeyDown(e: KeyboardEvent) {
  if (!recordingFor.value) return

  // Cancel on Escape
  if (e.key === 'Escape') {
    cancelRecording()
    return
  }

  e.preventDefault()

  // Capture modifiers
  const modifiers: HotkeyDto['modifiers'] = []
  if (e.ctrlKey) modifiers.push('ctrl')
  if (e.shiftKey) modifiers.push('shift')
  if (e.altKey) modifiers.push('alt')
  if (e.metaKey) modifiers.push('super')

  const key = codeToKey(e.code)
  if (key === '') {
    // Modifier-only and unsupported keys must never become a saved shortcut.
    currentRecording.value = { modifiers, key: '' }
    return
  }
  // Update recording with the main key
  currentRecording.value = { modifiers, key }
}

function handleKeyUp(e: KeyboardEvent) {
  if (!recordingFor.value || !currentRecording.value) return

  const releasedKey = codeToKey(e.code)
  if (releasedKey === '') return
  // Only finish if we're releasing the main key we captured
  if (currentRecording.value.key !== '' && releasedKey === currentRecording.value.key) {
    if (bindingOccupied(currentRecording.value)) {
      clearOccupied()
      keyOccupied.value = true
      occupiedTimeoutId = setTimeout(clearOccupied, 2000)
      currentRecording.value = { modifiers: [], key: '' }
      return
    }
    clearOccupied()
    // Save the hotkey
    saveHotkey(recordingFor.value, {
      modifiers: currentRecording.value.modifiers,
      key: currentRecording.value.key
    })

    // Cleanup
    document.removeEventListener('keydown', handleKeyDown)
    document.removeEventListener('keyup', handleKeyUp)
    currentRecording.value = null
  }
}

async function saveHotkey(name: HotkeyName | EditorHotkeyName, hotkey: HotkeyDto) {
  try {
    // Persist the current switch state explicitly so the backend never has to
    // infer it while replacing the combination.
    const payload: HotkeyDto = { ...hotkey, enabled: isHotkeyEnabled(getBinding(name)) }
    if (isEditorHotkeyName(name)) {
      await invoke('set_editor_hotkey', { actionId: name, hotkey: payload })
    } else {
      await invoke('set_hotkey', { name, hotkey: payload })
    }
    await reload()
    // set_hotkey уже вызывает reregister_hotkeys внутри, set_editor_hotkey — нет
    // Сбрасываем флаг записи
    await invoke('set_hotkey_recording', { recording: false })
  } catch (e) {
    // При ошибке нужно восстановить хоткеи вручную
    showMessage(t('hotkeys.error_prefix', { detail: normalizeCommandError(e).message }), 'error')
    try {
      await invoke('set_hotkey_recording', { recording: false })
      if (!isEditorHotkeyName(name)) {
        await invoke('reregister_hotkeys_cmd')
      }
    } catch {
      // Игнорируем ошибки при восстановлении хоткеев
    }
  } finally {
    recordingFor.value = null
    currentRecording.value = null
  }
}

// Toggle one hotkey without touching its stored combination.
async function toggleHotkeyEnabled(name: HotkeyName | EditorHotkeyName, event: Event) {
  if (pendingToggle.value !== null) return
  const input = event.target as HTMLInputElement | null
  if (!input) return
  const next = input.checked

  pendingToggle.value = name
  try {
    if (isEditorHotkeyName(name)) {
      await invoke('set_editor_hotkey_enabled', { actionId: name, enabled: next })
    } else {
      await invoke('set_hotkey_binding_enabled', { name, enabled: next })
    }
    await reload()

    if (name === 'ocr_capture' && !next && settings.value?.ocr?.enabled) {
      // OCR itself stays enabled; only its global shortcut is released until
      // the hotkey is switched back on.
      showMessage(
        `${t('hotkeys.name.ocr_capture')}: ${t('ocr.status.disabled')} — ${t('ocr.failure.runtime_unavailable')}`,
        'warning',
      )
    }
  } catch (e) {
    // The backend rejected the transition (for example an enabled hotkey
    // already owns this combination): put the switch back and show the error.
    input.checked = !next
    showMessage(t('hotkeys.error_prefix', { detail: normalizeCommandError(e).message }), 'error')
    await reload()
  } finally {
    pendingToggle.value = null
  }
}

// Reset to default
async function resetToDefault(name: HotkeyName) {
  try {
    await invoke('reset_hotkey_to_default', { name })
    await reload()
    showMessage(t('hotkeys.reset_success'), 'success')
  } catch (e) {
    showMessage(t('hotkeys.error_prefix', { detail: normalizeCommandError(e).message }), 'error')
  }
}

// Reset editor hotkey to default
async function resetEditorToDefault(name: EditorHotkeyName) {
  try {
    await invoke('reset_editor_hotkey', { actionId: name })
    await reload()
    showMessage(t('hotkeys.reset_success'), 'success')
  } catch (e) {
    showMessage(t('hotkeys.error_prefix', { detail: normalizeCommandError(e).message }), 'error')
  }
}

function resetToDefaultFor(name: HotkeyName | EditorHotkeyName) {
  if (isEditorHotkeyName(name)) {
    void resetEditorToDefault(name)
  } else {
    void resetToDefault(name)
  }
}

function formatHotkey(hotkey: HotkeyDto): string {
  const modMap: Record<string, string> = { ctrl: 'Ctrl', shift: 'Shift', alt: 'Alt', super: 'Win' }
  const mods = hotkey.modifiers.map(m => modMap[m])
  if (mods.length === 0) {
    return hotkey.key
  }
  return `${mods.join('+')}+${hotkey.key}`
}

function formatCurrentRecording(): string {
  if (!currentRecording.value) return ''
  const modMap: Record<string, string> = { ctrl: 'Ctrl', shift: 'Shift', alt: 'Alt', super: 'Win' }
  const mods = currentRecording.value.modifiers.map(m => modMap[m])
  if (mods.length === 0 && currentRecording.value.key === '') {
    return '...'
  }
  if (currentRecording.value.key === '') {
    return mods.length > 0 ? `${mods.join('+')}+?` : '...'
  }
  return mods.length > 0 ? `${mods.join('+')}+${currentRecording.value.key}` : currentRecording.value.key
}

function showMessage(msg: string, type: Exclude<HotkeyMessageType, null>) {
  errorMessage.value = msg
  messageState.value = type

  if (messageTimeoutId !== null) {
    clearTimeout(messageTimeoutId)
  }
  messageTimeoutId = setTimeout(() => {
    errorMessage.value = null
    messageState.value = null
    messageTimeoutId = null
  }, 3000)
}

// Cleanup on unmount
onUnmounted(async () => {
  clearOccupied()
  recordingGeneration++
  if (messageTimeoutId !== null) {
    clearTimeout(messageTimeoutId)
    messageTimeoutId = null
  }

  document.removeEventListener('keydown', handleKeyDown)
  document.removeEventListener('keyup', handleKeyUp)

  // Если компонент размонтируется во время записи, сбрасываем флаг и восстанавливаем хоткеи
  if (recordingFor.value) {
    const wasEditor = isEditorHotkeyName(recordingFor.value)
    try {
      await invoke('set_hotkey_recording', { recording: false })
      if (!wasEditor) {
        await invoke('reregister_hotkeys_cmd')
      }
    } catch (e) {
        debugError('Failed to cleanup on unmount:', e)
    }
  }
})
</script>

<template>
  <div class="hotkeys-panel">
    <!-- Error/Success/Warning Message Display -->
    <div v-if="errorMessage" class="message-box" :class="messageState">
      {{ errorMessage }}
    </div>

    <!-- Single section for all hotkeys -->
    <div class="setting-section">
      <div class="section-header">
        <Keyboard :size="18" class="section-icon" />
        <span class="section-title ui-section-title">{{ t('hotkeys.section.global') }}</span>
      </div>

      <div v-for="row in GLOBAL_HOTKEY_ROWS" :key="row.name" class="hotkey-row">
        <label class="toggle-switch" :title="toggleTitle(row.name)">
          <input
            type="checkbox"
            :data-hotkey="row.name"
            :checked="isHotkeyEnabled(getBinding(row.name))"
            :disabled="isLoading || recordingFor !== null || pendingToggle !== null || !hotkeys"
            :aria-label="toggleTitle(row.name)"
            @change="toggleHotkeyEnabled(row.name, $event)"
          />
          <span class="toggle-slider" />
        </label>
        <div class="hotkey-label ui-label">
          <component :is="row.icon" v-if="row.icon" :size="16" />
          <span>{{ hotkeyName(row.name) }}</span>
        </div>
        <div class="hotkey-actions">
          <span
            v-if="hotkeys && recordingFor !== row.name"
            class="hotkey-value"
            :class="{ disabled: !isHotkeyEnabled(getBinding(row.name)) }"
          >
            {{ formattedHotkey(row.name) }}
          </span>
          <span v-else-if="!hotkeys" class="hotkey-value placeholder">{{ t('hotkeys.loading') }}</span>

          <!-- Recording state -->
          <div v-if="recordingFor === row.name && currentRecording" class="hotkey-value recording">
            {{ keyOccupied ? t('intercept.key_occupied') : formatCurrentRecording() }}
          </div>

          <button
            v-if="recordingFor !== row.name"
            @click="startRecordingFor(row.name)"
            :disabled="recordingFor !== null || isLoading || !hotkeys || !isHotkeyEnabled(getBinding(row.name))"
            class="record-btn ui-icon-button"
           :title="t('hotkeys.action.change')" :aria-label="t('hotkeys.action.change')">
            <Keyboard :size="18" />
          </button>

          <button
            v-if="recordingFor === row.name"
            @click="cancelRecording"
            class="cancel-btn"
            :title="t('hotkeys.action.cancel_esc')"
            :aria-label="t('hotkeys.action.cancel_aria')"
          >
            ✕
          </button>

          <button
            @click="resetToDefaultFor(row.name)"
            :disabled="!hotkeys || !isHotkeyEnabled(getBinding(row.name))"
            class="reset-btn ui-icon-button"
           :title="t('hotkeys.reset_title')" :aria-label="t('hotkeys.reset_title')">
            <RotateCcw :size="18" />
          </button>
        </div>
      </div>
    </div>

    <!-- Main window local hotkey section -->
    <div class="setting-section" style="margin-top: 1rem;">
      <div class="section-header">
        <AppWindow :size="18" class="section-icon" />
        <span class="section-title ui-section-title">{{ t('hotkeys.section.main_window') }}</span>
      </div>

      <div v-for="row in MAIN_WINDOW_HOTKEY_ROWS" :key="row.name" class="hotkey-row">
        <label class="toggle-switch" :title="toggleTitle(row.name)">
          <input
            type="checkbox"
            :data-hotkey="row.name"
            :checked="isHotkeyEnabled(getBinding(row.name))"
            :disabled="isLoading || recordingFor !== null || pendingToggle !== null || !hotkeys"
            :aria-label="toggleTitle(row.name)"
            @change="toggleHotkeyEnabled(row.name, $event)"
          />
          <span class="toggle-slider" />
        </label>
        <div class="hotkey-label ui-label">
          <span>{{ hotkeyName(row.name) }}</span>
        </div>
        <div class="hotkey-actions">
          <span
            v-if="hotkeys && recordingFor !== row.name"
            class="hotkey-value"
            :class="{ disabled: !isHotkeyEnabled(getBinding(row.name)) }"
          >
            {{ formattedHotkey(row.name) }}
          </span>
          <span v-else-if="!hotkeys" class="hotkey-value placeholder">{{ t('hotkeys.loading') }}</span>

          <div v-if="recordingFor === row.name && currentRecording" class="hotkey-value recording">
            {{ keyOccupied ? t('intercept.key_occupied') : formatCurrentRecording() }}
          </div>

          <button
            v-if="recordingFor !== row.name"
            @click="startRecordingFor(row.name)"
            :disabled="recordingFor !== null || isLoading || !hotkeys || !isHotkeyEnabled(getBinding(row.name))"
            class="record-btn ui-icon-button"
           :title="t('hotkeys.action.change')" :aria-label="t('hotkeys.action.change')">
            <Keyboard :size="18" />
          </button>

          <button
            v-if="recordingFor === row.name"
            @click="cancelRecording"
            class="cancel-btn"
            :title="t('hotkeys.action.cancel_esc')"
            :aria-label="t('hotkeys.action.cancel_aria')"
          >
            ✕
          </button>

          <button
            @click="resetToDefaultFor(row.name)"
            :disabled="!hotkeys || !isHotkeyEnabled(getBinding(row.name))"
            class="reset-btn ui-icon-button"
           :title="t('hotkeys.reset_title')" :aria-label="t('hotkeys.reset_title')">
            <RotateCcw :size="18" />
          </button>
        </div>
      </div>
    </div>

    <!-- Editor hotkey section -->
    <div class="setting-section" style="margin-top: 1rem;">
      <div class="section-header">
        <SquarePen :size="18" class="section-icon" />
        <span class="section-title ui-section-title">{{ t('hotkeys.section.editor') }}</span>
      </div>

      <p class="section-note">
        {{ t('hotkeys.editor_note') }}
      </p>

      <div v-for="row in EDITOR_HOTKEY_ROWS" :key="row.name" class="hotkey-row">
        <label class="toggle-switch" :title="toggleTitle(row.name)">
          <input
            type="checkbox"
            :data-hotkey="row.name"
            :checked="isHotkeyEnabled(getBinding(row.name))"
            :disabled="isLoading || recordingFor !== null || pendingToggle !== null || !hotkeys"
            :aria-label="toggleTitle(row.name)"
            @change="toggleHotkeyEnabled(row.name, $event)"
          />
          <span class="toggle-slider" />
        </label>
        <div class="hotkey-label ui-label">
          <span>{{ hotkeyName(row.name) }}</span>
        </div>
        <div class="hotkey-actions">
          <span
            v-if="hotkeys && recordingFor !== row.name"
            class="hotkey-value"
            :class="{ disabled: !isHotkeyEnabled(getBinding(row.name)) }"
          >
            {{ formattedHotkey(row.name) }}
          </span>
          <span v-else-if="!hotkeys" class="hotkey-value placeholder">{{ t('hotkeys.loading') }}</span>

          <div v-if="recordingFor === row.name && currentRecording" class="hotkey-value recording">
            {{ keyOccupied ? t('intercept.key_occupied') : formatCurrentRecording() }}
          </div>

          <button
            v-if="recordingFor !== row.name"
            @click="startRecordingFor(row.name)"
            :disabled="recordingFor !== null || isLoading || !hotkeys || !isHotkeyEnabled(getBinding(row.name))"
            class="record-btn ui-icon-button"
           :title="t('hotkeys.action.change')" :aria-label="t('hotkeys.action.change')">
            <Keyboard :size="18" />
          </button>

          <button
            v-if="recordingFor === row.name"
            @click="cancelRecording"
            class="cancel-btn"
            :title="t('hotkeys.action.cancel_esc')"
            :aria-label="t('hotkeys.action.cancel_aria')"
          >
            ✕
          </button>

          <button
            @click="resetToDefaultFor(row.name)"
            :disabled="!hotkeys || !isHotkeyEnabled(getBinding(row.name))"
            class="reset-btn ui-icon-button"
           :title="t('hotkeys.reset_title')" :aria-label="t('hotkeys.reset_title')">
            <RotateCcw :size="18" />
          </button>
        </div>
      </div>

      <div class="hotkey-subgroup">
        <div class="subgroup-title">{{ t('hotkeys.subgroup.incoming') }}</div>
        <div v-for="row in INCOMING_HOTKEY_ROWS" :key="row.name" class="hotkey-row">
          <label class="toggle-switch" :title="toggleTitle(row.name)">
            <input
              type="checkbox"
              :data-hotkey="row.name"
              :checked="isHotkeyEnabled(getBinding(row.name))"
              :disabled="isLoading || recordingFor !== null || pendingToggle !== null || !hotkeys"
              :aria-label="toggleTitle(row.name)"
              @change="toggleHotkeyEnabled(row.name, $event)"
            />
            <span class="toggle-slider" />
          </label>
          <div class="hotkey-label ui-label">
            <span>{{ hotkeyName(row.name) }}</span>
          </div>
          <div class="hotkey-actions">
            <span
              v-if="hotkeys && recordingFor !== row.name"
              class="hotkey-value"
              :class="{ disabled: !isHotkeyEnabled(getBinding(row.name)) }"
            >
              {{ formattedHotkey(row.name) }}
            </span>
            <span v-else-if="!hotkeys" class="hotkey-value placeholder">{{ t('hotkeys.loading') }}</span>

            <div v-if="recordingFor === row.name && currentRecording" class="hotkey-value recording">
              {{ keyOccupied ? t('intercept.key_occupied') : formatCurrentRecording() }}
            </div>

            <button
              v-if="recordingFor !== row.name"
              @click="startRecordingFor(row.name)"
              :disabled="recordingFor !== null || isLoading || !hotkeys || !isHotkeyEnabled(getBinding(row.name))"
              class="record-btn ui-icon-button"
             :title="t('hotkeys.action.change')" :aria-label="t('hotkeys.action.change')">
              <Keyboard :size="18" />
            </button>

            <button
              v-if="recordingFor === row.name"
              @click="cancelRecording"
              class="cancel-btn"
              :title="t('hotkeys.action.cancel_esc')"
              :aria-label="t('hotkeys.action.cancel_aria')"
            >
              ✕
            </button>

            <button
              @click="resetToDefaultFor(row.name)"
              :disabled="!hotkeys || !isHotkeyEnabled(getBinding(row.name))"
              class="reset-btn ui-icon-button"
             :title="t('hotkeys.reset_title')" :aria-label="t('hotkeys.reset_title')">
              <RotateCcw :size="18" />
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.hotkeys-panel {
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
  padding: 12px 16px;
  backdrop-filter: blur(8px);
}

.section-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 1rem;
  padding-bottom: 0.75rem;
  border-bottom: 1px solid var(--color-border);
}

.section-icon {
  color: var(--color-text-secondary);
  flex-shrink: 0;
}

.section-title {
  color: var(--color-text-primary);
}

.section-note {
  margin: 0 0 1rem;
  font-size: 0.85rem;
  color: var(--color-text-muted);
}

.hotkey-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 1rem;
  margin-bottom: 1rem;
}

.hotkey-row:last-child {
  margin-bottom: 0;
}

.hotkey-label {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  min-width: 140px;
  color: var(--color-text-primary);
}

.hotkey-actions {
  margin-left: auto;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 0.5rem;
}

.hotkey-value {
  padding: 0.25rem 0.6rem;
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border);
  border-radius: 6px;
  font-family: var(--font-mono);
  font-size: 0.85rem;
  min-width: 80px;
  text-align: center;
  color: var(--color-text-primary);
}

.hotkey-value.disabled {
  opacity: 0.45;
}

.hotkey-value.placeholder {
  color: var(--color-text-muted);
}

.hotkey-value.recording {
  background: var(--warning-bg);
  border-color: var(--warning-border);
  color: var(--warning-text-bright);
  animation: pulse 1s infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.7; }
}

.cancel-btn {
  padding: 0.35rem 0.5rem;
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border);
  border-radius: 4px;
  color: var(--danger-text-bright);
  cursor: pointer;
  transition: all 0.2s ease;
  font-size: 1rem;
  line-height: 1;
}

.cancel-btn:hover {
  background: var(--danger-bg-hover);
}

.hotkey-subgroup {
  margin-top: 1rem;
  padding-top: 0.85rem;
  border-top: 1px dashed var(--color-border);
}

.subgroup-title {
  margin-bottom: 0.85rem;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  letter-spacing: 0.03em;
  color: var(--color-text-muted);
}

/* Compact per-hotkey switch, aligned with the shared .toggle-switch pattern. */
.toggle-switch {
  position: relative;
  display: inline-block;
  flex: 0 0 36px;
  width: 36px;
  height: 20px;
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
  border-radius: 20px;
}

.toggle-slider:before {
  position: absolute;
  content: "";
  height: 14px;
  width: 14px;
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
  transform: translateX(16px);
  background: var(--color-text-white);
}

.toggle-switch input:focus-visible + .toggle-slider {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

.toggle-switch input:disabled + .toggle-slider {
  opacity: 0.55;
  cursor: not-allowed;
}

@media (max-width: 560px) {
  .hotkey-row {
    gap: 0.6rem;
  }

  .hotkey-label {
    min-width: 0;
    flex: 1 1 auto;
  }

  .hotkey-actions {
    width: 100%;
  }
}
</style>
