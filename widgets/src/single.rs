//! Every pill in the hub's own process, drawn on the CPU (the default).
//!
//! The older mode, `--process-per-widget`, runs each pill as an eframe
//! process with its own GPU device, about 55 MiB each (see
//! docs/architecture.md). Here one thread owns
//! every pill window and runs their message loop. egui lays a pill out as
//! usual; `cpu_render` turns its triangles into pixels; `UpdateLayeredWindow`
//! shows them with per-pixel alpha, so a docked pill still lets the taskbar
//! show through. Docked, the window is a child of the taskbar as in process
//! mode, which for a layered window needs Windows 8 or later declared in the
//! program's manifest (`app.manifest`).
//!
//! `--process-per-widget` or `"process_per_widget": true` turns it off.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{Kind, WidgetApp};
use crate::config::SharedConfig;
use crate::cpu_render::{self, Canvas, Textures};

const CLASS: PCWSTR = w!("BetterTaskbarPill");
/// From WinUser.h; the windows crate keeps it behind the Controls feature.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Posted to the loop's thread when a pill wants a repaint from elsewhere.
const WM_WAKE: u32 = WM_APP + 1;
/// The most often a pill repaints, also while it is being dragged.
const FRAME: Duration = Duration::from_millis(16);
/// How often enabled widgets, lost windows and magnetic placement are checked.
const SUPERVISE: Duration = Duration::from_millis(500);

enum Input {
    Moved(i32, i32),
    Button(i32, i32, egui::PointerButton, bool),
    Left,
}

thread_local! {
    /// Mouse input seen by the window procedure, collected by the loop. The
    /// window procedure touches nothing else: it can run in the middle of a
    /// pill's frame, because a popup menu runs a message loop of its own.
    static INPUT: RefCell<Vec<(isize, Input)>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    let input = match msg {
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut track);
            Some(Input::Moved(x, y))
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
            SetCapture(hwnd);
            let button = if msg == WM_LBUTTONDOWN {
                egui::PointerButton::Primary
            } else {
                egui::PointerButton::Secondary
            };
            Some(Input::Button(x, y, button, true))
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let _ = ReleaseCapture();
            let button = if msg == WM_LBUTTONUP {
                egui::PointerButton::Primary
            } else {
                egui::PointerButton::Secondary
            };
            Some(Input::Button(x, y, button, false))
        }
        WM_MOUSELEAVE => Some(Input::Left),
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            return LRESULT(0);
        }
        _ => None,
    };
    if let Some(input) = input {
        INPUT.with(|queue| queue.borrow_mut().push((hwnd.0 as isize, input)));
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn register_class() -> bool {
    unsafe {
        let Ok(instance) = GetModuleHandleW(PCWSTR::null()) else {
            return false;
        };
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class) != 0
    }
}

fn create_window(kind: Kind) -> Option<HWND> {
    let title: Vec<u16> = kind.title().encode_utf16().chain([0]).collect();
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null()).ok()?;
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            CLASS,
            PCWSTR(title.as_ptr()),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

fn is_window(hwnd: HWND) -> bool {
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

fn scale_of(hwnd: HWND) -> f32 {
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => 1.0,
        dpi => dpi as f32 / 96.0,
    }
}

/// A 32-bit premultiplied DIB that `UpdateLayeredWindow` reads from.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    width: usize,
    height: usize,
}

