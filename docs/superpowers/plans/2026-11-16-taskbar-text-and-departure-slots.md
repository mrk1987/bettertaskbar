# Taskbar-aware text, wider Next Departure, departure slots — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Docked pills stay readable on light taskbars, Next Departure fits Norwegian text, and up to six Next Departure pills can be set up independently.

**Architecture:** A new `tone` module reads the taskbar colour from the registry and decides "light bar". `WidgetApp::frame` flips each pill's egui visuals (`dark_mode`, text colours) when it is docked on a light bar; widgets with their own colours pick a dark-bar or light-bar variant from `ui.visuals().dark_mode`. Departure slots turn `Kind::Departures` into `Kind::Departures(u8)` (0-5); slot 0 keeps today's config fields, slots 1-5 live in a new map.

**Tech Stack:** Rust (toolchain in `rust-toolchain.toml`), eframe/egui 0.37, `windows` 0.62, serde. Commands run in `widgets/`.

**Spec:** `docs/superpowers/specs/2026-11-16-taskbar-text-and-departure-slots-design.md`

## Global Constraints

- Gates, in `widgets/`: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace -- --test-threads=1`.
- Run `cargo build` before running `target/debug/*.exe` (clippy/test do not rebuild the exe).
- Floating pills keep `BG` card and light text; only docked pills (`on_bar`) adapt.
- Custom Studio themes keep their own colours; `theme.rs` painting is not changed.
- Default is one Next Departure pill; at most 6 (slot 0 plus 1-5).
- Existing config files load unchanged: slot 0 = `departures` + `departures_widget`.
- Removing a slot never renumbers other slots.
- Keys: `departures`, `departures-2` … `departures-6`. Titles: `BetterTaskbar Next Departure`, `BetterTaskbar Next Departure 2` … `6`.
- New user-visible strings get Norwegian entries in `i18n_nb.rs`.
- Do not stage `docs/banner.png` or `widgets/src/widgets/weather.rs` changes that predate this work; commit with `git commit -- <paths>` and check `git diff --cached --stat`.

## Review Focus

1. Windows theme switched while running → docked pills change text colour within ~5 s, no restart (Task 2 test: cache expiry).
2. Accent colour on taskbar with a light accent (e.g. yellow) → dark text (Task 1 test).
3. Old config with a configured, docked Next Departure → same stop, position and enabled state after update (Task 4 test).
4. Removing slot 2 while slots 3 and 4 exist → slots 3 and 4 keep their stops and keys (Task 4 test).
5. A non-existent slot never shows in tray, Widgets page or Studio, and never spawns a pill (Task 5 test on `Kind::exists`).

---

### Task 1: `tone` module — taskbar colour and the light/dark decision

**Files:**
- Create: `widgets/src/tone.rs`
- Modify: `widgets/src/main.rs` (add `mod tone;`), `widgets/Cargo.toml` (add `"Win32_System_Registry"` to windows features)

**Interfaces:**
- Produces:
  - `pub fn relative_luminance(rgb: [u8; 3]) -> f32` (sRGB → linear, WCAG formula, 0.0-1.0)
  - `pub fn bar_is_light(uses_light_theme: bool, accent_on_bar: bool, accent: Option<[u8; 3]>) -> bool`
  - `pub fn taskbar_is_light() -> bool` — cached; re-reads the registry when the cache is older than `REFRESH` (3 s)
  - `pub const LIGHT_THRESHOLD: f32 = 0.4`

- [ ] **Step 1: Write the failing tests** in `tone.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn luminance_endpoints() {
    assert!(relative_luminance([0, 0, 0]) < 0.001);
    assert!(relative_luminance([255, 255, 255]) > 0.999);
}
#[test]
fn plain_light_and_dark_bars() {
    assert!(bar_is_light(true, false, None));
    assert!(!bar_is_light(false, false, None));
}
#[test]
fn accent_on_bar_decides_by_its_brightness() {
    assert!(bar_is_light(false, true, Some([0xFF, 0xD7, 0x00])));   // yellow
    assert!(!bar_is_light(true, true, Some([0x00, 0x33, 0x66])));   // dark blue
    assert!(bar_is_light(true, true, None));                         // unreadable accent: fall back to theme
}
```

- [ ] **Step 2: Run** `cargo test tone::` → FAIL (module missing).
- [ ] **Step 3: Implement.** Pure functions as above. `taskbar_is_light` reads with `RegGetValueW` (`RRF_RT_REG_DWORD` / `RRF_RT_REG_BINARY`):
  - `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize` → `SystemUsesLightTheme`, `ColorPrevalence` (missing = 0)
  - `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Accent` → `AccentPalette` (32 bytes, 8 RGBA entries); use entry 5 (bytes 20..23) as the accent-on-taskbar shade. Verify the entry against a real screenshot in Task 7; change the index there if it does not match.
  - Cache in a `static Mutex<Option<(Instant, bool)>>`.
- [ ] **Step 4: Run** `cargo test tone::` → PASS; clippy clean.
- [ ] **Step 5: Commit** `tone: read the taskbar colour and decide light or dark`.

### Task 2: Docked pills switch text colours on a light bar

**Files:**
- Modify: `widgets/src/app.rs` (constants near line 26-33, `frame` around line 1015), `widgets/src/widgets/departures.rs` (`Level::color`), `widgets/src/widgets/weather.rs` (`WARM`, `COLD`, `temp_color`), `widgets/src/widgets/now_playing.rs` (glyph colour line ~297, hover circle ~295)

**Interfaces:**
- Consumes: `tone::taskbar_is_light()`
- Produces:
  - `pub fn on_light(ui: &egui::Ui) -> bool` in `app.rs` (`!ui.visuals().dark_mode`)
  - `app.rs` constants `TEXT_ON_LIGHT = from_gray(25)`, `TEXT_WEAK_ON_LIGHT = from_gray(80)`
  - `Level::color(self, on_light: bool) -> Color32`

- [ ] **Step 1: Write failing tests.** In `app.rs` tests, a pure helper `fn text_colors(docked: bool, light_bar: bool) -> (Color32, Color32, bool /*dark_mode*/)`:

```rust
#[test]
fn docked_on_light_bar_uses_dark_text() {
    assert_eq!(text_colors(true, true), (TEXT_ON_LIGHT, TEXT_WEAK_ON_LIGHT, false));
    assert_eq!(text_colors(true, false), (TEXT, TEXT_WEAK, true));
    assert_eq!(text_colors(false, true), (TEXT, TEXT_WEAK, true)); // floating card stays dark
}
```

In `departures.rs` and `weather.rs` tests, a contrast check (WCAG ratio from `tone::relative_luminance`) of every variant against its background: dark variants vs `#202020`, light variants vs `#F3F3F3`, each ≥ 3.0:

```rust
#[test]
fn level_colours_are_readable_on_both_bars() {
    for l in [Level::Unknown, Level::None, Level::Low, Level::Medium, Level::High] {
        assert!(contrast(l.color(false), [0x20; 3]) >= 3.0, "{l:?} dark");
        assert!(contrast(l.color(true), [0xF3; 3]) >= 3.0, "{l:?} light");
    }
}
```

(`contrast` lives in `tone` as `pub fn contrast(a: Color32, b: [u8; 3]) -> f32`; add it there with its own test `contrast(black, white) ≈ 21`.)

- [ ] **Step 2: Run** `cargo test` → FAIL.
- [ ] **Step 3: Implement.** In `frame`, compute `docked = on_bar` before the visuals block (move the existing `on_bar` computation up), call `text_colors(docked, docked && tone::taskbar_is_light())`, and apply via `ctx.all_styles_mut` only when it differs from the current style (same pattern as today: set `visuals.dark_mode`, `widgets.noninteractive.fg_stroke.color`, `weak_text_color`). Widgets choose variants with `crate::app::on_light(ui)`. Pick light-bar variants that pass the contrast test (darker red/orange/amber/green/blue).
- [ ] **Step 4: Run** gates → PASS.
- [ ] **Step 5: Commit** `Docked pills use dark text on a light taskbar`.

### Task 3: Next Departure wide enough for every traffic label

**Files:**
- Modify: `widgets/src/widgets/departures.rs` (`text_width`, ~line 694-711)

**Interfaces:**
- Produces: `fn widest(measure: impl Fn(&str, f32) -> f32) -> f32` (`text_width` becomes `widest(<egui measure>)`)
- `const WIDTH_MARGIN: f32 = 6.0`

- [ ] **Step 1: Failing test** with a fake measure (1 px per char), independent of the global language:

```rust
#[test]
fn width_fits_the_longest_traffic_label_plus_margin() {
    let w = widest(|s, _| s.chars().count() as f32);
    let longest = [Level::Unknown, Level::None, Level::Low, Level::Medium, Level::High]
        .iter()
        .map(|l| trf("Traffic: {level}", &[("level", &l.label())]).chars().count())
        .max().unwrap() as f32;
    assert!(w >= longest + WIDTH_MARGIN);
}
```

- [ ] **Step 2: Run** → FAIL. **Step 3:** implement: max over `Traffic: {level}` for all levels (11 pt) and `"31 · 10 min"` (12 pt), plus `WIDTH_MARGIN`, `.ceil()`. **Step 4:** gates PASS.
- [ ] **Step 5: Commit** `Next Departure: wide enough for every traffic level`.

### Task 4: Config and `Kind` for departure slots

