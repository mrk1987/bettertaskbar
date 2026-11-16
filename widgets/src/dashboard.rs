use crate::{
    app::Kind,
    config::{self, Config, SharedConfig},
    dashboard_style as style,
    i18n::{tr, trf},
    place_search::PlaceSearch,
    studio::Studio,
};
use eframe::egui;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc::{self, Receiver},
};
use std::time::Duration;
use windows::{
    core::w,
    Win32::{
        Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM},
        System::Threading::CreateMutexW,
        UI::{
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
};

pub const TITLE: &str = "BetterTaskbar Dashboard";
const PAGE_MESSAGE: u32 = WM_APP + 42;
static REQUESTED: AtomicUsize = AtomicUsize::new(usize::MAX);
const ACTION_MESSAGE: u32 = PAGE_MESSAGE + 1;
static REQUESTED_ACTION: AtomicUsize = AtomicUsize::new(usize::MAX);
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    General,
    Widgets,
    Weather,
    Calendar,
    Mail,
    NowPlaying,
    Departures,
    News,
    Usage,
    Studio,
    About,
}
impl Page {
    /// The settings page of a widget.
    pub fn of(kind: Kind) -> Self {
        match kind {
            Kind::Weather => Self::Weather,
            Kind::Calendar => Self::Calendar,
            Kind::Mail => Self::Mail,
            Kind::NowPlaying => Self::NowPlaying,
            Kind::Departures(_) => Self::Departures,
            Kind::News => Self::News,
            Kind::Usage(_) => Self::Usage,
        }
    }
    /// Pages with a Save button for their own settings.
    fn has_settings(self) -> bool {
        matches!(
            self,
            Self::Weather
                | Self::Calendar
                | Self::Mail
                | Self::NowPlaying
                | Self::Departures
                | Self::News
        )
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Widgets => "widgets",
            Self::Weather => "weather",
            Self::Calendar => "calendar",
            Self::Mail => "mail",
            Self::NowPlaying => "now-playing",
            Self::Departures => "departures",
            Self::News => "news",
            Self::Usage => "usage",
            Self::Studio => "studio",
            Self::About => "about",
        }
    }
    pub fn parse(key: &str) -> Self {
        match key {
            "general" => Self::General,
            "weather" => Self::Weather,
            "calendar" => Self::Calendar,
            "mail" => Self::Mail,
            "now-playing" => Self::NowPlaying,
            "departures" => Self::Departures,
            "news" => Self::News,
            "usage" => Self::Usage,
            "studio" => Self::Studio,
            "about" => Self::About,
            _ => Self::Widgets,
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::General => tr("General"),
            Self::Widgets => tr("Widgets"),
            Self::Weather => tr("Weather"),
            Self::Calendar => tr("Calendar"),
            Self::Mail => tr("Mail"),
            Self::NowPlaying => tr("Now Playing"),
            Self::Departures => tr("Next Departure"),
            Self::News => tr("News"),
            Self::Usage => tr("Usage providers"),
            Self::Studio => tr("Theme Studio"),
            Self::About => tr("About"),
        }
    }
    const ALL: [Self; 11] = [
        Self::General,
        Self::Widgets,
        Self::Weather,
        Self::Calendar,
        Self::Mail,
        Self::NowPlaying,
        Self::Departures,
        Self::News,
        Self::Usage,
        Self::Studio,
        Self::About,
    ];
}

pub fn open(page: Page) {
    open_request(page as usize);
}

fn settings_request(kind: Kind) -> usize {
    Page::ALL.len() + kind.index()
}

fn decode_page_request(code: usize) -> Option<(Page, Option<Kind>)> {
    if let Some(&page) = Page::ALL.get(code) {
        return Some((page, None));
    }
    let &kind = Kind::ALL.get(code.checked_sub(Page::ALL.len())?)?;
    Some((Page::of(kind), Some(kind)))
}

pub fn open_settings(kind: Kind) {
    open_request(settings_request(kind));
}

fn startup_page_request() -> Option<usize> {
    std::env::args().find_map(|a| {
        a.strip_prefix("--settings-request=")
            .and_then(|s| s.parse().ok())
    })
}

fn open_request(code: usize) {
    let Some((page, _)) = decode_page_request(code) else {
        return;
    };
    if let Some(hwnd) = crate::shell::find_by_title(TITLE) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), PAGE_MESSAGE, WPARAM(code), LPARAM(0));
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    if let Err(e) = std::env::current_exe().and_then(|exe| {
        std::process::Command::new(exe)
            .arg(format!("--dashboard={}", page.key()))
            .arg(format!("--settings-request={code}"))
            .spawn()
    }) {
        crate::log::line("dashboard", &format!("Cannot open dashboard: {e}"));
    }
}

/// Codes for `open_action`: one slot per widget, each with room for
/// "no preselection" plus every action.
const ACTION_SLOTS: usize = 8;
const _: () = assert!(crate::click_action::Action::ALL.len() < ACTION_SLOTS);

/// Open the selected widget's editor, optionally preselecting an unsaved mode.
pub fn open_action(kind: Kind, action: Option<crate::click_action::Action>) {
    let mode = action
        .and_then(|a| {
            crate::click_action::Action::ALL
                .iter()
                .position(|v| *v == a)
        })
        .map_or(0, |i| i + 1);
    let code = kind.index() * ACTION_SLOTS + mode;
    if let Some(hwnd) = crate::shell::find_by_title(TITLE) {
        open(Page::Widgets);
        unsafe {
            let _ = PostMessageW(Some(hwnd), ACTION_MESSAGE, WPARAM(code), LPARAM(0));
        }
    } else if let Err(e) = std::env::current_exe().and_then(|exe| {
        std::process::Command::new(exe)
            .arg("--dashboard=widgets")
            .arg(format!("--edit-action={code}"))
            .spawn()
    }) {
        crate::log::line("dashboard", &format!("Cannot open action settings: {e}"));
    }
}

struct Dashboard {
    cfg: SharedConfig,
    base: Config,
    draft: Config,
    page: Page,
    search: PlaceSearch,
    stop_search: crate::place_search::StopSearch,
    new_feed: crate::config::FeedSource,
    /// The Lines box as typed. Kept apart from the parsed list, so a comma
    /// just typed is not dropped before the next number follows it.
    lines_text: String,
    dep_slot: u8,
    notice: Option<(bool, String)>,
    test: Option<Receiver<Result<String, String>>>,
    test_configuration: Option<(bool, Config)>,
    studio: Studio,
    #[cfg(debug_assertions)]
    preview_frame: usize,
    hooked: bool,
    close_prompt: bool,
    last_refresh: std::time::Instant,
    action_focus: Option<Kind>,
}
impl Dashboard {
    fn new(ctx: &egui::Context, page: Page) -> Self {
        style::configure(ctx);
        let cfg = config::shared();
        let base = cfg.lock().unwrap().clone();
        let studio = Studio::new(Kind::Weather, &base);
        let mut dashboard = Self {
            cfg,
            base: base.clone(),
            draft: base,
            page,
            search: PlaceSearch::default(),
            stop_search: Default::default(),
            new_feed: Default::default(),
            lines_text: String::new(),
            dep_slot: 0,
            notice: None,
            test: None,
            test_configuration: None,
            studio,
            #[cfg(debug_assertions)]
            preview_frame: 0,
            hooked: false,
            close_prompt: false,
            last_refresh: std::time::Instant::now(),
            action_focus: None,
        };
        if let Some(code) = startup_page_request() {
            dashboard.focus_page(code);
        }
        if let Some(code) = std::env::args().find_map(|a| {
            a.strip_prefix("--edit-action=")
                .and_then(|s| s.parse().ok())
        }) {
            dashboard.focus_action(code);
        }
        dashboard
    }
    fn focus_page(&mut self, code: usize) {
        if let Some((page, kind)) = decode_page_request(code) {
            self.page = page;
            if let Some(Kind::Departures(slot)) = kind {
                self.select_departure_slot(slot);
            }
        }
    }
    fn focus_action(&mut self, code: usize) {
        if let Some(&kind) = Kind::ALL
            .get(code / ACTION_SLOTS)
            .filter(|k| k.exists(&self.draft))
        {
            self.page = Page::Widgets;
            if let Some(&action) = (code % ACTION_SLOTS)
                .checked_sub(1)
                .and_then(|i| crate::click_action::Action::ALL.get(i))
            {
                kind.state_mut(&mut self.draft).double_click.action = Some(action);
            }
            self.action_focus = Some(kind);
        }
    }
    fn dirty(&self) -> bool {
        self.base.weather != self.draft.weather
            || self.base.calendar != self.draft.calendar
            || self.base.mail != self.draft.mail
            || self.base.departures != self.draft.departures
            || self.base.departure_slots != self.draft.departure_slots
            || self.base.news != self.draft.news
            || Kind::ALL
                .into_iter()
                .any(|k| k.state(&self.base).double_click != k.state(&self.draft).double_click)
            || self.studio.dirty()
    }
    fn save(&mut self, all: bool) -> bool {
        let draft = settings_for_save(&self.base, &self.draft, self.page, all);
        if let Err(error) = validate(&draft) {
            self.notice = Some((false, error));
            return false;
        }
        let base = self.base.clone();
        match config::try_update_checked(&self.cfg, |latest| {
            merge_settings(latest, &base, &draft);
            validate(latest)
        }) {
            Ok(()) => {
                self.base.weather = draft.weather.clone();
                self.base.calendar = draft.calendar.clone();
                self.base.mail = draft.mail.clone();
                self.base.departures = draft.departures.clone();
                self.base.departure_slots = draft.departure_slots.clone();
                self.base.news = draft.news.clone();
                for kind in crate::app::user_facing_kinds(&draft) {
                    if !kind.exists(&self.base) {
                        continue;
                    }
                    kind.state_mut(&mut self.base).double_click =
                        kind.state(&draft).double_click.clone();
                }
                self.notice = Some((
                    true,
                    tr("Settings saved. Widgets pick up changes automatically.").into(),
                ));
                true
            }
            Err(error) => {
                self.notice = Some((false, error));
                false
            }
        }
    }
    fn refresh(&mut self) {
        if self.last_refresh.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_refresh = std::time::Instant::now();
        config::refresh(&self.cfg);
        let fresh = self.cfg.lock().unwrap().clone();
        macro_rules! sync {
            ($field:ident) => {
                if self.draft.$field == self.base.$field {
                    self.draft.$field = fresh.$field.clone();
                    self.base.$field = fresh.$field.clone();
                }
            };
        }
        sync!(weather);
        sync!(calendar);
        sync!(mail);
        sync!(departures);
        sync!(departure_slots);
        if !Kind::Departures(self.dep_slot).exists(&self.draft) {
            self.select_departure_slot(0);
        }
        sync!(news);
        for kind in crate::app::user_facing_kinds(&fresh) {
            if !kind.exists(&self.draft) || !kind.exists(&self.base) {
                continue;
            }
            kind.state_mut(&mut self.draft).double_click.refresh_from(
                &mut kind.state_mut(&mut self.base).double_click,
                &kind.state(&fresh).double_click,
            );
        }
    }

