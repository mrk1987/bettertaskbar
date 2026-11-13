//! Per-widget eframe app: the pill, its right-click menu, and how it moves.
//!
//! Moving follows Claude-Code-Usage-Monitor: press anywhere on the pill and
//! drag it freely. Near a taskbar it snaps into a free gap between the app
//! icons and the notification area; released there it docks, released
//! anywhere else it floats where it was dropped. A docked pill remembers its
//! distance from the notification area, not a screen position, so it keeps
//! its place when tray icons come and go or the resolution changes.

use std::time::{Duration, Instant};

use eframe::egui;
use windows::Win32::Foundation::{HWND, POINT, RECT};

use crate::config::{Placement, SharedConfig, WidgetState};
use crate::icons::IconCache;
use crate::occupancy;
use crate::shell::{self, Bar};
use crate::widgets::calendar::CalendarWidget;
use crate::widgets::mail::MailWidget;
use crate::widgets::weather::WeatherWidget;
use crate::widgets::Widget;
use codezeno_usage_core::providers::ProviderId;

pub const BG: egui::Color32 = egui::Color32::from_rgb(0x20, 0x24, 0x2c);
/// Every widget pill is exactly this tall so the three line up on the bar.
pub const PILL_HEIGHT: f32 = 48.0;
/// Text on a pill, as bright as the taskbar's own clock. egui's default
/// label gray (140) reads dim on a dark taskbar.
const TEXT: egui::Color32 = egui::Color32::from_gray(240);
/// Second lines (`RichText::weak`).
const TEXT_WEAK: egui::Color32 = egui::Color32::from_gray(175);
/// Tallest widget element (the weather symbol); the centered content row height.
const CONTENT_HEIGHT: f32 = 28.0;

/// The cursor may be this far outside a taskbar and still snap to it.
const SNAP_REACH: i32 = 20;
/// Share of the pill that must lie in a free gap to snap in, and the lower
/// share that keeps it snapped, so it does not flicker at the edge.
const SNAP_IN: f64 = 0.67;
const SNAP_KEEP: f64 = 0.45;
/// Dropped this close to the notification area, the pill sits flush with it.
const TRAY_SNAP: i32 = 8;
/// How often a docked pill checks that it is still where it belongs.
const DOCK_CHECK: Duration = Duration::from_millis(500);
/// How often an idle pill repaints. Each repaint also takes in new data,
/// rotates mail subjects and runs the dock check, and nothing a pill shows
/// changes faster than once a second; input, a config change and a finished
/// icon download repaint at once on their own. Debug builds keep the old
/// 200 ms so previews and the fake-input drag driver stay quick.
const IDLE_REPAINT: Duration =
    Duration::from_millis(if cfg!(debug_assertions) { 200 } else { 1000 });

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Weather,
    Calendar,
    Mail,
    Usage(ProviderId),
}

/// True while a pill is being dragged, one flag per widget kind, so pills
/// that share a process (the default since 1.0.1) do not see each other's drags.
static DRAGGING: [std::sync::atomic::AtomicBool; Kind::ALL.len()] =
    [const { std::sync::atomic::AtomicBool::new(false) }; Kind::ALL.len()];

/// A floating pill this close to a taskbar rests flush against it instead.
const FLOAT_SNAP: i32 = 16;

/// The y a floating pill at `r` must move to so it sits flush against the
/// bar's inner edge instead of overlapping it, or `None` if it is already
/// clear (or flush). Only pills within `FLOAT_SNAP` of the bar are moved.
fn clear_of_bar(r: RECT, bar: RECT, bar_at_top: bool) -> Option<i32> {
    if r.right <= bar.left || r.left >= bar.right {
        return None;
    }
    let h = r.bottom - r.top;
    let y = if bar_at_top {
        (r.top < bar.bottom + FLOAT_SNAP).then_some(bar.bottom)
    } else {
        (r.bottom > bar.top - FLOAT_SNAP).then_some(bar.top - h)
    }?;
    (y != r.top).then_some(y)
}

/// Whether `bar` runs along the top of its monitor.
fn bar_at_top(bar: &Bar) -> bool {
    shell::monitor_rect_at(POINT {
        x: bar.rect.left,
        y: bar.rect.top,
    })
    .is_some_and(|m| bar.rect.top - m.top < m.bottom - bar.rect.bottom)
}

/// Where a floating pill at (x, y) should go so it never sits on a taskbar.
fn float_position(x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    let r = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    shell::taskbars()
        .iter()
        .find_map(|bar| clear_of_bar(r, bar.rect, bar_at_top(bar)))
        .map_or((x, y), |snapped| (x, snapped))
}