impl Surface {
    fn new(width: usize, height: usize) -> Option<Self> {
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
                .unwrap_or_default();
            if bitmap.is_invalid() || bits.is_null() {
                if !bitmap.is_invalid() {
                    let _ = DeleteObject(bitmap.into());
                }
                let _ = DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, bitmap.into());
            Some(Self {
                dc,
                bitmap,
                old,
                bits: bits.cast(),
                width,
                height,
            })
        }
    }

    /// Copy `canvas` in and show it as the window's whole content and size.
    fn present(&mut self, hwnd: HWND, canvas: &Canvas) -> windows::core::Result<()> {
        let pixels = unsafe { std::slice::from_raw_parts_mut(self.bits, self.width * self.height) };
        for (out, &[r, g, b, a]) in pixels.iter_mut().zip(&canvas.pixels) {
            // Windows passes clicks through fully transparent pixels of a
            // layered window. Alpha 1 keeps the whole pill clickable.
            *out = if a == 0 {
                0x0100_0000
            } else {
                u32::from_be_bytes([a, r, g, b])
            };
        }
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            // A screen DC, as UpdateLayeredWindow expects; no destination
            // point, so placement stays with `shell::place` and the hub.
            let screen = GetDC(None);
            let result = UpdateLayeredWindow(
                hwnd,
                Some(screen),
                None,
                Some(&SIZE {
                    cx: self.width as i32,
                    cy: self.height as i32,
                }),
                Some(self.dc),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            ReleaseDC(None, screen);
            result
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// egui and the CPU rasterizer for one pill, with or without a window.
struct Painter {
    ctx: egui::Context,
    textures: Textures,
    /// The pill's size in points, as the pill last asked for it.
    size: egui::Vec2,
    start: Instant,
}

impl Painter {
    fn new(size: (f32, f32)) -> Self {
        Self {
            ctx: egui::Context::default(),
            textures: Textures::default(),
            size: egui::vec2(size.0, size.1),
            start: Instant::now(),
        }
    }

    /// Run one frame of `app` and draw it. Returns the pixels and how soon
    /// egui wants the next frame.
    fn frame(
        &mut self,
        app: &mut WidgetApp,
        window: Option<HWND>,
        events: Vec<egui::Event>,
        scale: f32,
    ) -> (Canvas, Option<Duration>) {
        let mut viewports = egui::ViewportIdMap::default();
        viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                native_pixels_per_point: Some(scale),
                focused: Some(false),
                ..Default::default()
            },
        );
        let input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            viewports,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            max_texture_side: Some(8192),
            time: Some(self.start.elapsed().as_secs_f64()),
            events,
            focused: false,
            system_theme: Some(egui::Theme::Dark),
            ..Default::default()
        };
        let output = self.ctx.run_ui(input, |ui| app.frame(ui, window));
        let egui::FullOutput {
            textures_delta,
            shapes,
            pixels_per_point,
            viewport_output,
            ..
        } = output;
        let mut repaint = None;
        if let Some(root) = viewport_output.get(&egui::ViewportId::ROOT) {
            for command in &root.commands {
                if let egui::ViewportCommand::InnerSize(size) = command {
                    if *size != self.size {
                        self.size = *size;
                        // Laid out for the old size; draw again at the new one.
                        repaint = Some(Duration::ZERO);
                    }
                }
            }
            repaint = repaint.or(Some(root.repaint_delay));
        }
        self.textures.set(&textures_delta);
        let primitives = self.ctx.tessellate(shapes, pixels_per_point);
        let mut canvas = Canvas::new(
            (self.size.x * pixels_per_point).round().max(1.0) as usize,
            (self.size.y * pixels_per_point).round().max(1.0) as usize,
        );
        cpu_render::paint(&mut canvas, &primitives, &self.textures, pixels_per_point);
        self.textures.free(textures_delta);
        (canvas, repaint)
    }
}

/// One pill: its window, its widget and its painter.
struct Pill {
    kind: Kind,
    hwnd: HWND,
    app: WidgetApp,
    painter: Painter,
    surface: Option<Surface>,
    events: Vec<egui::Event>,
    /// When the pill next wants to be drawn. Set from any thread through
    /// egui's repaint callback.
    wake: Arc<Mutex<Option<Instant>>>,
    last_frame: Option<Instant>,
}

