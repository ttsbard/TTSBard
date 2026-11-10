**English | [Русский](../../ru/user/silero-settings.md)**

# Manual Silero timing settings

Silero synthesis response timeout can be configured directly in the interface: in the **TTS → Silero Bot** card, click the gear icon **"[⚙]"** next to the proxy selector and enter the desired number of seconds.

Timeouts and delays can also be edited manually in `settings.json`.

## Find settings.json

Under **Settings → General → Folders**, click **Open** next to **Configuration**.
The file is `%APPDATA%\ttsbard\settings.json`, for example
`C:\Users\<Name>\AppData\Roaming\ttsbard\settings.json`.

## Parameters

All values are milliseconds under `tts.telegram`.

| Key | Default | Range | Purpose |
|---|---|---|---|
| `synthesis_response_timeout_ms` | `60000` | from `1000` (no upper limit) | Wait for Silero's response after sending text |
| `download_retry_delay_ms` | `1000` | `100`…`10000` | Delay before another download attempt |

## Example

```json
{
  "tts": {
    "telegram": {
      "synthesis_response_timeout_ms": 60000,
      "download_retry_delay_ms": 1000
    }
  }
}
```

Add fields individually; omitted fields use defaults.

## Normalization

Values below the minimum are clamped: `synthesis_response_timeout_ms: 0` becomes `1000`. Synthesis timeout has no upper limit. For `download_retry_delay_ms`, out-of-range values like `99999` become `10000`.

## Cancel waiting for a response

Canceling a phrase during generation stops the application's wait and frees
the queue for the next phrase without waiting for the timeout. A request
already sent remains with the Telegram bot. Audio replies must match the
original message, so late audio from a canceled phrase cannot replace the
next phrase's result. This does not speed up processing on the bot's side.

## Apply changes

- When changed via the interface, the new timeout **applies immediately** to subsequent phrases.
- When edited manually in `settings.json`, **restart the application** for changes to take effect.