/// Whether the upkeep thread should attach the pill to its taskbar.
fn upkeep_docks(magnetic: bool, placement: Placement, dragging: bool) -> bool {
    magnetic && placement == Placement::Taskbar && !dragging
}

static DISABLED_USAGE: WidgetState = WidgetState {
    placement: Placement::Taskbar,
    dock_side: Some(crate::config::DockSide::Right),
    manual_bar_offset: None,
    enabled: false,
    theme: None,
    visible: true,
    x: 24,
    y: 24,
    tray_offset: None,
    monitor: None,
};

impl Kind {
    pub const ALL: [Kind; 10] = [
        Kind::Weather,
        Kind::Calendar,
        Kind::Mail,
        Kind::Usage(ProviderId::Claude),
        Kind::Usage(ProviderId::Codex),
        Kind::Usage(ProviderId::Antigravity),
        Kind::Usage(ProviderId::OpenCode),
        Kind::Usage(ProviderId::Cursor),
        Kind::Usage(ProviderId::Grok),
        Kind::Usage(ProviderId::Copilot),
    ];

    /// Position in `Kind::ALL`.
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Weather => "weather",
            Self::Calendar => "calendar",
            Self::Mail => "mail",
            Self::Usage(p) => p.descriptor().key,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Weather => "Weather",
            Self::Calendar => "Calendar",
            Self::Mail => "Mail",
            Self::Usage(p) => p.descriptor().display_name,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::Weather => "BetterTaskbar Weather",
            Kind::Calendar => "BetterTaskbar Calendar",
            Kind::Mail => "BetterTaskbar Mail",
            Kind::Usage(p) => match p {
                ProviderId::Claude => "BetterTaskbar Claude Code",
                ProviderId::Codex => "BetterTaskbar Codex",
                ProviderId::Antigravity => "BetterTaskbar Antigravity",
                ProviderId::OpenCode => "BetterTaskbar OpenCode",
                ProviderId::Cursor => "BetterTaskbar Cursor",
                ProviderId::Grok => "BetterTaskbar Grok",
                ProviderId::Copilot => "BetterTaskbar GitHub Copilot",
            },
        }
    }

    pub fn state(self, c: &crate::config::Config) -> &WidgetState {
        match self {
            Kind::Weather => &c.weather_widget,
            Kind::Calendar => &c.calendar_widget,
            Kind::Mail => &c.mail_widget,
            Kind::Usage(p) => c.usage_widgets.get(&p).unwrap_or(&DISABLED_USAGE),
        }
    }

    pub fn state_mut(self, c: &mut crate::config::Config) -> &mut WidgetState {
        match self {
            Kind::Weather => &mut c.weather_widget,
            Kind::Calendar => &mut c.calendar_widget,
            Kind::Mail => &mut c.mail_widget,
            Kind::Usage(p) => c
                .usage_widgets
                .entry(p)
                .or_insert_with(|| DISABLED_USAGE.clone()),
        }
    }
}

/// A press on the pill, and the drag it may turn into.
struct Drag {
    start_cursor: POINT,
    start_origin: POINT,
    /// False until the cursor has moved past the system drag threshold; a
    /// press released before that is a click.
    moving: bool,
    /// The taskbar the pill is currently snapped to, if any.
    snapped: Option<Bar>,
    /// The other pills' windows, looked up once: their space is taken too.
    others: Vec<HWND>,
}