**Files:**
- Modify: `widgets/src/config.rs`, `widgets/src/app.rs` (`Kind` enum and `impl Kind`, lines ~56-280), every `Kind::Departures` match arm (`click_action.rs:69`, `dashboard.rs:54`, `theme.rs:245,270,290`, `single.rs:709`, `app.rs:373,425-431,840,890`) → `Kind::Departures(_)` or slot-aware

**Interfaces:**
- Produces:
  - `DeparturesCfg.name: String` (serde default empty)
  - `pub struct DepartureSlot { pub departures: DeparturesCfg, pub widget: WidgetState }` (`Default`: widget `WidgetState::opt_in()`)
  - `Config.departure_slots: BTreeMap<u8, DepartureSlot>` keyed 1..=5; `#[serde(default)]`
  - `pub const DEPARTURE_SLOTS: u8 = 6`
  - `impl Config { pub fn departures_for(&self, slot: u8) -> Option<&DeparturesCfg>; pub fn departures_for_mut(&mut self, slot: u8) -> Option<&mut DeparturesCfg>; pub fn add_departure_slot(&mut self) -> Option<u8> /* lowest free 1..=5 */; pub fn remove_departure_slot(&mut self, slot: u8) /* no-op for 0 */ }`
  - `Kind::Departures(u8)`; `Kind::ALL: [Kind; 18]` (six departure entries in slot order where the single one was)
  - `Kind::exists(self, c: &Config) -> bool` (true for all non-departure kinds and slot 0)
  - `Kind::display_name(self, c: &Config) -> String`: departures → `name` if non-empty, else stop name, else `"Next Departure"` (slot 0) / `"Next Departure N"` (N = slot+1, via `trf`); others → `label().into()`
  - `key()` / `title()` from static tables per the Global Constraints
  - `state`/`state_mut`: slot 0 → `departures_widget`; slot n → `departure_slots[n].widget`; missing → `&DISABLED_USAGE`-style static (`state`) / `entry().or_default()` (`state_mut`, only reached for existing slots in practice)

- [ ] **Step 1: Failing tests** in `config.rs` tests:

```rust
#[test]
fn old_config_is_slot_zero_unchanged() {
    let old = r#"{"departures":{"stop":{"id":"NSR:StopPlace:1","name":"A","lat":0.0,"lon":0.0}},
                  "departures_widget":{"enabled":true,"x":5,"y":6}}"#;
    let c: Config = serde_json::from_str(old).unwrap();
    assert_eq!(c.departures_for(0).unwrap().stop.as_ref().unwrap().name, "A");
    assert!(Kind::Departures(0).state(&c).enabled);
    assert!((1..6).all(|s| !Kind::Departures(s).exists(&c)));
}
#[test]
fn removing_a_slot_keeps_the_others() {
    let mut c = Config::default();
    assert_eq!((c.add_departure_slot(), c.add_departure_slot(), c.add_departure_slot()), (Some(1), Some(2), Some(3)));
    c.departures_for_mut(3).unwrap().direction = "Sentrum".into();
    c.remove_departure_slot(2);
    assert!(!Kind::Departures(2).exists(&c));
    assert_eq!(c.departures_for(3).unwrap().direction, "Sentrum");
    assert_eq!(c.add_departure_slot(), Some(2)); // reuses the gap
}
#[test]
fn at_most_six_and_slot_zero_stays() {
    let mut c = Config::default();
    for _ in 0..5 { assert!(c.add_departure_slot().is_some()); }
    assert_eq!(c.add_departure_slot(), None);
    c.remove_departure_slot(0);
    assert!(Kind::Departures(0).exists(&c));
}
#[test]
fn slot_keys_and_titles() {
    assert_eq!(Kind::Departures(0).key(), "departures");
    assert_eq!(Kind::Departures(5).key(), "departures-6");
    assert_eq!(Kind::Departures(1).title(), "BetterTaskbar Next Departure 2");
}
```

(Config files are JSON, loaded with `serde_json::from_str` as in the existing tests at `config.rs:546-648`.)

