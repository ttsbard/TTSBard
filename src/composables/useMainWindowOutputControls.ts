import { computed, effectScope, ref, watch, type ComputedRef, type EffectScope, type Ref } from 'vue'
import type { AppSettingsContext } from '../types/settings'
import { matchesEditorHotkey, type HotkeyEventLike } from '../components/editor/keymapArbitration'
import { matchesLocalHotkey } from '../utils/localHotkeys'
import { createOutputToggle, isVbanTargetConfigured } from './useOutputToggles'
import { createVirtualMicDeviceCheck } from './useVirtualMicDevices'
import { useErrorHandler } from './useErrorHandler'
import { presentCommandError } from '../ipc/commandError'
import { t } from '../i18n'

/**
 * Single root-lifetime owner of the main-window audio output controls
 * (ROADMAP-142/143): the shared controllers, availability and the document
 * keydown listener live here — independent of normal/compact/mono, the
 * active section and where the view is rendered. Views (titlebar group,
 * editor action bar group) are pure renderers of this state and may mount,
 * move or unmount freely; the listener is attached once for the app
 * lifetime by the root shell. This is a main-window document listener, not
 * an OS hook: it never fires in other applications or the separate
 * SoundPanel window.
 */
let outputControlsRecording = false

/**
 * Surfaces that capture keys for assignment (HotkeysPanel, InterceptPanel).
 * The frontend recording guard is shared but owner-counted: each surface
 * claims and releases only its own entry, so one surface finishing — or
 * failing — its cleanup can never release another surface's active recording.
 */
export type MainWindowOutputControlsRecordingOwner = 'hotkeys-panel' | 'intercept-panel'

const outputControlsRecordingOwners = new Set<MainWindowOutputControlsRecordingOwner>()

export function setMainWindowOutputControlsRecording(
  owner: MainWindowOutputControlsRecordingOwner,
  active: boolean,
): void {
  if (active) outputControlsRecordingOwners.add(owner)
  else outputControlsRecordingOwners.delete(owner)
  outputControlsRecording = outputControlsRecordingOwners.size > 0
}

export function isMainWindowOutputControlsRecording(): boolean {
  return outputControlsRecording
}

// ---------------------------------------------------------------------------
// Placement (titlebar / under the editor / hidden) — a local UI preference persisted
// like the editor tabs visibility, reactive for the current main window.
// ---------------------------------------------------------------------------

export type OutputControlsPlacement = 'titlebar' | 'editor' | 'hidden'

const PLACEMENT_STORAGE_KEY = 'app-output-controls-placement'

const outputControlsPlacement = ref<OutputControlsPlacement>('hidden')
let placementLoaded = false

function isPlacementValue(value: unknown): value is OutputControlsPlacement {
  return value === 'titlebar' || value === 'editor' || value === 'hidden'
}

/** Reactive placement; a missing/invalid stored value falls back to the default. */
export function getOutputControlsPlacement(): Ref<OutputControlsPlacement> {
  if (!placementLoaded) {
    placementLoaded = true
    try {
      const stored = localStorage.getItem(PLACEMENT_STORAGE_KEY)
      if (isPlacementValue(stored)) outputControlsPlacement.value = stored
    } catch {
      // Unavailable storage keeps the default placement; nothing to report.
    }
  }
  return outputControlsPlacement
}

/**
 * Persist the placement, persist-first: the reactive placement changes only
 * after the stored preference is confirmed. A failed write keeps the previous
 * placement and returns `false` so the caller can report it — nothing may
 * pretend the choice was saved (it would silently revert on the next launch).
 */
export function setOutputControlsPlacement(value: OutputControlsPlacement): boolean {
  try {
    localStorage.setItem(PLACEMENT_STORAGE_KEY, value)
  } catch {
    return false
  }
  outputControlsPlacement.value = value
  return true
}

/** Test/lifecycle helper: back to defaults (module state is per test file). */
export function resetOutputControlsPlacement(): void {
  placementLoaded = false
  outputControlsPlacement.value = 'hidden'
}

// ---------------------------------------------------------------------------
// Shared controllers, availability and the root keydown listener
// ---------------------------------------------------------------------------

