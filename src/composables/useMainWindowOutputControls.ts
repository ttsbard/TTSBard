/**
 * Recording state shared with the hotkeys panel (main output controls).
 *
 * While the hotkeys panel records a combination, captured key events must not
 * fire the main-window output toggles (the user is assigning the keys, not
 * using them). The panel already drives the backend `set_hotkey_recording`
 * flag at the same lifecycle points; this module mirrors that flag for the
 * frontend-side matching so the recording UI stays the only consumer of
 * key presses.
 */
let outputControlsRecording = false

export function setMainWindowOutputControlsRecording(active: boolean): void {
  outputControlsRecording = active
}

export function isMainWindowOutputControlsRecording(): boolean {
  return outputControlsRecording
}
