use crate::{
    app::Kind,
    config::{self, Config, SharedConfig},
    dashboard_style as style,
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
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Widgets,
    Weather,
    Calendar,
    Mail,
    Usage,
    Studio,
    About,
}
impl Page {
    pub fn key(self) -> &'static str {
        match self {
            Self::Widgets => "widgets",
            Self::Weather => "weather",
            Self::Calendar => "calendar",
            Self::Mail => "mail",
            Self::Usage => "usage",
            Self::Studio => "studio",
            Self::About => "about",
        }
    }
    pub fn parse(key: &str) -> Self {
        match key {
            "weather" => Self::Weather,
            "calendar" => Self::Calendar,
            "mail" => Self::Mail,
            "usage" => Self::Usage,
            "studio" => Self::Studio,
            "about" => Self::About,
            _ => Self::Widgets,
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Widgets => "Widgets",
            Self::Weather => "Weather",
            Self::Calendar => "Calendar",
            Self::Mail => "Mail",
            Self::Usage => "Usage providers",
            Self::Studio => "Theme Studio",
            Self::About => "About",
        }
    }
    const ALL: [Self; 7] = [
        Self::Widgets,
        Self::Weather,
        Self::Calendar,
        Self::Mail,
        Self::Usage,
        Self::Studio,
        Self::About,
    ];
}

pub fn open(page: Page) {
    if let Some(hwnd) = crate::shell::find_by_title(TITLE) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), PAGE_MESSAGE, WPARAM(page as usize), LPARAM(0));
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    if let Err(e) = std::env::current_exe().and_then(|exe| {
        std::process::Command::new(exe)
            .arg(format!("--dashboard={}", page.key()))
            .spawn()
    }) {
        crate::log::line("dashboard", &format!("Cannot open dashboard: {e}"));
    }
}