pub struct WidgetApp {
    kind: Kind,
    cfg: SharedConfig,
    widget: Box<dyn Widget>,
    icons: IconCache,
    #[cfg(debug_assertions)]
    preview_frame: usize,
    custom_fonts: bool,
    hwnd: Option<HWND>,
    applied_placement: Option<Placement>,
    applied_round: Option<bool>,
    last_size: egui::Vec2,
    drag: Option<Drag>,
    last_dock_check: Option<Instant>,
    last_magnetic_mode: Option<bool>,
    /// Drawn by `single.rs` in a process shared with other pills, instead of
    /// one eframe window per process.
    in_process: bool,
    /// Cleared when this pill's window is gone, so its upkeep thread stops.
    window_alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for WidgetApp {
    fn drop(&mut self) {
        self.window_alive
            .store(false, std::sync::atomic::Ordering::SeqCst);
        DRAGGING[self.kind.index()].store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl WidgetApp {
    /// A pill for `single.rs`, drawn on the CPU in a shared process.
    pub fn new_in_process(kind: Kind, cfg: SharedConfig) -> Self {
        let mut app = Self::new(kind, cfg);
        app.in_process = true;
        app
    }

    /// The pill's window was destroyed (Explorer took the taskbar with it).
    /// The next `frame` with a new window sets everything up again; data and
    /// background fetching carry on.
    pub fn window_lost(&mut self) {
        self.window_alive
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.window_alive = Default::default();
        self.hwnd = None;
        self.applied_placement = None;
        self.applied_round = None;
        self.last_size = egui::Vec2::ZERO;
        self.drag = None;
        self.last_dock_check = None;
        self.last_magnetic_mode = None;
        DRAGGING[self.kind.index()].store(false, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn new(kind: Kind, cfg: SharedConfig) -> Self {
        let widget: Box<dyn Widget> = match kind {
            Kind::Weather => Box::new(WeatherWidget::new(cfg.clone())),
            Kind::Calendar => Box::new(CalendarWidget::new(cfg.clone())),
            Kind::Mail => Box::new(MailWidget::new(cfg.clone())),
            Kind::Usage(p) => Box::new(crate::widgets::usage::UsageWidget::new(p, cfg.clone())),
        };
        Self {
            kind,
            cfg,
            widget,
            icons: IconCache::default(),
            #[cfg(debug_assertions)]
            preview_frame: 0,
            custom_fonts: false,
            hwnd: None,
            applied_placement: None,
            applied_round: None,
            last_size: egui::Vec2::ZERO,
            drag: None,
            last_dock_check: None,
            last_magnetic_mode: None,
            in_process: false,
            window_alive: Default::default(),
        }
    }

    pub fn size_hint(&self) -> (f32, f32) {
        self.widget.size()
    }

    pub fn window_title(&self) -> &'static str {
        self.kind.title()
    }

    /// Default distance from the notification area, in logical pixels, for a
    /// pill that has never been placed: the three sit side by side with
    /// weather nearest the tray. Widths match each widget's `size()` hint.
    fn default_tray_offset(&self) -> i32 {
        const GAP: i32 = 8;
        const WEATHER: i32 = 100;
        match self.kind {
            Kind::Weather => GAP,
            Kind::Calendar => GAP + WEATHER + GAP,
            Kind::Mail => GAP + WEATHER + GAP + 230 + GAP,
            Kind::Usage(p) => GAP + WEATHER + GAP + 230 + GAP + 230 + GAP + (p as i32) * 176,
        }
    }

    fn state(&self) -> WidgetState {
        self.kind.state(&self.cfg.lock().unwrap()).clone()
    }

    fn with_state(&self, f: impl FnOnce(&mut WidgetState)) {
        let kind = self.kind;
        crate::config::update(&self.cfg, |c| f(kind.state_mut(c)));
    }

    /// Put a docked pill where its saved tray distance says, if it is not
    /// there already. Runs on a timer, so it also follows the tray growing,
    /// a resolution change, and Explorer restarting with a new taskbar.
    fn keep_docked(&mut self, hwnd: HWND, state: &WidgetState, ppp: f32) {
        let Some(bar) = shell::bar_for(state.monitor.as_deref()) else {
            return;
        };
        let r = shell::screen_rect(hwnd);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let offset = match state.tray_offset {
            Some(offset) => offset,
            None => {
                // First dock. A position saved by an older version that lies
                // on the bar is kept; otherwise use the default line-up.
                let saved = RECT {
                    left: state.x,
                    top: state.y,
                    right: state.x + w,
                    bottom: state.y + h,
                };
                let offset = if shell::intersection(saved, bar.rect).is_some() {
                    (bar.tray_left - state.x - w).max(0)
                } else {
                    (self.default_tray_offset() as f32 * ppp).round() as i32
                };
                self.with_state(|s| s.tray_offset = Some(offset));
                offset
            }
        };
        // Become part of this taskbar (again, after a drag, or after Explorer
        // restarted and made a new one).
        if shell::parent_of(hwnd) != Some(bar.hwnd) && !shell::embed(hwnd, bar.hwnd) {
            return;
        }
        let magnetic = self.cfg.lock().unwrap().magnetized;
        let previous = self.last_magnetic_mode.replace(magnetic);
        if magnetic {
            return;
        } // Only the hub positions magnetic groups.
        let manual = if previous == Some(true) && shell::intersection(r, bar.rect).is_some() {
            r.left - bar.rect.left
        } else {
            state
                .manual_bar_offset
                .unwrap_or_else(|| shell::dock_xy(&bar, w, h, offset).0 - bar.rect.left)
        };
        if state.manual_bar_offset != Some(manual) {
            self.with_state(|s| s.manual_bar_offset = Some(manual));
        }
        let x =
            (bar.rect.left + manual).clamp(bar.rect.left, (bar.rect.right - w).max(bar.rect.left));
        let y = bar.rect.top + (bar.rect.bottom - bar.rect.top - h) / 2;
        if (r.left, r.top) != (x, y) {
            shell::place(hwnd, x, y);
        }
    }

    /// Where a pill dragged to `free` should snap, if the cursor is at a
    /// taskbar with a gap wide enough that the pill mostly covers already.
    fn snap_target(drag: &Drag, free: RECT, cursor: POINT) -> Option<(Bar, i32, i32)> {
        let (w, h) = (free.right - free.left, free.bottom - free.top);
        let bar = shell::taskbars().into_iter().find(|b| {
            cursor.x >= b.rect.left - SNAP_REACH
                && cursor.x <= b.rect.right + SNAP_REACH
                && cursor.y >= b.rect.top - SNAP_REACH
                && cursor.y <= b.rect.bottom + SNAP_REACH
        })?;
        let y = bar.rect.top + (bar.rect.bottom - bar.rect.top - h) / 2;

        // What is taken: the shell's own controls if they could be read, the
        // notification area in any case, and the other pills.
        let mut taken = occupancy::occupied(bar.hwnd, bar.rect).unwrap_or_default();
        taken.push(RECT {
            left: bar.tray_left,
            right: bar.rect.right,
            ..bar.rect
        });
        taken.extend(
            drag.others
                .iter()
                .filter_map(|other| shell::visible_rect(*other)),
        );

        let slot = |&(left, right): &(i32, i32)| RECT {
            left,
            top: y,
            right,
            bottom: y + h,
        };
        let best = occupancy::free_slots(bar.rect, &taken, y, y + h)
            .into_iter()
            .filter(|(left, right)| right - left >= w)
            .max_by(|a, b| {
                shell::overlap_ratio(free, slot(a)).total_cmp(&shell::overlap_ratio(free, slot(b)))
            })?;
        let needed = if drag.snapped.is_some() {
            SNAP_KEEP
        } else {
            SNAP_IN
        };
        (shell::overlap_ratio(free, slot(&best)) >= needed).then(|| {
            let x = free.left.clamp(best.0, best.1 - w);
            (bar, x, y)
        })
    }

    /// Advance a press or drag by one frame; returns false when it has ended.
    fn step_drag(&mut self, hwnd: HWND) -> bool {
        let Some(drag) = self.drag.as_mut() else {
            return false;
        };
        if !shell::primary_button_down() {
            return false;
        }
        let cursor = shell::cursor();
        let (dx, dy) = (
            cursor.x - drag.start_cursor.x,
            cursor.y - drag.start_cursor.y,
        );
        if !drag.moving {
            let (tx, ty) = shell::drag_threshold();
            if dx.abs() < tx && dy.abs() < ty {
                return true;
            }
            drag.moving = true;
            // Before detaching: the upkeep thread would otherwise see a
            // taskbar placement and a free pill, and re-attach it mid-drag.
            DRAGGING[self.kind.index()].store(true, std::sync::atomic::Ordering::SeqCst);
            // A desktop-pinned pill is a child window; free it to follow the cursor.
            shell::make_popup(hwnd);
        }
        let r = shell::screen_rect(hwnd);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (x, y) = (drag.start_origin.x + dx, drag.start_origin.y + dy);
        let free = RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        match Self::snap_target(drag, free, cursor) {
            Some((bar, sx, sy)) => {
                shell::place(hwnd, sx, sy);
                drag.snapped = Some(bar);
            }
            None => {
                // Not docking: never hover over or behind a taskbar; rest
                // on its edge instead, so the drop shows where it will land.
                let (x, y) = float_position(x, y, w, h);
                shell::place(hwnd, x, y);
                drag.snapped = None;
            }
        }
        true
    }

    /// The button came up: dock where the pill snapped, or float where it is.
    fn finish_drag(&mut self, hwnd: HWND) {
        let Some(drag) = self.drag.take() else { return };
        occupancy::set_active(false);
        if !drag.moving {
            return;
        }
        let r = shell::screen_rect(hwnd);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        match drag.snapped {
            Some(bar) => {
                let gap = (bar.tray_left - r.right).max(0);
                let offset = if gap <= TRAY_SNAP { 0 } else { gap };
                self.with_state(|s| {
                    s.placement = Placement::Taskbar;
                    s.tray_offset = Some(offset);
                    s.monitor = Some(bar.monitor.clone());
                    s.manual_bar_offset = Some(r.left - bar.rect.left);
                    s.dock_side = crate::magnet::side_for_drop(&bar, r.left, w);
                });
                self.last_dock_check = None;
                self.applied_placement = Some(Placement::Taskbar);
            }
            None => {
                // Keep the whole pill on the monitor it was dropped on.
                let (mut x, mut y) = (r.left, r.top);
                if let Some(m) = shell::monitor_rect_at(shell::cursor()) {
                    x = x.clamp(m.left, (m.right - w).max(m.left));
                    y = y.clamp(m.top, (m.bottom - h).max(m.top));
                }
                // The monitor clamp can push a pill back onto a bottom bar.
                (x, y) = float_position(x, y, w, h);
                shell::place(hwnd, x, y);
                self.with_state(|s| {
                    s.placement = Placement::Floating;
                    s.x = x;
                    s.y = y;
                });
                self.applied_placement = Some(Placement::Floating);
            }
        }
        // Only now that the new placement is saved may upkeep dock again.
        DRAGGING[self.kind.index()].store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Apply placement changes and keep a docked pill where it belongs.
    fn manage_window(&mut self, hwnd: HWND, state: &WidgetState, ppp: f32, moving: bool) {
        // A placement chosen from a menu (here, the tray, or the file).
        if !moving && self.applied_placement != Some(state.placement) {
            match state.placement {
                // `keep_docked` below makes the pill a child of the taskbar.
                Placement::Taskbar => {
                    self.last_dock_check = None;
                }
                Placement::Desktop | Placement::Floating => {
                    if state.placement == Placement::Desktop {
                        shell::pin_desktop(hwnd);
                    } else {
                        shell::make_popup(hwnd);
                    }
                    // Leaving the taskbar, the pill would stay on top of it:
                    // hidden behind the bar on the desktop, or covering the
                    // bar's buttons when floating. Move it just off the bar.
                    let r = shell::screen_rect(hwnd);
                    if let Some(bar) = shell::taskbars()
                        .into_iter()
                        .find(|b| shell::intersection(r, b.rect).is_some())
                    {
                        let h = r.bottom - r.top;
                        let monitor = shell::monitor_rect_at(POINT {
                            x: r.left,
                            y: bar.rect.top,
                        });
                        let bar_at_top = monitor
                            .is_some_and(|m| bar.rect.top - m.top < m.bottom - bar.rect.bottom);
                        let y = if bar_at_top {
                            bar.rect.bottom + 8
                        } else {
                            bar.rect.top - h - 8
                        };
                        shell::place(hwnd, r.left, y);
                    }
                }
            }
            if state.placement != Placement::Taskbar {
                // Remember where the pill now is on screen.
                let r = shell::screen_rect(hwnd);
                if (state.x, state.y) != (r.left, r.top) {
                    self.with_state(|s| {
                        s.x = r.left;
                        s.y = r.top;
                    });
                }
            }
            self.applied_placement = Some(state.placement);
        }
        // A pill that is not meant for the taskbar but is still its child is
        // clipped to the bar and invisible anywhere else. That happens when a
        // re-dock raced a placement change; free it at its saved position.
        if !moving && state.placement != Placement::Taskbar && shell::on_a_taskbar(hwnd) {
            crate::log::line(
                self.kind.key(),
                "was still attached to the taskbar after leaving it; detached",
            );
            if state.placement == Placement::Desktop {
                shell::pin_desktop(hwnd);
            } else {
                shell::make_popup(hwnd);
            }
            shell::place(hwnd, state.x, state.y);
        }
        // Wait for the window to take the pill's size before docking by it.
        let sized = self.last_size != egui::Vec2::ZERO;
        if !moving
            && sized
            && state.placement == Placement::Taskbar
            && self
                .last_dock_check
                .is_none_or(|t| t.elapsed() >= DOCK_CHECK)
        {
            self.last_dock_check = Some(Instant::now());
            self.keep_docked(hwnd, state, ppp);
        }
    }

    /// Start, advance or finish a press or drag of the pill.
    fn drag_frame(&mut self, hwnd: HWND, pressed: bool) {
        if self.drag.is_none() && (pressed || shell::fake_press_on(hwnd)) {
            let r = shell::screen_rect(hwnd);
            let others = Kind::ALL
                .into_iter()
                .filter(|k| *k != self.kind)
                .filter_map(|k| shell::find_by_title(k.title()))
                .collect();
            self.drag = Some(Drag {
                start_cursor: shell::cursor(),
                start_origin: POINT {
                    x: r.left,
                    y: r.top,
                },
                moving: false,
                snapped: None,
                others,
            });
            // Start reading the taskbar layout now so it is there by the
            // time the press has become a drag.
            occupancy::set_active(true);
        }
        if self.drag.is_some() && !self.step_drag(hwnd) {
            self.finish_drag(hwnd);
        }
    }

    fn show_menu(&mut self) {
        use crate::popup::Item;
        enum Action {
            Place(Placement),
            Hide,
            Weather(crate::config::Place),
            Search,
            Settings,
            Quit,
        }
        crate::config::refresh(&self.cfg);
        let placement = self.state().placement;
        let mut items: Vec<_> = [
            (Placement::Taskbar, "Dock on taskbar"),
            (Placement::Desktop, "Pin to desktop"),
            (Placement::Floating, "Float on top"),
        ]
        .into_iter()
        .map(|(p, label)| Item::Command(label.into(), placement == p, Action::Place(p)))
        .collect();
        items.push(Item::Command("Settings…".into(), false, Action::Settings));
        items.push(Item::Command("Hide widget".into(), false, Action::Hide));
        if self.kind == Kind::Weather {
            let weather = self.cfg.lock().unwrap().weather.clone();
            items.push(Item::Separator);
            let mut places = Vec::new();
            if let Some(current) = &weather.place {
                places.push(Item::Command(
                    current.name.clone(),
                    true,
                    Action::Weather(current.clone()),
                ));
            }
            for favorite in weather.favorites {
                if weather.place.as_ref() != Some(&favorite) {
                    places.push(Item::Command(
                        favorite.name.clone(),
                        false,
                        Action::Weather(favorite),
                    ));
                }
            }
            if !places.is_empty() {
                items.push(Item::Submenu("Place".into(), places));
            }
            items.push(Item::Command("Search place…".into(), false, Action::Search));
        }
        items.push(Item::Separator);
        items.push(Item::Command(
            "Quit all widgets".into(),
            false,
            Action::Quit,
        ));
        match crate::popup::show(items) {
            Ok(Some(Action::Place(p))) => self.with_state(|s| s.placement = p),
            Ok(Some(Action::Hide)) => self.with_state(|s| s.visible = false),
            Ok(Some(Action::Weather(place))) => {
                crate::widgets::weather::set_place(&self.cfg, place)
            }
            Ok(Some(Action::Search)) => crate::dashboard::open(crate::dashboard::Page::Weather),
            Ok(Some(Action::Settings)) => crate::dashboard::open(match self.kind {
                Kind::Weather => crate::dashboard::Page::Weather,
                Kind::Calendar => crate::dashboard::Page::Calendar,
                Kind::Mail => crate::dashboard::Page::Mail,
                Kind::Usage(_) => crate::dashboard::Page::Usage,
            }),
            Ok(Some(Action::Quit)) => crate::quit_all(),
            Ok(None) => {}
            Err(e) => crate::log::line("menu", &e.to_string()),
        }
    }
}

/// Show and hide the pill from a thread of its own. A hidden window gets no
/// paint messages, so `ui` stops running and could never show it again.
/// Also reloads the config when another process (or the user) changes it.
///
/// `alive` is cleared when the window is replaced or the pill is dropped; a
/// pill drawn in a shared process (`in_process`) then just stops this thread.
fn spawn_upkeep(
    kind: Kind,
    hwnd: HWND,
    cfg: SharedConfig,
    ctx: egui::Context,
    in_process: bool,
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let hwnd = hwnd.0 as isize;
    alive.store(true, std::sync::atomic::Ordering::SeqCst);
    std::thread::spawn(move || {
        let hwnd = HWND(hwnd as *mut _);
        let mut last_mtime = crate::config::mtime();
        let mut shown: Option<bool> = None;
        loop {
            if !alive.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            // Destroying Explorer's taskbar can destroy its cross-process
            // children without an orderly eframe close. A pill process exits
            // so the hub can recreate it; never keep polling a stale HWND.
            if !unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd)) }.as_bool() {
                if in_process {
                    return;
                }
                std::process::exit(0);
            }
            let mtime = crate::config::mtime();
            if mtime != last_mtime {
                last_mtime = mtime;
                crate::config::refresh(&cfg);
                ctx.request_repaint();
            }
            let state = kind.state(&cfg.lock().unwrap()).clone();
            let magnetic = cfg.lock().unwrap().magnetized;
            let dragging = DRAGGING[kind.index()].load(std::sync::atomic::Ordering::SeqCst);
            // Finding the bar enumerates every top-level window; skip it
            // while the pill is already part of a taskbar.
            if upkeep_docks(magnetic, state.placement, dragging) && !shell::on_a_taskbar(hwnd) {
                if let Some(bar) = shell::bar_for(state.monitor.as_deref()) {
                    if shell::parent_of(hwnd) != Some(bar.hwnd) {
                        shell::embed(hwnd, bar.hwnd);
                    }
                }
            }
            let want = state.visible
                && !(magnetic
                    && state.placement == Placement::Taskbar
                    && !crate::magnet::ready(kind));
            if shown != Some(want) {
                shown = Some(want);
                shell::set_visible(hwnd, want);
                ctx.request_repaint();
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}

impl eframe::App for WidgetApp {
    /// Nothing behind the pill's own drawing: the window is see-through.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        match self.hwnd.or_else(|| shell::hwnd_of(frame)) {
            Some(hwnd) => self.frame(ui, Some(hwnd)),
            None => ui.ctx().request_repaint(),
        }
    }
}

impl WidgetApp {
    /// One frame of the pill. `window` is the pill's window; `None` draws
    /// the pill off screen without touching any window (`--cpu-shot`).
    pub fn frame(&mut self, ui: &mut egui::Ui, window: Option<HWND>) {
        let ctx = ui.ctx().clone();
        let hwnd = match (self.hwnd, window) {
            (Some(hwnd), _) => Some(hwnd),
            (None, Some(hwnd)) => {
                self.hwnd = Some(hwnd);
                // winit's style rewrites need undoing; a window of our own
                // (single.rs) has none, and the hook's one global host would
                // misfire with several pills in the process.
                if !self.in_process {
                    shell::hook_window(hwnd);
                }
                shell::make_popup(hwnd);
                let state = self.state();
                if state.placement != Placement::Taskbar {
                    shell::place(hwnd, state.x, state.y);
                } else if self.cfg.lock().unwrap().magnetized {
                    if let Some(bar) = shell::bar_for(state.monitor.as_deref()) {
                        shell::embed(hwnd, bar.hwnd);
                    }
                }
                spawn_upkeep(
                    self.kind,
                    hwnd,
                    self.cfg.clone(),
                    ctx.clone(),
                    self.in_process,
                    self.window_alive.clone(),
                );
                Some(hwnd)
            }
            (None, None) => None,
        };
        if ui.visuals().widgets.noninteractive.fg_stroke.color != TEXT {
            ctx.all_styles_mut(|style| {
                style.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
                style.visuals.weak_text_color = Some(TEXT_WEAK);
            });
        }
        self.widget.tick();
        let ppp = ctx.pixels_per_point();
        let state = self.state();
        let themed = state.theme.as_ref().is_some_and(|t| t.validate().is_ok())
            || matches!(self.kind, Kind::Usage(_));
        if themed != self.custom_fonts {
            if themed {
                crate::dashboard_style::install_fonts(&ctx);
            } else {
                ctx.set_fonts(egui::FontDefinitions::default());
            }
            self.custom_fonts = themed;
            ctx.request_repaint();
            return;
        }
        let moving = self.drag.as_ref().is_some_and(|d| d.moving);
        if let Some(hwnd) = hwnd {
            self.manage_window(hwnd, &state, ppp, moving);
        }

        // On a taskbar the pill is square and painted the bar's own color so
        // it reads as part of the bar; anywhere else it is a rounded card.
        let on_bar = match &self.drag {
            Some(d) if d.moving => d.snapped.is_some(),
            _ => state.placement == Placement::Taskbar,
        };
        if let Some(hwnd) = hwnd.filter(|_| self.applied_round != Some(!on_bar)) {
            self.applied_round = Some(!on_bar);
            shell::round_corners(hwnd, !on_bar);
        }
        // On the taskbar the pill has no background at all: the bar shows
        // through. Anywhere else it is an opaque card.
        let fill = if on_bar {
            egui::Color32::TRANSPARENT
        } else {
            BG
        };

        if self.drag.is_some() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(IDLE_REPAINT);
        }

        // Side padding of a floating card. On the bar every pill uses the
        // compact inset so neighbours sit close; taskbar width is scarce.
        let card_inset: i8 = if matches!(self.kind, Kind::Usage(_)) {
            4
        } else {
            10
        };
        let inset: i8 = if on_bar { 4 } else { card_inset };
        // DWM rounds a floating eframe window's corners; a layered window
        // (single.rs) gets no frame from DWM, so its card is drawn rounded.
        let corners: u8 = if self.in_process && !on_bar { 8 } else { 0 };
        let frame_resp = egui::Frame::NONE
            .fill(fill)
            .corner_radius(corners)
            .inner_margin(egui::Margin::symmetric(inset, 0))
            .show(ui, |ui| {
                // Fixed width and full pill height keep every pill the same size.
                // Built-in content keeps the card's content width, so a docked
                // pill is narrower by the padding it drops. A custom theme's
                // width is the whole pill and stays exact.
                let custom = state
                    .theme
                    .as_ref()
                    .filter(|theme| theme.validate().is_ok());
                ui.set_min_width(match custom {
                    Some(t) => t.width - (2 * inset) as f32,
                    None => self.widget.size().0 - (2 * card_inset) as f32,
                });
                let height = custom.map(|t| t.height).unwrap_or(PILL_HEIGHT);
                ui.set_min_height(height);
                ui.set_max_height(height);
                // Center a fixed-height content row on the pill midline by
                // explicit rect. Do not replace this with a bare
                // `ui.horizontal` plus a taller `interact_size`: that value
                // leaks into every nested row and pushes content out of the
                // pill, and egui's cross-align alone shifts content down.
                let full = ui.max_rect();
                let row = egui::Rect::from_min_size(
                    egui::pos2(
                        full.left(),
                        full.top() + (full.height() - CONTENT_HEIGHT) / 2.0,
                    ),
                    egui::vec2(full.width(), CONTENT_HEIGHT),
                );
                if let Some(theme) = custom {
                    let mut values = self.widget.values();
                    values.insert("title".into(), self.kind.label().into());
                    theme.paint(
                        ui,
                        egui::pos2(full.left() - inset as f32, full.top()),
                        1.0,
                        &values,
                        &mut self.icons,
                    );
                } else {
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(row)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.spacing_mut().interact_size.y = CONTENT_HEIGHT;
                            self.widget.ui(ui, &mut self.icons);
                        },
                    );
                }
                let rect = ui.max_rect();
                ui.interact(
                    rect,
                    ui.scope_id().with("body"),
                    egui::Sense::click_and_drag(),
                )
            });
        let resp = frame_resp.inner;
        // Keep the window exactly the painted pill size; leftover client area
        // would show the wgpu clear color as a black band. Height is fixed so
        // all three widgets line up on the bar.
        let mut desired = frame_resp.response.rect.size();
        desired.y = state
            .theme
            .as_ref()
            .filter(|t| t.validate().is_ok())
            .map(|t| t.height)
            .unwrap_or(PILL_HEIGHT);
        if (desired - self.last_size).length() > 0.5 {
            self.last_size = desired;
            self.last_dock_check = None;
            // winit resets window styles when it resizes; apply ours again.
            self.applied_placement = None;
            self.applied_round = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired));
        }