    fn save_action(&mut self, kind: Kind) {
        if !kind.exists(&self.draft) || !kind.exists(&self.base) {
            self.notice = Some((false, tr("This pill no longer exists.").into()));
            return;
        }
        let draft = kind.state(&self.draft).double_click.clone();
        let base = kind.state(&self.base).double_click.clone();
        let result = config::try_update_checked(&self.cfg, |latest| {
            if !kind.exists(latest) {
                return Err(tr("This pill no longer exists.").into());
            }
            draft.merge_into(&base, &mut kind.state_mut(latest).double_click);
            kind.state(latest)
                .double_click
                .resolve(kind, latest)
                .map(|_| ())
        });
        match result {
            Ok(()) => {
                let saved = kind.state(&self.cfg.lock().unwrap()).double_click.clone();
                kind.state_mut(&mut self.base).double_click = saved.clone();
                kind.state_mut(&mut self.draft).double_click = saved;
                self.notice = Some((
                    true,
                    trf(
                        "{widget} double-click action saved.",
                        &[("widget", &kind.label())],
                    ),
                ));
            }
            Err(error) => self.notice = Some((false, error)),
        }
    }

    fn discard_page(&mut self) {
        self.draft = settings_for_save(&self.draft, &self.base, self.page, false);
        self.notice = None;
    }