impl Pill {
    fn new(kind: Kind, cfg: SharedConfig, thread: u32) -> Option<Self> {
        let hwnd = create_window(kind)?;
        let app = WidgetApp::new_in_process(kind, cfg);
        let painter = Painter::new(app.size_hint());
        let wake = Arc::new(Mutex::new(Some(Instant::now())));
        let requested = wake.clone();
        painter.ctx.set_request_repaint_callback(move |info| {
            if let Some(at) = Instant::now().checked_add(info.delay) {
                let mut wake = requested.lock().unwrap();
                if wake.is_none_or(|t| at < t) {
                    *wake = Some(at);
                }
            }
            unsafe {
                let _ = PostThreadMessageW(thread, WM_WAKE, WPARAM(0), LPARAM(0));
            }
        });
        Some(Self {
            kind,
            hwnd,
            app,
            painter,
            surface: None,
            events: Vec::new(),
            wake,
            last_frame: None,
        })
    }

    /// Explorer destroyed the window with its taskbar: make a new one.
    fn replace_window(&mut self) {
        if let Some(hwnd) = create_window(self.kind) {
            self.app.window_lost();
            self.hwnd = hwnd;
            self.surface = None;
            *self.wake.lock().unwrap() = Some(Instant::now());
        }
    }

    /// When the next frame is due, if one is wanted.
    fn due(&self) -> Option<Instant> {
        let wanted = if self.events.is_empty() {
            *self.wake.lock().unwrap()
        } else {
            Some(Instant::now())
        }?;
        Some(match self.last_frame {
            Some(last) => wanted.max(last + FRAME),
            None => wanted,
        })
    }

    fn frame(&mut self) {
        *self.wake.lock().unwrap() = None;
        self.last_frame = Some(Instant::now());
        let window = is_window(self.hwnd).then_some(self.hwnd);
        let scale = scale_of(self.hwnd);
        let events = std::mem::take(&mut self.events);
        let (canvas, repaint) = self.painter.frame(&mut self.app, window, events, scale);
        if let Some(at) = repaint.and_then(|delay| Instant::now().checked_add(delay)) {
            let mut wake = self.wake.lock().unwrap();
            if wake.is_none_or(|t| at < t) {
                *wake = Some(at);
            }
        }
        // A popup menu runs a message loop inside the frame; the window may
        // have been destroyed meanwhile.
        let Some(hwnd) = window.filter(|h| is_window(*h)) else {
            return;
        };
        if self
            .surface
            .as_ref()
            .is_none_or(|s| (s.width, s.height) != (canvas.width, canvas.height))
        {
            self.surface = Surface::new(canvas.width, canvas.height);
        }
        if let Some(surface) = self.surface.as_mut() {
            if let Err(e) = surface.present(hwnd, &canvas) {
                crate::log::line(self.kind.key(), &format!("cannot show the pill: {e}"));
            }
        }
    }
}

