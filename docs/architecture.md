# Architecture

How the widgets are put together, and why. Several choices here look odd until
you know what was tried first; those are written down so nobody has to find
out again. Measured on Windows 11 (3440x1440, NVIDIA).

## Processes

```
bettertaskbar-widgets.exe              tray hub: tray icon, starts and watches the widgets
├── … --widget=weather                 one eframe window per process
├── … --widget=calendar
└── … --widget=mail
```

The dashboard (`--dashboard`) is another independent eframe root process.
Enabled widgets are supervised from the `Kind::ALL` catalog; disabled widgets
are terminated and do not fetch in the background. Usage widgets are opt-in.

Each widget is its own process with one eframe root window. winit allows one
event loop per process, and eframe's child viewports never presented on the
test machine, so separate processes are the simple way to get three windows.

### Memory and CPU

**Since 1.0.1 the pills run in the hub's process and are drawn on the CPU**
(`single.rs`, `cpu_render.rs`). One thread owns every pill window and its
message loop; egui lays each pill out as before, a small rasterizer turns the
triangles into premultiplied pixels with the same math as egui-wgpu, and
`UpdateLayeredWindow` shows them. Fully transparent pixels get alpha 1 so the
whole pill stays clickable. A layered child window needs Windows 8 or later
declared in the manifest (`widgets/app.manifest`, embedded by `build.rs`).
Measured 2026-11-14, release build, five widgets: 20 to 22 MiB private
working set and 0.5 to 0.7 % of one core for the whole process. Two traps
found on the way:

- A docked pill's thread shares an input queue with Explorer, so waiting with
  `MWMO_INPUTAVAILABLE` woke the loop for Explorer's own mouse input, 100 to
  400 times a second. The loop waits for new messages only.
- A wait time rounded down to whole milliseconds became 0 ms just before a
  frame was due, and the loop spun. It rounds up.

`--process-per-widget` keeps the model below. The rest of this section
describes it.

Measured 2026-11-13 on the release build, five widgets (weather, calendar,
mail, two usage widgets) plus the hub, over 40 seconds:

| Process | CPU, % of one core | Private working set |
| --- | ---: | ---: |
| hub | 0.8 | 2.7 MiB |
| each widget | 0.3 to 0.4 | 53.5 to 55.3 MiB |
| total | 2.7 | 275 MiB |

A window that does nothing but show one label, built with the same eframe,
wgpu and DirectComposition settings, measures 53 to 56 MiB. So the cost of a
widget is the GPU stack a process needs for a see-through window, not the
widget's own code, fonts or icons, and trimming a widget cannot lower it.
What would:

- **One process, one wgpu device, many windows** through `egui-winit` and
  `egui-wgpu` directly (not eframe child viewports, which never presented).
  The open risks are per-surface DirectComposition alpha and re-parenting
  several windows of one event loop onto the taskbar.
- **No GPU for the pills:** rasterize on the CPU and present with
  `UpdateLayeredWindow`. This needs a second render path beside the
  dashboard's.

Neither is done. `tools/bench.ps1` and `tools/sample.ps1` reproduce the table.

Two things were ours and are fixed:

- **The weather widget's HTTPS requests cost it about 20 MiB for good.** In
  the bare test window, one request to `api.met.no` took the private working
  set from 53 to 74 MiB, and it stayed there, with or without certificate
  checks and whether or not the body was parsed. The weather widget therefore
  runs its forecast and icon downloads in a short-lived child
  (`--http-get=<url>`, limited to `api.met.no` and `www.yr.no`; see
  `http::get_isolated`), which brought it from 75.6 to 53.5 MiB. Which part
  of the Windows TLS stack holds the memory was not found. The calendar, mail
  and usage widgets measured 54 to 56 MiB without this, on a machine where
  calendar and mail use local bridges; a public CalDAV or IMAP server may
  behave like the weather API did.
- **An idle pill repaints once a second**, not five times. A repaint is also
  what takes in new data and runs the dock check. Input, a config change and
  a finished icon download repaint at once.

The hub restarts a widget that exits. That matters because a docked widget is
a child of the taskbar, and dies with it when Explorer restarts. The hub also
adds its tray icon again when Explorer broadcasts `TaskbarCreated`.

