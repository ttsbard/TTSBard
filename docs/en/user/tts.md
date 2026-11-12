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

Pocket TTS synthesizes speech locally on CPU without external servers, API keys,
or dedicated GPUs. Powered by the Kyutai architecture (Mimi neural audio codec +
FlowLM autoregressive language model), it runs via ONNX Runtime within TTSBard's
native Rust runtime.

### Models and Installation

Model packages are installed via the built-in catalog in **Settings → Models**:
- **Shtorm RU v2 Fast** — Russian voice cloning model from a reference audio clip,
  optimized for fast inference (INT8 quantization);
- **Shtorm RU v2 Slow** — Russian voice cloning model with more expressive intonations;
- **Kyutai English Clone** — English voice cloning model from a reference WAV;
- **Bundled voice models** (such as Kyutai English with prepackaged voices like Alba
  stored as safetensors embeddings).

Model files reside in `%LOCALAPPDATA%\ttsbard\models\pocket\<package-id>`
(or `models\pocket\<package-id>` under the working directory). Each package
contains a `config.yaml` manifest, weights, a tokenizer (SentencePiece for
Russian, JSON for English), and a set of ONNX graphs (`mimi_encoder`,
`text_conditioner`, `flow_lm_main`, `flow_lm_flow`, `mimi_decoder`).

### Voice Modes

1. **Voice cloning (`reference_wav`):**
   - In the Pocket TTS card, click **"Choose WAV"** to select an audio file containing
     a voice sample.
   - Click **"Listen"** to preview the active reference audio via the preview device.
   - Click **"Prepare"** to normalize and adapt the reference file for optimal neural
     synthesis (detailed below).
2. **Preset voices (`voice_state`):**
   - If the package includes bundled voice states (`embeddings/*.safetensors`),
     a dropdown menu will list the available voices. Select the desired voice
     without supplying an audio clip.

### Reference Requirements and Model Peculiarities

Cloned voice fidelity directly depends on the characteristics of the reference clip:

- **Duration:** 0.05 to 12 seconds (the 12s hard cap prevents ONNX memory overflow).
  The recommended sweet spot is **5–8 seconds** of continuous speech.
- **Content:** clean, coherent speech from a single speaker with natural intonation
  and a **complete sentence** (avoid clipping mid-word, background music, background
  talkers, or heavy room reverb).
- **Sampling and quality:** use high-quality audio (24–48 kHz) without aggressively
  low-passed high frequencies.

### Why Reference Normalization is Essential

Acoustic investigation of the Mimi + FlowLM pipeline revealed critical physical
behavior:

1. **No amplitude invariance in the Mimi encoder:**
   `mimi_encoder` lacks built-in automatic gain control or input normalization.
   The raw amplitude scale of the input audio passes directly into the latent space
   and into FlowLM.
2. **Pitch (F0) collapse on loud references:**
   When given a typical loud studio recording (active speech RMS ~0.15–0.20 or
   peaked at 0 dBFS), FlowLM **depresses the fundamental frequency (F0) by 1.5–2.5
   semitones** (`-1.7..-2.4 st`), and halves the intonation range (IQR). A female voice
   sounds unnaturally deep, muffled, and flat, while male voices lose articulation.
3. **Pitch inflation on quiet references:**
   If the input is too quiet (RMS below 0.03), FlowLM artificially shifts the pitch
   upward (`+3..+5 st`), making deep male voices unnaturally high.
4. **Treble attenuation in the Mimi codec:**
   The Mimi codec naturally attenuates frequencies above 2 kHz by 3.7–5.4 dB (dropping
   the spectral centroid by 80–105 Hz) even in roundtrip encode-decode. Starting
   with a muffled reference makes the synthetic speech noticeably dark.

**The optimal F0 calibration target** across both female and male voices is an
active speech level of **`RMS ~0.045–0.050 (~ -26 dBFS)`**.

### The "Prepare" Function (Reference Normalization)

To eliminate pitch and timbre distortion, the Pocket TTS card features a dedicated
**"Prepare"** button next to the selected reference WAV:

1. The app decodes the source WAV (leaving the original file completely untouched).
2. The audio is downmixed to mono and resampled to 24 kHz using a windowed-sinc
   polyphase filter (with anti-aliasing cutoff aligned to the target Nyquist
   frequency: `cutoff = min(1.0, f_out / f_in)`, preserving the full spectrum up
   to 12 kHz without high-frequency attenuation).
3. Active speech frames (20 ms frames with energy >= 10% of overall RMS) are
   analyzed, isolating speech from silence.
4. The signal is scaled precisely to **`RMS = 0.048`** with a protective peak cap of
   `0.98` to avoid clipping.
5. The prepared copy is saved to `%APPDATA%\ttsbard\pocket-references\<package-id>.wav`,
   and the model immediately switches to this optimized file.

### Automatic Output Gain Compensation

Because the normalized reference (RMS ~0.048) causes raw model output to be quieter
(`out_rms ~0.04–0.05`), the TTSBard Pocket TTS runtime applies automatic soft gain
compensation:
- Output PCM is gently boosted toward **`RMS = 0.12`** (with peak limit `0.98` and a
  maximum boost factor of 4.0×).
- This ensures Pocket TTS matches the loudness of other engines (Silero, Piper,
  ElevenLabs) without distortion.

## Check your setup

New jobs use the selected provider. Accepted jobs retain their provider and voice.
If silent, check provider status, queue and output in the
[FAQ](../faq.md#tts-does-not-speak).