- [ ] **Step 2: Run** → FAIL. **Step 3:** implement; fix all match arms so it compiles (slot-aware behaviour lands in Task 5; here `Kind::Departures(_)` keeps today's behaviour). Existing test `config.rs:600-613` must still pass. **Step 4:** gates PASS.
- [ ] **Step 5: Commit** `Config: Next Departure slots`.

### Task 5: Run one pill per existing slot

**Files:**
- Modify: `widgets/src/widgets/mod.rs` (`poll`), `widgets/src/widgets/departures.rs` (`pick`, `DeparturesWidget::new`), `widgets/src/app.rs` (`new`, default position ~425, lines 840/890 stop picking), `widgets/src/main.rs` (47, 90, 137, 202, 316), `widgets/src/single.rs` (534), `widgets/src/tray.rs` (42, 69, 114, 346), `widgets/src/magnet.rs` (369), `widgets/src/studio.rs` (123)

**Interfaces:**
- Consumes: Task 4 API.
- Produces: `poll` takes `pick: impl Fn(&Config) -> (C, u64, bool) + Send + 'static`; `DeparturesWidget::new(slot: u8, cfg: SharedConfig)`; poll tag `departures` / `departures-N`.

- [ ] **Step 1: Failing test** (`app.rs` tests): `user_facing_kinds(&c)` — the iterator tray/Studio/Widgets page use — returns `Kind::Departures(0)` and only added slots:

```rust
#[test]
fn only_existing_slots_are_listed() {
    let mut c = Config::default();
    c.add_departure_slot();
    let listed: Vec<_> = user_facing_kinds(&c).filter(|k| matches!(k, Kind::Departures(_))).collect();
    assert_eq!(listed, [Kind::Departures(0), Kind::Departures(1)]);
}
```

- [ ] **Step 2: Run** → FAIL. **Step 3:** add `pub fn user_facing_kinds(c: &Config) -> impl Iterator<Item = Kind> + '_` in `app.rs` (= `Kind::ALL` filtered by `exists`) and use it in every listed loop that shows or spawns pills. Pill spawn must also require `exists` (a removed slot's leftover `enabled` must not spawn). Default position: slots continue after News, `slot = 3 + n`. `--check=departures-N` and the `"departures"` dump accept slot keys. Right-click "pick stop" (app.rs ~890) writes to its own slot.
- [ ] **Step 4:** gates PASS. **Step 5: Commit** `Run a Next Departure pill per slot`.

### Task 6: Dashboard slot row, name, add and remove

**Files:**
- Modify: `widgets/src/dashboard.rs` (state ~189/215; dirty check ~251; save ~273; `departures` page ~853-965; validation ~1323; per-page apply ~1373-1384; Widgets page lists ~447/616/1286), `widgets/src/i18n_nb.rs`

**Interfaces:**
- Consumes: Task 4/5 API.
- Produces: `Dashboard.dep_slot: u8` (selected slot, reset to 0 if it stops existing).

- [ ] **Step 1: Failing test** for validation (pure fn already at ~1323, make it check every existing slot):

```rust
#[test]
fn every_slot_is_validated() {
    let mut c = Config::default();
    let s = c.add_departure_slot().unwrap();
    c.departures_for_mut(s).unwrap().refresh_secs = 5;
    assert!(validate(&c).is_err());
}
```

(`validate` is `dashboard.rs:1285`.)

- [ ] **Step 2: Run** → FAIL. **Step 3:** implement:
  - Row above the stop group: one selectable button per existing slot labelled `1`…`6` (hover text = `display_name`), then `+` (disabled at 6; adds via `add_departure_slot` on the draft and selects it), then `Remove this pill` for slot ≥ 1 (removes on the draft; saved with Save changes).
  - `Name` field (`style::field`, not password) bound to the selected slot's `DeparturesCfg.name`; hint "Shown in the tray and dashboard; empty uses the stop name".
  - All existing departures controls (stop search, favourites, lines, direction, refresh, road traffic, double-click action) bind to the selected slot; reset `lines_text` when switching slot.
  - Dirty check, save and per-page apply compare/copy `departures` **and** `departure_slots`.
  - Norwegian: "Name"→"Navn", "Remove this pill"→"Fjern denne", "Next Departure {n}"→"Neste avgang {n}", hint text.
- [ ] **Step 4:** gates PASS. **Step 5: Commit** `Dashboard: add, switch and remove Next Departure pills`.

### Task 7: Live verification and docs

**Files:**
- Modify: `docs/departures.md`, `docs/configuration.md`, `docs/dashboard.md`, `CHANGELOG.md` (new `## Unreleased` section: Added slots; Changed text colour on light taskbar, wider Next Departure)

- [ ] **Step 1:** `cargo build`; run the app with a copy of the config; add a second slot in the dashboard, set a different line, save → two pills, each with its own data (screenshot).
- [ ] **Step 2:** Read the current `SystemUsesLightTheme`/`ColorPrevalence`. Verify the colours through a forced-tone render, not by changing Andreas' Windows theme: add an env override `BTW_FORCE_LIGHT_BAR=1` read by `tone::taskbar_is_light` (documented as a test aid in `docs/configuration.md`), render the pills over a `#F3F3F3` background with the existing banner/PNG path (`single.rs` ~669), and look at the PNGs: all text readable. If Andreas uses accent-on-taskbar, compare palette entry 5 with a screenshot of the bar and fix the index in Task 1's code if needed.
- [ ] **Step 3:** docs + CHANGELOG; gates PASS.
- [ ] **Step 4: Commit** `Docs and changelog for light taskbars and departure slots`.