    fn action_settings(&mut self, ui: &mut egui::Ui, kind: Kind) {
        use crate::click_action::{Action, Target};
        if !kind.exists(&self.draft) {
            return;
        }
        ui.push_id(("double-click", kind.key()), |ui| {
            let default_url = kind.default_url(&self.draft);
            let click = &mut kind.state_mut(&mut self.draft).double_click;
            let mut action = click.action(kind);
            ui.horizontal(|ui| {
                let label = ui.label(tr("On double-click"));
                egui::ComboBox::from_id_salt("mode").selected_text(action.label_for(kind)).show_ui(ui, |ui| {
                    for a in Action::for_kind(kind) { ui.selectable_value(&mut action, a, a.label_for(kind)); }
                }).response.labelled_by(label.id);
            });
            if action != click.action(kind) { click.action = Some(action); }
            match action {
                Action::OpenUrl => {
                    let label = ui.label("URL");
                    ui.add(egui::TextEdit::singleline(&mut click.url).hint_text(&default_url).desired_width(ui.available_width())).labelled_by(label.id);
                    ui.label(egui::RichText::new(trf("Blank uses {url}", &[("url", &default_url)])).color(style::MUTED));
                }
                Action::Refresh => { ui.label(egui::RichText::new(tr("Fetches this widget’s data again. Repeated requests wait for the 30-second cooldown.")).color(style::MUTED)); }
                Action::OpenItem => {
                    let text = match kind {
                        Kind::News => tr("Opens the article for the headline on screen. The headline holds still while you click."),
                        Kind::NowPlaying => tr("Brings up the app that is playing."),
                        _ => tr("Opens what the widget shows."),
                    };
                    ui.label(egui::RichText::new(text).color(style::MUTED));
                }
                Action::LaunchApp => {
                    let label = ui.label(tr("Application (.exe)"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut click.app).hint_text("C:\\Program Files\\App\\App.exe").desired_width((ui.available_width() - 100.0).max(120.0))).labelled_by(label.id);
                        if ui.button(tr("Browse…")).clicked() {
                            if let Some(path) = crate::click_action::choose_app() { click.app = path; }
                        }
                    });
                }
            }
            let valid = kind.state(&self.draft).double_click.resolve(kind, &self.draft);
            if let Err(error) = &valid { ui.colored_label(style::DANGER, error); }
            else if let Ok(Target::App(path)) = &valid {
                if !path.is_file() { ui.colored_label(style::MUTED, tr("This app is not currently at that path.")); }
            }
            let dirty = kind.state(&self.draft).double_click != kind.state(&self.base).double_click;
            ui.horizontal(|ui| {
                if ui.add_enabled(dirty && valid.is_ok() && kind.exists(&self.base), egui::Button::new(tr("Apply action")).fill(style::ACCENT)).clicked() { self.save_action(kind); }
                if ui.add_enabled(dirty, egui::Button::new(tr("Discard action edits"))).clicked() {
                    kind.state_mut(&mut self.draft).double_click = kind.state(&self.base).double_click.clone();
                }
            });
        });
    }
    fn widgets(&mut self, ui: &mut egui::Ui, usage_only: bool) {
        style::heading(
            ui,
            if usage_only {
                tr("Usage providers")
            } else {
                tr("Widgets")
            },
            if usage_only {
                tr("Usage widgets use the sign-in from each provider’s app or CLI.")
            } else {
                tr("Choose what lives on your taskbar. Each widget has its own settings and theme.")
            },
        );
        let current = self.cfg.lock().unwrap().clone();
        if usage_only {
            style::group(ui, |ui| {
                ui.label(egui::RichText::new(tr("Refresh")).size(16.0).strong());
                ui.horizontal(|ui| {
                    for minutes in config::USAGE_REFRESH_CHOICES {
                        let chosen = current.usage_refresh().as_secs() == minutes * 60;
                        if ui
                            .selectable_label(
                                chosen,
                                trf("Every {minutes} min", &[("minutes", &minutes)]),
                            )
                            .clicked()
                            && !chosen
                        {
                            if let Err(error) =
                                config::try_update(&self.cfg, |c| c.usage_refresh_minutes = minutes)
                            {
                                self.notice = Some((false, error));
                            }
                        }
                    }
                });
                ui.colored_label(
                    style::MUTED,
                    tr("How often each enabled provider is asked for its usage. Less often means fewer requests to the provider."),
                );
            });
            ui.add_space(12.0);
        }
        for kind in crate::app::user_facing_kinds(&current) {
            if usage_only && !matches!(kind, Kind::Usage(_)) {
                continue;
            }
            style::group(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(kind.display_name(&current))
                                .size(16.0)
                                .strong(),
                        );
                        ui.label(egui::RichText::new(description(kind)).color(style::MUTED));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut enabled = kind.state(&current).enabled;
                        if style::toggle(ui, &mut enabled).changed() {
                            let result = config::try_update(&self.cfg, |c| {
                                if !kind.exists(c) {
                                    return;
                                }
                                let state = kind.state_mut(c);
                                state.enabled = enabled;
                                state.visible = enabled;
                            });
                            if let Err(e) = result {
                                self.notice = Some((false, e));
                            }
                        }
                    });
                });
                ui.horizontal(|ui| {
                    // On the Usage page the providers' settings are this page:
                    // a Settings button would lead back to itself.
                    if (Page::of(kind) != Page::Usage || !usage_only)
                        && ui.button(tr("Settings")).clicked()
                    {
                        self.page = Page::of(kind);
                        if let Kind::Departures(slot) = kind {
                            self.select_departure_slot(slot);
                        }
                    }
                    if ui.button(tr("Edit theme")).clicked() {
                        self.studio.select(kind, &current);
                        self.page = Page::Studio;
                    }
                    let mut placement = kind.state(&current).placement;
                    egui::ComboBox::from_id_salt(("widget-placement", kind.key()))
                        .selected_text(match placement {
                            config::Placement::Taskbar => tr("Taskbar"),
                            config::Placement::Desktop => tr("Desktop"),
                            config::Placement::Floating => tr("Floating"),
                        })
                        .show_ui(ui, |ui| {
                            for (p, label) in [
                                (config::Placement::Taskbar, tr("Taskbar")),
                                (config::Placement::Desktop, tr("Desktop")),
                                (config::Placement::Floating, tr("Floating")),
                            ] {
                                ui.selectable_value(&mut placement, p, label);
                            }
                        });
                    if placement != kind.state(&current).placement {
                        if let Err(error) = config::try_update(&self.cfg, |c| {
                            if kind.exists(c) {
                                kind.state_mut(c).placement = placement;
                            }
                        }) {
                            self.notice = Some((false, error));
                        }
                    }
                    if kind.state(&current).enabled
                        && !kind.state(&current).visible
                        && ui.button(tr("Show widget")).clicked()
                    {
                        if let Err(e) = config::try_update(&self.cfg, |c| {
                            if kind.exists(c) {
                                kind.state_mut(c).visible = true;
                            }
                        }) {
                            self.notice = Some((false, e));
                        }
                    }
                });
                if current.magnetized
                    && kind.state(&current).placement == config::Placement::Taskbar
                {
                    if let Some(message) = crate::magnet::message(kind) {
                        ui.colored_label(style::MUTED, message);
                    }
                }
                if let Kind::Usage(provider) = kind {
                    ui.label(egui::RichText::new(setup(provider)).color(style::MUTED));
                    if kind.state(&current).enabled {
                        ui.colored_label(style::MUTED, crate::widgets::usage::status(provider));
                    }
                }
                let label = trf(
                    "Double-click: {action}",
                    &[(
                        "action",
                        &kind.state(&current).double_click.action(kind).label(),
                    )],
                );
                let focus = self.action_focus == Some(kind);
                let response = egui::CollapsingHeader::new(label)
                    .id_salt(("action-editor", kind.key()))
                    .open(focus.then_some(true))
                    .show(ui, |ui| self.action_settings(ui, kind));
                if focus {
                    response
                        .header_response
                        .scroll_to_me(Some(egui::Align::Center));
                    self.action_focus = None;
                }
            });
            ui.add_space(8.0);
        }
    }
    fn general(&mut self, ui: &mut egui::Ui) {
        style::heading(
            ui,
            tr("General"),
            tr("Settings that apply to every widget and to this window."),
        );
        let current = self.cfg.lock().unwrap().clone();
        style::group(ui, |ui| {
            ui.label(egui::RichText::new(tr("Language")).size(16.0).strong());
            let mut language = current.language;
            ui.horizontal(|ui| {
                for choice in crate::i18n::Language::ALL {
                    ui.selectable_value(&mut language, choice, choice.label());
                }
            });
            if language != current.language {
                if let Err(error) = config::try_update(&self.cfg, |c| c.language = language) {
                    self.notice = Some((false, error));
                }
            }
            ui.colored_label(
                style::MUTED,
                tr("The dashboard, the tray menu and the text on the widgets. \
                    Same as Windows shows Norwegian when Windows is set to Norwegian."),
            );
        });
        ui.add_space(12.0);
        style::group(ui, |ui| {
            ui.label(
                egui::RichText::new(tr("Taskbar placement"))
                    .size(16.0)
                    .strong(),
            );
            let mut magnetized = current.magnetized;
            if ui
                .checkbox(&mut magnetized, tr("Magnetic taskbar placement"))
                .changed()
            {
                if let Err(error) = config::try_update(&self.cfg, |c| c.magnetized = magnetized) {
                    self.notice = Some((false, error));
                }
            }
            ui.colored_label(
                style::MUTED,
                tr(
                    "Right-side widgets pack toward the notification area and left-side \
                    widgets toward the left edge, and move along when app icons come \
                    and go. Switch it off to keep each widget exactly where you drop it.",
                ),
            );
            ui.add_space(6.0);
            ui.label(tr("Place every widget, as in the tray menu"));
            let common = crate::tray::common_placement(&current);
            ui.horizontal(|ui| {
                for (placement, label) in [
                    (config::Placement::Taskbar, tr("Pin all to taskbar")),
                    (config::Placement::Desktop, tr("Pin all to desktop")),
                    (config::Placement::Floating, tr("Float all on top")),
                ] {
                    if ui
                        .selectable_label(common == Some(placement), label)
                        .clicked()
                    {
                        if let Err(error) = config::try_update(&self.cfg, |c| {
                            for kind in crate::app::user_facing_kinds(c).collect::<Vec<_>>() {
                                if kind.exists(c) {
                                    kind.state_mut(c).placement = placement;
                                };
                            }
                        }) {
                            self.notice = Some((false, error));
                        }
                    }
                }
            });
        });
    }

    fn weather(&mut self, ui: &mut egui::Ui) {
        style::heading(
            ui,
            tr("Weather"),
            tr("Choose a place, keep favorites, or enter coordinates."),
        );
        style::group(ui, |ui| self.action_settings(ui, Kind::Weather));
        ui.add_space(12.0);
        style::group(ui, |ui| {
            if let Some(place) = &mut self.draft.weather.place {
                style::field(ui, tr("Place"), &mut place.name, false);
                ui.horizontal(|ui| {
                    ui.label(tr("Latitude"));
                    ui.add(
                        egui::DragValue::new(&mut place.lat)
                            .speed(0.001)
                            .range(-90.0..=90.0)
                            .max_decimals(4),
                    );
                    ui.label(tr("Longitude"));
                    ui.add(
                        egui::DragValue::new(&mut place.lon)
                            .speed(0.001)
                            .range(-180.0..=180.0)
                            .max_decimals(4),
                    );
                });
            } else if ui.button(tr("Set a place")).clicked() {
                self.draft.weather.place = config::WeatherCfg::default().place;
            }
            ui.horizontal(|ui| {
                ui.label(tr("Refresh interval (seconds)"));
                ui.add(
                    egui::DragValue::new(&mut self.draft.weather.refresh_secs).range(1800..=86400),
                );
            });
            if let Some(place) = &self.draft.weather.place {
                if ui.button(tr("Add current place to favorites")).clicked()
                    && !self.draft.weather.favorites.contains(place)
                {
                    self.draft.weather.favorites.push(place.clone());
                }
            }
        });
        ui.add_space(14.0);
        style::group(ui, |ui| {
            ui.label(egui::RichText::new(tr("Find a place")).size(16.0).strong());
            if let Some(place) = self.search.ui(ui) {
                if !self.draft.weather.favorites.contains(&place) {
                    self.draft.weather.favorites.push(place.clone());
                }
                self.draft.weather.place = Some(place);
            }
        });
        ui.add_space(14.0);
        style::group(ui, |ui| {
            ui.label(egui::RichText::new(tr("Favorites")).size(16.0).strong());
            let mut remove = None;
            if self.draft.weather.favorites.is_empty() {
                ui.colored_label(
                    style::MUTED,
                    tr("No favorites yet. Search for a place or add your current one."),
                );
            }
            for (i, place) in self.draft.weather.favorites.iter().enumerate() {
                ui.push_id(i, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(
                                self.draft.weather.place.as_ref() == Some(place),
                                &place.name,
                            )
                            .clicked()
                        {
                            self.draft.weather.place = Some(place.clone());
                        }
                        if ui.small_button(tr("Remove")).clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                self.draft.weather.favorites.remove(i);
            }
        });
    }
    fn connection(&mut self, ui: &mut egui::Ui, mail: bool) {
        style::heading(
            ui,
            if mail { tr("Mail") } else { tr("Calendar") },
            if mail {
                tr("Connect an IMAP server, such as Proton Mail Bridge.")
            } else {
                tr("Connect a CalDAV server, such as iOxide.")
            },
        );
        style::group(ui, |ui| {
            self.action_settings(ui, if mail { Kind::Mail } else { Kind::Calendar })
        });
        ui.add_space(12.0);
        style::group(ui, |ui| {
            if mail {
                let c = &mut self.draft.mail;
                style::field(ui, tr("Server"), &mut c.host, false);
                ui.horizontal(|ui| {
                    ui.label(tr("Port"));
                    ui.add(egui::DragValue::new(&mut c.port).range(1..=65535));
                    ui.label(tr("Connection"));
                    egui::ComboBox::from_id_salt("mail-security")
                        .selected_text(&c.security)
                        .show_ui(ui, |ui| {
                            for mode in ["starttls", "tls", "plain"] {
                                ui.selectable_value(&mut c.security, mode.into(), mode);
                            }
                        });
                });
                style::field(ui, tr("Username"), &mut c.username, false);
                style::field(ui, tr("Password"), &mut c.password, true);
                ui.horizontal(|ui| {
                    ui.label(tr("Refresh (seconds)"));
                    ui.add(egui::DragValue::new(&mut c.refresh_secs).range(30..=86400));
                    ui.label(tr("Rotate subject (seconds)"));
                    ui.add(egui::DragValue::new(&mut c.rotate_secs).range(1..=3600));
                });
            } else {
                let c = &mut self.draft.calendar;
                style::field(ui, tr("Server"), &mut c.host, false);
                ui.horizontal(|ui| {
                    ui.label(tr("Port"));
                    ui.add(egui::DragValue::new(&mut c.port).range(1..=65535));
                    ui.checkbox(&mut c.tls, tr("Use TLS (HTTPS)"));
                });
                style::field(ui, tr("Calendar path"), &mut c.path, false);
                style::field(ui, tr("Username"), &mut c.username, false);
                style::field(ui, tr("Password"), &mut c.password, true);
                ui.horizontal(|ui| {
                    ui.label(tr("Refresh interval (seconds)"));
                    ui.add(egui::DragValue::new(&mut c.refresh_secs).range(30..=86400));
                });
            }
            ui.add_space(8.0);
            ui.colored_label(
                style::MUTED,
                tr("Credentials stay in your local widgets.json file and are stored as plain text."),
            );
        });
        if ui
            .add_enabled(
                self.test.is_none(),
                egui::Button::new(tr("Test connection")),
            )
            .clicked()
        {
            let draft = self.draft.clone();
            if let Err(e) = validate(&draft) {
                self.notice = Some((false, e));
                return;
            }
            let (tx, rx) = mpsc::channel();
            self.test = Some(rx);
            self.test_configuration = Some((mail, draft.clone()));
            self.notice = Some((true, tr("Testing connection…").into()));
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let result = if mail {
                    crate::imap::fetch(&draft.mail).map(|u| {
                        trf(
                            "Connected. {count} unread messages.",
                            &[("count", &u.count)],
                        )
                    })
                } else {
                    crate::caldav::fetch(&draft.calendar).map(|f| {
                        trf(
                            "Connected. {calendars} calendars, {events} upcoming events.",
                            &[("calendars", &f.calendars), ("events", &f.events)],
                        )
                    })
                };
                let result = result.map_err(|error| {
                    let secret = if mail {
                        &draft.mail.password
                    } else {
                        &draft.calendar.password
                    };
                    if secret.is_empty() {
                        error
                    } else {
                        error.replace(secret, "[redacted]")
                    }
                });
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        }
        if self.test.is_some() {
            ui.spinner();
        }
    }
}