struct Dashboard {
    cfg: SharedConfig,
    base: Config,
    draft: Config,
    page: Page,
    search: PlaceSearch,
    notice: Option<(bool, String)>,
    test: Option<Receiver<Result<String, String>>>,
    test_configuration: Option<(bool, Config)>,
    studio: Studio,
    #[cfg(debug_assertions)]
    preview_frame: usize,
    hooked: bool,
    close_prompt: bool,
    last_refresh: std::time::Instant,
}
impl Dashboard {
    fn new(ctx: &egui::Context, page: Page) -> Self {
        style::configure(ctx);
        let cfg = config::shared();
        let base = cfg.lock().unwrap().clone();
        let studio = Studio::new(Kind::Weather, &base);
        Self {
            cfg,
            base: base.clone(),
            draft: base,
            page,
            search: PlaceSearch::default(),
            notice: None,
            test: None,
            test_configuration: None,
            studio,
            #[cfg(debug_assertions)]
            preview_frame: 0,
            hooked: false,
            close_prompt: false,
            last_refresh: std::time::Instant::now(),
        }
    }
    fn dirty(&self) -> bool {
        self.base.weather != self.draft.weather
            || self.base.calendar != self.draft.calendar
            || self.base.mail != self.draft.mail
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
                self.base.weather = draft.weather;
                self.base.calendar = draft.calendar;
                self.base.mail = draft.mail;
                self.notice = Some((
                    true,
                    "Settings saved. Widgets pick up changes automatically.".into(),
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
                    self.base.$field = fresh.$field;
                }
            };
        }
        sync!(weather);
        sync!(calendar);
        sync!(mail);
    }
    fn widgets(&mut self, ui: &mut egui::Ui, usage_only: bool) {
        style::heading(
            ui,
            if usage_only {
                "Usage providers"
            } else {
                "Widgets"
            },
            if usage_only {
                "Usage widgets use the sign-in from each provider’s app or CLI."
            } else {
                "Choose what lives on your taskbar. Each widget has its own settings and theme."
            },
        );
        let current = self.cfg.lock().unwrap().clone();
        if usage_only {
            style::group(ui, |ui| {
                ui.label(egui::RichText::new("Refresh").size(16.0).strong());
                ui.horizontal(|ui| {
                    for minutes in config::USAGE_REFRESH_CHOICES {
                        let chosen = current.usage_refresh().as_secs() == minutes * 60;
                        if ui
                            .selectable_label(chosen, format!("Every {minutes} min"))
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
                    "How often each enabled provider is asked for its usage. Less often means fewer requests to the provider.",
                );
            });
            ui.add_space(12.0);
        }
        if !usage_only {
            style::group(ui, |ui| {
                ui.label(egui::RichText::new("Taskbar placement").size(16.0).strong());
                let mut unmagnetise = !current.magnetized;
                if ui
                    .checkbox(&mut unmagnetise, "Unmagnetise taskbar widgets")
                    .changed()
                {
                    if let Err(error) =
                        config::try_update(&self.cfg, |c| c.magnetized = !unmagnetise)
                    {
                        self.notice = Some((false, error));
                    }
                }
                ui.colored_label(style::MUTED,"By default, right-side widgets pack toward the system tray and left-side widgets pack toward the left edge. Unmagnetise to keep manual positions.");
            });
            ui.add_space(12.0);
        }
        for kind in Kind::ALL {
            if usage_only && !matches!(kind, Kind::Usage(_)) {
                continue;
            }
            style::group(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(kind.label()).size(16.0).strong());
                        ui.label(egui::RichText::new(description(kind)).color(style::MUTED));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut enabled = kind.state(&current).enabled;
                        if style::toggle(ui, &mut enabled).changed() {
                            let result = config::try_update(&self.cfg, |c| {
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
                    if ui.button("Settings").clicked() {
                        self.page = match kind {
                            Kind::Weather => Page::Weather,
                            Kind::Calendar => Page::Calendar,
                            Kind::Mail => Page::Mail,
                            Kind::Usage(_) => Page::Usage,
                        };
                    }
                    if ui.button("Edit theme").clicked() {
                        self.studio.select(kind, &current);
                        self.page = Page::Studio;
                    }
                    let mut placement = kind.state(&current).placement;
                    egui::ComboBox::from_id_salt(("widget-placement", kind.key()))
                        .selected_text(match placement {
                            config::Placement::Taskbar => "Taskbar",
                            config::Placement::Desktop => "Desktop",
                            config::Placement::Floating => "Floating",
                        })
                        .show_ui(ui, |ui| {
                            for (p, label) in [
                                (config::Placement::Taskbar, "Taskbar"),
                                (config::Placement::Desktop, "Desktop"),
                                (config::Placement::Floating, "Floating"),
                            ] {
                                ui.selectable_value(&mut placement, p, label);
                            }
                        });
                    if placement != kind.state(&current).placement {
                        if let Err(error) = config::try_update(&self.cfg, |c| {
                            kind.state_mut(c).placement = placement
                        }) {
                            self.notice = Some((false, error));
                        }
                    }
                    if kind.state(&current).enabled
                        && !kind.state(&current).visible
                        && ui.button("Show widget").clicked()
                    {
                        if let Err(e) =
                            config::try_update(&self.cfg, |c| kind.state_mut(c).visible = true)
                        {
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
            });
            ui.add_space(8.0);
        }
    }
    fn weather(&mut self, ui: &mut egui::Ui) {
        style::heading(
            ui,
            "Weather",
            "Choose a place, keep favorites, or enter coordinates.",
        );
        style::group(ui, |ui| {
            if let Some(place) = &mut self.draft.weather.place {
                style::field(ui, "Place", &mut place.name, false);
                ui.horizontal(|ui| {
                    ui.label("Latitude");
                    ui.add(
                        egui::DragValue::new(&mut place.lat)
                            .speed(0.001)
                            .range(-90.0..=90.0)
                            .max_decimals(4),
                    );
                    ui.label("Longitude");
                    ui.add(
                        egui::DragValue::new(&mut place.lon)
                            .speed(0.001)
                            .range(-180.0..=180.0)
                            .max_decimals(4),
                    );
                });
            } else if ui.button("Set a place").clicked() {
                self.draft.weather.place = config::WeatherCfg::default().place;
            }
            ui.horizontal(|ui| {
                ui.label("Refresh interval (seconds)");
                ui.add(
                    egui::DragValue::new(&mut self.draft.weather.refresh_secs).range(1800..=86400),
                );
            });
            if let Some(place) = &self.draft.weather.place {
                if ui.button("Add current place to favorites").clicked()
                    && !self.draft.weather.favorites.contains(place)
                {
                    self.draft.weather.favorites.push(place.clone());
                }
            }
        });
        ui.add_space(14.0);
        style::group(ui, |ui| {
            ui.label(egui::RichText::new("Find a place").size(16.0).strong());
            if let Some(place) = self.search.ui(ui) {
                if !self.draft.weather.favorites.contains(&place) {
                    self.draft.weather.favorites.push(place.clone());
                }
                self.draft.weather.place = Some(place);
            }
        });
        ui.add_space(14.0);
        style::group(ui, |ui| {
            ui.label(egui::RichText::new("Favorites").size(16.0).strong());
            let mut remove = None;
            if self.draft.weather.favorites.is_empty() {
                ui.colored_label(
                    style::MUTED,
                    "No favorites yet. Search for a place or add your current one.",
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
                        if ui.small_button("Remove").clicked() {
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
            if mail { "Mail" } else { "Calendar" },
            if mail {
                "Connect an IMAP server, such as Proton Mail Bridge."
            } else {
                "Connect a CalDAV server, such as iOxide."
            },
        );
        style::group(ui, |ui| {
            if mail {
                let c = &mut self.draft.mail;
                style::field(ui, "Server", &mut c.host, false);
                ui.horizontal(|ui| {
                    ui.label("Port");
                    ui.add(egui::DragValue::new(&mut c.port).range(1..=65535));
                    ui.label("Connection");
                    egui::ComboBox::from_id_salt("mail-security")
                        .selected_text(&c.security)
                        .show_ui(ui, |ui| {
                            for mode in ["starttls", "tls", "plain"] {
                                ui.selectable_value(&mut c.security, mode.into(), mode);
                            }
                        });
                });
                style::field(ui, "Username", &mut c.username, false);
                style::field(ui, "Password", &mut c.password, true);
                ui.horizontal(|ui| {
                    ui.label("Refresh (seconds)");
                    ui.add(egui::DragValue::new(&mut c.refresh_secs).range(30..=86400));
                    ui.label("Rotate subject (seconds)");
                    ui.add(egui::DragValue::new(&mut c.rotate_secs).range(1..=3600));
                });
            } else {
                let c = &mut self.draft.calendar;
                style::field(ui, "Server", &mut c.host, false);
                ui.horizontal(|ui| {
                    ui.label("Port");
                    ui.add(egui::DragValue::new(&mut c.port).range(1..=65535));
                    ui.checkbox(&mut c.tls, "Use TLS (HTTPS)");
                });
                style::field(ui, "Calendar path", &mut c.path, false);
                style::field(ui, "Username", &mut c.username, false);
                style::field(ui, "Password", &mut c.password, true);
                ui.horizontal(|ui| {
                    ui.label("Refresh interval (seconds)");
                    ui.add(egui::DragValue::new(&mut c.refresh_secs).range(30..=86400));
                });
            }
            ui.add_space(8.0);
            ui.colored_label(
                style::MUTED,
                "Credentials stay in your local widgets.json file and are stored as plain text.",
            );
        });
        if ui
            .add_enabled(self.test.is_none(), egui::Button::new("Test connection"))
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
            self.notice = Some((true, "Testing connection…".into()));
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let result = if mail {
                    crate::imap::fetch(&draft.mail)
                        .map(|u| format!("Connected. {} unread messages.", u.count))
                } else {
                    crate::caldav::fetch(&draft.calendar).map(|f| {
                        format!(
                            "Connected. {} calendars, {} upcoming events.",
                            f.calendars, f.events
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

impl eframe::App for Dashboard {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if !self.hooked {
            if let Some(hwnd) = crate::shell::hwnd_of(frame) {
                hook(hwnd);
                self.hooked = true;
            }
        }
        let request = REQUESTED.swap(usize::MAX, Ordering::Relaxed);
        if let Some(page) = Page::ALL.get(request) {
            self.page = *page;
        }
        self.refresh();
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
                        (false,"Settings changed during the test. Test again to check the current settings.".into())
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
                    self.notice = Some((false, "Connection check stopped. Try again.".into()));
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
                        Page::Widgets=>self.widgets(ui,false),Page::Usage=>self.widgets(ui,true),Page::Weather=>self.weather(ui),Page::Calendar=>self.connection(ui,false),Page::Mail=>self.connection(ui,true),
                        Page::About=>{style::heading(ui,"BetterTaskbar","Your taskbar widgets, in one place.");style::group(ui,|ui|{ui.label("Usage providers and dashboard styling are adapted from Claude Code Usage Monitor by Code Zeno Pty Ltd.");ui.hyperlink_to("Claude Code Usage Monitor (MIT)","https://github.com/CodeZeno/Claude-Code-Usage-Monitor");ui.hyperlink_to("BetterTaskbar source and third-party notices","https://github.com/mrk1987/bettertaskbar");ui.label("Weather: MET Norway / Yr. Place search: Open-Meteo / GeoNames.");egui::CollapsingHeader::new("CodeZeno MIT license").show(ui,|ui|{ui.label(include_str!("../vendor/codezeno-usage-core/LICENSE"));ui.label(include_str!("../assets/lobe-icons.LICENSE"));});});},Page::Studio=>{}
                    }
                    if matches!(self.page,Page::Weather|Page::Calendar|Page::Mail){ui.add_space(16.0);ui.horizontal(|ui|{if ui.add(egui::Button::new("Save changes").fill(style::ACCENT)).clicked(){self.save(false);}
if ui.button("Discard edits").clicked(){match self.page {Page::Weather=>self.draft.weather=self.base.weather.clone(),Page::Calendar=>self.draft.calendar=self.base.calendar.clone(),Page::Mail=>self.draft.mail=self.base.mail.clone(),_=>{}}self.notice=None;}});}
                    });
                });
            });
        });
        });
        if self.close_prompt {
            egui::Window::new("Unsaved changes")
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label("Save your settings and theme before closing?");
                    ui.horizontal(|ui| {
                        if ui.button("Keep editing").clicked() {
                            self.close_prompt = false;
                        }
                        if ui.button("Discard and close").clicked() {
                            self.base = self.draft.clone();
                            self.studio.discard();
                            self.close_prompt = false;
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Save and close").clicked()
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
            open(page);
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
        Kind::Weather => "Forecast, temperatures and precipitation",
        Kind::Calendar => "What is on now and what comes next",
        Kind::Mail => "Unread count and rotating message subjects",
        Kind::Usage(_) => "Usage limits and reset times",
    }
}
fn setup(p: codezeno_usage_core::providers::ProviderId) -> &'static str {
    use codezeno_usage_core::providers::ProviderId::*;
    match p{Claude=>"Sign in with Claude Code or the Claude desktop app.",Codex=>"Sign in with the Codex CLI or desktop app.",Antigravity=>"Sign in with the Antigravity CLI.",OpenCode=>"Use OPENCODE_GO_WORKSPACE_ID and OPENCODE_GO_AUTH_COOKIE, or OpenCode Go’s config.json.",Cursor=>"Sign in with Cursor. Its local session is detected automatically.",Grok=>"Run grok login to sign in with your xAI account.",Copilot=>"Sign in with the Copilot CLI or GitHub CLI (gh auth login)."}
}
fn validate(c: &Config) -> Result<(), String> {
    for host in [&c.calendar.host, &c.mail.host] {
        if host.trim().is_empty() || host.contains(['\r', '\n', '/', '\\']) {
            return Err("Enter a server hostname or IP address, without a URL or path.".into());
        }
    }
    if c.calendar.port == 0 || c.mail.port == 0 {
        return Err("Server ports must be between 1 and 65535.".into());
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
        return Err("Usernames and passwords cannot contain line breaks.".into());
    }
    if !["starttls", "tls", "plain", "none"]
        .contains(&c.mail.security.to_ascii_lowercase().as_str())
    {
        return Err("Choose STARTTLS, TLS or plain for the mail connection.".into());
    }
    if !(1800..=86400).contains(&c.weather.refresh_secs)
        || !(30..=86400).contains(&c.calendar.refresh_secs)
        || !(30..=86400).contains(&c.mail.refresh_secs)
        || !(1..=3600).contains(&c.mail.rotate_secs)
    {
        return Err("Choose a weather refresh from 1800–86400 seconds, mail/calendar refresh from 30–86400 seconds, and subject rotation from 1–3600 seconds.".into());
    }
    if let Some(place) = &c.weather.place {
        if !place.lat.is_finite()
            || !place.lon.is_finite()
            || !(-90.0..=90.0).contains(&place.lat)
            || !(-180.0..=180.0).contains(&place.lon)
        {
            return Err("Enter valid latitude and longitude coordinates.".into());
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
        _ => {}
    }
    selected
}

fn merge_settings(latest: &mut Config, base: &Config, draft: &Config) {
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
}
#[cfg(test)]
mod tests {
    use super::*;
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
    fn invalid_coordinates_and_multiline_credentials_are_rejected() {
        let mut c = Config::default();
        c.mail.password = "a\r\nb".into();
        assert!(validate(&c).is_err());
        c.mail.password.clear();
        c.weather.place.as_mut().unwrap().lat = f64::NAN;
        assert!(validate(&c).is_err());
    }
}
