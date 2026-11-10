**English | [Русский](../../ru/user/guide.md)**

# User guide

TTSBard is a Windows 10/11 (64-bit) application for typing and speaking phrases
from an editor opened with a hotkey.

## First launch

1. Install your version from the [project releases](https://github.com/alksef/TTSBard/releases) and start TTSBard.
2. Open **TTS**, configure and select a [provider or local model](tts.md).
   Cloud providers require credentials; Piper requires model files.
3. In **Audio**, select your speakers and use the playback test button to check
   the volume. To send speech to Discord, OBS, VTube Studio, a game or another
   application, install and configure a [virtual audio cable](virtual-audio-cable.md).
4. Return to **Text**, select **Voice only**, type “Voice test” and press
   **Speak** or `Enter`.
5. From another application, press `Ctrl+Shift+F3` and type your next phrase.
   For window or focus problems, see the [FAQ](../faq.md).

## Daily use

- `Ctrl+Shift+F3`: show the editor.
- `Enter`: send a phrase and perform the configured window action.
- `Ctrl+Enter`: send and keep typing.
- `Ctrl+Shift+F7`: open Playback control.

In **Settings → Editor → Quick editor**, choose whether sending collapses the
window or returns focus to the previous window. Clearing the text is a separate
setting. See the [editor guide](editor.md) for shortcuts, history, spelling,
routes and [minimizing, the tray and quitting](editor.md#minimizing-the-tray-and-quitting).
Minimizing keeps the application running in the background.

## Compact mode and Mono

- **Compact mode** (`Ctrl+M` or the bottom-right button) keeps the editor and
  bottom actions. The **…** menu can show/hide tabs or switch to **Mono**.
  These menu actions also work in the normal window; tab visibility is saved.
- **Mono** keeps a single text field. The title bar, tabs, actions and history
  are hidden. A thin 18 px top strip contains a drag handle, minimize button
  and a down arrow returning to compact mode. Integration icons appear only
  on errors; clicking one shows the cause without resizing the window.

Both views share saved custom window dimensions. **Start in compact mode**
under **Settings → General** restores the last compact view, including Mono.
See [compact mode](editor.md#compact-mode).

## Key interception

Assign global actions to individual NumPad, F1–F24, Page Up, End, Page Down,
Home or Insert keys, including OCR capture. **Allow interception of any key**
extends the available keys. Turning it off retains extra bindings but disables
them. An intercepted key does not reach the active application.

An occupied key shows a busy message and recording continues. With unrestricted
keys, Escape can itself be assigned: use the close button to cancel recording.
Existing interception actions are suspended while a binding is being recorded.

## Interface appearance

Under **Settings → Interface**, search installed system fonts, choose a size
from 14 to 20 px or reset the selection. Changes apply to the main window, Sound
panel and Playback control. The editor text font is configured separately.
Larger sizes have not been checked on every screen; some labels may clip or move.
**Settings → General** can hide the title-bar buttons for Sound panel and
Playback control.

## Folders and moving data

Open **Settings → General → Folders** and use **Open** next to:

- **Configuration**: settings, history and custom language packs.
- **Program data (audio cache and models)**: default `%LOCALAPPDATA%\ttsbard`.

**Change…** for application data selects a new folder and moves managed models
and audio cache with progress feedback. If model/cache folders on both sides
already contain data, move the files manually: automatic merging is not
supported. Restart if requested after a successful move.

On startup, legacy models and cache under `%APPDATA%\ttsbard` migrate
automatically, with separate completion markers. Models move to `models` in
the current application data folder before discovery. A failed migration does
not prevent startup and is retried next time. Existing files are not overwritten:
identical copies are accepted, differing copies need manual conflict resolution.
Models in the process working directory are not moved.

## Settings recovery

If settings cannot be loaded, a recovery dialog appears instead of silently
resetting them. Open the original file to repair it or confirm a reset to
defaults. The dialog reports whether a backup was created. Start the application
again yourself after repairing or resetting the file.

## Configure features

| Task | Guide |
|---|---|
| Configure a voice or install Piper | [TTS providers](tts.md) |
| Send speech to Discord, OBS, VTube Studio or games | [Virtual audio cable](virtual-audio-cable.md) |
| Select outputs or troubleshoot THX distortion | [Audio devices and output format](audio-devices.md) |
| Change the interface language or add a translation | [Language packs](language-packs.md) |
| Configure or disable suggestions | [Autocomplete](editor.md#autocomplete) |
| Send phrases to different destinations | [Phrase routes](editor.md#phrase-route) |
| Play short sounds | [Sound panel](soundpanel.md) |
| Change pitch/tempo, denoise or process audio | [Audio effects](audio-effects.md), [DSP](dsp-settings.md) |
| Correct text with AI | [AI features](ai-text.md) |
| Place Russian stress marks and resolve homographs | [RUAccent](ruaccent.md) |
| Speak text from the screen | [OCR and models](ocr.md) |
| Review incoming phrases | [Incoming](incoming.md) |
| Receive text from another application | [Incoming HTTP server](input-server.md) |
| Send text and typing state to VRChat | [VRChat](../integrations/vrchat.md) |
| Connect OBS, Twitch, VTube Studio or LunaTranslator | [Integrations](../integrations/README.md) |

## Requirements and limitations

### Language support

The interface language does not select a speech or recognition language.
TTS languages depend on the provider and voice; OCR languages depend on the
installed model pack. The OCR pack in [this guide](ocr.md) declares Russian
and English and does not translate text.

Built-in [spell checking](editor.md#spellchecking), [number conversion](editor.md#number-conversion)
and [RUAccent](ruaccent.md) currently support Russian only. Number conversion
runs before AI correction and synthesis, even with an English interface or
voice: for example, `123` becomes `сто двадцать три`. To preserve English
pronunciation, type numbers as English words or turn off **Settings → Editor →
Convert numbers to words** to pass digits to the speech engine. Conversion is
on by default. AI correction may change numbers independently of this setting.

The default [AI correction prompt](ai-text.md) also targets Russian; replace it
with instructions for your language before using correction. The separate
**AI: grammar** command always uses a built-in instruction for Russian.
Changing the Prompt field does not change that command.

### Other requirements

Cloud TTS requires internet access. Local models need a download first, then
recognition/synthesis runs on your computer. Cloud AI correction remains a
separate network step even with local TTS. Disk and memory requirements depend
on the Piper, RUAccent and OCR models and effects. There is no verified universal
CPU/RAM minimum for all configurations. Game overlays and hotkeys depend on
display mode and process privileges; see the [FAQ](../faq.md).

## Troubleshooting and support

Start with [TTS troubleshooting](../faq.md#tts-does-not-speak) if a phrase is
silent. For model/integration errors, use the corresponding guide above.
Include the application version, steps and error text in a bug report.

License: [GPL-3.0-only](../../../LICENSE); third-party models may have different
terms. The [developer statement](../../../DEVELOPER_STATEMENT.md) describes
the author's position and support boundaries (Russian).
