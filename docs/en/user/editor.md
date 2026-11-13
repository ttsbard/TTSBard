**English | [Русский](../../ru/user/editor.md)**

# Editor, routes and shortcuts

For initial TTS configuration, start with the [user guide](guide.md).

## Global shortcuts

| Default | Action |
|---|---|
| `Ctrl+Shift+F2` | Show Sound panel |
| `Ctrl+Shift+F3` | Show and focus the main window above others |
| `Ctrl+Shift+F4` | Pause/resume playback |
| `Ctrl+Shift+F5` | Stop playback |
| `Ctrl+Shift+F6` | Repeat the last phrase |
| `Ctrl+Shift+F7` | Show Playback control |
| `Ctrl+Shift+P` | Select an area for one-shot OCR |
| `Ctrl+F` | Return focus to the previous window |

## Configure shortcuts

Open **Hotkeys** in the sidebar, choose a function, click record, press a
combination and save. Sound panel, main window, playback actions/control and
return-focus bindings can be customized.

### Local editor shortcuts

The **Editor** section applies only inside CodeMirror and the tab bar. These
are not registered as Windows system shortcuts. You can change each combination
and turn it on or off in the **Editor** section of the Hotkeys panel.

| Default | Action |
|---|---|
| `Ctrl+E` | Show spelling corrections for the word at the cursor |
| `Ctrl+Enter` | Speak and keep editing |
| `Alt+Enter` | Speak with the clear-text setting inverted for this send |
| `Ctrl+Alt+Enter` | Speak, retain text and keep editing |
| `F7` / `Shift+F7` | Next/previous spelling error |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next/previous tab |
| `Ctrl+R` | Cycle phrase route (off by default) |
| `Ctrl+T` | Toggle typing state delivery to WebView/VTube Studio/VRChat (off by default) |
| `Ctrl+W` | Quick editor mode (off by default) |
| `Ctrl+H` | Phrase history |
| `Ctrl+U` | Place stresses once |

Letter bindings use physical keys and work in both Russian/English layouts.
Ordinary Enter performs the configured Quick editor action: collapse, return
focus or stay. Ctrl+Enter always keeps the window open and refocuses the editor;
it does not disable Quick editor for the next ordinary Enter.

The full editor shows an Enter-action hint beside Speak, including the binding
for sending and continuing. Compact mode uses an icon with the hint in its
tooltip and accessible label.

### Editor font and size

The top card under **Settings → Editor** selects text font and size **12–32 px**,
with a sample. Choices include Default, System, Arial, Georgia and Consolas;
the initial size is **16 px**. Changes immediately affect working tabs and
compact mode and persist without losing text/undo history. Controls are briefly
disabled while saving; retry if a save fails.

### Keep text after sending

By default, successful sending clears text. Enable the keep-text setting under
Settings → Editor to retain it for corrections or repetition; use Ctrl+A to
replace it when done. Text clearing and window behavior are independent:

- **Alt+Enter** inverts clearing for one send while applying the usual Quick
  editor action.
- **Ctrl+Alt+Enter** always sends, retains text and keeps the editor focused.

Repeating identical text with the same voice/provider and unchanged effects
uses the local phrase cache instead of spending provider quota.

### Autocomplete

Suggestions come from your dictionary, phrase history, AI continuations and
quick replacements. Usage frequency affects ranking but is not displayed.
Disable **Show autocomplete suggestions** under **Settings → Editor → Proofing
and suggestions** to stop opening the list and querying local/AI sources.
Successfully sent phrases still populate history; re-enabling suggestions uses
the statistics accumulated meanwhile.
**AI text continuation (autocomplete)** controls only AI; both switches must be
on for AI suggestions. Manual [AI continuation](ai-text.md) is separate.

### Phrase history

History retains the last 500 sent phrases. Replaying uses the selected entry,
even if several entries have identical text. Open with Ctrl+H. The list may
show stress markers actually sent to TTS. Replace/append actions insert clean
text without technical Silero/Piper markers for editing and resending.

## Application layout

### Sidebar

- **Text**: editor.
- **TTS**: providers and voices.
- **Audio**: devices and volume.
- **WebView**: OBS server.
- **Twitch Chat**: Twitch API or classic IRC.
- **VTube Studio**: connection and typing action.
- **[VRChat](../integrations/vrchat.md)**: text/typing over OSC.
- **OCR**: local screen recognition.
- **Input server**: incoming text for speech.
- **Quick insert**: replacements/presets.
- **Hotkeys**, **Key interception**, **Settings**: supporting tools.

Sound panel is a separate floating window opened from the title bar or a global
hotkey, not a sidebar page.

### Integration status in the title bar

WebView, Twitch, VTube Studio, VRChat and Input server icons appear when active
or in error; stopped integrations are hidden. VRChat means OSC sending is
enabled, not that a client acknowledged connection. Manual start-failure details
are also available on each integration panel. Hover for details. Normal/compact
icons are indicators; Mono error icons open the error on click.

### Compact mode

Toggle with the bottom-right button or Ctrl+M, configurable in Hotkeys' main
window section. The editor and bottom actions remain. The **…** menu shows or
hides tabs in either normal/compact mode. This persists across restarts and
leaving Mono without losing tabs or text; shortcuts still switch tabs. Incoming
keeps tabs visible so you can return to text.

