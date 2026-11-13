# BetterTaskbar

Weather, your next calendar event and unread mail, right on the Windows 11
taskbar. Written in Rust with [egui](https://github.com/emilk/egui).

[![CI](https://github.com/mrk1987/bettertaskbar/actions/workflows/ci.yml/badge.svg)](https://github.com/mrk1987/bettertaskbar/actions/workflows/ci.yml)

![Three widgets on the taskbar: weather, next calendar event, unread mail](docs/taskbar.png)

If you, like me, use Proton Mail Bridge and my other project
[iOxide](https://github.com/mrk1987/iOxide), want a better weather widget with
data from Norway's sweetheart yr.no, and have an ultrawide monitor with unused
real estate on your taskbar, then you are in a very small niche. But also; I have 
just the thing for you!

The widgets sit in the taskbar's free space as part of the bar, with no
background of their own, so they match whatever color or effect your taskbar
has. Drag them anywhere on the bar, or off it to float on the desktop.

## The widgets

| Widget | Shows | Double-click opens |
| --- | --- | --- |
| [Weather](docs/weather.md) | Yr symbol, next 24 hours' high and low, precipitation | The forecast on yr.no |
| [Calendar](docs/calendar.md) | What is on now and what is next, from any CalDAV server | Proton Calendar |
| [Mail](docs/mail.md) | The newest unread subject, rotating, and the unread count | Proton Mail |

The calendar and mail widgets need an account. They were built for Proton
through [iOxide](https://github.com/mrk1987/iOxide) (calendar) and Proton Mail
Bridge (mail), but any CalDAV or IMAP server works.

Ideas for more widgets are welcome:
[open an issue](https://github.com/mrk1987/bettertaskbar/issues/new) and say
what you would like to see on your taskbar.

## Moving them

Press anywhere on a widget and drag. Near a taskbar it snaps into a free gap:
never onto the Start button, your app icons, the notification area or another
widget. Release it there and it docks. Release it anywhere else and it floats
where you dropped it.

A docked widget remembers its distance from the notification area, so it keeps
its place when tray icons come and go or the resolution changes. It works on
secondary monitors too. This follows the behavior of
[Claude-Code-Usage-Monitor](https://github.com/CodeZeno/Claude-Code-Usage-Monitor).

## Install

You need Windows 10 or 11 and a GPU with DirectX 12.

1. Download `bettertaskbar-<version>-windows-x64.zip` from the
   [latest release](https://github.com/mrk1987/bettertaskbar/releases/latest)
   and unzip it anywhere.
2. Run `bettertaskbar-widgets.exe`.

The program is not code-signed, so Windows SmartScreen warns the first time:
choose **More info**, then **Run anyway**. Each release lists the SHA-256 of the
zip if you want to check your download. There is no installer and no
auto-start; add a shortcut to `shell:startup` if you want the widgets at
sign-in.

That starts a tray icon and the three default widgets. Quit from the tray icon,
or from any widget's right-click menu.

To build it yourself you need [Rust](https://rustup.rs) (stable):

```sh
git clone https://github.com/mrk1987/bettertaskbar.git
cd bettertaskbar/widgets
cargo run --release
```

## Dashboard

Click the tray icon, or choose **Dashboard…** from its menu. You can also run:

```sh
bettertaskbar-widgets.exe --dashboard
```

- **Widgets:** enable or disable Weather, Calendar, Mail, Claude Code, Codex,
  Antigravity, OpenCode Go, Cursor, Grok Build and GitHub Copilot. Disabling a
  widget stops its process and background fetching.
- **Weather:** search for places, manage favorites, or enter coordinates.
- **Calendar / Mail:** enter CalDAV and IMAP connection settings and test them.
- **Usage providers:** reuse each provider's existing app or CLI sign-in. These
  widgets are off by default. Their provider polling is adapted from CodeZeno's
  MIT-licensed Claude Code Usage Monitor; the separate monitor is not needed.
- **Theme Studio:** edit each widget independently using a canvas, layers and
  property inspector. Add text, icons, rectangles and progress bars; drag,
  resize, reorder, change colors, bind live data, and import/export themes.
  **Apply theme** updates that widget. **Use original** restores its built-in
  appearance. The studio preview uses sample data.

Settings are saved locally in `%APPDATA%\BetterTaskbar\widgets.json`. Passwords
are masked in the dashboard but stored as plain text in this file. Theme exports
contain the visual layout, without connection settings or credentials.
See [the dashboard guide](docs/dashboard.md) and [configuration guide](docs/configuration.md).

Diagnostic commands are still available:

```sh
bettertaskbar-widgets.exe --check=calendar | more
bettertaskbar-widgets.exe --check=mail | more
bettertaskbar-widgets.exe --check=taskbar | more
```

## How it works

Each widget is its own small process, started and watched by a tray hub.

- **On the taskbar:** the window is made a child of `Shell_TrayWnd`. That is
  what keeps it visible when the taskbar is clicked, which lifts the taskbar
  above every ordinary window.
- **Transparency:** wgpu presents through a DirectComposition visual, so the
  window has real alpha and the taskbar shows through. Renderers use smaller
  allocation pools and 4,096 resource bindings rather than wgpu's default
  one-million-slot DX12 heaps. Each enabled widget still has its own renderer,
  which is where the memory goes (see Limits).
- **Free space:** Windows 11 draws the taskbar with XAML, so its buttons are not
  windows. UI Automation reports their rectangles. Magnetic placement caches
  that layout, refreshing after application windows change, taskbar clicks or
  taskbar geometry changes, with a 30-second fallback. Faster sampling runs
  only while dragging a widget.
- **Data:** the calendar is read with plain CalDAV discovery and `REPORT`
  queries, including time zones and recurring events. Mail is read over IMAP in
  read-only mode.

The details, including the dead ends, are in
[docs/architecture.md](docs/architecture.md).

## Limits

- Only horizontal taskbars are dock targets.
- Magnetic placement follows taskbar changes after a short delay. Shell-only
  changes that escape detection can take up to 30 seconds. Unmagnetised widgets
  keep their manual position when app icons grow toward them.
- Text and icons are drawn for a dark taskbar.
- Each enabled widget uses about 55 MiB of RAM, nearly all of it the GPU
  renderer a process needs to draw a see-through window. Five widgets measure
  about 275 MiB and 3 % of one CPU core. The numbers and the alternatives are
  in [docs/architecture.md](docs/architecture.md#memory-and-cpu).
- With an auto-hiding taskbar the widgets hide and return with the bar. This
  was checked by switching the setting, not yet by revealing the bar with the
  mouse ([#1](https://github.com/mrk1987/bettertaskbar/issues/1)).
- Credentials are stored as plain text in `widgets.json`. Keep it private.

## Credits

- Weather data from [MET Norway](https://api.met.no) (CC BY 4.0), with icons
  from [Yr](https://www.yr.no).
- Provider polling and dashboard styling are adapted from CodeZeno’s MIT-licensed
  Claude Code Usage Monitor. Full copyright and license notices are in
  [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
- The docking and drag behavior follows
  [Claude-Code-Usage-Monitor](https://github.com/CodeZeno/Claude-Code-Usage-Monitor)
  by CodeZeno (MIT).

## License

[MIT](LICENSE)