impl Drop for Pill {
    fn drop(&mut self) {
        if is_window(self.hwnd) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// Hand queued mouse input to the pills it belongs to, in points.
fn deliver_input(pills: &mut [Pill]) {
    let queued = INPUT.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
    for (hwnd, input) in queued {
        let Some(pill) = pills.iter_mut().find(|p| p.hwnd.0 as isize == hwnd) else {
            continue;
        };
        let scale = scale_of(pill.hwnd);
        let pos = |x: i32, y: i32| egui::pos2(x as f32 / scale, y as f32 / scale);
        pill.events.push(match input {
            Input::Moved(x, y) => egui::Event::PointerMoved(pos(x, y)),
            Input::Button(x, y, button, pressed) => egui::Event::PointerButton {
                pos: pos(x, y),
                button,
                pressed,
                modifiers: egui::Modifiers::default(),
            },
            Input::Left => egui::Event::PointerGone,
        });
    }
}

/// Start and stop pills as widgets are enabled and disabled, replace windows
/// Explorer destroyed, and let the magnet place the docked ones.
fn supervise(cfg: &SharedConfig, pills: &mut Vec<Pill>, thread: u32) {
    crate::config::refresh(cfg);
    let settings = cfg.lock().unwrap().clone();
    for kind in Kind::ALL {
        let enabled = kind.state(&settings).enabled;
        match pills.iter().position(|p| p.kind == kind) {
            None if enabled => match Pill::new(kind, cfg.clone(), thread) {
                Some(pill) => pills.push(pill),
                None => crate::log::line("hub", &format!("Cannot create {}", kind.label())),
            },
            Some(i) if !enabled => {
                pills.remove(i);
            }
            Some(i) if !is_window(pills[i].hwnd) => pills[i].replace_window(),
            _ => {}
        }
    }
    let windows: Vec<_> = pills.iter().map(|p| (p.kind, p.hwnd)).collect();
    crate::magnet::arrange(cfg, &windows);
}

/// Run every enabled pill on this thread until the hub quits.
pub fn run(cfg: SharedConfig, tray: &std::thread::JoinHandle<()>) {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    if !register_class() {
        crate::log::line("hub", "cannot register the pill window class");
        return;
    }
    let thread = unsafe { GetCurrentThreadId() };
    crate::magnet::reset();
    let mut pills: Vec<Pill> = Vec::new();
    let mut next_supervise = Instant::now();
    'run: loop {
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                break 'run;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if Instant::now() >= next_supervise {
            if crate::QUITTING.load(std::sync::atomic::Ordering::SeqCst) || tray.is_finished() {
                break;
            }
            supervise(&cfg, &mut pills, thread);
            next_supervise = Instant::now() + SUPERVISE;
        }
        deliver_input(&mut pills);
        for pill in &mut pills {
            if pill.due().is_some_and(|at| at <= Instant::now()) {
                pill.frame();
            }
        }
        let next = pills
            .iter()
            .filter_map(Pill::due)
            .chain([next_supervise])
            .min()
            .unwrap_or(next_supervise);
        // Round up: rounding down waits 0 ms when the next frame is under a
        // millisecond away, and the loop spins until it is due.
        let timeout = next
            .saturating_duration_since(Instant::now())
            .as_micros()
            .div_ceil(1000);
        // Wait for new messages only. A docked pill is a child of Explorer's
        // taskbar, so this thread shares an input queue with Explorer;
        // MWMO_INPUTAVAILABLE would wake for Explorer's own unread input,
        // hundreds of times a second while the mouse moves.
        unsafe {
            MsgWaitForMultipleObjectsEx(
                None,
                timeout.min(u32::MAX as u128) as u32,
                QS_ALLINPUT,
                MSG_WAIT_FOR_MULTIPLE_OBJECTS_EX_FLAGS(0),
            );
        }
    }
}

/// `--cpu-shot=<png> --widget=<key>`: draw one pill off screen with live
/// data, for `wait`, and save it over a dark taskbar color. Touches no
/// window and writes no settings.
pub fn shot(kind: Kind, path: &str, wait: Duration, scale: f32) -> Result<(), String> {
    let cfg = crate::config::shared();
    let mut app = WidgetApp::new_in_process(kind, cfg);
    let mut painter = Painter::new(app.size_hint());
    let start = Instant::now();
    let canvas = loop {
        let (canvas, _) = painter.frame(&mut app, None, Vec::new(), scale);
        if start.elapsed() >= wait {
            break canvas;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    const BACKGROUND: [u8; 3] = [0x20, 0x20, 0x20];
    let pixels: Vec<u8> = canvas
        .pixels
        .iter()
        .flat_map(|&[r, g, b, a]| {
            let under =
                |c: u8, bg: u8| c.saturating_add(((bg as u16 * (255 - a as u16)) / 255) as u8);
            [
                under(r, BACKGROUND[0]),
                under(g, BACKGROUND[1]),
                under(b, BACKGROUND[2]),
                255,
            ]
        })
        .collect();
    image::save_buffer(
        path,
        &pixels,
        canvas.width as u32,
        canvas.height as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}
