**English | [Русский](../../ru/user/guide.md)**

# User guide

TTSBard is a Windows 10/11 (64-bit) application for typing and speaking phrases
from an editor opened with a hotkey.

## First launch

1. Install your version from the [project releases](https://github.com/ttsbard/TTSBard/releases) and start TTSBard.
2. Open **Audio → Devices**. Turn on **Speaker**, select your headphones or
   speakers by name under **Device**, and click ▶ **Test playback**. Check that
   you hear the test sound on the intended output.

   Selecting a device by name keeps speech on that output and lets TTSBard
   reuse the audio stream between phrases. With **(default)**, TTSBard checks
   the Windows output and opens a new stream for each phrase. Selecting a device
   by name is especially useful if you use multichannel audio such as 5.1 or 7.1
   surround sound and want to keep speech on a dedicated output.

   To send speech to Discord, OBS, or a game, enable **Virtual microphone** and
   select the cable's playback device in TTSBard. For VB-CABLE, select
   **CABLE Input**. In the receiving app, select **CABLE Output** as the microphone.
   Click ▶ next to the virtual microphone in TTSBard and check that the meter in
   the receiving app responds. [Set up a virtual audio cable](virtual-audio-cable.md).

   To send speech and Sound panel clips to another PC, open **Audio → VBAN**,
   enter the receiver's IP and run the test. In Receptor on the receiving PC,
   select the stream under **Stream Name** while audio is being sent: that is
   when it appears in the list. Then save the receiver settings and enable VBAN. [VBAN setup](vban.md).
3. Set up a voice using one of these options.
   - **Silero via Telegram.** Open **TTS → Silero**, sign in to Telegram, choose
     a voice, and set Silero as the active provider. [Set up Silero](tts.md#silero-bot-telegram).
   - **Local model.** Open **Settings → Models**. If the list is empty, click
     **Download catalog from repository**. Download a **Pocket TTS** package,
     return to **TTS**, and select the installed model. Depending on the model,
     choose a ready voice or provide a WAV reference. Activate the model for speech.
     [Set up Pocket TTS](tts.md#pocket-tts) · [Other options, including Piper](tts.md).
4. Open **Text**, select **Voice only**, type “Voice test”, and press **Speak**
   or `Enter`. Check that you hear the phrase on the selected output or in the
   receiving app.
5. From another application, press `Ctrl+Shift+F3` and type your next phrase.
   For window or focus problems, see the [FAQ](../faq.md).

After your first phrase, you can set up [OCR to read text from the screen](ocr.md#enable-and-test)
and assign shortcuts in **Hotkeys**. With [Key interception](#key-interception),
you can assign an action such as OCR capture to a single key. Press `Ctrl+M` for
a smaller window. To keep just the text field, select **Switch to Mono mode**
from the editor's **…** menu.

## Update TTSBard

Open **Settings → General → Updates** and click **Check now**. If a new
version is available, the card shows its release notes.

Automatic checks are configured by two independent switches in the same card:

- **“Check for updates when the app starts”** — enabled by default. TTSBard
  performs one check each time the app starts. Restoring from the tray,
  calling a hotkey, or opening the window or settings does not run the check
  again.
- **“Check for updates while the app is running”** — disabled by default. Turn it
  on and pick an interval of 1 to 7 days, 1 day by default. The check keeps
  running while the window sits in the tray. While the switch is off, the
  interval selector is unavailable, but the chosen value is kept.

You can turn both switches off; then the version is only checked manually
with **Check now**.

The card keeps the outcome of the last completed check — “You’re up to date”,
the found release notes, or a calm error message — until the next check,
including across app restarts. A failed check does not hide a previously
found release and does not change the time of the last successful check.
There are no five-minute retries after a failure: if only the startup check
is enabled, the next automatic attempt happens the next time TTSBard starts;
with the schedule enabled, it happens after the chosen interval, including
in the tray.

- If you installed TTSBard with `*_x64-setup.exe`, click **Update** below the
  release notes when a signed update is available. You can cancel while it
  downloads. After the download and signature check, TTSBard closes and starts
  the installer.
- If you use a standalone `ttsbard.exe`, click **Open on GitHub**, download
  the new `ttsbard.exe`, close TTSBard, and replace the file. This also applies
  if you have another copy of TTSBard installed on the same PC.

If the card offers **Open on GitHub**, download and run `*_x64-setup.exe`
manually. Future in-app updates require an installed build with a public
verification key and a signed installer for the new release.

## Daily use

- **Show the editor** with `Ctrl+Shift+F3` or the key assigned to **Main window**
  in [Key interception](#key-interception).
- **In the editor**
  - `Enter` sends the phrase and performs the configured window action.
  - `Ctrl+Enter` sends the phrase and lets you keep typing.

### Communicate through TTSBard while gaming or working

Here are two ways to set up the quick editor. Choose what happens after sending
a phrase under **Settings → Editor → Quick editor**.

In either setup, open the editor with a [shortcut](editor.md#configure-shortcuts)
(`Ctrl+Shift+F3` by default) or press a single key assigned to the **Main window**
action in [Key interception](#key-interception).

- **Open the editor for each phrase.** If you use one monitor and want to clear
  the screen after sending, choose **Collapse**. Open the editor, type your
  phrase, and press `Enter`. TTSBard hides until you open it again.
- **Keep the editor open.** If you use two monitors, move TTSBard to the second
  screen and choose **Return focus to the previous window**. Open the editor from
  your game or work app, type your phrase, and press `Enter`. TTSBard stays on the
  second screen, and focus returns to the previous window. This also works on
  one monitor if there's room for the compact editor beside your work window.

Press `Ctrl+Enter` to send a phrase and keep typing in the editor. Clearing the
text is a separate setting. TTSBard keeps running in the background when hidden.
See the [editor guide](editor.md) and
[Minimizing, the tray and quitting](editor.md#minimizing-the-tray-and-quitting) for details.

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
Home or Insert keys. To open the editor with a single key, assign the **Main
window** action. You can also assign OCR capture to read text from the screen.
**Allow interception of any key** extends the available keys. Turning it off
retains extra bindings and disables them. While interception is on, Windows
and other applications do not receive presses of the assigned keys.

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
| Configure a voice or set up Piper or Pocket TTS | [TTS providers](tts.md) |
| Send speech to Discord, OBS, VTube Studio or games | [Virtual audio cable](virtual-audio-cable.md) |
| Select outputs or troubleshoot THX distortion | [Audio devices and output format](audio-devices.md) |
| Send speech and Sound panel clips to another PC | [VBAN](vban.md) |
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
