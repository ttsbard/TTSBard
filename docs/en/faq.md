**English | [Русский](../ru/faq.md)**

# Frequently asked questions

## Changing Silero's response format

For unsupported-format errors, switch the bot from **OGG/Opus voice messages**
to **MP3**. Send `/mp3` to [@silero_voice_bot](https://t.me/silero_voice_bot)
from the Telegram account connected to TTSBard. Wait for confirmation and retry.
It changes subsequent audio responses.
You can send `/mp3` from TTSBard with Silero selected because bot commands are
not filtered, but TTSBard expects audio and may time out even when the format
changed. Sending directly in Telegram lets you see confirmation.

## Audio decode failed / unsupported codec

Current builds advise switching Silero to MP3; older builds may show the raw
`Audio decode failed` / `unsupported codec` error. Audio arrived, but a decoder
for its codec could not be created. Send `/mp3` and retry. Renaming `.ogg` to
`.mp3` does not convert audio.
Silero voice replies in this scenario are OGG/Opus; this build lacks an Opus
decoder. [Symphonia 0.5.5](https://docs.rs/crate/symphonia/0.5.5) support for
OGG/Vorbis does not imply OGG/Opus support. If MP3 still fails, report the error,
TTSBard version and, if shareable, the problematic audio file.

## Window lost after disconnecting a monitor

Click the tray icon, choose **Show main window**, or launch the shortcut again.
If the window is off-screen or mostly hidden, it moves to the primary monitor's
center and saves the new position. Startup checks this too. Relaunching restores
the existing instance with its text and speech queue.

## Window or hotkeys do not work over a game

Some games interfere with showing/focusing the window or hotkeys at ordinary
privileges. Exit TTSBard and try **Run as administrator**. Marvel Rivals is one
user-reported example. Elevation may help a privilege mismatch, but cannot
guarantee display over every game. [Windows input restrictions](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)
describe cross-privilege limits, not a confirmed diagnosis for that game.

## TTS does not speak

1. Check the selected [TTS provider](user/tts.md): cloud credentials, Telegram,
   local server or loaded Piper model.
2. Select a voice route; Twitch only creates no audio.
3. Check Speaker, volume and the queue in Playback control (`Ctrl+Shift+F7`).
4. Check whether external text awaits manual action in [Incoming](user/incoming.md).

Read a failed phrase's notification, fix the cause and retry. One failed phrase
does not cancel subsequent jobs.

## VTube Studio's action test works, but typing does not activate the parameter

Check the keyboard-icon button at the bottom of the editor. Hover over it:
the **Send typing status (WebView, VTube Studio, VRChat)** tooltip should say
**enabled**. Clicking the button toggles typing status for all three
integrations. The choice persists between launches.

With typing status disabled, you can still type, speak and send text to chat.
The **Test action** button in the VTube Studio panel also continues to work,
since it runs the action independently of this toggle.

If you use mono mode, temporarily leave it to see the button. Enable typing
status and type a few letters in the editor. If the parameter still does not
respond, check the connection and INPUT-to-model-parameter mapping in the
[VTube Studio guide](integrations/vtube-studio.md).

## THX Spatial Audio rings or distorts

Stop current sound, select **Audio → Devices → Output format → 32-bit integer
(i32)** and retest. This solved ringing on the tested THX system. It persists
and also affects Virtual microphone, preview and Sound panel, without changing
effects/DSP. If unsupported or unhelpful, return to **Default**. See
[output format](user/audio-devices.md).

## Virtual microphone is silent

VB-CABLE is a third-party driver, not part of Windows/TTSBard. TTSBard sends
to **CABLE Input**; Discord, OBS, VTube Studio or the game records **CABLE Output**.
See [installation and testing](user/virtual-audio-cable.md). If absent, restart
Windows after driver installation, refresh Audio's devices and check Windows
sound settings. If the local test works, verify the receiver selects CABLE Output.
For i32 errors, try Default under Audio → Devices → Output format; it is shared
by both outputs.

## A hotkey does not trigger

Check **Hotkeys** and conflicts with other applications. Local editor bindings
work only in the editor; see [global/local shortcuts](user/editor.md#global-shortcuts).
For game-only failures, see the answer above.
