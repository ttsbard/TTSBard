**English | [Русский](../ru/ttsbard-overview.md)**

# TTSBard: quick speech for communication and streaming

**TTSBard** helps you communicate through text-to-speech and create content.
Open the window with a hotkey, type and send: the application speaks your phrase.
Send speech to games/voice chats, display it on stream and post it to Twitch.
TTSBard runs in the background; global shortcuts can be changed or disabled.

| Feature | Default shortcut |
|---|---|
| Editor | `Ctrl+Shift+F3` |
| Sound panel | `Ctrl+Shift+F2` |
| Playback control | `Ctrl+Shift+F7` |
| Screen OCR | `Ctrl+Shift+P` |

Start with the [first-launch guide](user/guide.md), or download the
[short PDF overview](ttsbard-overview.pdf).

## 1. Quick access and editor

The main window opens above other applications ready to type. After sending,
Quick editor can collapse it, return focus to the previous application or keep
the editor active, according to your settings.

- **Tabs and delivery**: text, order and selected tab persist. Each tab can
  route to speech, OBS, Twitch and/or VRChat; change individual phrase routes.
- **History and suggestions**: search, reinsert or replay the last 500 phrases.
  Autocomplete uses history and a personal dictionary and can be disabled.
- **Spelling**: local Russian spellchecking underlines typos with right-click corrections.
- **Fonts**: choose installed Windows fonts and editor text size independently
  of the interface font.
- **Compact and Mono**: resize both axes, hide tabs independently or use a
  single-field Mono view. Tab visibility, view and size persist; compact startup
  is available.
- **Audio reuse/export**: repeat synthesized phrases without another request
  or export original provider audio without effects.

Minimize to the taskbar or optionally tray-only while continuing background
work. Relaunching restores the existing copy, text and queue.
See [editor, history, routes and windows](user/editor.md).

## 2. TTS providers

Combine cloud services and locally installed voices:

- **Silero Bot**: your Telegram account with [@silero_voice_bot](https://t.me/silero_voice_bot),
  cloud-password/2FA login, voice selection, remaining quota, SOCKS5 and MTProxy.
- **Piper**: installed voice models, each representing a voice; synthesis runs
  inside TTSBard without network or a separate server. Add compatible models.
- **OpenAI TTS**: alloy, echo, fable, onyx, nova and shimmer as exposed by
  TTSBard, with API-key authentication and proxy support.
- **ElevenLabs**: account-specific model/voice catalogs, stability, similarity,
  style, voice boost and proxy; requires a suitably permitted API key.
- **Fish Audio**: saved voice models, synthesis/audio-format settings and proxy;
  requires an API key.
- **Local HTTP TTS**: compatible servers including TTS Voice Wizard's Locally
  Hosted mode; distinct from embedded Piper.

See [provider setup and models](user/tts.md).

## 3. Audio processing and outputs

Adjust pitch, tempo and volume, preserve timbre and preview WAV/MP3 files.
DeepFilterNet removes noise; EQ, compressor and safety limiter add independent
processing. Listen locally while sending the same speech through a virtual
microphone, using VB-CABLE or VoiceMeeter for Discord/games. Outputs have
independent switches and volumes. Default devices follow Windows output changes.
The separate Playback control window pauses/resumes/stops/repeats; history
can replay older phrases.

See [effects](user/audio-effects.md), [DSP settings](user/dsp-settings.md),
[devices](user/audio-devices.md) and [virtual microphone](user/virtual-audio-cable.md).

## 4. Text preparation

**Quick insert** expands short codes to phrases/names from your lists.
See [quick insert](user/editor.md#quick-insert).
**Number preparation** converts numbers to Russian words with agreement.
It is on by default; turn it off in Editor settings to pass digits to the speech engine;
see [number conversion](user/editor.md#number-conversion).
**RUAccent** locally places Russian stress and distinguishes homographs
(for example castle/lock: `за́мок`/`замо́к`). It supports Silero/Piper,
automatically before synthesis or manually in the editor. Install its model
separately: [RUAccent](user/ruaccent.md).
**Experimental AI correction** fixes typos/punctuation or changes style using
your instructions. Enable/disable correction in the editor. OpenAI, DeepSeek,
Z.ai and custom OpenAI-compatible services are supported; see [AI](user/ai-text.md).

## 5. Sound panel

A floating grid assigns files to `A–Z`/`0–9` across independent sets. Play with
keys/clicks and edit assignments in the same panel. Formats: MP3, WAV, OGG,
FLAC, M4A, AAC and WMA. Pin the panel to keep it visible after returning to a
game; configure its color/transparency separately. It shares speech outputs
but has its own sequential queue, so rapid sounds do not overlap or interrupt
speech. See [Sound panel](user/soundpanel.md).

## 6. Text from other sources

- **Incoming**: automatically speak or manually speak/edit/reject HTTP/OCR
  text. Selected voice, enabled preparation and effects apply; route to audio
  alone or also OBS/Twitch/VRChat. See [Incoming](user/incoming.md).
- **Web form**: type in an ordinary browser, Steam Overlay during a game, or
  another LAN device while Input server runs. See [overlay form](user/input-server.md#overlay-input-form).
- **Screen OCR**: select an area on a chosen monitor for local recognition
  using separately installed models, without LunaTranslator. See [OCR](user/ocr.md).
- **LunaTranslator**: receive visual-novel/game text from clipboard, screen
  areas or game engines, then process/speak in TTSBard. See
  [integration](integrations/lunatranslator/README.md).
- **HTTP server**: other applications on this PC or LAN send text for speech;
  remote access requires a token. See [Input server](user/input-server.md).

## 7. Communication and streaming integrations

**OBS** displays spoken phrases with custom WebView templates, LAN access and
token-protected external access, optionally UPnP. See [WebView](integrations/webview.md).
**Twitch Chat** uses official Helix with Device Code Flow (no bot moderator
rights) or classic OAuth-token IRC. API credentials/tokens use Windows DPAPI.
Startup/reconnection are available; editor routes can exclude Twitch for a
phrase. See [Twitch](integrations/twitch.md).
**VRChat** receives completed phrases and optional typing over OSC, never the
draft. Everywhere/No Twitch include it; Voice only excludes it. Long text is
shortened only for the chatbox; original/speech remain. UDP confirms no receipt.
See [VRChat](integrations/vrchat.md).
**VTube Studio** drives a model parameter, hotkeys or scene item while typing,
locally or on another PC. See [VTube Studio](integrations/vtube-studio.md).
TTSBard windows can be excluded from OBS, Discord and other capture tools to
hide text being typed on stream.

## 8. Key interception

Assign individual keys to open windows, pause/resume/stop/repeat speech or
capture screen text. NumPad, F1–F24 and navigation keys are available by default;
an option allows any key. Assigned presses do not reach the active application.
Disabling unrestricted choice preserves extra bindings but stops using them.
This is configured separately from ordinary global shortcuts; see
[key interception](user/guide.md#key-interception).

## 9. Platform, language and settings

Windows 10/11, light/dark themes, window color/transparency and installed system
interface fonts are supported. Editor font is independent; extra title-bar
window buttons can be hidden. English/Russian interface changes apply after
restart. Add external [language packs](user/language-packs.md) without rebuilding.
Interface translations do not add text-processing languages. Offline spelling,
number conversion and RUAccent are Russian-only; default AI correction
instructions also target Russian. Digits become Russian words before AI/TTS,
even with an English voice. Speech languages depend on the provider/voice;
OCR languages depend on the model and OCR does not translate text. See
[language limitations](user/guide.md#language-support).
Open configuration/data folders, move models/cache with progress and recover
from settings errors by repair or confirmed reset rather than silent loss.
See [folders and moving data](user/guide.md#folders-and-moving-data).
