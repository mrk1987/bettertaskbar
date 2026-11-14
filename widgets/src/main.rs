#![windows_subsystem = "windows"]

mod app;
mod caldav;
mod click_action;
mod config;
mod cpu_render;
mod dashboard;
mod dashboard_style;
mod http;
mod ical;
mod icons;
mod imap;
mod log;
mod magnet;
mod occupancy;
mod place_search;
mod popup;
#[cfg(debug_assertions)]
mod preview;
mod process_lock;
mod provider_icons;
mod rendering;
mod shell;
mod single;
mod studio;
mod theme;
mod theme_file;
mod tray;
mod widgets;

use eframe::egui;
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

static QUITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const HUB_TITLE: &str = "BetterTaskbar Widgets";

/// Close every widget process and the hub. Each eframe root exits on WM_CLOSE.
pub fn quit_all() {
    QUITTING.store(true, std::sync::atomic::Ordering::SeqCst);
    for kind in app::Kind::ALL {
        shell::close_by_title(kind.title());
    }
    shell::close_by_title(dashboard::TITLE);
    shell::close_by_title(HUB_TITLE);
}

fn main() {
    if std::env::args().any(|a| a == "--close-dashboard") {
        shell::close_by_title(dashboard::TITLE);
        return;
    }
    if let Some(page) = std::env::args().find_map(|a| {
        if a == "--dashboard" {
            Some("widgets".to_owned())
        } else {
            a.strip_prefix("--dashboard=").map(str::to_owned)
        }
    }) {
        dashboard::run(dashboard::Page::parse(&page));
        return;
    }
    if std::env::args().any(|a| a == "--search-place") {
        dashboard::run(dashboard::Page::Weather);
        return;
    }
    if let Some(url) =
        std::env::args().find_map(|a| a.strip_prefix("--http-get=").map(str::to_string))
    {
        std::process::exit(http::serve_get(&url));
    }
    if let Some(what) =
        std::env::args().find_map(|a| a.strip_prefix("--check=").map(str::to_string))
    {
        std::process::exit(check(&what));
    }
    let widget = std::env::args().find_map(|a| a.strip_prefix("--widget=").map(str::to_string));
    if let Some(path) =
        std::env::args().find_map(|a| a.strip_prefix("--cpu-shot=").map(str::to_string))
    {
        std::process::exit(cpu_shot(widget.as_deref(), &path));
    }
    match widget.as_deref() {
        Some(key) => match app::Kind::ALL.into_iter().find(|kind| kind.key() == key) {
            Some(kind) => run_widget(kind),
            None => log::line("main", "Unknown widget kind"),
        },
        None => run_hub(),
    }
}

/// One eframe wgpu root window per widget, in its own process. Child viewports
/// never present on this machine, and winit allows only one event loop.
fn run_widget(kind: app::Kind) {
    // Present through a DirectComposition visual instead of a swapchain bound
    // to the window. Only that kind of swapchain has an alpha channel the
    // compositor honors, which is what lets a docked pill be see-through.
    std::env::set_var("WGPU_BACKEND", "dx12");
    std::env::set_var("WGPU_DX12_PRESENTATION_SYSTEM", "visual");
    let cfg = config::shared();
    let app = app::WidgetApp::new(kind, cfg);
    let (w, h) = app.size_hint();
    let title = app.window_title();
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([w, h])
            .with_decorations(false)
            .with_transparent(true)
            // Required for docking: while winit has the window registered as
            // an OLE drop target, SetParent onto the taskbar fails with
            // "invalid parameter" and the pill cannot become part of the bar.
            .with_drag_and_drop(false)
            .with_resizable(false)
            .with_taskbar(false),
        ..Default::default()
    };
    rendering::configure(&mut options);
    let result = eframe::run_native(title, options, Box::new(move |_cc| Ok(Box::new(app))));
    if let Err(e) = result {
        log::line("main", &format!("{title} stopped with an error: {e}"));
    }
}

/// `--cpu-shot=<png> --widget=<key> [--cpu-shot-wait=<s>] [--cpu-shot-scale=<f>]`:
/// render one pill with the CPU renderer, off screen, and save it.
fn cpu_shot(widget: Option<&str>, path: &str) -> i32 {
    let arg = |name: &str| {
        std::env::args().find_map(|a| a.strip_prefix(name).and_then(|v| v.parse::<f32>().ok()))
    };
    let Some(kind) = app::Kind::ALL
        .into_iter()
        .find(|kind| Some(kind.key()) == widget)
    else {
        println!("FAILED: name a widget with --widget=<key>");
        return 1;
    };
    let wait = std::time::Duration::from_secs_f32(arg("--cpu-shot-wait=").unwrap_or(8.0));
    match single::shot(kind, path, wait, arg("--cpu-shot-scale=").unwrap_or(1.0)) {
        Ok(()) => 0,
        Err(e) => {
            println!("FAILED: {e}");
            1
        }
    }
}

