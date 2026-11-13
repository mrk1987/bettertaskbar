# Configuration

All settings are in one file, created with defaults on first run:

```
%APPDATA%\BetterTaskbar\widgets.json
```

Edit it while the widgets run. Changes are picked up within about a second, and
new calendar or mail settings are tried at once. A field you leave out takes its
default. If the file does not parse (a typo while you edit), the widgets keep
their last settings and leave the file alone.

```jsonc
{
  "weather": {
    "place": { "name": "Oslo", "lat": 59.9139, "lon": 10.7522 },
    "favorites": [],
    "refresh_secs": 1800
  },
  "calendar": {
    "host": "127.0.0.1",       // your CalDAV server or bridge
    "port": 8081,
    "tls": false,
    "username": "",            // CalDAV username
    "password": "",            // CalDAV password
    "path": "/caldav/",        // where discovery starts
    "refresh_secs": 300
  },
  "mail": {
    "host": "127.0.0.1",       // your IMAP server or Proton Mail Bridge
    "port": 1143,
    "security": "starttls",    // "starttls" | "tls" | "plain"
    "username": "",            // IMAP username
    "password": "",            // IMAP password (the Bridge mailbox password)
    "refresh_secs": 120,
    "rotate_secs": 8
  },
  "weather_widget":  { "placement": "taskbar", "visible": true, "x": 24, "y": 24, "tray_offset": 8,   "monitor": null },
  "calendar_widget": { "placement": "taskbar", "visible": true, "x": 24, "y": 24, "tray_offset": 116, "monitor": null },
  "mail_widget":     { "placement": "taskbar", "visible": true, "x": 24, "y": 24, "tray_offset": 354, "monitor": null }
}
```

The settings for each widget are described on its own page:
[weather](weather.md), [calendar](calendar.md), [mail](mail.md).

## Placement

You rarely need to edit these; dragging sets them.

- `placement`: `taskbar`, `floating` or `desktop` (behind your windows).
- `tray_offset`: when docked, the distance in pixels between the widget and the
  notification area.
- `monitor`: when docked, the device name of the monitor whose taskbar holds
  the widget. `null`, or a monitor that is not connected, means the main one.
- `x`, `y`: screen position when floating or on the desktop.
- `visible`: `false` hides the widget. The tray menu toggles it.

## Credentials and certificates

- Credentials are stored as **plain text**. Keep the file private; it is not
  synced anywhere.
- The calendar and mail widgets do nothing until `username` is set.
- **TLS certificates are checked**, except when `host` is `localhost` or a
  private address (loopback, 10/8, 172.16/12, 192.168/16, 100.64/10 as used by
  Tailscale, link-local). Local bridges use self-signed certificates.

## Tray menu

Right-click the tray icon to show or hide each widget, to dock, pin or float
all of them at once, and to quit.

## When something is wrong

- The widget's second line says what failed, for example
  `HTTP 401, check username and password` or `Connection refused`.
- The full message is in `%APPDATA%\BetterTaskbar\widgets.log`. An outcome is
  logged when it changes, not on every refresh.
- Run one check by hand. It prints what the widget would show, or why it cannot:

  ```sh
  bettertaskbar-widgets.exe --check=calendar | more
  bettertaskbar-widgets.exe --check=mail | more
  bettertaskbar-widgets.exe --check=taskbar | more   # taskbars and their free gaps
  ```

- **A widget will not dock where you drop it?** That stretch is taken. List the
  free gaps with `--check=taskbar`.
- **Blank or white windows?** Your GPU or driver lacks DirectX 12 support.

## Dashboard-managed widgets and themes

Each widget state also has `enabled` (whether its process should run) and
`theme` (an optional bounded layout document). `visible` still controls hiding
an enabled widget. Usage provider states live under `usage_widgets`, keyed by
`claude`, `codex`, `antigravity`, `open_code`, `cursor`, `grok`, and `copilot`.
Missing provider states default to disabled. Existing weather/calendar/mail
configurations remain enabled and keep their positions and credentials.

Use [the dashboard](dashboard.md) to manage these fields and edit themes. Theme
exports contain only the visual document. Provider usage snapshots under
`usage-state` contain readings and typed errors, without sign-in credentials.