        let pressed = resp.is_pointer_button_down_on()
            && ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        if let Some(hwnd) = hwnd {
            self.drag_frame(hwnd, pressed);
        }

        if resp.double_clicked() && !self.widget.double_click() {
            shell::open_url(&self.widget.url());
        }
        #[cfg(debug_assertions)]
        {
            self.preview_frame += 1;
            crate::preview::capture(&ctx, self.preview_frame);
        }
        if resp.secondary_clicked() {
            self.show_menu();
            // Apply what the menu chose now, not at the next idle repaint.
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn a_floating_pill_near_or_on_the_bar_sits_flush_against_it() {
        let bar = rect(0, 1392, 3440, 1440);
        // Overlapping a bottom bar, or just above it: rest on top of it.
        assert_eq!(
            clear_of_bar(rect(100, 1380, 272, 1428), bar, false),
            Some(1344)
        );
        assert_eq!(
            clear_of_bar(rect(100, 1335, 272, 1383), bar, false),
            Some(1344)
        );
        // Already flush, or far enough away: leave it.
        assert_eq!(clear_of_bar(rect(100, 1344, 272, 1392), bar, false), None);
        assert_eq!(clear_of_bar(rect(100, 1300, 272, 1348), bar, false), None);
        // Beside the bar horizontally: not its business.
        assert_eq!(clear_of_bar(rect(3500, 1380, 3672, 1428), bar, false), None);
        // A top bar pushes the pill below it.
        let top = rect(0, 0, 3440, 48);
        assert_eq!(clear_of_bar(rect(100, 20, 272, 68), top, true), Some(48));
        assert_eq!(clear_of_bar(rect(100, 60, 272, 108), top, true), Some(48));
        assert_eq!(clear_of_bar(rect(100, 80, 272, 128), top, true), None);
    }

    #[test]
    fn upkeep_never_redocks_a_pill_under_a_drag() {
        assert!(upkeep_docks(true, Placement::Taskbar, false));
        assert!(!upkeep_docks(true, Placement::Taskbar, true));
        assert!(!upkeep_docks(true, Placement::Floating, false));
        assert!(!upkeep_docks(true, Placement::Desktop, false));
        assert!(!upkeep_docks(false, Placement::Taskbar, false));
    }
}
