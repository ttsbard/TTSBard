**English | [Русский](../../ru/user/tts.md)**

# TTS providers

Open **TTS**. The `⋯` menu shows or hides provider cards and models; the active
provider always remains visible. Hiding a card retains its settings. Expand a
card, fill in the fields, save, then select it with its provider switch.

Speech languages depend on the selected provider and voice/model, not the
interface language. A Piper voice's language is defined by its installed model.
Russian text preparation still runs before synthesis; see
[language limitations](guide.md#language-support) before speaking another language.

## Silero Bot (Telegram)

1. In **Silero**, open the Telegram connection and sign in.
2. Select a voice after connecting.
3. In Telegram, set **@silero_voice_bot** to MP3 if necessary:
   [change the response format](../faq.md#changing-sileros-response-format).
4. Return to the editor and speak a test phrase.

No TTS API key is required, but a connected Telegram account and network access
are needed. Additional [timeouts and delays](silero-settings.md) are edited manually.

## OpenAI TTS

1. Enter your API key in **OpenAI** and click **Save**.
2. Select alloy, echo, fable, onyx, nova or shimmer: these are the options
   exposed by the TTSBard interface.
3. Configure a proxy if needed and select this provider.

To change playback tempo, use [audio effects](audio-effects.md).

## ElevenLabs

1. Show **ElevenLabs** through `⋯`; the card is hidden by default.
2. Create a key with **Text to Speech — Access**, **Models — Access** and
   **Voices — Read**, enter it and click **Save**. Enable the shared proxy first
   if needed.
3. Wait for the independent voice and model catalogs to load. If one fails,
   retry with its refresh button.
4. Choose a voice and model. Voice and proxy changes apply immediately;
   after changing output format or generation parameters, click **Apply**.
5. Select ElevenLabs and speak a short test phrase.

Catalogs depend on the account and its models, and reload when the key changes.
Text is sent to ElevenLabs. Streaming, voice cloning and playback of external
preview audio are not supported in the application.

## Fish Audio

Set the API key and voice/reference ID, save and select the provider. Audio
format, temperature, sample rate and proxy use configure synthesis at the
provider. TTSBard post-processing is configured separately under Audio.

## Local HTTP server

Start a compatible TTS server, such as TITTS.py for TTS Voice Wizard's Locally
Hosted mode. Set its URL (default `http://127.0.0.1:8124`), save and select it.
TTSBard requests audio from this URL. It is different from the
[incoming server](input-server.md), which receives text on port `10101`.

## Piper

Piper synthesizes speech inside TTSBard without a separate HTTP server.

1. In **Settings → General → Folders**, click **Open** next to **Program data
   (audio cache and models)**. Create `models\piper` and put matching `name.onnx` and
   `name.onnx.json` voice files there.
2. Restart TTSBard and open **TTS**.
3. Show a hidden model through `⋯` if necessary.
4. Select a **Piper** row and wait until ready.
5. Speak a test phrase using **Voice only**.

```text
%LOCALAPPDATA%\ttsbard\models\piper\
  name.onnx
  name.onnx.json
```

For a custom data folder use `models\piper` inside it. Models are also discovered
in `<process working directory>\models\piper`; a valid working-directory model
takes precedence for duplicate IDs. The working directory depends on how you
start the application and may differ from the executable folder.
Legacy `%APPDATA%\ttsbard\models` migrate on startup to `models` under the
current data folder without overwriting existing files. Resolve differing
copies manually; see [folders and moving data](guide.md#folders-and-moving-data).

`espeak-ng-data` ships with the application. TTSBard searches application
resources, application data, the working directory and beside the executable;
system eSpeak NG discovery remains available if local data is absent.
Phonemization readiness is checked before loading the voice. Restore missing or
corrupted data or the eSpeak NG installation, then restart.

The voice and configuration must match; a configuration from another voice will
not work. After installation Piper itself needs no network. RUAccent is separate
text processing with its own [model package](ruaccent.md).

## Pocket TTS

Pocket TTS runs locally on CPU without external servers or API keys.

1. Install a Pocket TTS model package (such as **Shtorm RU v2 Fast** or **Kyutai English**)
   under **Settings → Models**.
2. Open the **TTS** panel, expand the **Pocket TTS** card, and select the model.
3. For voice-cloning models (reference WAV mode):
   - Click **"Choose WAV"** and pick a short voice clip (up to 12 seconds; 5–8 seconds
     of clear speech with a complete sentence is recommended).
   - If the cloned voice sounds too low in pitch or muffled due to an overly loud
     recording, click **"Prepare"** next to the selected reference. The app will
     generate an optimized copy normalized for the model (target speech RMS ~0.048,
     24 kHz mono) in the application data folder and switch to it automatically without
     modifying your original file.
   - Use the **"Listen"** button to preview the active reference audio.
4. For models with bundled voices (such as Kyutai English with Alba), select the
   desired voice from the list.

## Check your setup

New jobs use the selected provider. Accepted jobs retain their provider and voice.
If silent, check provider status, queue and output in the
[FAQ](../faq.md#tts-does-not-speak).
