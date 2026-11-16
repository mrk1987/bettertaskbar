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
  "language": "system",        // "system" (follow Windows) | "english" | "norwegian"
  "magnetized": true,          // pack docked widgets toward the bar's edges
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
[weather](weather.md), [calendar](calendar.md), [mail](mail.md),
[now playing](now-playing.md) (`now_playing_widget`, no settings),
[next departure](departures.md) (`departures`, `departures_widget`,
`departure_slots`) and [news](news.md) (`news`, `news_widget`). The last three are off until switched
on.

## Next Departure pills

Pill 1 uses `departures` and `departures_widget` as before. `departures` has a
new `name` field, empty by default. A name replaces the stop name in the tray
menu, dashboard widget list and slot hover text.

Pills 2-6 are objects in `departure_slots`, keyed `"1"` to `"5"`. Each holds a
`departures` object and a `widget` object with the same fields as pill 1's:

```json
"departure_slots": {
  "1": {
    "departures": {
      "name": "Buses",
      "stop": { "id": "NSR:StopPlace:58366", "name": "Jernbanetorget, Oslo", "lat": 59.9119, "lon": 10.7504 },
      "favorites": [],
      "lines": ["31"],
      "direction": "",
      "refresh_secs": 60,
      "road_traffic": true,
      "traffic_radius_m": 300
    },
    "widget": { "enabled": true, "placement": "taskbar", "visible": true, "x": 24, "y": 24, "tray_offset": 8, "monitor": null }
  }
}
```

New pills start disabled. Enable them under **Widgets** after saving. The
example above sets `enabled` to `true` to start pill 2; `visible` alone only
controls whether an enabled pill is shown.

A key that is missing has no pill: no window is started, and the tray menu,
Widgets page and Theme Studio do not list it. Removing a pill deletes its key
and does not move the others. Key `"1"` is the window
`BetterTaskbar Next Departure 2`, and so on.

## Taskbar text

A docked pill has no background of its own, so its text follows the taskbar
colour Windows reports: dark text on a light taskbar or a light accent colour,
light text on a dark one. A change shows within a few seconds. Floating pills
keep their dark card and light text, and a custom Studio theme keeps its own
colours.

Set `BTW_FORCE_LIGHT_BAR=1` in the process environment to make docked pills
using the built-in layout use light-taskbar colours, whatever Windows reports.
Only the exact value `1` forces light mode; other values keep Windows detection.
It is a test aid for screenshots and checks. Floating pills keep their colours.

For off-screen PNGs from fixed sample data, build first, then run this from
`widgets/` in PowerShell (the output directory must exist):

```powershell
cargo build
$env:BTW_FORCE_LIGHT_BAR = "1"
$env:BTW_BANNER_DIR = "C:/Temp/btw-light"
$env:BTW_BANNER_LANGUAGE = "norwegian" # or "english"
cargo test readme_banner -- --test-threads=1
```

This writes one PNG per sample pill over `#F3F3F3`, without loading or saving
your settings. It fetches the public Yr weather symbol. It does not update
`docs/banner.png`. To use a local copy of the sample weather SVG instead of
fetching it, set `BTW_BANNER_WEATHER_ICON` to its path.

## Placement

You rarely need to edit these; dragging sets them.

- `placement`: `taskbar`, `floating` or `desktop` (behind your windows).
- `tray_offset`: when docked, the distance in pixels between the widget and the
  notification area.
- `monitor`: when docked, the device name of the monitor whose taskbar holds
  the widget. `null`, or a monitor that is not connected, means the main one.
- `x`, `y`: screen position when floating or on the desktop.
- `visible`: `false` hides the widget. The tray menu toggles it.

## Double-click action

Each widget state (including each entry in `usage_widgets`) accepts:

```json
{
  "double_click": {
    "action": "open_url",
    "url": "https://example.com/",
    "app": "C:\\Program Files\\Example\\Example.exe"
  }
}
```

`action` is `open_url`, `refresh`, `launch_app`, or `open_item` (News and
Now Playing only: open the headline on screen, or the player). Missing or
`null` keeps the default: open the default website for weather/calendar/mail,
refresh for usage widgets and Next Departure, and `open_item` for News and
Now Playing. An empty `url` uses the widget's default
website. App paths are full `.exe` paths, without quotes or arguments.
Both targets are retained when changing the action. Theme exports contain
neither target.

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
