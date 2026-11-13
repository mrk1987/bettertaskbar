# Calendar widget

The Proton Calendar icon plus two lines.

| Situation | First line | Second line |
| --- | --- | --- |
| Something is on | `Now: <title> · to 11:00` | `Next: <title> · 12:00` |
| Nothing on right now | `Next: <title> · 12:00` | `Free now` |
| Nothing in the next 14 days | `No upcoming events` | `Next 14 days` |
| The server cannot be reached | the last known line | what failed |

A start time on another day gets the weekday (`Tue 09:00`), or the date when
it is a week or more away (`27 Nov 07:15`). Long titles are cut with an
ellipsis.

All-day events are left out on purpose: one would occupy `Now:` all day.
Cancelled events are left out too.

## Configure

Open **Calendar** in the dashboard to enter and test the connection settings.
Choose **Save changes** to apply them. The settings below can also be edited
in `widgets.json`.

Settings are under `calendar` in [widgets.json](configuration.md):

```json
"calendar": {
  "host": "127.0.0.1",
  "port": 8081,
  "tls": false,
  "username": "you@example.com",
  "password": "your-caldav-password",
  "path": "/caldav/",
  "refresh_secs": 300
}
```

- `host` / `port` / `tls`: your CalDAV server. The defaults fit an
  [iOxide](https://github.com/mrk1987/iOxide) Proton Calendar bridge on this PC.
- `username` / `password`: what the server expects. With `username` empty
  nothing is fetched.
- `path`: where discovery starts. It may be a principal, a calendar home, or
  one calendar; a missing trailing slash is tried both ways. For iOxide use
  `/caldav/`. To show a single calendar, give its own path.
- `refresh_secs`: re-fetch interval (default 5 min, minimum 30 s). A changed
  setting is tried at once.

The TLS certificate is checked unless `host` is `localhost` or a private
address, where bridges use self-signed certificates. Credentials are stored
in plaintext in the config file; keep it private.

## Interactions

- **Drag**: see [Moving them](../README.md#moving-them).
- **Double-click**: opens https://calendar.proton.me.
- **Right-click**: placement, hide, quit all.

## How it fetches

1. **Discover** (`PROPFIND`): the configured path is asked what it is. A
   calendar is used directly. Otherwise its `calendar-home-set` (or that of its
   `current-user-principal`) is listed and every calendar in it is used.
2. **Fetch** (`REPORT calendar-query`) each calendar for now → +14 days. The
   time range is only a hint: some servers ignore it and send everything, so
   the widget filters by time itself.
3. **Read** the iCalendar data (`src/ical.rs`): line unfolding, `TZID` time
   zones (IANA names; an unknown zone is read as local time), `DTEND` or
   `DURATION`, recurring events (`RRULE`, `RDATE`, `EXDATE`, expanded with the
   `rrule` crate), and `RECURRENCE-ID` overrides that move or cancel a single
   occurrence.

If one calendar fails and another works, the widget shows what it has. Only when
all fail does it show the error.

## Troubleshooting

Run `bettertaskbar-widgets.exe --check=calendar | more`. It prints the number
of calendars and events found and the two lines, or the failing request with
the server's answer. The same message goes to
`%APPDATA%\BetterTaskbar\widgets.log`.

- `the server listed nothing there` or `no calendars found under …`: `path`
  does not lead to a calendar. Try the server's principal path.
- `HTTP 401`: wrong username or password. A bridge may issue a new password
  each time you sign in to it.
- `… recurrence rules not understood` in the log: those events count once, at
  their first start, instead of repeating.
