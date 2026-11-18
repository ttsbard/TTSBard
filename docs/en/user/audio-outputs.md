**English | [Русский](../../ru/user/audio-outputs.md)**

# Quickly enable or disable audio outputs

TTSBard can independently disable its virtual microphone and VBAN outputs.
These are application-wide controls: they affect both speech and Sound panel
clips. Speakers and text delivery remain independent.

## How to use

1. Configure the device under **Audio → Devices** or the receiver under **Audio → VBAN**.
2. Bring up the main TTSBard window. In normal and compact modes, the output
   controls are in the title bar, separate from the editor action bar.
3. Open the output menu and clear the checkbox for the output you want to stop.
   A checked box means the output is enabled. Select it again to allow playback.

Assign output shortcuts under **Hotkeys → Main window**. They work while the
main window is active, regardless of the selected section or focus in a text
field. They do not run in another application or the separate Sound panel
window: first bring up the main window using its existing global shortcut.

## What disabling an output does

Disabling stops current delivery to the selected output without waiting for the
speech phrase, Sound panel clip or queue to finish. Other outputs continue.
A short tail may remain in the device's existing audio buffer.

This disables **TTSBard's audio output**; it does not pause playback or disable
the Windows device or driver. The selected device, volume and VBAN settings are
preserved. Re-enabling does not automatically resume an interrupted sound or
replay omitted audio; the output is available for subsequent playback.

An output is greyed out and unavailable if no microphone device is selected,
the selected device is missing, or VBAN is not configured. Hover over its row
for an explanation. Rechecking devices keeps the last confirmed state visible
until the result arrives, so opening the menu does not cause flickering.
There is no separate editor or Sound panel mute in these controls.

See [audio devices](audio-devices.md), [VBAN](vban.md),
[Sound panel](soundpanel.md) and [editor and hotkeys](editor.md).
