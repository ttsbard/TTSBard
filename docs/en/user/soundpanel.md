**English | [Русский](../../ru/user/soundpanel.md)**

# Sound panel

Sound panel is a separate floating keyboard grid for triggering sounds. There
is no sidebar sound tab; configuration, sets and playback share one window.

## Open the panel

- `Ctrl+Shift+F2` shows and focuses it by default; change this under **Hotkeys**.
- The main title bar also has a Sound panel button.

TTSBard remembers the active external window before showing the panel. Selecting
a sound or pressing Escape returns focus there before playback begins.

## Grid and modes

The QWERTY grid contains `A–Z` and `0–9`:

- **runtime**: a physical key, `Enter` on the selected cell or a click plays it.
- **config**: a key or click opens the file/description assignment dialog.

Switch with the gear or `Ctrl+B`. Normal keys are not intercepted while the
configuration dialog is open, so you can type in its fields. Supported files:
MP3, WAV, OGG, FLAC, M4A, AAC and WMA. Arrow keys move selection; Enter activates.

## Playback and audio outputs

The panel uses TTS's enabled Speaker, Virtual microphone and [VBAN](vban.md)
outputs, each with its own volume. Local outputs use their selected devices;
VBAN uses the saved **Audio → VBAN** settings.
See [virtual audio cable](virtual-audio-cable.md)
for delivery to Discord, OBS, VTube Studio or games.
The shared [output format](audio-devices.md#output-format) under Audio → Devices
also applies to local outputs; stop and restart audio after changing it.
VBAN keeps its fixed 16-bit PCM, 48 kHz stereo format.

Enabled outputs work independently: preparing the network copy does not delay
local playback. Failure on one output does not cancel another working output.
VBAN can also be the only output.
The panel has a separate sequential queue: rapid triggers do not overlap.
This is independent of TTS, so sounds neither interrupt speech nor wait for it.
A full queue rejects new triggers; retry after a queued sound finishes.

VBAN mixes simultaneous speech and clips into one stream. Pausing or stopping
speech does not stop the separate Sound panel queue.

## Sets

A set is an independent grid of assignments. Select it in the title-bar selector.

- `PageUp` / `PageDown` cycle previous/next set.
- `F1` through `F12` select the corresponding set; missing positions do nothing.
- While Sound panel is active, these local keys take precedence over TTSBard's
  global interception. Interception works normally outside the panel.

Config mode adds create/delete buttons beside the selector. You cannot delete
the last set.

## Pinning

The pin controls visibility consistently:

- Crossed out: hide after selecting a sound, Escape or losing focus.
- Highlighted: remain visible after clicks, Enter, physical keys, Escape and
  losing focus.

Both policies return focus to the previously active external window. **×** always
hides the panel. Pin state persists across launches.

## Appearance

Under **Settings → Interface**, inherit the main window's color or choose a
separate one. Transparency is always independent of the color/source, from
`0%` opaque to `90%` maximum transparency. Light/dark theme changes also update
the open window and native controls.
Color/transparency are saved in `windows.json`; sets/assignments in
`soundpanel_bindings.json`.

## Shortcut reference

| Key | In the active panel |
|---|---|
| `A–Z`, `0–9` | Play/configure a cell according to mode |
| `Ctrl+B` | Toggle runtime/config |
| Arrow keys | Move selection |
| `Enter` | Activate selected cell |
| `PageUp` / `PageDown` | Previous/next set |
| `F1–F12` | Select set by position |
| `Escape` | Return focus and hide unless pinned |
