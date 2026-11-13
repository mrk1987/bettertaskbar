# Dashboard and Theme Studio

Open the dashboard by clicking the tray icon or choosing **Settings…** on any
widget. Closing it leaves the widgets running. Unsaved edits prompt before closing.

## Settings

Widget toggles apply immediately. Settings forms use **Save changes**; that saves
the current page’s edited connection/place fields without overwriting widget positions or
unrelated changes made by another process. Failed writes are shown as errors.
**Test connection** checks the entered settings without saving them. Mail uses
read-only IMAP commands; calendar checks list upcoming events.

Usage providers read existing local sign-ins only when enabled. Claude Code and
Codex use their default CLI/desktop account. Cursor uses its local session.
Antigravity and Grok use their CLI sign-in. Copilot uses its CLI or GitHub CLI
sign-in. OpenCode Go uses `OPENCODE_GO_WORKSPACE_ID` and
`OPENCODE_GO_AUTH_COOKIE`, or `%APPDATA%\opencode-go\config.json` as described
in the upstream provider setup. The dashboard shows connection failures and
stale readings. Each enabled provider is asked for its usage every 5 minutes;
the Usage page offers 10, 15 or 30 minutes for fewer requests
(`usage_refresh_minutes` in `widgets.json`). Double-click a usage widget to
read it again at once; within 30 seconds of the last reading it waits until
those 30 seconds have passed.

BetterTaskbar does not use inference requests or CLI prompt execution to renew
Claude/Codex/Grok sessions. When those sessions expire, sign in again in the
owning provider app. No installed copy of Usage Monitor is required.

## Individual widget themes

Select a widget in **Theme Studio**. Each widget's theme is stored independently.
The original appearance remains active until you apply a custom layout.

- Add text, rectangle, icon or progress layers.
- Select layers in the list or on the canvas. Drag to move; use the selected
  layer's bottom-right handle to resize. Grid snapping uses four logical pixels.
- Reorder layers with **Up / Down**; the top of the layer list paints last.
- Edit position, dimensions, text, font size, colors, corner radius and bindings
  in the inspector. Undo and redo keep up to 64 changes.
- **Apply theme** saves and updates only the selected widget.
- **Discard** returns to the last saved layout. Unsaved edits are retained when
  switching dashboard pages; apply or discard before selecting another widget.
- **Use original** removes the custom theme for that widget.
- Import/export portable JSON themes. Account settings are never included.
  Imported geometry and file size are bounded and validated before rendering.

Text supports literal data bindings such as `{primary}`, `{secondary}` and
`{title}`. Weather also supplies `{high}`, `{low}`, `{precip}`; mail supplies
`{count}`; usage widgets supply `{usage}`, `{weekly}` and `{reset}`. Progress
layers take a numeric binding name such as `usage`, with values from 0 to 100.
The inspector lists the sample values for the selected widget. Missing values
show a dash. Bindings never execute expressions or scripts.

The preview uses sample data. Live widgets use the same rendering code and their
current data. Themes use logical pixels and follow Windows display scaling.
BetterTaskbar has its own theme format; Usage Monitor themes cannot be imported
directly.

## Credits

Dashboard colors, spacing and navigation follow Claude Code Usage Monitor.
Its MIT-licensed provider core is included with its copyright notice and a
record of the local changes. See [third-party notices](../THIRD_PARTY_NOTICES.md).

## Magnetic taskbar placement

Magnetisation is on by default. Right-side widgets pack toward the system tray;
left-side widgets pack toward the left edge, two logical pixels apart. Docked
pills also drop to a 4 px side inset (floating cards keep their wider padding),
so neighbours sit about 10 px apart. The hub positions the group together and keeps it out of taskbar
controls. Drag a widget to the other side to change its anchor. If a gap fills,
packing uses the next available gap. A widget with no room is temporarily hidden;
the dashboard explains this and it returns when space becomes available.

Check **Unmagnetise taskbar widgets** on the Widgets page to keep manual taskbar
positions. The setting applies immediately. Floating and desktop widgets do not
follow the magnetic rule. The placement dropdown on each card can move a widget
between Taskbar, Desktop and Floating.