impl Dashboard {
    fn now_playing(&mut self, ui: &mut egui::Ui) {
        style::heading(
            ui,
            tr("Now Playing"),
            tr("Shows what Windows’ own media controls show: Spotify, browsers, Media Player and other apps that report playback."),
        );
        style::group(ui, |ui| self.action_settings(ui, Kind::NowPlaying));
        ui.add_space(12.0);
        style::group(ui, |ui| {
            ui.colored_label(
                style::MUTED,
                tr("Nothing to set up. The widget reads the current media session every two seconds, on this PC only."),
            );
        });
    }

    fn select_departure_slot(&mut self, slot: u8) {
        self.dep_slot = if Kind::Departures(slot).exists(&self.draft) {
            slot
        } else {
            0
        };
        self.lines_text = self
            .draft
            .departures_for(self.dep_slot)
            .unwrap()
            .lines
            .join(", ");
        self.stop_search = Default::default();
    }

    fn departures(&mut self, ui: &mut egui::Ui) {
        style::heading(
            ui,
            tr("Next Departure"),
            tr("Upcoming departures from a saved stop, with a traffic level for the route."),
        );
        if !Kind::Departures(self.dep_slot).exists(&self.draft) {
            self.select_departure_slot(0);
        }
        ui.horizontal_wrapped(|ui| {
            for slot in 0..config::DEPARTURE_SLOTS {
                let kind = Kind::Departures(slot);
                if kind.exists(&self.draft)
                    && ui
                        .selectable_label(self.dep_slot == slot, (slot + 1).to_string())
                        .on_hover_text(kind.display_name(&self.draft))
                        .clicked()
                {
                    self.select_departure_slot(slot);
                }
            }
            let can_add = (1..config::DEPARTURE_SLOTS)
                .any(|slot| !Kind::Departures(slot).exists(&self.draft));
            if ui
                .add_enabled(can_add, egui::Button::new("+"))
                .on_hover_text(tr("Add"))
                .clicked()
            {
                if let Some(slot) = self.draft.add_departure_slot() {
                    self.select_departure_slot(slot);
                }
            }
            if self.dep_slot != 0 && ui.button(tr("Remove this pill")).clicked() {
                self.draft.remove_departure_slot(self.dep_slot);
                self.select_departure_slot(0);
            }
        });
        ui.add_space(12.0);
        style::group(ui, |ui| {
            self.action_settings(ui, Kind::Departures(self.dep_slot))
        });
        let departures = self.draft.departures_for_mut(self.dep_slot).unwrap();
        let stop_search = &mut self.stop_search;
        let lines_text = &mut self.lines_text;
        ui.add_space(12.0);
        style::group(ui, |ui| {
            style::field(ui, tr("Name"), &mut departures.name, false);
            ui.colored_label(
                style::MUTED,
                tr("Shown in the tray and dashboard; empty uses the stop name"),
            );
        });
        ui.add_space(12.0);
        style::group(ui, |ui| {
            ui.label(egui::RichText::new(tr("Stop")).size(16.0).strong());
            match &departures.stop {
                Some(stop) => {
                    ui.label(&stop.name);
                    ui.colored_label(style::MUTED, &stop.id);
                }
                None => {
                    ui.colored_label(
                        style::MUTED,
                        tr("No stop chosen yet. Search for one below."),
                    );
                }
            }
            ui.add_space(6.0);
            if let Some(stop) = stop_search.ui(ui) {
                let favorites = &mut departures.favorites;
                if !favorites.contains(&stop) {
                    favorites.push(stop.clone());
                }
                departures.stop = Some(stop);
            }
        });
        ui.add_space(12.0);
        style::group(ui, |ui| {
            ui.label(egui::RichText::new(tr("Saved stops")).size(16.0).strong());
            ui.colored_label(
                style::MUTED,
                tr("Switch between these from the widget’s right-click menu."),
            );
            let mut remove = None;
            for (i, stop) in departures.favorites.iter().enumerate() {
                ui.push_id(("stop", i), |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(departures.stop.as_ref() == Some(stop), &stop.name)
                            .clicked()
                        {
                            departures.stop = Some(stop.clone());
                        }
                        if ui.small_button(tr("Remove")).clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                departures.favorites.remove(i);
            }
        });
        ui.add_space(12.0);
        style::group(ui, |ui| {
            ui.label(egui::RichText::new(tr("Filters")).size(16.0).strong());
            // Start again from the saved list only when it changed some
            // other way (loaded, discarded, saved elsewhere).
            if parse_lines(lines_text) != departures.lines {
                *lines_text = departures.lines.join(", ");
            }
            ui.label(tr("Lines (comma separated, blank for all)"));
            if ui
                .add(
                    egui::TextEdit::singleline(lines_text)
                        .hint_text("31, 37")
                        .desired_width(240.0),
                )
                .changed()
            {
                departures.lines = parse_lines(lines_text);
            }
            ui.label(tr("Direction (comma separated, blank for all)"));
            ui.add(
                egui::TextEdit::singleline(&mut departures.direction)
                    .hint_text("Sentrum, Helsfyr")
                    .desired_width(240.0),
            );
            ui.colored_label(
                style::MUTED,
                tr(
                    "Shows only departures whose final destination, as written on the front \
                 of the bus or train, contains one of these. Not case-sensitive, and part \
                 of a name is enough. Use it to see one direction of a line; list every \
                 end stop in that direction when the line has several.",
                ),
            );
            ui.horizontal(|ui| {
                ui.label(tr("Refresh (seconds)"));
                ui.add(egui::DragValue::new(&mut departures.refresh_secs).range(30..=3600));
            });
        });
        ui.add_space(12.0);
        style::group(ui, |ui| {
            ui.label(egui::RichText::new(tr("Traffic")).size(16.0).strong());
            ui.checkbox(
                &mut departures.road_traffic,
                tr("Read road situations along the route from Statens vegvesen"),
            );
            ui.horizontal(|ui| {
                ui.label(tr("Counts situations within (metres)"));
                ui.add(egui::DragValue::new(&mut departures.traffic_radius_m).range(50..=2000));
            });
            ui.colored_label(
                style::MUTED,
                tr("None / Low / Medium / High comes from road situations (accidents, closures, roadworks) near the stops ahead, and from Entur’s disruption messages for the line. A bus running late does not count. With no road data, buses and trams show Unknown rather than None."),
            );
        });
    }

    fn news(&mut self, ui: &mut egui::Ui) {
        style::heading(
            ui,
            tr("News"),
            tr("Rotating headlines with their source and age. Double-click opens the headline on screen."),
        );
        style::group(ui, |ui| self.action_settings(ui, Kind::News));
        ui.add_space(12.0);
        style::group(ui, |ui| {
            ui.label(egui::RichText::new(tr("Sources")).size(16.0).strong());
            ui.colored_label(
                style::MUTED,
                tr("Feeds list the newest stories first, which is not always the outlet’s own ranking."),
            );
            let mut remove = None;
            let mut swap = None;
            let count = self.draft.news.feeds.len();
            // Measured once: inside a row the available width grows with the
            // row's own content, and the buttons would be pushed off the page.
            let width = ui.available_width();
            let url_width = (width - 340.0).max(160.0);
            let new_url_width = (width - 220.0).max(160.0);
            for (i, feed) in self.draft.news.feeds.iter_mut().enumerate() {
                ui.push_id(("feed", i), |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut feed.enabled, "");
                        ui.add(egui::TextEdit::singleline(&mut feed.name).desired_width(120.0));
                        ui.add(egui::TextEdit::singleline(&mut feed.url).desired_width(url_width));
                        if ui
                            .add_enabled(i > 0, egui::Button::new("↑").small())
                            .clicked()
                        {
                            swap = Some(i - 1);
                        }
                        if ui
                            .add_enabled(i + 1 < count, egui::Button::new("↓").small())
                            .clicked()
                        {
                            swap = Some(i);
                        }
                        if ui.small_button(tr("Remove")).clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = swap {
                self.draft.news.feeds.swap(i, i + 1);
            }
            if let Some(i) = remove {
                self.draft.news.feeds.remove(i);
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new(tr("Add a source")).strong());
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_feed.name)
                        .hint_text(tr("Name"))
                        .desired_width(120.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_feed.url)
                        .hint_text(tr("https://example.com/feed.xml (RSS or Atom)"))
                        .desired_width(new_url_width),
                );
                let valid = !self.new_feed.name.trim().is_empty()
                    && crate::click_action::web_url(&self.new_feed.url).is_ok();
                if ui
                    .add_enabled(valid, egui::Button::new(tr("Add")))
                    .clicked()
                {
                    let mut feed = std::mem::take(&mut self.new_feed);
                    feed.name = feed.name.trim().to_owned();
                    feed.url = feed.url.trim().to_owned();
                    feed.enabled = true;
                    self.draft.news.feeds.push(feed);
                }
            });
            if ui.button(tr("Add missing built-in sources")).clicked() {
                for preset in config::news_presets() {
                    if !self.draft.news.feeds.iter().any(|f| f.url == preset.url) {
                        self.draft.news.feeds.push(crate::config::FeedSource {
                            enabled: false,
                            ..preset
                        });
                    }
                }
            }
        });
        ui.add_space(12.0);
        style::group(ui, |ui| {
            ui.label(egui::RichText::new(tr("Rotation")).size(16.0).strong());
            let n = &mut self.draft.news;
            ui.horizontal(|ui| {
                ui.label(tr("Next headline every (seconds)"));
                ui.add(egui::DragValue::new(&mut n.rotate_secs).range(3..=600));
            });
            ui.checkbox(
                &mut n.pause_on_hover,
                tr("Pause while the pointer is on the widget"),
            );
            ui.horizontal(|ui| {
                ui.label(tr("Refresh feeds (seconds)"));
                ui.add(egui::DragValue::new(&mut n.refresh_secs).range(300..=86400));
            });
            ui.horizontal(|ui| {
                ui.label(tr("Skip headlines older than (hours)"));
                ui.add(egui::DragValue::new(&mut n.max_age_hours).range(1..=720));
            });
            ui.colored_label(
                style::MUTED,
                tr("The same story from two sources is shown once. When feeds cannot be reached, the widget keeps the last headlines and marks them offline."),
            );
        });
    }
}

