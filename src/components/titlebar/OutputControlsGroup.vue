<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from 'vue'
import { useAppSettings, useAudioSettings } from '../../composables/useAppSettings'
import {
  createOutputToggle,
  isVbanTargetConfigured,
} from '../../composables/useOutputToggles'
import { createVirtualMicDeviceCheck } from '../../composables/useVirtualMicDevices'
import {
  isMainWindowOutputControlsRecording,
} from '../../composables/useMainWindowOutputControls'
import { matchesEditorHotkey } from '../editor/keymapArbitration'
import { matchesLocalHotkey } from '../../utils/localHotkeys'
import { t } from '../../i18n'
import { presentCommandError } from '../../ipc/commandError'
import { useErrorHandler } from '../../composables/useErrorHandler'
import OutputToggles from '../editor/OutputToggles.vue'

/**
 * Main-window audio output controls (ROADMAP-142, moved from the editor
 * action bar): the popover lives in the titlebar next to the integration
 * status cluster, and the local toggle hotkeys are matched at the
 * main-window level — on every section of the normal/compact window,
 * including editor/textarea/input focus, but never in other apps or the
 * separate SoundPanel window (this is a main-window document listener, not
 * an OS hook).
 *
 * The toggles change the same shared `audio.virtual_mic_enabled` /
 * `audio.vban.enabled` flags the Audio screen owns — no separate mute state.
 * SoundPanel keeps obeying those shared flags as before.
 */
const { showError } = useErrorHandler()
const appSettings = useAppSettings()
const audioSettings = useAudioSettings()

// The virtual mic is available only with a selected device that still exists
// in the live device list. The list refreshes on menu open, on selected
// device change (immediate) and at mount; the last confirmed list stays
// visible during a re-check, and an unknown list never counts as available.
const micDeviceCheck = createVirtualMicDeviceCheck()

const virtualMicAvailable = computed(() => {
  const device = audioSettings.value?.virtual_mic_device
  return !!device && micDeviceCheck.isSelectedDeviceAvailable(device)
})

// VBAN availability mirrors the saved-target validation only; `enabled` does
// not make an unconfigured output available and receiver health does not
// make a configured one unavailable.
const vbanAvailable = computed(() => isVbanTargetConfigured(audioSettings.value?.vban))

watch(
  () => audioSettings.value?.virtual_mic_device,
  () => {
    void micDeviceCheck.refresh()
  },
  { immediate: true },
)

const virtualMicToggle = createOutputToggle({
  read: () => audioSettings.value?.virtual_mic_enabled,
  command: 'set_virtual_mic_enabled',
  reload: appSettings.reload,
  isLoading: appSettings.isLoading,
  isAvailable: () => virtualMicAvailable.value,
  onError: (e) => showError(presentCommandError(e, t('editor.outputs.error_mic'))),
})

const vbanToggle = createOutputToggle({
  read: () => audioSettings.value?.vban?.enabled,
  command: 'set_vban_enabled',
  reload: appSettings.reload,
  isLoading: appSettings.isLoading,
  isAvailable: () => vbanAvailable.value,
  onError: (e) => showError(presentCommandError(e, t('editor.outputs.error_vban'))),
})

const virtualMicEnabled = virtualMicToggle.enabled
const vbanEnabled = vbanToggle.enabled

const audioOutputsReady = computed(() => virtualMicToggle.ready.value && vbanToggle.ready.value)
const outputsPending = computed(() => virtualMicToggle.pending.value || vbanToggle.pending.value)

function toggleVirtualMicOutput() {
  return virtualMicToggle.toggle()
}

function toggleVbanOutput() {
  return vbanToggle.toggle()
}

// A combination owned by an existing main-window-local binding must not fire
// two actions: those bindings keep priority, the output toggle skips (the
// assignment-time conflict check already prevents new duplicates).
function isClaimedByMainWindowBinding(event: KeyboardEvent): boolean {
  const hotkeys = appSettings.settings.value?.hotkeys
  if (!hotkeys) return false
  return (
    matchesLocalHotkey(hotkeys.return_previous_window, event) ||
    matchesLocalHotkey(hotkeys.toggle_minimal_mode, event)
  )
}

/**
 * Main-window keydown matching for the two output toggles. Unlike the
 * minimal-mode handler there is no input/textarea filter: the toggles must
 * work wherever the focus is inside the main window, including the editor.
 * Modifier matching, `enabled` and the physical code mapping reuse the
 * existing editor matcher (exact modifiers, NumPad aware).
 */
function handleMainOutputKeydown(event: KeyboardEvent) {
  // Key auto-repeat must not toggle the output on and off in a loop, IME
  // composition is not a command, and an event another handler already
  // consumed is not ours to act on.
  if (event.repeat || event.isComposing || event.defaultPrevented) return
  if (isMainWindowOutputControlsRecording()) return
  const editor = appSettings.settings.value?.hotkeys?.editor
  if (!editor) return
  if (isClaimedByMainWindowBinding(event)) return

  // Both stored bindings matching the same combo is a legacy conflict: the
  // mic toggle wins and exactly one action fires.
  if (matchesEditorHotkey(editor.toggle_virtual_mic, event)) {
    event.preventDefault()
    void toggleVirtualMicOutput()
    return
  }
  if (matchesEditorHotkey(editor.toggle_vban, event)) {
    event.preventDefault()
    void toggleVbanOutput()
  }
}

onMounted(() => document.addEventListener('keydown', handleMainOutputKeydown))
onUnmounted(() => document.removeEventListener('keydown', handleMainOutputKeydown))
</script>

<template>
  <OutputToggles
    class="titlebar-output-controls"
    :virtual-mic-enabled="virtualMicEnabled"
    :vban-enabled="vbanEnabled"
    :virtual-mic-available="virtualMicAvailable"
    :vban-available="vbanAvailable"
    :compact="true"
    :ready="audioOutputsReady"
    :pending="outputsPending"
    direction="down"
    @open="micDeviceCheck.refresh()"
    @toggle-virtual-mic="toggleVirtualMicOutput"
    @toggle-vban="toggleVbanOutput"
  />
</template>

<style scoped>
.titlebar-output-controls {
  margin-right: 0.35rem;
}

/* The popover must not be a drag region and must stay above the window
   content; OutputToggles owns its own positioning. */
.titlebar-output-controls :deep(.output-menu) {
  z-index: 1200;
}
</style>
