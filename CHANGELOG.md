# Changelog

Notable changes to BetterTaskbar. Versions follow
[Semantic Versioning](https://semver.org/).

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
