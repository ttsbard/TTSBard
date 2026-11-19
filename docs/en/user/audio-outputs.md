**English | [Русский](../../ru/user/audio-outputs.md)**

# Quickly enable or disable audio outputs

TTSBard can independently disable its virtual microphone and VBAN outputs.
These are application-wide controls: they affect both speech and Sound panel
clips. Speakers and text delivery remain independent.

## How to use

1. Configure the device under **Audio → Devices** or the receiver under **Audio → VBAN**.
2. Bring up the main TTSBard window. Under **Settings → General → Audio output status**,
   choose **In the title bar**, **Below the editor** or **Hidden**. The default is **Hidden**;
   choose a location to show the buttons. Your choice is saved between launches;
   an existing saved location is preserved. Below-editor controls appear on
   the input surface. The group is never shown in both locations at once.
3. Open the output menu and clear the checkbox for the output you want to stop.
   A checked box means the output is enabled. Select it again to allow playback.

Assign output shortcuts under **Hotkeys → Main window**. Both actions start
disabled with no assigned keys: assign a shortcut and enable each action you need.
This does not change whether the audio outputs themselves are enabled. They work while the
main window is active, regardless of the selected section, text field focus or
group location. They work in normal, compact and Mono modes. Hiding the visual
group in Mono does not disable its shortcuts. An assigned Ctrl+P switches the
output instead of opening the standard print dialog. They do not run in another application or the separate Sound panel
window: first bring up the main window using its existing global shortcut.

With title bar placement, Mono shows a tiny selector on the right with a
full-size menu. Below-editor controls are not displayed in Mono. Hidden removes
the buttons in every mode while keeping the outputs and their shortcuts active.

## What disabling an output does

Disabling stops current delivery to the selected output without waiting for the
speech phrase, Sound panel clip or queue to finish. Other outputs continue.
A short tail may remain in the device's existing audio buffer.

This disables **TTSBard's audio output**; it does not pause playback or disable
the Windows device or driver. The selected device, volume and VBAN settings are
preserved. Re-enabling does not automatically resume an interrupted sound or
replay omitted audio; the output is available for subsequent playback.

For phrases and clips already queued for playback, the output's enabled state
is checked when playback actually starts. Enabling it before an item starts
allows that item to use the output, retaining the device and volume captured
when queued. This does not resume an interrupted sound or restore jobs already
rejected because all outputs were disabled.

An output is greyed out and unavailable if no microphone device is selected,
the selected device is missing, or VBAN is not configured. Hover over its row
for an explanation. Rechecking devices keeps the last confirmed state visible
until the result arrives, so opening the menu does not cause flickering.
There is no separate editor or Sound panel mute in these controls.

See [audio devices](audio-devices.md), [VBAN](vban.md),
[Sound panel](soundpanel.md) and [editor and hotkeys](editor.md).
