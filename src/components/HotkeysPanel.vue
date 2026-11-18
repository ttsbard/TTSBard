<script setup lang="ts">
import { ref, computed, onUnmounted, watch, type Component } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Keyboard, RotateCcw, AppWindow, Music, MonitorPlay, SquarePen, ScanLine } from 'lucide-vue-next'
import type { HotkeyDto } from '../types/settings'
import { useAppSettings } from '../composables/useAppSettings'
import { debugError } from '../utils/debug'
import { normalizeCommandError } from '../ipc/commandError'
import { numpadKeyNameFromCode, formatNumpadKeyLabel } from '../utils/interceptKeys'
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

// These editor actions ship switched off. Their visible hints explain the
// actual effect of the combination, which the short row label cannot carry.
const HINTED_EDITOR_HOTKEYS: readonly EditorHotkeyName[] = [
  'cycle_route',
  'toggle_typing',
  'cycle_quick_mode',
]

const recordingFor = ref<HotkeyName | EditorHotkeyName | null>(null)
const pendingToggle = ref<HotkeyName | EditorHotkeyName | null>(null)
const keyOccupied = ref(false)
let occupiedTimeoutId: ReturnType<typeof setTimeout> | null = null

// A capture that is being persisted. The captured value stays visible in its
// own row until the save settles, so the row never blanks out between the end
// of recording and the end of the async save/reload.
const saving = ref<{ name: HotkeyName | EditorHotkeyName; binding: { modifiers: HotkeyDto['modifiers']; key: string } } | null>(null)
let saveGeneration = 0

// Bindings already confirmed by a successful save whose settings refresh has
// not landed yet. The row keeps showing them even if the refresh or the
// recording-flag cleanup fails, and a real failure cannot resurrect the old
// value for a binding the backend has already stored.
const confirmedBindings = ref<Partial<Record<HotkeyName | EditorHotkeyName, { modifiers: HotkeyDto['modifiers']; key: string }>>>({})

// A fresh settings snapshot was loaded after the overrides were recorded, so
// the stored values are authoritative again and the overrides expire.
watch(settings, () => {
  confirmedBindings.value = {}
})

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

// What the row should display as its confirmed binding: a just-saved value the
// settings snapshot does not reflect yet, or the stored binding itself.
function displayedBinding(name: HotkeyName | EditorHotkeyName): { modifiers: HotkeyDto['modifiers']; key: string } | undefined {
  return confirmedBindings.value[name] ?? getBinding(name)
}

function formattedHotkey(name: HotkeyName | EditorHotkeyName): string {
  const binding = displayedBinding(name)
  return binding ? formatHotkey(binding) : ''
}

function savingLabel(name: HotkeyName | EditorHotkeyName): string {
  if (saving.value?.name !== name) return ''
  return formatHotkey(saving.value.binding)
}

function hotkeyName(name: HotkeyName | EditorHotkeyName): string {
  return t(`hotkeys.name.${name}`)
}

function hotkeyHint(name: HotkeyName | EditorHotkeyName): string {
  return HINTED_EDITOR_HOTKEYS.includes(name as EditorHotkeyName)
    ? t(`hotkeys.hint.${name}`)
    : ''
}

function hotkeyHintId(name: HotkeyName | EditorHotkeyName): string {
  return `hotkey-hint-${name}`
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

// ROADMAP-140: NumPad records in every scope — the OS-registered globals, the
// main-window-local rows and the editor actions share the canonical names and
// labels. NumpadEnter stays outside the restricted set everywhere.
function codeToKey(code: string): string {
  const numpad = numpadKeyNameFromCode(code)
  if (numpad !== null) return numpad
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
    // Save the hotkey; the captured value stays on screen while it is in flight.
    beginSave(recordingFor.value, {
      modifiers: [...currentRecording.value.modifiers],
      key: currentRecording.value.key,
    })
  }
}

// Hand a finished capture over to the saving state and persist it. Recording
// ends here; the row keeps showing the captured binding until the save settles.
function beginSave(name: HotkeyName | EditorHotkeyName, binding: { modifiers: HotkeyDto['modifiers']; key: string }) {
  document.removeEventListener('keydown', handleKeyDown)
  document.removeEventListener('keyup', handleKeyUp)
  recordingFor.value = null
  currentRecording.value = null
  saving.value = { name, binding }
  const generation = ++saveGeneration
  void saveHotkey(name, binding, generation)
}