/// Tray hub: single instance, spawns the three widget processes, and shuts them
/// all down when the tray quits.
fn run_hub() {
    // Keep the handle alive for the hub's entire lifetime, and close it on exit.
    let mutex = unsafe { CreateMutexW(None, false, w!("BetterTaskbarWidgetsHub")) };
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let _mutex = match mutex {
        Ok(handle) => unsafe { windows::core::Owned::new(handle) },
        Err(e) => {
            log::line("hub", &format!("cannot create single-instance mutex: {e}"));
            return;
        }
    };
    if already_exists {
        return;
    }

    let cfg = config::shared();
    let tray = tray::spawn(cfg.clone());

    let per_process = std::env::args().any(|a| a == "--process-per-widget")
        || cfg.lock().unwrap().process_per_widget;
    if !per_process {
        single::run(cfg, &tray);
        quit_all();
        // Let the tray remove its icon before the process ends.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !tray.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        return;
    }

    if let Ok(exe) = std::env::current_exe() {
        std::thread::spawn(move || supervise(&exe, cfg));
    }

    if tray.join().is_err() {
        log::line("hub", "the tray thread panicked; closing the widgets");
    }
    quit_all();
}

/// Start the three widgets and start any of them again if it stops. A
/// docked pill is a child window of the taskbar, so it is destroyed with the
/// taskbar when Explorer restarts or crashes, and its process ends.
fn supervise(exe: &std::path::Path, cfg: config::SharedConfig) {
    crate::magnet::reset();
    let mut children: Vec<(app::Kind, Option<std::process::Child>)> = app::Kind::ALL
        .into_iter()
        .map(|kind| (kind, None))
        .collect();
    while !QUITTING.load(std::sync::atomic::Ordering::SeqCst) {
        config::refresh(&cfg);
        let settings = cfg.lock().unwrap().clone();
        for (kind, child) in &mut children {
            if !kind.state(&settings).enabled {
                if let Some(mut running) = child.take() {
                    let _ = running.kill();
                    let _ = running.wait();
                }
                continue;
            }
            let stopped = match child {
                Some(running) => !matches!(running.try_wait(), Ok(None)),
                None => true,
            };
            if stopped && !QUITTING.load(std::sync::atomic::Ordering::SeqCst) {
                *child = std::process::Command::new(exe)
                    .arg(format!("--widget={}", kind.key()))
                    .spawn()
                    .inspect_err(|e| {
                        log::line("hub", &format!("Cannot start {}: {e}", kind.label()))
                    })
                    .ok();
            }
        }
        crate::magnet::arrange(&cfg, &crate::magnet::process_windows(&children));
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// What a dragged pill sees: each taskbar, what occupies it and the gaps left.
fn check_taskbar() -> String {
    let mut out = String::new();
    for (bar, read) in occupancy::read_now() {
        let r = bar.rect;
        out.push_str(&format!(
            "taskbar on {}: x {}..{}, y {}..{}, notification area from x {}
",
            bar.monitor, r.left, r.right, r.top, r.bottom, bar.tray_left
        ));
        let mut taken = match read {
            Ok(Some(taken)) => taken,
            Ok(None) => {
                out.push_str(
                    "  layout could not be read; only the notification area counts as taken
",
                );
                vec![]
            }
            Err(e) => {
                out.push_str(&format!(
                    "  layout could not be read: {e}
"
                ));
                vec![]
            }
        };
        out.push_str(&format!(
            "  {} controls take space
",
            taken.len()
        ));
        taken.push(windows::Win32::Foundation::RECT {
            left: bar.tray_left,
            right: r.right,
            ..r
        });
        for (left, right) in occupancy::free_slots(r, &taken, r.top, r.bottom) {
            out.push_str(&format!(
                "  free gap: x {left}..{right} ({} px)
",
                right - left
            ));
        }
    }
    out.trim_end().to_owned()
}

/// `--check=calendar|mail|taskbar`: run one fetch with the saved settings and print
/// what the pill would show, or why it cannot. Redirect the output to see it
/// (`bettertaskbar-widgets.exe --check=calendar | more`); a GUI program has no
/// console of its own.
fn check(what: &str) -> i32 {
    let cfg = config::shared();
    let cfg = cfg.lock().unwrap();
    let outcome = match what {
        "calendar" => caldav::fetch(&cfg.calendar).map(|f| {
            format!(
                "{} calendars, {} events in the next 14 days, {} unreadable recurrence rules
{}
{}",
                f.calendars,
                f.events,
                f.bad_rules,
                f.lines.now.unwrap_or_else(|| "Now: nothing".into()),
                f.lines.next.unwrap_or_else(|| "Next: nothing".into())
            )
        }),
        "mail" => imap::fetch(&cfg.mail).map(|u| {
            format!(
                "{} unread
{}",
                u.count,
                u.subjects.join(
                    "
"
                )
            )
        }),
        "taskbar" => Ok(check_taskbar()),
        "docking" => magnet::diagnostics(&cfg),
        other => Err(format!(
            "unknown check \"{other}\"; use calendar, mail or taskbar"
        )),
    };
    match outcome {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(e) => {
            println!("FAILED: {e}");
            1
        }
    }
}
