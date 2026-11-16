# Changelog

Notable changes to BetterTaskbar. Versions follow
[Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- Several Next Departure pills: one by default, up to six. Each pill has its
  own stop, lines, direction, position, double-click action and optional name.
  Manage them with the `1` `2` … `+` row on the dashboard's **Next Departure**
  page. An existing setup becomes pill 1.

### Changed

- Docked pills use dark text on a light taskbar or a light accent colour, and
  follow a Windows theme change within a few seconds. Floating pills and
  custom Studio themes keep their colours.
- Next Departure is slightly wider, so every traffic level fits in Norwegian.

## 1.1.2 - 2026-11-15

### Added

- Norwegian (bokmål). The dashboard, the tray menu, the widgets' right-click
  menus and the text on the widgets (`Regn 40m`, `Tørt 6t`, `Trafikk: Lav`,
  `Ingen ulest e-post`, `Økt` / `Uke`) are in English or Norwegian. By default
  the language follows Windows; choose it under **General → Language**.
  Calendar dates use Norwegian day and month names (`ma. 14:30`). Server
  error details and `--check` output stay in English.
- A **General** page at the top of the dashboard: language, magnetic
  placement, and pinning or floating every widget at once, as in the tray
  menu.

### Changed

- **Unmagnetise taskbar widgets** is now **Magnetic taskbar placement** on
  the General page: the same setting, as a checkbox that is on when packing
  is on.

### Fixed

- A themed Weather pill's `{secondary}` line is the rain hint (`Rain 40m`,
  `Dry 6h`), as on the default pill, instead of the day's precipitation
  total. The total is still `{precip}`.
- Theme Studio themes show each widget's own icon. Calendar, Mail, News, Now
  Playing and Next Departure used to get a square with a letter, because
  their icons are drawn in code and were never offered to themes. An icon
  layer bound to `icon` now draws the same icon as the default pill, at the
  layer's size, and follows the widget (paused player, bus or tram).
- The Theme Studio preview shows the real icons too, including the Yr symbol
  for Weather, and its sample text matches the current pills.

## 1.1.1 - 2026-11-15

### Changed

- Next Departure shows just the line and countdown (`31 · 6 min`); the icon
  already shows the mode. The pill is compact and keeps one width; longer
  text scrolls.
- The Next Departure direction filter takes several destinations, comma
  separated, for lines that end at different stops in one direction. The
  dashboard explains what to enter.
- README rewritten for everything 1.1 added, with fresh resource figures
  for all eight widgets.

### Fixed

- Commas can be typed in the Next Departure **Lines** box; they used to
  vanish before the next line number could follow.

## 1.1.0 - 2026-11-15

### Added

