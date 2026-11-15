**English | [Русский](../../ru/user/audio-devices.md)**

# Audio devices and output format

**Audio → Devices** configures two local outputs:

- **Speaker**: the speakers/headphones you listen through.
- **Virtual microphone**: a virtual cable's playback device, forwarding sound
  to another application. See [virtual audio cable](virtual-audio-cable.md).

Each has its own device, enable switch and volume. The playback button beside
each device list plays a short test. The refresh button next to the tabs reloads
available devices. It is available on Devices and disabled with a loading
indicator while refreshing. Both TTS and [Sound panel](soundpanel.md) use these outputs.

A third output, [VBAN](vban.md), has its own **Audio → VBAN** tab and sends
speech and Sound panel clips to another PC. Use it alongside local outputs or
on its own. Device test buttons only test local output; use **Test** on the
VBAN tab for the network output.

## Output format

The shared **Output format** selector sits below both cards:

| Value | Behavior |
|---|---|
| **Default** | Uses each device's default configuration; the initial setting. |
| **32-bit integer (i32)** | Requests integer samples at the default configuration's sample rate and channel count. |

This applies to **local TTSBard audio devices**: Speaker and Virtual microphone
for TTS, previews, test audio and Sound panel. VBAN uses its own fixed 16-bit PCM,
48 kHz stereo format; selecting i32 does not change it.
Support is checked per device. Windows
settings and other applications are unaffected. The choice persists across
restarts; upgrading from a version without it selects Default.

## When to try i32

Try i32 for ringing or severe distortion. It resolved ringing on the tested THX
Spatial Audio setup; the virtual cable on that system also worked. Results
depend on the driver; this does not solve every audio issue.

1. Stop speech, preview and Sound panel sounds.
2. Select **32-bit integer (i32)**.
3. Test the output, then replay the same phrase/file at the same volume.
4. For a cable, listen in the receiving application.

An already open stream does not change format mid-playback. The panel shows
**Applies after current playback stops** while an old-format stream remains.
The setting takes effect when a new stream opens; merely speaking the next
phrase through an existing stream does not guarantee the change.

## If i32 is unsupported

An unsupported rate/channel combination or failed stream opening reports an
error; i32 does not silently fall back to another format. Select **Default**,
stop playback and retest. Failure on one output does not cancel another
independent output that opened successfully.

## Effects and files

Effects, DSP and mixing remain in f32. i32 converts samples immediately before
device delivery; effect settings, audio cache and source-audio export do not
change. i32 is a sample representation, not a quality boost. Overloaded signals
are clipped to its range, so volume and [limiter settings](dsp-settings.md) matter.