impl eframe::App for Dashboard {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if !self.hooked {
            if let Some(hwnd) = crate::shell::hwnd_of(frame) {
                hook(hwnd);
                self.hooked = true;
            }
        }
        let request = REQUESTED.swap(usize::MAX, Ordering::Relaxed);
        self.focus_page(request);
        let action = REQUESTED_ACTION.swap(usize::MAX, Ordering::Relaxed);
        if action != usize::MAX {
            self.focus_action(action);
        }
        self.refresh();
        crate::i18n::apply(self.cfg.lock().unwrap().language);
        if let Some(rx) = &self.test {
            match rx.try_recv() {
                Ok(result) => {
                    self.test = None;
                    let stale = self
                        .test_configuration
                        .take()
                        .is_some_and(|(mail, tested)| {
                            !test_settings_match(&tested, &self.draft, mail)
                        });
                    self.notice = Some(if stale {
                        (false,tr("Settings changed during the test. Test again to check the current settings.").into())
                    } else {
                        match result {
                            Ok(s) => (true, s),
                            Err(e) => (false, e),
                        }
                    });
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.test = None;
                    self.test_configuration = None;
                    self.notice = Some((false, tr("Connection check stopped. Try again.").into()));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if ui.ctx().input(|i| i.viewport().close_requested()) && self.dirty() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_prompt = true;
        }
        ui.painter().rect_filled(ui.max_rect(), 0, style::SURFACE);
        egui::Frame::new().inner_margin(egui::Margin{left:10,right:10,top:0,bottom:10}).show(ui,|ui|{
        let full_height = ui.available_height();
        ui.horizontal_top(|ui|{
            ui.allocate_ui_with_layout(egui::vec2(156.0,full_height),egui::Layout::top_down(egui::Align::Min),|ui|{
                ui.set_width(156.0);ui.add_space(18.0);ui.label(egui::RichText::new("BetterTaskbar").size(18.0).strong());ui.add_space(16.0);
                for page in Page::ALL {if style::nav(ui,self.page==page,page.title()).clicked(){self.page=page;}}
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min),|ui|{ui.label(egui::RichText::new(format!("v{}",env!("CARGO_PKG_VERSION"))).color(style::MUTED));});
            });ui.separator();
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),full_height),egui::Layout::top_down(egui::Align::Min),|ui|{
                egui::Frame::new().inner_margin(egui::Margin::symmetric(20,16)).show(ui,|ui|{
                    ui.set_width(ui.available_width());
                    if let Some((ok,message))=&self.notice{ui.colored_label(if *ok{style::MUTED}else{style::DANGER},message);ui.add_space(8.0);}
                    if self.page==Page::Studio {self.studio.ui(ui,&self.cfg);return;}
                    egui::ScrollArea::vertical().id_salt("dashboard-page").show(ui,|ui|{match self.page{
                        Page::General=>self.general(ui),Page::Widgets=>self.widgets(ui,false),Page::Usage=>self.widgets(ui,true),Page::Weather=>self.weather(ui),Page::Calendar=>self.connection(ui,false),Page::Mail=>self.connection(ui,true),Page::NowPlaying=>self.now_playing(ui),Page::Departures=>self.departures(ui),Page::News=>self.news(ui),
                        Page::About=>{style::heading(ui,"BetterTaskbar",tr("Your taskbar widgets, in one place."));style::group(ui,|ui|{ui.label(tr("Usage providers and dashboard styling are adapted from Claude Code Usage Monitor by Code Zeno Pty Ltd."));ui.hyperlink_to("Claude Code Usage Monitor (MIT)","https://github.com/CodeZeno/Claude-Code-Usage-Monitor");ui.hyperlink_to(tr("BetterTaskbar source and third-party notices"),"https://github.com/mrk1987/bettertaskbar");ui.label(tr("Weather: MET Norway / Yr. Place search: Open-Meteo / GeoNames."));ui.label(tr("Departures and stop search: Entur (NLOD). Road situations: Statens vegvesen DATEX II (NLOD)."));ui.label(tr("News headlines link to each outlet’s own site."));egui::CollapsingHeader::new(tr("CodeZeno MIT license")).show(ui,|ui|{ui.label(include_str!("../vendor/codezeno-usage-core/LICENSE"));ui.label(include_str!("../assets/lobe-icons.LICENSE"));});});},Page::Studio=>{}
                    }
                    if self.page.has_settings(){ui.add_space(16.0);ui.horizontal(|ui|{if ui.add(egui::Button::new(tr("Save changes")).fill(style::ACCENT)).clicked(){self.save(false);}
if ui.button(tr("Discard edits")).clicked(){self.discard_page();}});}
                    });
                });
            });
        });
        });
        if self.close_prompt {
            egui::Window::new(tr("Unsaved changes"))
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label(tr("Save your settings and theme before closing?"));
                    ui.horizontal(|ui| {
                        if ui.button(tr("Keep editing")).clicked() {
                            self.close_prompt = false;
                        }
                        if ui.button(tr("Discard and close")).clicked() {
                            self.base = self.draft.clone();
                            self.studio.discard();
                            self.close_prompt = false;
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button(tr("Save and close")).clicked()
                            && self.save(true)
                            && self.studio.save(&self.cfg)
                        {
                            self.close_prompt = false;
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
        }
        #[cfg(debug_assertions)]
        {
            self.preview_frame += 1;
            crate::preview::capture(ui.ctx(), self.preview_frame);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
}

fn hook(hwnd: HWND) {
    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        w: WPARAM,
        l: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == PAGE_MESSAGE {
            REQUESTED.store(w.0, Ordering::Relaxed);
            return LRESULT(0);
        }
        if msg == ACTION_MESSAGE {
            REQUESTED_ACTION.store(w.0, Ordering::Relaxed);
            return LRESULT(0);
        }
        DefSubclassProc(hwnd, msg, w, l)
    }
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(proc), 42, 0);
        let color = 0x00202020u32;
        let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            windows::Win32::Graphics::Dwm::DWMWA_CAPTION_COLOR,
            &color as *const _ as *const _,
            4,
        );
    }
}