export interface MainWindowOutputControls {
  virtualMicEnabled: ComputedRef<boolean>
  vbanEnabled: ComputedRef<boolean>
  virtualMicAvailable: ComputedRef<boolean>
  vbanAvailable: ComputedRef<boolean>
  audioOutputsReady: ComputedRef<boolean>
  outputsPending: ComputedRef<boolean>
  toggleVirtualMicOutput: () => Promise<void>
  toggleVbanOutput: () => Promise<void>
  refreshAvailability: () => Promise<void>
  /** Root-lifetime capture-phase keydown matching for the two toggles. */
  attachListener: () => void
  detachListener: () => void
}

let instance: MainWindowOutputControls | null = null
let instanceScope: EffectScope | null = null

/**
 * The shared controller instance for the whole main window. `context` is the
 * app settings context provided by the root shell — the controller binds to
 * exactly this one context for its whole lifetime: every read (`audioSettings`,
 * hotkeys, `isLoading`, `reload`) goes through the same object, so a remounted
 * view shares the state instead of re-targeting it. A new context means the
 * previous shell was finally unmounted (`disposeMainWindowOutputControls`)
 * and the next call builds a fresh controller around it; normal/compact/mono
 * and moving the view never unmount the shell, so the instance survives them.
 */
export function getMainWindowOutputControls(context: AppSettingsContext): MainWindowOutputControls {
  if (instance) return instance

  const { showError } = useErrorHandler()
  // The singleton must survive the unmount of whichever view created it
  // (e.g. the titlebar group in mono): keep its reactive graph in a detached
  // scope instead of the creating component's own.
  const scope = effectScope(true)
  const built = scope.run(() => {
    const audioSettings = computed(() => context.settings.value?.audio)

    // The virtual mic is available only with a selected device that still
    // exists in the live device list. The list refreshes exactly once per
    // actual selection: the init refresh runs when the saved device is first
    // seen (settings already loaded at controller creation, or the snapshot
    // arriving later); later selection changes and menu opens refresh again.
    // An empty selection has nothing to confirm — the next selection or menu
    // open refreshes instead. The last confirmed list stays visible during a
    // re-check, and an unknown list never counts as available. (The tested
    // ROADMAP-142 check and latest-request contract, shared verbatim.)
    const micDeviceCheck = createVirtualMicDeviceCheck()

    const virtualMicAvailable = computed(() => {
      const device = audioSettings.value?.virtual_mic_device
      return !!device && micDeviceCheck.isSelectedDeviceAvailable(device)
    })

    watch(
      () => audioSettings.value?.virtual_mic_device,
      (device) => {
        if (device) void micDeviceCheck.refresh()
      },
      { immediate: true },
    )

    // VBAN availability mirrors the saved-target validation only; `enabled`
    // does not make an unconfigured output available and receiver health does
    // not make a configured one unavailable. (The tested ROADMAP-142 mirror
    // of the backend target validation, shared verbatim.)
    const vbanAvailable = computed(() => isVbanTargetConfigured(audioSettings.value?.vban))

    const virtualMicToggle = createOutputToggle({
      read: () => audioSettings.value?.virtual_mic_enabled,
      command: 'set_virtual_mic_enabled',
      reload: () => context.reload(),
      isLoading: context.isLoading,
      isAvailable: () => virtualMicAvailable.value,
      onError: (e) =>
        showError(presentCommandError(e, t('editor.outputs.error_mic'))),
    })

    const vbanToggle = createOutputToggle({
      read: () => audioSettings.value?.vban?.enabled,
      command: 'set_vban_enabled',
      reload: () => context.reload(),
      isLoading: context.isLoading,
      isAvailable: () => vbanAvailable.value,
      onError: (e) =>
        showError(presentCommandError(e, t('editor.outputs.error_vban'))),
    })

    return {
      micDeviceCheck, virtualMicToggle, vbanToggle, virtualMicAvailable, vbanAvailable,
      // Derived controller state, constructed here together with the rest of
      // the reactive graph inside the detached scope block: it shares the
      // controller's lifetime and location rather than being created in
      // whichever component's scope happens to mount the first view.
      audioOutputsReady: computed(
        () => virtualMicToggle.ready.value && vbanToggle.ready.value,
      ),
      outputsPending: computed(
        () => virtualMicToggle.pending.value || vbanToggle.pending.value,
      ),
    }
  })!

  const {
    micDeviceCheck, virtualMicToggle, vbanToggle, virtualMicAvailable, vbanAvailable,
    audioOutputsReady, outputsPending,
  } = built
  // A combination owned by an existing main-window-local binding must not
  // fire two actions: those bindings keep priority, the output toggle skips.
  function isClaimedByMainWindowBinding(event: HotkeyEventLike): boolean {
    const hotkeys = context.settings.value?.hotkeys
    if (!hotkeys) return false
    return (
      matchesLocalHotkey(hotkeys.return_previous_window, event) ||
      matchesLocalHotkey(hotkeys.toggle_minimal_mode, event)
    )
  }

  function handleMainOutputKeydown(event: KeyboardEvent): void {
    // An event another capture handler already consumed is not ours.
    if (event.defaultPrevented) return
    if (outputControlsRecording) return
    const editor = context.settings.value?.hotkeys?.editor
    if (!editor) return
    if (isClaimedByMainWindowBinding(event)) return

    // Both stored bindings matching the same combo is a legacy conflict: the
    // mic toggle wins and exactly one action fires.
    const micMatch = matchesEditorHotkey(editor.toggle_virtual_mic, event)
    const vbanMatch = !micMatch && matchesEditorHotkey(editor.toggle_vban, event)
    if (!micMatch && !vbanMatch) return

    // A matched event is always consumed — the browser default (print) must
    // not replace an assigned command, repeats must not re-print, and the
    // editor must not see the same key a second time. The toggle itself
    // ignores repeats and IME composition so a held key cannot switch the
    // output in a loop; an unavailable output stays a consumed no-op instead
    // of falling back to the browser command.
    // An IME composition keydown is different: it must reach the composition
    // untouched, so it is skipped before the consumption — neither toggle,
    // nor preventDefault, nor stopImmediatePropagation. Only auto-repeat of a
    // matched combo stays consumed without toggling.
    if (event.isComposing) return
    event.preventDefault()
    event.stopImmediatePropagation()
    if (event.repeat) return
    if (micMatch) void virtualMicToggle.toggle()
    else void vbanToggle.toggle()
  }

  let listenerAttached = false

  instance = {
    virtualMicEnabled: virtualMicToggle.enabled,
    vbanEnabled: vbanToggle.enabled,
    virtualMicAvailable,
    vbanAvailable,
    audioOutputsReady,
    outputsPending,
    toggleVirtualMicOutput: () => virtualMicToggle.toggle(),
    toggleVbanOutput: () => vbanToggle.toggle(),
    refreshAvailability: () => micDeviceCheck.refresh(),
    attachListener: () => {
      if (listenerAttached) return
      listenerAttached = true
      // Capture phase: a matched event is consumed before the editor and the
      // browser default (print), wherever the focus is inside this window.
      document.addEventListener('keydown', handleMainOutputKeydown, true)
      // No extra launch-time device refresh here: the device watch already
      // fires the single init refresh when the saved selection is first seen
      // (immediately for an already-loaded snapshot, or once the settings
      // arrive) — this call would only duplicate that request.
    },
    detachListener: () => {
      if (!listenerAttached) return
      listenerAttached = false
      document.removeEventListener('keydown', handleMainOutputKeydown, true)
    },
  }

  instanceScope = scope
  return instance
}

/**
 * Final root cleanup (the App shell unmount): release the document listener,
 * stop the detached reactive scope (watchers/computeds) and drop the
 * singleton so a later shell mounts a fresh controller around its own
 * settings context. In-flight device replies keep writing only into the
 * discarded instance's check and cannot touch a controller created after
 * this — the new owner refreshes and decides availability from its own state.
 */
export function disposeMainWindowOutputControls(): void {
  if (!instance) return
  instance.detachListener()
  instanceScope?.stop()
  instanceScope = null
  instance = null
}