async function saveHotkey(
  name: HotkeyName | EditorHotkeyName,
  hotkey: { modifiers: HotkeyDto['modifiers']; key: string },
  generation: number,
) {
  // Phase 1 — persist. Only this call decides whether the assignment changed;
  // a failure here returns the last confirmed value and explains why.
  try {
    // Persist the current switch state explicitly so the backend never has to
    // infer it while replacing the combination.
    const payload: HotkeyDto = { ...hotkey, enabled: isHotkeyEnabled(getBinding(name)) }
    if (isEditorHotkeyName(name)) {
      await invoke('set_editor_hotkey', { actionId: name, hotkey: payload })
    } else {
      await invoke('set_hotkey', { name, hotkey: payload })
    }
  } catch (e) {
    // A newer save owns the row; its completion will settle the state.
    if (generation !== saveGeneration) return
    // Restore the last confirmed value immediately and explain why.
    saving.value = null
    showMessage(t('hotkeys.error_prefix', { detail: normalizeCommandError(e).message }), 'error')
    try {
      await invoke('set_hotkey_recording', { recording: false })
      if (!isEditorHotkeyName(name)) {
        await invoke('reregister_hotkeys_cmd')
      }
    } catch {
      // Игнорируем ошибки при восстановлении хоткеев
    }
    return
  }
  // Phase 2 — confirmed. The backend has stored the new binding, so the row
  // must show it from now on even if the refresh or cleanup below fails.
  if (generation === saveGeneration) {
    saving.value = null
    confirmedBindings.value = { ...confirmedBindings.value, [name]: hotkey }
  }
  // Phase 3 — refresh. reload() never rejects: it logs into the settings
  // error state and leaves the old snapshot in place on failure, so a failed
  // refresh must not undo the assignment on screen; the override above keeps
  // the stored value visible until the next successful snapshot.
  try {
    await reload()
  } catch (e) {
    // Defensive only: the composable is documented not to throw.
    debugError('Hotkeys panel: settings refresh after save failed:', e)
  }
  // Phase 4 — release the recording flag. set_hotkey уже вызывает
  // reregister_hotkeys внутри, set_editor_hotkey не регистрирует глобально.
  // Best-effort: a failed release must not report a failed assignment.
  try {
    await invoke('set_hotkey_recording', { recording: false })
  } catch (e) {
    debugError('Hotkeys panel: recording flag release after save failed:', e)
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
      // the hotkey is switched back on, so the notice must not claim that
      // recognition is unavailable.
      showMessage(
        t('hotkeys.ocr_capture_disabled', { name: t('hotkeys.name.ocr_capture') }),
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

function hotkeyKeyLabel(key: string): string {
  return formatNumpadKeyLabel(key) ?? key
}

function formatHotkey(hotkey: Pick<HotkeyDto, 'modifiers' | 'key'>): string {
  const modMap: Record<string, string> = { ctrl: 'Ctrl', shift: 'Shift', alt: 'Alt', super: 'Win' }
  const mods = hotkey.modifiers.map(m => modMap[m])
  if (mods.length === 0) {
    return hotkeyKeyLabel(hotkey.key)
  }
  return `${mods.join('+')}+${hotkeyKeyLabel(hotkey.key)}`
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
  return formatHotkey(currentRecording.value)
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

  // Если компонент размонтируется во время записи или сохранения, сбрасываем
  // флаг и восстанавливаем хоткеи; идущий в полёте saveHotkey завершит свою
  // собственную последовательность очистки (idempotent).
  if (recordingFor.value || saving.value) {
    const wasEditor = recordingFor.value !== null
      ? isEditorHotkeyName(recordingFor.value)
      : saving.value !== null && isEditorHotkeyName(saving.value.name)
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
    <!-- Compact severity notice: shared ui-notice form in a zero-height layer
         at the top centre of this panel's own area. The legacy message-box and
         bare severity classes stay only as hooks used by existing tests. -->
    <div v-if="errorMessage" class="ui-notice-area">
      <div
        class="message-box ui-notice"
        :class="[messageState, messageState && `ui-notice--${messageState}`]"
      >
        {{ errorMessage }}
      </div>
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
            :disabled="isLoading || recordingFor !== null || pendingToggle !== null || saving !== null || !hotkeys"
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
            v-if="hotkeys && recordingFor !== row.name && saving?.name !== row.name"
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

          <!-- Save in flight: the captured value stays visible in its row -->
          <div v-else-if="saving?.name === row.name" class="hotkey-value saving">
            {{ savingLabel(row.name) }}
          </div>

          <button
            v-if="recordingFor !== row.name"
            @click="startRecordingFor(row.name)"
            :disabled="recordingFor !== null || saving !== null || isLoading || !hotkeys || !isHotkeyEnabled(getBinding(row.name))"
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
            :disabled="!hotkeys || saving !== null || !isHotkeyEnabled(getBinding(row.name))"
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
            :disabled="isLoading || recordingFor !== null || pendingToggle !== null || saving !== null || !hotkeys"
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
            v-if="hotkeys && recordingFor !== row.name && saving?.name !== row.name"
            class="hotkey-value"
            :class="{ disabled: !isHotkeyEnabled(getBinding(row.name)) }"
          >
            {{ formattedHotkey(row.name) }}
          </span>
          <span v-else-if="!hotkeys" class="hotkey-value placeholder">{{ t('hotkeys.loading') }}</span>

          <div v-if="recordingFor === row.name && currentRecording" class="hotkey-value recording">
            {{ keyOccupied ? t('intercept.key_occupied') : formatCurrentRecording() }}
          </div>

          <!-- Save in flight: the captured value stays visible in its row -->
          <div v-else-if="saving?.name === row.name" class="hotkey-value saving">
            {{ savingLabel(row.name) }}
          </div>

          <button
            v-if="recordingFor !== row.name"
            @click="startRecordingFor(row.name)"
            :disabled="recordingFor !== null || saving !== null || isLoading || !hotkeys || !isHotkeyEnabled(getBinding(row.name))"
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
            :disabled="!hotkeys || saving !== null || !isHotkeyEnabled(getBinding(row.name))"
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
            :disabled="isLoading || recordingFor !== null || pendingToggle !== null || saving !== null || !hotkeys"
            :aria-label="toggleTitle(row.name)"
            :aria-describedby="hotkeyHint(row.name) ? hotkeyHintId(row.name) : undefined"
            @change="toggleHotkeyEnabled(row.name, $event)"
          />
          <span class="toggle-slider" />
        </label>
        <div class="hotkey-label ui-label" :class="{ 'hotkey-label--with-hint': hotkeyHint(row.name) }">
          <span>{{ hotkeyName(row.name) }}</span>
          <span
            v-if="hotkeyHint(row.name)"
            :id="hotkeyHintId(row.name)"
            class="hotkey-hint ui-hint"
          >
            {{ hotkeyHint(row.name) }}
          </span>
        </div>
        <div class="hotkey-actions">
          <span
            v-if="hotkeys && recordingFor !== row.name && saving?.name !== row.name"
            class="hotkey-value"
            :class="{ disabled: !isHotkeyEnabled(getBinding(row.name)) }"
          >
            {{ formattedHotkey(row.name) }}
          </span>
          <span v-else-if="!hotkeys" class="hotkey-value placeholder">{{ t('hotkeys.loading') }}</span>

          <div v-if="recordingFor === row.name && currentRecording" class="hotkey-value recording">
            {{ keyOccupied ? t('intercept.key_occupied') : formatCurrentRecording() }}
          </div>

          <!-- Save in flight: the captured value stays visible in its row -->
          <div v-else-if="saving?.name === row.name" class="hotkey-value saving">
            {{ savingLabel(row.name) }}
          </div>

          <button
            v-if="recordingFor !== row.name"
            @click="startRecordingFor(row.name)"
            :disabled="recordingFor !== null || saving !== null || isLoading || !hotkeys || !isHotkeyEnabled(getBinding(row.name))"
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
            :disabled="!hotkeys || saving !== null || !isHotkeyEnabled(getBinding(row.name))"
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
              :disabled="isLoading || recordingFor !== null || pendingToggle !== null || saving !== null || !hotkeys"
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
              v-if="hotkeys && recordingFor !== row.name && saving?.name !== row.name"
              class="hotkey-value"
              :class="{ disabled: !isHotkeyEnabled(getBinding(row.name)) }"
            >
              {{ formattedHotkey(row.name) }}
            </span>
            <span v-else-if="!hotkeys" class="hotkey-value placeholder">{{ t('hotkeys.loading') }}</span>

            <div v-if="recordingFor === row.name && currentRecording" class="hotkey-value recording">
              {{ keyOccupied ? t('intercept.key_occupied') : formatCurrentRecording() }}
            </div>

            <!-- Save in flight: the captured value stays visible in its row -->
            <div v-else-if="saving?.name === row.name" class="hotkey-value saving">
              {{ savingLabel(row.name) }}
            </div>

            <button
              v-if="recordingFor !== row.name"
              @click="startRecordingFor(row.name)"
              :disabled="recordingFor !== null || saving !== null || isLoading || !hotkeys || !isHotkeyEnabled(getBinding(row.name))"
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
              :disabled="!hotkeys || saving !== null || !isHotkeyEnabled(getBinding(row.name))"
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
  /* The name column owns the remaining row width and wraps inside it. A zero
     flex basis keeps the label's hypothetical line-break size small, so line
     breaking is decided by the right group and this floor: the group drops to
     its own line once the container cannot hold switch + gap + a readable
     label (110 px) + gap + group, and never because the name or its hint is
     long. */
  flex: 1 1 0;
  min-width: 110px;
  display: flex;
  align-items: center;
  gap: 0.6rem;
  color: var(--color-text-primary);
}

/* Editor rows with a contextual hint stack the name and the secondary text.
   The switch and the grouped actions keep their places; both text lines wrap
   inside the label's own column. */
.hotkey-label--with-hint {
  flex-direction: column;
  align-items: flex-start;
  gap: 0;
}

.hotkey-hint {
  max-width: 34rem;
}

/* Shortcut value, record/cancel and reset form one unsplittable group.
   nowrap + the automatic min-width (min-content of the inner controls) make
   the whole group indivisible: the parent row wraps it as a single unit based
   on the container width, so the buttons never separate from the shortcut. */
.hotkey-actions {
  margin-left: auto;
  display: flex;
  align-items: center;
  flex-wrap: nowrap;
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

/* Save in flight: same in-progress language as the recording chip. */
.hotkey-value.saving {
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
</style>