pub fn run(page: Page) {
    let mutex = unsafe { CreateMutexW(None, false, w!("Local\\BetterTaskbarDashboard")) };
    let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let _guard = match mutex {
        Ok(handle) => unsafe { windows::core::Owned::new(handle) },
        Err(e) => {
            crate::log::line("dashboard", &e.to_string());
            return;
        }
    };
    if exists {
        if crate::shell::find_by_title(TITLE).is_some() {
            open_request(startup_page_request().unwrap_or(page as usize));
        }
        return;
    }
    if !std::env::args().any(|a| a == "--preview")
        && crate::shell::find_by_title(crate::HUB_TITLE).is_none()
    {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).spawn();
        }
    }
    std::env::set_var("WGPU_BACKEND", "dx12");
    std::env::set_var("WGPU_DX12_PRESENTATION_SYSTEM", "visual");
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([1040.0, 640.0]),
        ..Default::default()
    };
    crate::rendering::configure(&mut options);
    if let Err(e) = eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(Dashboard::new(&cc.egui_ctx, page)))),
    ) {
        crate::log::line("dashboard", &e.to_string());
    }
}

fn description(kind: Kind) -> &'static str {
    match kind {
        Kind::Weather => tr("Forecast, temperatures and precipitation"),
        Kind::Calendar => tr("What is on now and what comes next"),
        Kind::Mail => tr("Unread count and rotating message subjects"),
        Kind::NowPlaying => tr("Track, artist and playback state from Windows media"),
        Kind::Departures(_) => tr("Next bus, tram, metro, train or boat, with a traffic level"),
        Kind::News => tr("Rotating headlines from news feeds you choose"),
        Kind::Usage(_) => tr("Usage limits and reset times"),
    }
}
fn setup(p: codezeno_usage_core::providers::ProviderId) -> &'static str {
    use codezeno_usage_core::providers::ProviderId::*;
    match p{Claude=>tr("Sign in with Claude Code or the Claude desktop app."),Codex=>tr("Sign in with the Codex CLI or desktop app."),Antigravity=>tr("Sign in with the Antigravity CLI."),OpenCode=>tr("Use OPENCODE_GO_WORKSPACE_ID and OPENCODE_GO_AUTH_COOKIE, or OpenCode Go’s config.json."),Cursor=>tr("Sign in with Cursor. Its local session is detected automatically."),Grok=>tr("Run grok login to sign in with your xAI account."),Copilot=>tr("Sign in with the Copilot CLI or GitHub CLI (gh auth login).")}
}
fn validate(c: &Config) -> Result<(), String> {
    for kind in crate::app::user_facing_kinds(c) {
        kind.state(c)
            .double_click
            .resolve(kind, c)
            .map_err(|e| format!("{}: {e}", kind.label()))?;
    }
    for host in [&c.calendar.host, &c.mail.host] {
        if host.trim().is_empty() || host.contains(['\r', '\n', '/', '\\']) {
            return Err(tr("Enter a server hostname or IP address, without a URL or path.").into());
        }
    }
    if c.calendar.port == 0 || c.mail.port == 0 {
        return Err(tr("Server ports must be between 1 and 65535.").into());
    }
    if [
        &c.calendar.username,
        &c.calendar.password,
        &c.mail.username,
        &c.mail.password,
    ]
    .iter()
    .any(|s| s.contains(['\r', '\n']))
    {
        return Err(tr("Usernames and passwords cannot contain line breaks.").into());
    }
    if !["starttls", "tls", "plain", "none"]
        .contains(&c.mail.security.to_ascii_lowercase().as_str())
    {
        return Err(tr("Choose STARTTLS, TLS or plain for the mail connection.").into());
    }
    if !(1800..=86400).contains(&c.weather.refresh_secs)
        || !(30..=86400).contains(&c.calendar.refresh_secs)
        || !(30..=86400).contains(&c.mail.refresh_secs)
        || !(1..=3600).contains(&c.mail.rotate_secs)
    {
        return Err(tr("Choose a weather refresh from 1800–86400 seconds, mail/calendar refresh from 30–86400 seconds, and subject rotation from 1–3600 seconds.").into());
    }
    for slot in 0..config::DEPARTURE_SLOTS {
        let Some(departures) = c.departures_for(slot) else {
            continue;
        };
        if !(30..=3600).contains(&departures.refresh_secs)
            || !(50..=2000).contains(&departures.traffic_radius_m)
        {
            return Err(tr("Choose a departure refresh from 30–3600 seconds and a traffic radius from 50–2000 metres.").into());
        }
    }
    if !(300..=86400).contains(&c.news.refresh_secs)
        || !(3..=600).contains(&c.news.rotate_secs)
        || !(1..=720).contains(&c.news.max_age_hours)
    {
        return Err(tr("Choose a news refresh from 300–86400 seconds, rotation from 3–600 seconds and a maximum age from 1–720 hours.").into());
    }
    for feed in &c.news.feeds {
        if feed.name.trim().is_empty() || feed.name.contains(['\r', '\n']) {
            return Err(tr("Give every news source a name on one line.").into());
        }
        crate::click_action::web_url(&feed.url).map_err(|e| {
            trf(
                "News source “{name}”: {error}",
                &[("name", &feed.name), ("error", &e)],
            )
        })?;
    }
    if let Some(place) = &c.weather.place {
        if !place.lat.is_finite()
            || !place.lon.is_finite()
            || !(-90.0..=90.0).contains(&place.lat)
            || !(-180.0..=180.0).contains(&place.lon)
        {
            return Err(tr("Enter valid latitude and longitude coordinates.").into());
        }
    }
    Ok(())
}
fn test_settings_match(tested: &Config, current: &Config, mail: bool) -> bool {
    if mail {
        tested.mail == current.mail
    } else {
        tested.calendar == current.calendar
    }
}

fn settings_for_save(base: &Config, draft: &Config, page: Page, all: bool) -> Config {
    if all {
        return draft.clone();
    }
    let mut selected = base.clone();
    match page {
        Page::Weather => selected.weather = draft.weather.clone(),
        Page::Calendar => selected.calendar = draft.calendar.clone(),
        Page::Mail => selected.mail = draft.mail.clone(),
        Page::Departures => {
            selected.departures = draft.departures.clone();
            selected.departure_slots = draft.departure_slots.clone();
        }
        Page::News => selected.news = draft.news.clone(),
        _ => {}
    }
    for kind in crate::app::user_facing_kinds(draft) {
        if !kind.exists(&selected) {
            continue;
        }
        let include = match page {
            Page::Widgets => true,
            Page::Weather => kind == Kind::Weather,
            Page::Calendar => kind == Kind::Calendar,
            Page::Mail => kind == Kind::Mail,
            Page::NowPlaying => kind == Kind::NowPlaying,
            Page::Departures => matches!(kind, Kind::Departures(_)),
            Page::News => kind == Kind::News,
            Page::Usage => matches!(kind, Kind::Usage(_)),
            _ => false,
        };
        if include {
            kind.state_mut(&mut selected).double_click = kind.state(draft).double_click.clone();
        }
    }
    selected
}