Resize both axes using window edges: minimum 300 × 300 physical pixels,
maximum equivalent to 800 × 630 logical pixels at the Windows scale. Dimensions
are saved. Resizing is disabled while history is open; its list scrolls inside.

The **…** menu's Mono action works in normal/compact mode, including empty
text and disabled AI. From normal mode it uses the saved compact size;
switching compact ↔ Mono preserves size. Mono hides the title bar, tabs,
history and bottom actions, retaining text and active tab. A thin top drag strip
offers minimize and return-to-compact controls. Only integration errors appear;
click for details without expanding. The corner button restores the normal
window while remembering Mono as the compact view.
**Start in compact mode** under Settings → General restores the last view/size.

### Minimizing, the tray and quitting

Minimizing does not stop speech or integrations.

- **Minimize** leaves the window on the taskbar and an icon in the tray.
  The hide-from-taskbar-on-minimize setting under Settings → General is off
  by default; enable it for tray-only hiding at the next minimize. It persists.
- **Restore** via the tray icon/menu or starting TTSBard again from Start/a
  shortcut. The same window, text and queue return. If a disconnected monitor
  leaves the window off-screen or mostly invisible, it is centered on the
  primary monitor. Startup performs the same check and saves the recovered position.
- **Quit** with Alt+F4 in the main window, the system close command or tray
  **Quit**. The title bar has no close button. All exit paths save data normally.
- **One instance** runs per Windows session. A second launch asks the existing
  instance to show its window and exits.

## Text processing

Basic preparation resolves route prefixes, expands quick inserts and, when
enabled, converts numbers to Russian words with grammatical agreement. [AI](ai-text.md) and
[RUAccent](ruaccent.md) are additional steps.

### Sending prefixes

| Prefix | Example | Destinations |
|---|---|---|
| None | `Hello everyone` | The selected tab route |
| `!` | `!Hello` | Voice + WebView + VRChat, no Twitch |
| `!!` | `!!Hello` | TTS only |
| `!t` | `!t Hello` | Twitch only, no speech |

External destinations receive text only when enabled. `!t` must be a complete
leading word (end of input or followed by a space). `!thanks` keeps the ordinary
single-`!` behavior. A prefix takes precedence over the selected route.

### Phrase route

The route selector shows current destinations and updates while you type a
prefix. Mouse selection rewrites the leading prefix; the list opens upward.

- **Everywhere**: voice + WebView + Twitch + VRChat.
- **No Twitch** (`!`): voice + WebView + VRChat.
- **Voice only** (`!!`).
- **Twitch only** (`!t`): requires Twitch to be connected, changes the button to send
  to Twitch and creates no synthesis job.

Everywhere is listed without a “no prefix” label: absence of a prefix uses the
tab's selected route. Icons/descriptions enumerate permitted destinations;
disabled integrations receive nothing. Incoming routes include VRChat in
Everywhere/No Twitch, not Voice/Voice + Twitch.
The star saves a default route for empty editors/new tabs. A successful send
resets the tab route to that default. Delivery errors preserve text and route
so you can correct and resend.

### Typing state

The typing switch beside the route controls WebView, VTube Studio and VRChat
updates. Turning it off while typing first sends the final stopped-typing state,
then suppresses new updates. It persists and applies in normal/compact modes.

### Quick insert

| Syntax | Purpose | Example |
|---|---|---|
| `\key` | Replace a preset code | `\name` + space expands to its configured name |
| `%username` | Replace a username | `%admin` + space expands to its configured value |

Configure replacement/username lists under **Quick insert**.

### Number conversion

Russian number conversion agrees with the following word:

| Input | Result |
|---|---|
| `У меня 5 яблок` | `У меня пять яблок` |
| `1 книга` | `одна книга` |
| `2` | `два` |
| `5 книг` | `пять книг` |
| `123` | `сто двадцать три` |
| `-10` | `минус десять` |

Conversion uses Russian words regardless of the interface or voice language
and runs before AI correction and TTS. For example, `123` in English text
still becomes `сто двадцать три`. Type numbers as English words when needed.

You can turn the built-in conversion off with **Convert numbers to words**
under **Settings → Editor**, directly below the font card. It is on by
default, including for existing settings files, and the choice persists
between launches. Disabling it skips only this conversion: quick-insert
replacements, routes, [AI correction](ai-text.md) and [RUAccent](ruaccent.md)
still apply. Digits produced by replacements also skip this conversion.
External integrations keep their existing text delivery settings;
AI may still convert numbers according to its prompt.

Gender adjustment for 1 and 2 uses a heuristic based on the next word's ending;
it can be wrong. Other words are left unchanged: this is not full grammatical
inflection. English number-to-word conversion is not provided.

## Spellchecking

Offline spell checking uses a built-in **Russian** dictionary. English and
other spelling dictionaries cannot currently be selected or installed through
interface language packs. English words may be underlined as unknown.
Enable it under **Settings → Editor →
Proofing and suggestions**. Unknown words get a red wavy underline. Right-click
a word, or use Ctrl+E at the cursor, for up to five corrections. Up/Down selects,
Enter applies, Escape closes. Only the selected occurrence changes. Clicking
outside or switching tabs also closes the menu.
F7/Shift+F7 navigate current errors within the editor.
If the dictionary fails to load, an unavailable message appears and underlines
stop. After fixing the files, checking resumes automatically on the next input.