| File | Does |
| --- | --- |
| `main.rs` | Argument dispatch, the hub, `--check=` diagnostics |
| `app.rs` | One widget window: painting, dragging, docking, menu actions |
| `popup.rs`, `place_search.rs` | Native popup menus and a separate weather search window |
| `shell.rs` | Win32: finding taskbars, hosting the window, window styles |
| `occupancy.rs` | Which parts of a taskbar are taken (UI Automation) |
| `tray.rs` | Tray icon and menu on its own message-loop thread |
| `config.rs` | `widgets.json`: schema and safe concurrent updates |
| `http.rs` | HTTPS agents, the private-host rule, child-process downloads |
| `magnet.rs` | The hub's packing of docked widgets into free taskbar gaps |
| `caldav.rs`, `ical.rs` | Calendar discovery, fetch and iCalendar parsing |
| `imap.rs` | Unread mail over IMAP |
| `widgets/*.rs` | The three widgets, behind the `Widget` trait |

## Docking on the taskbar

A docked widget is a **child window of the taskbar** (`Shell_TrayWnd`, or
`Shell_SecondaryTrayWnd` on other monitors), as in
[Claude-Code-Usage-Monitor](https://github.com/CodeZeno/Claude-Code-Usage-Monitor).

Why not a topmost window floating over the bar: when the taskbar is clicked,
Windows lifts it above every ordinary window, topmost ones included. A
floating widget then disappears behind the bar although the z-order still
lists it on top. Re-raising it, foreground hooks and owner windows do not
help. Only a child of the taskbar stays visible.

Three things stand in the way of a child window with eframe/winit:

1. **`SetParent` fails with "invalid parameter".** winit registers its window
   as an OLE drop target, and the taskbar refuses such a child. Fixed by
   creating the viewport with `with_drag_and_drop(false)`. DPI awareness,
   topmost and layered styles, and the swapchain type were ruled out.
2. **winit rewrites the window style** whenever it resizes the window, which
   turns the child back into a popup. `shell::hook_window` subclasses the
   window and restores `WS_CHILD` in `WM_STYLECHANGING` while it is hosted.
3. **winit moves the client area one pixel down** for undecorated windows (its
   way of getting a drop shadow, always on in egui-winit). That leaves a dead
   strip along the top and cuts off the bottom row. The same hook answers
   `WM_NCCALCSIZE` itself: the client area is the whole window.

A window that has `WS_CHILD` set but no parent is invisible. If widgets ever
vanish, check that first.

## Transparency

A docked widget has no background, so the taskbar shows through it. eframe is
told to present through DirectComposition (`WGPU_DX12_PRESENTATION_SYSTEM=visual`,
set in `main.rs` before eframe starts), the viewport is transparent and the
clear color is zero alpha. Only a swapchain on a composition visual has an
alpha channel that the compositor honors; one bound straight to the window
does not.

The earlier approach was an opaque widget painted in the taskbar's color,
sampled from the screen. It matched the bar in every screenshot, yet on the
real display a difference was visible along the top row. Screenshots cannot
show this kind of difference, so trust your eyes over captures here.

## Moving

`app.rs` handles a press itself instead of using a title-bar drag:

1. A press past the system drag threshold (`SM_CXDRAG`) starts a drag and turns
   the widget into a free topmost popup that follows the cursor.
2. Within 20 px of a taskbar it looks for a free gap. Taken are the shell's
   controls, read by `occupancy.rs` through UI Automation (Windows 11 draws the
   taskbar with XAML, so its buttons are not windows), the notification area
   and the other widgets. The widget snaps in when 67% of it lies in a gap, and
   stays snapped down to 45%, so it does not flicker at the edge.
3. On release it docks if snapped, and floats where dropped otherwise.

A docked widget stores its distance from the notification area and its
monitor's device name, not a screen position. About once a second it puts itself where
that says, which also covers tray icons coming and going, resolution changes
and a new taskbar after an Explorer restart.

UI Automation is slow and can block on Explorer, so it runs on its own thread
during a drag and in the hub while magnetic taskbar placement is active.

## Shared configuration

The hub, dashboard and enabled widgets share `widgets.json`. Every write goes through
`config::update`, which reads the file, applies one change and writes it back
through a temporary file and a rename. A named Windows mutex covers the whole
read/change/write transaction across processes, including first-run creation.
A process never writes back its own
copy, so it cannot undo another process's change, or the user's. A file that
does not parse (half-saved in an editor) is left alone. Every process polls the
file's modification time and reloads.

## Calendar

`caldav.rs` follows RFC 4791 discovery: the configured path can be a principal,
a calendar home or a calendar, with or without a trailing slash. The XML is
parsed with `roxmltree`, so entities such as `&#xD;&#xA;` decode correctly.
Absolute discovery links must stay on the configured scheme, host and port
before credentials are attached.
Some servers ignore the time range in a `calendar-query` and return everything,
so the client filters by time itself.

`ical.rs` unfolds lines, resolves `TZID` with `chrono-tz`, expands `RRULE`,
`RDATE` and `EXDATE` with the `rrule` crate, and applies `RECURRENCE-ID`
overrides that move or cancel one occurrence. All-day events are left out on
purpose.

## Mail

`imap.rs` opens INBOX with `EXAMINE` (read-only), searches `UNSEEN` and fetches
the `From` and `Subject` headers of the 30 newest. Encoded words are decoded
for UTF-8, ISO-8859-1/15 and Windows-1252.

## Layout pitfall: centering in egui

`Ui::horizontal` sizes its row to `spacing().interact_size.y`, and a value set
on an outer layout leaks into every nested row. Setting it to the widget height
for centering pushed the weather widget's second line out of view. The content
row is therefore an explicit 28 px rectangle centered in the 48 px widget
(`ui.scope_builder` with a fixed `max_rect`). Keep it that way.

## Testing

- `cargo test` covers parsing, discovery, encodings, dock geometry, gap finding
  and concurrent config writes from four processes. Two native tests build
  disposable menus/windows to check command mapping, parent movement,
  destruction and failed docking. They never touch Explorer or a server.
- `tools/explorer-restart-test.ps1` stops Explorer and reports whether the
  docked widgets come back; `tools/autohide-test.ps1` switches taskbar
  auto-hide on and back. Both passed on 2026-11-13: after an Explorer restart
  every widget had a new process, was attached to the new taskbar and stood
  where it did before; with auto-hide the widgets moved and hid with the bar
  and were visible 0.2 s after it returned. Revealing an auto-hidden bar with
  the mouse still needs a person at an interactive desktop (issue #1).
- `--check=calendar`, `--check=mail` and `--check=taskbar` run one real fetch or
  taskbar reading and print the result. Pipe to `more`; the program has no
  console of its own.
- **Screenshots:** a screen capture shows the composed desktop, which is not
  always what the display shows (see Transparency). Some capture APIs miss
  these windows entirely.
- **Synthetic input:** `SetCursorPos` and `SendInput` do nothing while the
  machine is used over a minimized Remote Desktop session. Debug builds read a
  fake cursor from a file instead: set `BTW_FAKE_INPUT` to a file holding
  `x y down widget`, for example `2000 600 1 weather`, and write it atomically.
- A scripted press at exactly the point where the cursor last left the window
  is ignored, because winit sends no move event for an unchanged position. Vary
  the point.

## Rules that are easy to break

- Keep the wgpu backend. The OpenGL (glow) backend never presented on NVIDIA.
- Keep `with_drag_and_drop(false)`; without it docking fails.
- Do not use `SetWindowRgn` for rounded corners. It breaks the swapchain; DWM's
  corner preference does the job for floating widgets.
- Do not write the whole config from memory. Use `config::update`.

## Dashboard and themes

`dashboard.rs` edits connection/place fields against the latest config through
checked, atomic updates. `studio.rs` edits bounded theme documents; `theme.rs`
is shared with the widget renderer. Default drawing remains in each widget.
Custom themes opt in per widget; returning to the original removes that override.
Dashboard styling is centralized in `dashboard_style.rs` and follows the pinned
CodeZeno reference recorded in the third-party notices.

The provider subset is a workspace library in `vendor/codezeno-usage-core`.
Its full MIT notice and adaptation notes stay with it. The dashboard reads only
typed non-secret usage status snapshots; it never copies sign-in secrets into a
theme or usage cache. Provider errors are surfaced as typed status messages.

Debug builds can read back their own rendered frame for visual checks using
`--preview --preview-shot=<png path>` with a synthetic APPDATA fixture. This is
an app-renderer capture, not a capture of the Windows desktop. The preview flag
also prevents the dashboard from starting a hub. It is intended for local QA.
