**English | [Русский](../../ru/user/vban.md)**

# Network audio output: VBAN

**Audio → VBAN** sends TTSBard speech and Sound panel clips to another PC on
your local network. For example, keep headphones and a game microphone on the
gaming PC while receiving a separate voice source for OBS, Discord or
microphone-based VTube Studio lip sync on the streaming PC.

TTSBard sends the audio. On the receiving PC, choose either option:

- **[VBAN Receptor](#receive-audio-with-vban-receptor)** is a simple receiver with one output.
- **[Voicemeeter](#receive-audio-with-voicemeeter)** is a mixer that can route
  audio to speakers and OBS at the same time.

To route audio through **CABLE Input → CABLE Output**, install a
[virtual audio cable](virtual-audio-cable.md) on the receiving PC.

## Audio flow

```text
TTSBard PC → VBAN over LAN → VBAN Receptor on receiving PC
                            └→ A1: CABLE Input
                                   └→ CABLE Output → OBS / Discord / VTube Studio
```

Select ordinary headphones or speakers in Receptor if you only need to listen
on the other PC. Sending VBAN does not require a cable on the TTSBard PC or a
separate VBAN Talkie sender.

## Receive audio with VBAN Receptor

Start [VBAN Receptor for Windows](https://vb-audio.com/Voicemeeter/vban.htm)
on the receiving PC.

1. Connect both PCs to the local network.
2. Find the receiving PC's IP address: in VBAN Receptor, click the top panel
   containing the graph to open the settings window, which shows the IP address.
   Alternatively,
   run `ipconfig` in a Windows terminal and find IPv4 for the active Ethernet
   or Wi-Fi adapter.
3. Check Receptor's VBAN port: the default is **UDP 6980**. Allow Receptor's
   incoming traffic through the firewall for your private network. You do not
   need to disable the firewall.
4. Under Receptor's **A1**, select **CABLE Input (VB-Audio Virtual Cable)** to
   pass audio to applications as a microphone, or headphones to listen directly.

## Configure TTSBard and select the stream in Receptor

1. Open **Audio → VBAN** in TTSBard.
2. Enter the **receiving PC's IPv4 address** under **IP**, for example
   `192.168.1.50`. This field takes an address; it is not a discovered-device list.
3. Match **Port** to Receptor, usually `6980`.
4. Keep **Stream name** as `TTSBard` or enter your own: 1–16 printable ASCII
   characters, such as Latin letters and digits.
5. Check network output **Volume**: 0–100%, with a default of 100%.
6. Click **Test**. It sends the built-in sound using the current form values,
   even with the output disabled. It does not change saved settings or play
   through the local Speaker or Virtual microphone outputs.
7. **While audio is being sent, click `Stream Name` in VBAN Receptor and select
   the incoming `TTSBard` stream** (or your custom name from step 4).
   You must explicitly select the stream in the receiver; entering the IP in
   TTSBard alone is not enough. **The stream appears in Receptor's list while
   audio is being sent.** If the list is empty or you missed it, run **Test**
   again and open `Stream Name` during transmission.
8. Check Receptor's meter and output sound. Repeat the test after selecting
   the stream if needed.
9. Click **Save**, then turn on **Enable VBAN output** to send regular speech
   and Sound panel audio. Address, port and stream name apply after saving.
   The output toggle and volume apply and save automatically.

In OBS, add an **Audio Input Capture** source using **CABLE Output**. In Discord,
VTube Studio or a game, select **CABLE Output** as the microphone on the receiving
PC. Check the VBAN test first, then a regular TTSBard phrase and a Sound panel clip.

To hear the cable through headphones on the receiving PC, enable
[Windows monitoring through `mmsys.cpl`](virtual-audio-cable.md#listen-to-the-cable-through-windows).

## Receive audio with Voicemeeter

This example uses [Voicemeeter](https://vb-audio.com/Voicemeeter/index.htm)
with two outputs on the streaming PC.

1. Open **VBAN** in Voicemeeter and click **VBAN is OFF** to enable VBAN.
2. Under **Incoming Streams**, manually enter the exact stream name from TTSBard
   in **Stream Name** and the TTSBard PC's LAN IP address in **IP Address From**.
   In TTSBard, enter the streaming PC's IP address under **Audio → VBAN**. Use the same UDP port on
   both sides (default `6980`).
3. In the same row, switch **On** and select a mixer input under **Destination**.
4. Under **Hardware Out**, assign your speakers to **A1** and **CABLE Input
   (VB-Audio Virtual Cable)** to **A2**. On the selected input strip, enable
   **A**. This bus sends audio to both devices.
5. In OBS, add an **Audio Input Capture** source and select **CABLE Output
   (VB-Audio Virtual Cable)**. Click **Test** in TTSBard and check the
   incoming stream indicator, speaker playback, and the OBS audio meter.
6. In TTSBard, click **Save**, then turn on **Enable VBAN output** to send
   regular speech and Sound panel audio.

## What gets sent

- Regular speech, incoming phrases, repeats and history playback.
- Sound panel clips. Simultaneous speech and clips mix into one VBAN stream;
  speech controls do not stop the separate Sound panel queue.
- Speaker, Virtual microphone and VBAN have independent switches. You can use
  VBAN alone by disabling both local outputs under **Audio → Devices**.
- Local device tests and effects/DSP previews stay local. Use **Test**
  specifically to test the network output.

VBAN's volume controls the network mix. The network format is fixed at
**16-bit PCM, 48 kHz, stereo**, sent to one IPv4 destination. The i32 selector
under **Audio → Devices → Output format** affects local devices and does not
change VBAN. Network output is disabled by default.

## Receiver check

While VBAN is enabled, the tab's dot shows receiver availability:
grey — no result yet, green — responding, red — not responding.
**Ping** sets the interval from 1 to 300 seconds (default: 30).
Apply the change with **Save**.

A 5-second warning appears when the connection is lost or audio is sent
to an unavailable receiver. Audio sending continues. If the local UDP port
is busy, the message asks you to free it. A successful ping confirms that
the service is reachable, but does not confirm audio playback.

## If there is no sound

**Test sent** confirms sending from this PC. The tab's **Ready to send**,
**Sending audio...** and **Send error** statuses also describe the local sender.
UDP does not confirm that the receiver received or played the audio.

For Voicemeeter, check both VBAN switches, the stream name, sender IP address,
UDP port, the **A** button on the selected input strip, and the **A1** and **A2** devices.

1. If the stream is missing, repeat the test and open **Stream Name** during
   transmission. Check the destination IPv4, matching UDP port, network and
   Receptor's firewall permission. With multiple adapters, use the shared LAN.
2. If the stream is visible but silent, select it in Receptor, check **A1**,
   turn off Mute and check volumes in TTSBard and Receptor.
3. If Receptor receives audio but OBS/Discord does not, verify
   **Receptor → CABLE Input**, **application → CABLE Output** on the receiving PC.
4. If the test works but regular speech does not, confirm VBAN is enabled and
   settings are **saved**. Use **Voice only** to test with a phrase.
5. If the test reports the stream is occupied by regular playback, wait for
   speech and clips to finish, then retry.

A network error does not cancel a working local output. With local outputs
disabled, check playback directly on the receiving PC.