- Now Playing widget: track, artist and playback state from Windows' media
  session, with play/pause and next-track buttons (through the same media
  session, falling back to the keyboard's media keys). Double-click brings up
  the player.
- Next Departure widget: the next departure from a saved stop (Entur), with
  line and direction filters, a live countdown, delays, an icon for bus, tram,
  metro, train or boat, and a None / Low / Medium / High traffic level from
  Statens vegvesen road situations along the route and Entur disruptions.
  Unknown when road data is missing or stale.
- News widget: rotating headlines from built-in Norwegian, international and
  tech feeds or any RSS/Atom feed, with source, age, de-duplication,
  pause on hover and an offline marker.
- Double-click action **Open current item**: opens the headline on screen
  (News) or the media player (Now Playing). The headline holds still while
  it is clicked.
- Weather shows when rain, sleet or snow starts within six hours
  (`Rain 40m`, `Rain now`), or `Dry 6h`, in place of the day's total.
- Text too long for its pill scrolls to show the rest, snaps back and
  starts again after a pause. News and Mail move on to the next headline or
  subject only after the current one has scrolled past once.
- Mail and Calendar are compact pills of the same width, just wide enough
  for "No unread mail"; longer text scrolls.
- The tray menu lists every widget, with the AI usage providers in a
  submenu, and checks the placement all enabled widgets share.
- `--check=departures`, `--check=news` and `--check=now-playing`.
- Per-widget double-click actions: open a configurable URL, refresh data,
  or launch a chosen application. Configure targets in the dashboard and
  switch actions from the widget's right-click menu. Existing defaults
  are preserved, and manual refresh is available for every widget.

### Fixed

- A pill dragged onto the taskbar could stay blank while still taking
  clicks: its layered surface is now re-bound after each move into a
  taskbar, as Claude-Code-Usage-Monitor does.
- Docked pills keep the order they are placed in on each side of the
  taskbar, also after a restart, instead of a fixed order.
- The Usage page no longer shows a Settings button that led to itself.
- Enabled Windows clipboard access so dashboard text fields support pasting
  from other applications, as well as copying and cutting.

- CPU-rendered widgets keep their taskbar parent while dragged along the
  bar, and detach when dragged away. Snapped previews no longer rely on a
  floating window that Explorer can cover.
- CPU-mode window upkeep runs on the window thread. Magnetic placement
  waits until the release frame has finished saving the drop.
- A drop whose taskbar disappeared falls back to floating instead of
  recording a docking operation that failed.

## 1.0.1 - 2026-11-14

### Changed

- All widgets now run in the tray process and are drawn on the CPU. Five
  widgets measure about 20 MiB of RAM and 0.5 to 0.7 % of one CPU core,
  against about 275 MiB and 2.7 % in 1.0.0. The 1.0.0 behavior, one GPU
  process per widget, is still there: start with `--process-per-widget` or
  set `"process_per_widget": true` in `widgets.json`.
- Widget text is near-white, like the taskbar clock; it was a dim gray.
- Usage widgets read each provider every 5 minutes instead of every 2. The
  Usage page in the dashboard offers 10, 15 or 30 minutes.
- Docking checks find the taskbars once per check instead of once per widget.

### Added

- Double-click a usage widget to read it again at once (at most once every
  30 seconds).

### Not yet checked in the new mode

- Dragging between taskbar, desktop and floating, and Pin to desktop. If
  either misbehaves, `--process-per-widget` restores the 1.0.0 behavior.

## 1.0.0 - 2026-11-13

First public release.

### Widgets

- Weather from MET Norway with Yr symbols: the next 24 hours' high, low and
  precipitation. Favorite places and place search.
- Calendar: what is on now and what is next, from any CalDAV server, with time
  zones and recurring events.
- Mail: unread count and rotating subjects over read-only IMAP.
- Usage widgets, off by default, for Claude Code, Codex, Antigravity, OpenCode
  Go, Cursor, Grok Build and GitHub Copilot. They reuse each tool's existing
  sign-in.

### Placement

- Widgets dock on the taskbar as part of the bar, with no background of their
  own, on any monitor. Drag to move; a widget snaps into free gaps and never
  covers Start, app icons, the notification area or another widget.
- Magnetic placement packs docked widgets against the left or right group and
  moves them when app icons grow or shrink. It can be switched off.
- Widgets can also float on top or be pinned to the desktop.
- Docked widgets come back after Explorer restarts.

### Dashboard

- Enable widgets, set places, enter and test CalDAV and IMAP settings.
- Theme Studio: a per-widget visual editor with text, icons, rectangles and
  progress bars bound to live data, and theme import and export.

### Resource use

- Five widgets measure about 275 MiB of RAM and 3 % of one CPU core. Each
  widget is a process of about 55 MiB, nearly all of it the GPU renderer
  needed for a see-through window. See
  [docs/architecture.md](docs/architecture.md#memory-and-cpu).

### Known limits

- Only horizontal taskbars are dock targets.
- Text and icons are drawn for a dark taskbar.
- Credentials are stored as plain text in `%APPDATA%\BetterTaskbar\widgets.json`.
- The program is not code-signed; Windows SmartScreen warns on first run.
- Revealing an auto-hidden taskbar with the mouse has not been checked
  ([#1](https://github.com/mrk1987/bettertaskbar/issues/1)).
