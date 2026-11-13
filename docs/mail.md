# Mail widget

The Proton Mail icon plus two lines, a rotating unread message
(its subject, or the sender if it has none) and the unread count. The newest
message is shown first. Long subjects are cut with an ellipsis.

With nothing unread it says `No unread mail`. If the server cannot be reached,
the second line says what failed.

## Configure

Open **Mail** in the dashboard to enter and test the connection settings.
Choose **Save changes** to apply them. The settings below can also be edited
in `widgets.json`.

Settings are under `mail` in [widgets.json](configuration.md):

```json
"mail": {
  "host": "127.0.0.1",
  "port": 1143,
  "security": "starttls",
  "username": "you@example.com",
  "password": "your-bridge-mailbox-password",
  "refresh_secs": 120,
  "rotate_secs": 8
}
```

- `host` / `port`: your IMAP server. The defaults fit Proton Mail Bridge on
  this PC.
- `security`: `"starttls"` (connect, then upgrade; the Bridge default),
  `"tls"` (TLS from the start), or `"plain"` (no encryption; local testing
  only). Anything else is reported as an error.
- `username` / `password`: for Proton Mail Bridge, the **bridge-generated
  mailbox password**, not your Proton account login. With `username` empty
  nothing is fetched.
- `refresh_secs`: how often unread mail is checked (default 2 min, minimum
  30 s). A changed setting is tried at once.
- `rotate_secs`: how long each subject stays before the next (default 8 s).

The TLS certificate is checked unless `host` is `localhost` or a private
address, where the Bridge uses a self-signed certificate. Credentials are
stored in plaintext in the config file; keep it private.

## Interactions

- **Drag**: see [Moving them](../README.md#moving-them).
- **Double-click**: opens https://mail.proton.me.
- **Right-click**: placement, hide, quit all.

## How it fetches

IMAP: `LOGIN`, `EXAMINE INBOX` (read-only, so nothing can be marked as read),
`UID SEARCH UNSEEN`, then `UID FETCH` of the `From` and `Subject` headers of
the 30 newest unread messages. The count covers all unread mail, also beyond
those 30. Encoded subjects (RFC 2047, `=?UTF-8?Q?…?=` and `?B?`) are decoded,
in UTF-8, ISO-8859-1/15 and Windows-1252. See `src/imap.rs`.

## Troubleshooting

Run `bettertaskbar-widgets.exe --check=mail | more`. It prints the unread
count and subjects, or what failed. The same message goes to
`%APPDATA%\BetterTaskbar\widgets.log`.

- `cannot connect to …`: nothing is listening there. Start the Bridge, or
  check `host` and `port` against the Bridge's IMAP settings.
- `login failed: NO …`: wrong username or password. The Bridge makes a new
  mailbox password when the account is re-added.
- `server refused STARTTLS`: set `security` to match the server.