/// Line numbers from the Lines box: comma separated, blanks skipped.
fn parse_lines(text: &str) -> Vec<String> {
    text.split(',')
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

fn merge_settings(latest: &mut Config, base: &Config, draft: &Config) {
    for slot in 1..config::DEPARTURE_SLOTS {
        match (
            base.departure_slots.get(&slot),
            draft.departure_slots.get(&slot),
        ) {
            (Some(_), None) => {
                latest.departure_slots.remove(&slot);
            }
            (None, Some(added)) => {
                latest.departure_slots.insert(slot, added.clone());
            }
            (Some(before), Some(edited)) => {
                if let Some(current) = latest.departure_slots.get_mut(&slot) {
                    macro_rules! merge { ($($field:ident),+) => { $(
                        if edited.departures.$field != before.departures.$field {
                            current.departures.$field = edited.departures.$field.clone();
                        }
                    )+ } }
                    merge!(
                        name,
                        stop,
                        favorites,
                        lines,
                        direction,
                        refresh_secs,
                        road_traffic,
                        traffic_radius_m
                    );
                }
            }
            _ => {}
        }
    }
    for kind in crate::app::user_facing_kinds(draft) {
        if !kind.exists(latest) {
            continue;
        }
        kind.state(draft).double_click.merge_into(
            &kind.state(base).double_click,
            &mut kind.state_mut(latest).double_click,
        );
    }
    macro_rules! fields {($group:ident,$($field:ident),+)=>{$(if draft.$group.$field!=base.$group.$field{latest.$group.$field=draft.$group.$field.clone();})+}}
    fields!(weather, place, favorites, refresh_secs);
    fields!(
        calendar,
        host,
        port,
        tls,
        username,
        password,
        path,
        refresh_secs
    );
    fields!(
        mail,
        host,
        port,
        security,
        username,
        password,
        refresh_secs,
        rotate_secs
    );
    fields!(
        departures,
        name,
        stop,
        favorites,
        lines,
        direction,
        refresh_secs,
        road_traffic,
        traffic_radius_m
    );
    fields!(
        news,
        feeds,
        refresh_secs,
        rotate_secs,
        pause_on_hover,
        max_age_hours
    );
}
#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Dashboard {
        let cfg = Config::default();
        Dashboard {
            cfg: std::sync::Arc::new(std::sync::Mutex::new(cfg.clone())),
            base: cfg.clone(),
            draft: cfg.clone(),
            page: Page::Widgets,
            search: PlaceSearch::default(),
            stop_search: Default::default(),
            new_feed: Default::default(),
            lines_text: String::new(),
            dep_slot: 0,
            notice: None,
            test: None,
            test_configuration: None,
            studio: Studio::new(Kind::Weather, &cfg),
            preview_frame: 0,
            hooked: false,
            close_prompt: false,
            last_refresh: std::time::Instant::now(),
            action_focus: None,
        }
    }

    #[test]
    fn removed_departure_slot_is_not_recreated_by_action_editor() {
        let mut dashboard = fixture();
        let slot = dashboard.base.add_departure_slot().unwrap();
        dashboard.base.departures_for_mut(slot).unwrap().name = "Saved route".into();
        dashboard.draft = dashboard.base.clone();
        let before = dashboard.base.departure_slots[&slot].clone();
        dashboard.draft.remove_departure_slot(slot);
        dashboard.action_focus = Some(Kind::Departures(slot));
        *dashboard.cfg.lock().unwrap() = dashboard.base.clone();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| dashboard.widgets(ui, false));
        output.textures_delta.clear();
        assert!(!Kind::Departures(slot).exists(&dashboard.draft));
        assert_eq!(dashboard.base.departure_slots[&slot], before);
    }

    #[test]
    fn removed_departure_slot_is_not_recreated_by_save_action() {
        // Isolate persistence from the user's real widgets.json, even on failure.
        if std::env::var_os("BTW_REMOVED_SLOT_TEST").is_none() {
            let dir = std::env::temp_dir()
                .join(format!("BetterTaskbar-removed-slot-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "dashboard::tests::removed_departure_slot_is_not_recreated_by_save_action",
                ])
                .env("APPDATA", &dir)
                .env("BTW_REMOVED_SLOT_TEST", "1")
                .status()
                .unwrap();
            std::fs::remove_dir_all(&dir).unwrap();
            assert!(status.success());
            return;
        }
        let mut dashboard = fixture();
        let slot = dashboard.base.add_departure_slot().unwrap();
        dashboard.base.departures_for_mut(slot).unwrap().name = "Saved route".into();
        dashboard.draft = dashboard.base.clone();
        *dashboard.cfg.lock().unwrap() = dashboard.base.clone();
        let before = dashboard.base.departure_slots[&slot].clone();
        dashboard.draft.remove_departure_slot(slot);
        dashboard.save_action(Kind::Departures(slot));
        assert!(!Kind::Departures(slot).exists(&dashboard.draft));
        assert_eq!(dashboard.base.departure_slots[&slot], before);
        assert_eq!(dashboard.cfg.lock().unwrap().departure_slots[&slot], before);
        assert!(!config::path().exists(), "rejected action must not save");
    }

    #[test]
    fn settings_page_request_selects_the_requested_departure_slot() {
        let mut dashboard = fixture();
        for _ in 1..config::DEPARTURE_SLOTS {
            dashboard.draft.add_departure_slot();
        }
        for slot in 0..config::DEPARTURE_SLOTS {
            dashboard.draft.departures_for_mut(slot).unwrap().lines = vec![slot.to_string()];
        }
        for kind in Kind::ALL {
            dashboard.select_departure_slot(0);
            dashboard.focus_page(settings_request(kind));
            assert!(dashboard.page == Page::of(kind));
            if let Kind::Departures(slot) = kind {
                assert_eq!(dashboard.dep_slot, slot);
                assert_eq!(dashboard.lines_text, slot.to_string());
            }
        }
    }

    #[test]
    fn departure_slot_selection_and_page_fit_with_six_slots() {
        use crate::cpu_render::{self, Canvas, Textures};
        let mut dashboard = fixture();
        for _ in 1..config::DEPARTURE_SLOTS {
            dashboard.draft.add_departure_slot();
        }
        dashboard.draft.departures_for_mut(1).unwrap().lines = vec!["31".into()];
        dashboard.draft.departures_for_mut(2).unwrap().lines = vec!["5".into()];
        dashboard.select_departure_slot(1);
        assert_eq!(dashboard.lines_text, "31");
        dashboard.lines_text.push(',');
        dashboard.select_departure_slot(2);
        assert_eq!(dashboard.lines_text, "5");
        assert_eq!(dashboard.draft.departures_for(1).unwrap().lines, ["31"]);
        assert!(dashboard.dirty());
        let ctx = egui::Context::default();
        style::configure(&ctx);
        let mut textures = Textures::default();
        for language in [
            crate::i18n::Language::English,
            crate::i18n::Language::Norwegian,
        ] {
            crate::i18n::apply(language);
            for width in [440, 820] {
                for _ in 0..3 {
                    let output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width as f32, 1400.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            egui::Frame::new()
                                .fill(style::SURFACE)
                                .inner_margin(20)
                                .show(ui, |ui| {
                                    dashboard.departures(ui);
                                    assert!(
                                        ui.min_rect().right() <= width as f32 + 1.0,
                                        "departure page overflows at {width}"
                                    );
                                });
                        },
                    );
                    textures.set(&output.textures_delta);
                    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                    assert!(!primitives.is_empty());
                    if let Some(dir) = std::env::var_os("BTW_DEPARTURE_SHOT_DIR") {
                        let mut canvas = Canvas::new(width, 1400);
                        cpu_render::paint(
                            &mut canvas,
                            &primitives,
                            &textures,
                            output.pixels_per_point,
                        );
                        let pixels: Vec<u8> = canvas.pixels.iter().flat_map(|p| *p).collect();
                        let dir = std::path::PathBuf::from(dir);
                        std::fs::create_dir_all(&dir).unwrap();
                        image::save_buffer(
                            dir.join(format!("departures-{language:?}-{width}.png")),
                            &pixels,
                            width as u32,
                            1400,
                            image::ColorType::Rgba8,
                        )
                        .unwrap();
                    }
                    textures.free(output.textures_delta);
                }
            }
        }
        crate::i18n::apply(crate::i18n::Language::English);
        dashboard.draft.remove_departure_slot(2);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| dashboard.departures(ui));
        output.textures_delta.clear();
        assert_eq!(dashboard.dep_slot, 0);
        assert_eq!(dashboard.lines_text, "");
    }

    #[test]
    fn departure_page_saves_and_discards_slots_without_other_pages() {
        let base = Config::default();
        let mut draft = base.clone();
        let slot = draft.add_departure_slot().unwrap();
        draft.departures_for_mut(slot).unwrap().name = "Metro".into();
        draft.departures_for_mut(slot).unwrap().lines = vec!["5".into()];
        Kind::Departures(slot)
            .state_mut(&mut draft)
            .double_click
            .action = Some(crate::click_action::Action::OpenUrl);
        draft.mail.username = "pending".into();
        let selected = settings_for_save(&base, &draft, Page::Departures, false);
        assert_eq!(selected.departure_slots, draft.departure_slots);
        assert_eq!(selected.mail, base.mail);
        let discarded = settings_for_save(&draft, &base, Page::Departures, false);
        assert!(discarded.departure_slots.is_empty());
        assert_eq!(discarded.mail.username, "pending");
    }

    #[test]
    fn departure_slot_merge_preserves_concurrent_positions_and_unedited_settings() {
        let mut base = Config::default();
        base.add_departure_slot();
        base.add_departure_slot();
        base.add_departure_slot();
        let mut draft = base.clone();
        draft.departures_for_mut(1).unwrap().name = "Bus".into();
        draft.departures.name = "Home".into();
        draft.remove_departure_slot(2);
        let mut latest = base.clone();
        Kind::Departures(1).state_mut(&mut latest).x = 900;
        latest.departures_for_mut(1).unwrap().direction = "External".into();
        latest.departures_for_mut(3).unwrap().lines = vec!["31".into()];
        merge_settings(&mut latest, &base, &draft);
        assert_eq!(latest.departures.name, "Home");
        assert_eq!(latest.departures_for(1).unwrap().name, "Bus");
        assert_eq!(latest.departures_for(1).unwrap().direction, "External");
        assert_eq!(Kind::Departures(1).state(&latest).x, 900);
        assert!(!Kind::Departures(2).exists(&latest));
        assert_eq!(latest.departures_for(3).unwrap().lines, ["31"]);
        // A concurrent removal is not undone by an unrelated settings save.
        latest.remove_departure_slot(1);
        merge_settings(&mut latest, &base, &draft);
        assert!(!Kind::Departures(1).exists(&latest));
    }

    #[test]
    fn every_slot_is_validated() {
        let mut c = Config::default();
        let slot = c.add_departure_slot().unwrap();
        c.departures_for_mut(slot).unwrap().refresh_secs = 5;
        assert!(validate(&c).is_err());
    }
    #[test]
    fn a_comma_typed_in_lines_survives_until_the_next_number() {
        // The Lines box frame by frame, as `departures` runs it: the reset
        // check, then one key typed, then the list parsed from the box.
        let mut saved: Vec<String> = Vec::new();
        let mut text = String::new();
        for key in "81, 87".chars() {
            if parse_lines(&text) != saved {
                text = saved.join(", ");
            }
            text.push(key);
            saved = parse_lines(&text);
        }
        assert_eq!(text, "81, 87", "every key typed stays in the box");
        assert_eq!(saved, ["81", "87"]);
        // Changed elsewhere (discarded, reloaded): the box follows.
        saved = vec!["31".into()];
        if parse_lines(&text) != saved {
            text = saved.join(", ");
        }
        assert_eq!(text, "31");
        assert_eq!(parse_lines("81,"), ["81"]);
        assert_eq!(parse_lines(" 81 , ,87 "), ["81", "87"]);
        assert!(parse_lines(" , ").is_empty());
    }

    #[test]
    fn action_editor_fits_the_settings_column_and_can_render_a_fixture() {
        use crate::{
            click_action::Action,
            cpu_render::{self, Canvas, Textures},
        };
        let mut dashboard = fixture();
        dashboard.draft.calendar_widget.double_click.action = Some(Action::Refresh);
        dashboard.draft.mail_widget.double_click.action = Some(Action::LaunchApp);
        dashboard.draft.mail_widget.double_click.app = "C:\\Windows\\System32\\notepad.exe".into();
        let ctx = egui::Context::default();
        style::configure(&ctx);
        let mut textures = Textures::default();
        for width in [820, 440] {
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width as f32, 900.0),
                    )),
                    ..Default::default()
                };
                let output = ctx.run_ui(input, |ui| {
                    egui::Frame::new()
                        .fill(style::SURFACE)
                        .inner_margin(20)
                        .show(ui, |ui| {
                            style::heading(
                                ui,
                                "Double-click actions",
                                "Choose what each widget does when double-clicked.",
                            );
                            for kind in [Kind::Weather, Kind::Calendar, Kind::Mail] {
                                style::group(ui, |ui| {
                                    ui.strong(kind.label());
                                    dashboard.action_settings(ui, kind);
                                });
                                ui.add_space(12.0);
                            }
                            assert!(
                                ui.min_rect().right() <= width as f32 + 1.0,
                                "action editor overflows at width {width}"
                            );
                        });
                });
                textures.set(&output.textures_delta);
                let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                assert!(!primitives.is_empty());
                if let Some(dir) = std::env::var_os("BTW_ACTION_SHOT_DIR") {
                    let mut canvas = Canvas::new(width, 900);
                    cpu_render::paint(&mut canvas, &primitives, &textures, output.pixels_per_point);
                    let pixels: Vec<u8> = canvas.pixels.iter().flat_map(|p| *p).collect();
                    let dir = std::path::PathBuf::from(dir);
                    std::fs::create_dir_all(&dir).unwrap();
                    image::save_buffer(
                        dir.join(format!("actions-{width}.png")),
                        &pixels,
                        width as u32,
                        900,
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                }
                textures.free(output.textures_delta);
            }
        }
    }
    #[test]
    fn changed_connection_fields_invalidate_old_test_results() {
        let tested = Config::default();
        let mut current = tested.clone();
        current.mail.port += 1;
        assert!(!test_settings_match(&tested, &current, true));
        assert!(test_settings_match(&tested, &current, false));
    }
    #[test]
    fn saving_one_page_keeps_unrelated_unsaved_edits_pending() {
        let base = Config::default();
        let mut draft = base.clone();
        draft.mail.username = "mail edit".into();
        draft.calendar.username = "calendar edit".into();
        let selected = settings_for_save(&base, &draft, Page::Mail, false);
        assert_eq!(selected.mail.username, "mail edit");
        assert_eq!(selected.calendar, base.calendar);
        assert_eq!(draft.calendar.username, "calendar edit");
        assert_eq!(
            settings_for_save(&base, &draft, Page::Mail, true)
                .calendar
                .username,
            "calendar edit"
        );
    }
    #[test]
    fn settings_save_keeps_concurrent_drags_and_unedited_fields() {
        let base = Config::default();
        let mut draft = base.clone();
        draft.mail.port = 1993;
        let mut latest = base.clone();
        latest.weather_widget.x = 200;
        latest.mail.username = "external edit".into();
        merge_settings(&mut latest, &base, &draft);
        assert_eq!(latest.mail.port, 1993);
        assert_eq!(latest.mail.username, "external edit");
        assert_eq!(latest.weather_widget.x, 200);
    }
    #[test]
    fn action_saves_and_discards_are_scoped_to_the_selected_page() {
        use crate::click_action::Action;
        let base = Config::default();
        let mut draft = base.clone();
        draft.mail_widget.double_click.action = Some(Action::Refresh);
        draft.weather_widget.double_click.url = "https://example.com/weather".into();
        let selected = settings_for_save(&base, &draft, Page::Mail, false);
        assert_eq!(
            selected.mail_widget.double_click.action,
            Some(Action::Refresh)
        );
        assert_eq!(
            selected.weather_widget.double_click,
            base.weather_widget.double_click
        );
        let discarded = settings_for_save(&draft, &base, Page::Mail, false);
        assert_eq!(
            discarded.mail_widget.double_click,
            base.mail_widget.double_click
        );
        assert_eq!(
            discarded.weather_widget.double_click,
            draft.weather_widget.double_click
        );
        let mut latest = base.clone();
        latest.mail_widget.x = 900;
        latest.mail_widget.double_click.app = "C:\\Apps\\Mail.exe".into();
        merge_settings(&mut latest, &base, &selected);
        assert_eq!(latest.mail_widget.x, 900);
        assert_eq!(latest.mail_widget.double_click.app, "C:\\Apps\\Mail.exe");
        assert_eq!(
            latest.mail_widget.double_click.action,
            Some(Action::Refresh)
        );
    }
    #[test]
    fn invalid_coordinates_and_multiline_credentials_are_rejected() {
        let mut c = Config::default();
        c.mail.password = "a\r\nb".into();
        assert!(validate(&c).is_err());
        c.mail.password.clear();
        c.weather.place.as_mut().unwrap().lat = f64::NAN;
        assert!(validate(&c).is_err());
    }
}
