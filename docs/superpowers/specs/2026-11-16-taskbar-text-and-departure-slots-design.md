# Taskbar-aware text, wider Next Departure, several Next Departure pills

Date: 2026-11-16. Three independent changes, built and committed in this order.

## 1. Text colour follows the taskbar

**Why.** A pill docked on the taskbar has no background (`app.rs` sets the fill to
`TRANSPARENT` when `on_bar`), so its near-white text (`TEXT` 240, `TEXT_WEAK` 175)
sits straight on the bar. On a dark taskbar that reads well; on a light taskbar the
text disappears.

**Goal.** Docked pills pick dark or light text from the brightness of the taskbar
colour Windows uses. Floating pills keep their dark card (`BG`) and light text.
The pill itself stays transparent on the bar; Andreas does not want a pill
background change.

**How (decided).**
- Taskbar colour comes from Windows settings, not screen pixels:
  - `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`
    `SystemUsesLightTheme` (1 = light taskbar) and `ColorPrevalence`
    (1 = accent colour on Start and taskbar).
  - With `ColorPrevalence` = 1 the bar uses an accent shade; take it from
    `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Accent` `AccentPalette`
    (or `DWM\AccentColor`). Which palette entry Windows actually paints is the
    builder's to confirm against a screenshot.
- Text is dark when the bar's relative luminance is above a threshold, else light.
  Dark text: about gray 25 primary, gray 80 weak (builder tunes for contrast).
- Re-read at most every few seconds (pills repaint every 1 s anyway). A change of
  Windows theme must show within ~1-5 s without restarting.
- Coloured text that is tuned for a dark bar also needs a dark-bar/light-bar pair
  where it would be unreadable on light: the traffic-level colours in
  `departures.rs` (yellow `E8D86A`, light green `7ED38B` vanish on white). Builder
  checks other widgets' coloured text the same way.
- Applies to both render paths: the shared-process CPU renderer (`single.rs`) and
  the process-per-widget eframe path (`app.rs`), which both set `TEXT`/`TEXT_WEAK`
  through egui visuals.

- Custom Studio themes keep their own colours (Andreas' decision): `theme.rs`
  paints a theme's saved colours as they are. Only the built-in layout adapts.
  No theme-format change.

**Non-goals.** Sampling screen pixels; third-party taskbars (ExplorerPatcher,
TranslucentTB) beyond what the registry reports; the Yr weather icons (dark-mode
set) unless a screenshot shows them unreadable on a light bar, then raise it.

## 2. Next Departure slightly wider by default

**Why.** The Norwegian text does not fit. `text_width` in `departures.rs` measures
only `tr("Traffic: Unknown")` ("Trafikk: Ukjent"), but other levels are longer
(e.g. "Trafikk: Middels").

**How.** Measure `Traffic: {level}` for every level in the current language and
the line-one sample, take the widest, add a small margin (~6-8 px). Long notices
still scroll as today.

## 3. Several Next Departure pills

**Why.** Separate pills for, say, the subway lines and the buses at a stop. A
pill's line and direction filters already cover "only these lines"; no transport
mode filter is needed. The only requirement is more than one instance.

**Approaches.**
- **A. Fixed slots, at most 6 (chosen).** `Kind::Departures` becomes
  `Kind::Departures(slot)` with slot 0-5. `Kind::ALL` stays a const array, so
  `DRAGGING` and every `for kind in Kind::ALL` loop keep working. Smaller and safer.
- B. Unlimited instances: rework how every widget is identified (~70 call sites)
  for a need that tops out around four. Rejected.

**Behaviour.**
- Default: one Next Departure pill (slot 0), as today. Slots 1-5 do not exist
  until added; up to 6 in total.
- Each slot has its own stop, favourites, lines, direction, refresh, road-traffic
  settings, `WidgetState` (position, enabled, double-click, theme), and an
  optional name. Display name: the name if set, else the stop name, else
  "Next Departure N".
- Dashboard's Next Departure page gets a slot row `[1] [2] … [+]`: switch slot,
  add (up to 6), remove (not slot 0). Removing asks no confirmation but is only
  saved with "Save changes", like other edits.
- Widgets page, tray menu, Studio list and anywhere else `Kind::ALL` is listed
  show only slots that exist.

**Config (decided).**
- Slot 0 keeps today's fields `departures` and `departures_widget`, so existing
  config files load unchanged and nobody has to set anything up again.
- New field holds slots 1-5, with an empty marker for a removed slot, so removing
  slot 2 does not renumber slots 3+ (live pills and keys stay stable). Exact shape
  (`[Option<DepartureSlot>; 5]` or a map keyed by slot) is the builder's call.
- Keys: `departures`, `departures-2` … `departures-6`; window titles
  "BetterTaskbar Next Departure", "… Next Departure 2", etc.

**Left to the builder.** Polling per slot (one fetch thread per existing, enabled
slot), the `main.rs` `"departures"` dump path, magnet/occupancy ordering, docs
(`docs/departures.md`, `docs/configuration.md`), CHANGELOG, tests.

## Anti-goals
- An update must not move, hide or reset anyone's existing Next Departure pill.
- No pill appears on the taskbar that the user did not add.
