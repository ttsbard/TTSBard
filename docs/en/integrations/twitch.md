**English | [Русский](../../ru/integrations/twitch.md)**

# Twitch Chat

TTSBard can also send spoken phrases to Twitch chat using either connection mode.

- **IRC is easier to set up.** You need a Twitch account username, its OAuth token, and the channel name. Messages appear under the account that owns the token. You can use your own Twitch account or a separate bot account.
- **Twitch API requires authorization from the channel owner.** Register a Twitch application and authorize the bot and channel owner separately. You can save access to several channels and choose which one receives messages.

## Classic IRC mode

Connect directly through an IRC socket:

1. Obtain an OAuth token for the Twitch account you want to use using [twitchtokengenerator.com](https://twitchtokengenerator.com).
2. Copy only the OAuth token, without `oauth:`.
3. Enter the account username, target channel and token, then connect.

- **Username**: the token's account, 1–25 Latin letters, digits or `_`, no `@`/spaces.
- **Token**: no `oauth:` prefix.
- **Channel**: name only, no URL/`#`, for example `twitchdev`; same 1–25-character rules.

An initial IRC connection unconfirmed for 30 seconds fails. Check channel and
network, then retry manually. Temporary network loss or Twitch `RECONNECT`
triggers automatic reconnection, shown in the title bar. Manual disconnect
stops retries. A phrase interrupted during delivery is not resent automatically;
resend only if appropriate.

Field-format errors remain until corrected. A username/token-account mismatch
stops connecting with a specific error. Connection errors remain until retry or
disconnect. Connect/reconnect saves edited credentials before connecting;
the operation notification reports saving separately.
Logs include failure codes/retry details, not raw library IRC traffic even at
TRACE: PASS/PRIVMSG can contain tokens and user text.

## Twitch API mode

Uses Twitch Helix `POST /helix/chat/messages` to send as an authorized bot
account to a streamer's chat. The bot does **not** need channel moderator rights.

1. Create an application in the [Twitch Developer Console](https://dev.twitch.tv/console/apps),
   category Chat Bot, client type Confidential.
2. Enter Client ID/Client Secret and save.
3. Authorize the bot: explicitly open the activation link and approve using
   a separate bot account (`user:write:chat`, `user:bot`).
4. Add a channel and approve using its owner's account (`channel:bot`). Copy
   the link to the owner or open it in another browser/device. The browser does
   not open automatically. Keep TTSBard open; waiting is limited to five minutes.
5. Select one recipient channel. The first added channel is selected automatically.
6. Connect to activate delivery.

Both accounts use Device Code Flow, without a local callback. If registration
requires an OAuth Redirect URL, `http://localhost` can remain; code-based login
does not redirect there.
Resetting the bot clears its local login, cancels pending authorization and
stops API delivery, retaining Client ID/Secret. Revoke access on Twitch itself
under Twitch settings → Connections. Changing recipients while connected
restarts delivery for the newly selected channel.

Removing channel access forgets that grant **locally**, deleting saved tokens
on this PC without a Twitch revoke request. Add it again if needed. Removing
the selected channel clears the recipient and stops API delivery.

## Sending messages

After switching modes, click Connect to resume sending messages.

Select an [editor route](../user/editor.md#phrase-route) including Twitch.
HTTP/OCR use the separate [Incoming route](../user/incoming.md#automatic-acceptance),
which defaults to voice only.
