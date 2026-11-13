//! Desktop shell integration: find the taskbars and where a pill docks on
//! them, pin a pill onto the desktop (behind app windows), float it always on
//! top, and style its frame.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMNCRP_ENABLED, DWMWA_BORDER_COLOR,
    DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MapWindowPoints, MonitorFromPoint, MonitorFromWindow, HMONITOR,
    MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::*;

const WS_CHILD: u32 = 0x4000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_DLGFRAME: u32 = 0x0040_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const WS_EX_APPWINDOW: u32 = 0x0004_0000;

const FRAME_BITS: u32 = WS_CAPTION
    | WS_THICKFRAME
    | WS_DLGFRAME
    | WS_BORDER
    | WS_SYSMENU
    | WS_MINIMIZEBOX
    | WS_MAXIMIZEBOX;

fn borderless(s: u32) -> u32 {
    s & !FRAME_BITS
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Resolve the native HWND from an eframe frame's raw window handle.
pub fn hwnd_of(handle: &impl HasWindowHandle) -> Option<HWND> {
    let h = handle.window_handle().ok()?;
    match h.as_raw() {
        RawWindowHandle::Win32(w) => Some(HWND(w.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// Find a window by exact title, searching top-level windows and their children
/// (desktop-pinned widgets are children of the desktop WorkerW, not top-level).
pub fn find_by_title(title: &str) -> Option<HWND> {
    struct Ctx {
        want: Vec<u16>,
        found: Option<HWND>,
    }
    unsafe extern "system" fn check(hwnd: HWND, l: LPARAM) -> windows::core::BOOL {
        let c = &mut *(l.0 as *mut Ctx);
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n > 0 && buf[..n as usize] == c.want[..] {
            c.found = Some(hwnd);
            return windows::core::BOOL(0);
        }
        windows::core::BOOL(1)
    }
    unsafe extern "system" fn top(hwnd: HWND, l: LPARAM) -> windows::core::BOOL {
        if check(hwnd, l).0 == 0 {
            return windows::core::BOOL(0);
        }
        let _ = EnumChildWindows(Some(hwnd), Some(check), l);
        let c = &*(l.0 as *const Ctx);
        if c.found.is_some() {
            windows::core::BOOL(0)
        } else {
            windows::core::BOOL(1)
        }
    }
    let mut ctx = Ctx {
        want: title.encode_utf16().collect(),
        found: None,
    };
    unsafe {
        let _ = EnumWindows(Some(top), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.found
}

/// Ask a window (by title) to close; eframe roots exit on WM_CLOSE.
pub fn close_by_title(title: &str) {
    if let Some(hwnd) = find_by_title(title) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

fn style(hwnd: HWND) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 }
}

fn set_style(hwnd: HWND, value: u32) {
    unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, value as isize) };
}

fn ex_style(hwnd: HWND) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 }
}

fn set_ex_style(hwnd: HWND, value: u32) {
    unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, value as isize) };
}

pub fn screen_rect(hwnd: HWND) -> RECT {
    rect_of(hwnd).unwrap_or_default()
}

pub fn rect_of(hwnd: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r).ok()? };
    Some(r)
}

/// A window's screen rectangle, if it is currently shown.
pub fn visible_rect(hwnd: HWND) -> Option<RECT> {
    unsafe { IsWindowVisible(hwnd).as_bool() }
        .then(|| rect_of(hwnd))
        .flatten()
}

/// True while the window is a child of the desktop host.
fn is_child(hwnd: HWND) -> bool {
    style(hwnd) & WS_CHILD != 0
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// The wallpaper WorkerW that sits behind the desktop icons, or Progman.
pub fn desktop_host() -> Option<HWND> {
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        let out = &mut *(lparam.0 as *mut Option<HWND>);
        if FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), PCWSTR::null()).is_ok() {
            if let Ok(worker) = FindWindowExW(None, Some(hwnd), w!("WorkerW"), PCWSTR::null()) {
                *out = Some(worker);
                return windows::core::BOOL(0);
            }
        }
        windows::core::BOOL(1)
    }

    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null()).ok()?;
        // Ask Explorer to spawn the wallpaper WorkerW on shells that hide it.
        SendMessageW(progman, 0x052C, Some(WPARAM(0)), Some(LPARAM(0)));
        let mut found: Option<HWND> = None;
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut found as *mut _ as isize));
        Some(found.unwrap_or(progman))
    }
}

fn apply_ex(hwnd: HWND) {
    let ex = ex_style(hwnd);
    set_ex_style(
        hwnd,
        (ex | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) & !(WS_EX_APPWINDOW | WS_EX_TOPMOST),
    );
}

/// Put the pill on the desktop, behind app windows, where it is on screen.
pub fn pin_desktop(hwnd: HWND) {
    if let Some(host) = desktop_host() {
        embed(hwnd, host);
    }
}

/// The window a hosted pill should be a child of; 0 when it is a free popup.
/// Read by the window procedure hook to keep winit from undoing WS_CHILD.
static HOST: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Whether the pill is currently a child of a taskbar (primary or secondary).
pub fn on_a_taskbar(hwnd: HWND) -> bool {
    parent_of(hwnd).is_some_and(|host| {
        matches!(
            class_name(host).as_str(),
            "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        )
    })
}

pub fn parent_of(hwnd: HWND) -> Option<HWND> {
    is_child(hwnd)
        .then(|| unsafe { GetParent(hwnd).ok() })
        .flatten()
}

/// Make the pill a child window of `host` (a taskbar or the desktop),
/// keeping its place on screen.
///
/// On a taskbar this is what makes the pill part of the bar: it shares the
/// bar's z-order and visibility, so it stays in view when the taskbar is
/// clicked (Windows then lifts the taskbar above every ordinary window,
/// topmost ones included), and it goes away with the bar under a fullscreen
/// app or when the bar auto-hides. Same technique as Claude-Code-Usage-Monitor.
pub fn embed(hwnd: HWND, host: HWND) -> bool {
    let rect = screen_rect(hwnd);
    HOST.store(host.0 as isize, std::sync::atomic::Ordering::Relaxed);
    apply_ex(hwnd);
    set_style(
        hwnd,
        (borderless(style(hwnd)) & !WS_POPUP) | WS_CHILD | WS_CLIPSIBLINGS,
    );
    unsafe {
        if GetParent(hwnd).ok() != Some(host) {
            if let Err(e) = SetParent(hwnd, Some(host)) {
                crate::log::line(
                    "shell",
                    &format!(
                        "cannot attach to {}: {e} (style {:08x}, ex {:08x})",
                        class_name(host),
                        style(hwnd),
                        ex_style(hwnd)
                    ),
                );
                // A parentless WS_CHILD is invisible. Restore a usable popup
                // and let the next docking tick try the new taskbar again.
                make_popup(hwnd);
                return false;
            }
        }
        let mut pts = [POINT {
            x: rect.left,
            y: rect.top,
        }];
        MapWindowPoints(None, Some(host), &mut pts);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            pts[0].x,
            pts[0].y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    true
}

/// A taskbar a pill can dock on.
#[derive(Clone, Debug)]
pub struct Bar {
    pub hwnd: HWND,
    pub rect: RECT,
    /// Left edge of the notification area, or the bar's right edge if it has
    /// none (secondary bars may not). Docked pills are measured from here.
    pub tray_left: i32,
    /// Device name of the bar's monitor, e.g. `\\.\DISPLAY1`.
    pub monitor: String,
}

/// Every horizontal taskbar, primary first. Vertical bars are not offered as
/// dock targets: Windows 11 has none and the pills are laid out for a row.
pub fn taskbars() -> Vec<Bar> {
    unsafe extern "system" fn collect(hwnd: HWND, l: LPARAM) -> windows::core::BOOL {
        let out = &mut *(l.0 as *mut Vec<HWND>);
        if class_name(hwnd) == "Shell_SecondaryTrayWnd" {
            out.push(hwnd);
        }
        windows::core::BOOL(1)
    }
    let mut handles: Vec<HWND> = Vec::new();
    unsafe {
        if let Ok(primary) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) {
            handles.push(primary);
        }
        let _ = EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    handles
        .into_iter()
        .filter_map(|hwnd| {
            let rect = rect_of(hwnd)?;
            if (rect.right - rect.left) < (rect.bottom - rect.top) {
                return None;
            }
            let tray = unsafe {
                FindWindowExW(Some(hwnd), None, w!("TrayNotifyWnd"), PCWSTR::null()).ok()
            }
            .and_then(rect_of)
            .filter(|t| t.right > t.left);
            let tray_left = tray
                .map(|t| t.left.clamp(rect.left, rect.right))
                .unwrap_or(rect.right);
            Some(Bar {
                hwnd,
                rect,
                tray_left,
                monitor: monitor_of(hwnd).map(|m| m.0).unwrap_or_default(),
            })
        })
        .collect()
}

/// The bar on the named monitor, falling back to the primary bar.
pub fn bar_for(monitor: Option<&str>) -> Option<Bar> {
    pick_bar(&taskbars(), monitor).cloned()
}

/// `bar_for` on a list from `taskbars`, so a caller that needs several
/// lookups enumerates the windows once.
pub fn pick_bar<'a>(bars: &'a [Bar], monitor: Option<&str>) -> Option<&'a Bar> {
    monitor
        .and_then(|m| bars.iter().find(|b| b.monitor == m))
        .or(bars.first())
}

/// Device name and full rectangle of the monitor a window is on.
fn monitor_of(hwnd: HWND) -> Option<(String, RECT)> {
    monitor_info(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) })
}

/// Full rectangle of the monitor under a screen point.
pub fn monitor_rect_at(pt: POINT) -> Option<RECT> {
    monitor_info(unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) }).map(|m| m.1)
}

fn monitor_info(monitor: HMONITOR) -> Option<(String, RECT)> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    unsafe {
        GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut _)
            .ok()
            .ok()?
    };
    let len = info
        .szDevice
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(info.szDevice.len());
    Some((
        String::from_utf16_lossy(&info.szDevice[..len]),
        info.monitorInfo.rcMonitor,
    ))
}

pub fn intersection(a: RECT, b: RECT) -> Option<RECT> {
    let r = RECT {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    };
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

/// Share of `a`'s area that lies inside `b`, 0.0 to 1.0.
pub fn overlap_ratio(a: RECT, b: RECT) -> f64 {
    let Some(i) = intersection(a, b) else {
        return 0.0;
    };
    let area = |r: RECT| ((r.right - r.left) as f64) * ((r.bottom - r.top) as f64);
    area(i) / area(a).max(1.0)
}

/// Where a docked pill of size `w` x `h` sits: `tray_offset` pixels left of
/// the notification area, centered on the bar's height, kept on the bar.
pub fn dock_xy(bar: &Bar, w: i32, h: i32, tray_offset: i32) -> (i32, i32) {
    let max_x = (bar.tray_left - w).max(bar.rect.left);
    let x = (bar.tray_left - tray_offset.max(0) - w).clamp(bar.rect.left, max_x);
    let y = bar.rect.top + (bar.rect.bottom - bar.rect.top - h) / 2;
    (x, y)
}

/// Hook the window procedure, once, on the window's own thread, to stop two
/// things winit does to an undecorated window:
///
/// - It moves the client area one pixel down (its way of getting a drop
///   shadow; egui-winit always turns it on). That leaves a dead strip along
///   the pill's top, seen on the taskbar as black corner pixels and a
///   see-through line, and cuts off the pill's bottom row. WM_NCCALCSIZE is
///   answered here instead: client area = window.
/// - It rewrites the window style whenever it resizes or restyles the window,
///   which would turn a hosted child pill back into a popup. While the pill
///   is hosted (`embed`), WM_STYLECHANGING puts WS_CHILD back.
pub fn hook_window(hwnd: HWND) {
    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_NCCALCSIZE && wparam.0 != 0 {
            return LRESULT(0);
        }
        if msg == WM_STYLECHANGING
            && wparam.0 as i32 == GWL_STYLE.0
            && HOST.load(std::sync::atomic::Ordering::Relaxed) != 0
        {
            let change = &mut *(lparam.0 as *mut STYLESTRUCT);
            change.styleNew =
                (borderless(change.styleNew) & !WS_POPUP) | WS_CHILD | WS_CLIPSIBLINGS;
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(proc), 1, 0);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Make the window a free top-level, always-on-top popup, keeping its place
/// on screen. This is the state for floating and for the whole of a drag.
pub fn make_popup(hwnd: HWND) {
    let rect = screen_rect(hwnd);
    HOST.store(0, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        // Detach before clearing WS_CHILD, as SetParent documents.
        if is_child(hwnd) {
            let _ = SetParent(hwnd, None);
        }
    }
    set_style(
        hwnd,
        (borderless(style(hwnd)) & !(WS_CHILD | WS_CLIPSIBLINGS)) | WS_POPUP,
    );
    apply_ex(hwnd);
    set_ex_style(hwnd, ex_style(hwnd) | WS_EX_TOPMOST);
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Move a pill to a screen position. A hosted pill is positioned in its
/// host's coordinates and kept in front of the host's other children; a free
/// one is kept topmost.
pub fn place(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        match parent_of(hwnd) {
            Some(host) => {
                let mut pts = [POINT { x, y }];
                MapWindowPoints(None, Some(host), &mut pts);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOP),
                    pts[0].x,
                    pts[0].y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            None => {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }
}

/// Debug builds only: when `BTW_FAKE_INPUT` names a file holding
/// `x y down title-word` (`down` is 0 or 1), the cursor position and button
/// state come from it, for the pill whose window title ends in that word.
/// This lets the drag logic be driven on a session that refuses injected
/// mouse input (a minimized RDP window). Release builds contain none of it.
#[cfg(debug_assertions)]
fn fake_input() -> Option<(POINT, bool)> {
    static MINE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let mine = MINE.get_or_init(|| {
        std::env::args()
            .find_map(|a| a.strip_prefix("--widget=").map(str::to_owned))
            .unwrap_or_default()
    });
    let text = std::fs::read_to_string(std::env::var_os("BTW_FAKE_INPUT")?).ok()?;
    let mut parts = text.split_whitespace();
    let mut n = parts.by_ref().take(3).map(|v| v.parse::<i32>());
    let (x, y, down) = (n.next()?.ok()?, n.next()?.ok()?, n.next()?.ok()?);
    parts
        .next()?
        .eq_ignore_ascii_case(mine)
        .then_some((POINT { x, y }, down != 0))
}

#[cfg(not(debug_assertions))]
fn fake_input() -> Option<(POINT, bool)> {
    None
}

/// Debug builds only: the fake input file reports a press inside this window.
pub fn fake_press_on(hwnd: HWND) -> bool {
    match (fake_input(), rect_of(hwnd)) {
        (Some((pt, true)), Some(r)) => {
            pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
        }
        _ => false,
    }
}

pub fn cursor() -> POINT {
    if let Some((pt, _)) = fake_input() {
        return pt;
    }
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    pt
}

/// Whether the primary mouse button is physically held, read from the system
/// rather than from window messages so a drag cannot get stuck if the
/// button-up event goes elsewhere.
pub fn primary_button_down() -> bool {
    if let Some((_, down)) = fake_input() {
        return down;
    }
    unsafe {
        let key = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
            VK_RBUTTON
        } else {
            VK_LBUTTON
        };
        (GetAsyncKeyState(key.0 as i32) as u16 & 0x8000) != 0
    }
}

/// How far the cursor must travel before a press becomes a drag.
pub fn drag_threshold() -> (i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_CXDRAG).max(1),
            GetSystemMetrics(SM_CYDRAG).max(1),
        )
    }
}

/// Frame styling via DWM. Free-floating pills get rounded corners, the normal
/// shadow and the system border. On the taskbar all three are removed: they
/// would outline the pill against the bar (the border shows as a white line
/// along the pill's top). A window region (SetWindowRgn) breaks the
/// flip-model swapchain, so use compositor attributes instead.
pub fn round_corners(hwnd: HWND, round: bool) {
    const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
    let corner: u32 = if round {
        DWMWCP_ROUND.0
    } else {
        DWMWCP_DONOTROUND.0
    } as u32;
    let nc: i32 = if round {
        DWMNCRP_ENABLED.0
    } else {
        DWMNCRP_DISABLED.0
    };
    let border: u32 = if round {
        DWMWA_COLOR_DEFAULT
    } else {
        DWMWA_COLOR_NONE
    };
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            4,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &nc as *const _ as *const _,
            4,
        );
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &border as *const _ as *const _, 4);
    }
}

pub fn set_visible(hwnd: HWND, visible: bool) {
    unsafe {
        // Compare this window's own style, not IsWindowVisible (which also
        // includes its auto-hidden taskbar parent's visibility).
        if (GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0) != visible {
            let _ = ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }
}

pub fn open_url(url: &str) {
    let op = wide("open");
    let target = wide(url);
    unsafe {
        let _ = ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SHOW_WINDOW_CMD(5),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_window_follows_its_host_and_failed_embedding_restores_popup() {
        // Exercise only disposable windows owned by this test, never Explorer.
        unsafe {
            let create = || {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    PCWSTR::null(),
                    WINDOW_STYLE(WS_POPUP),
                    100,
                    100,
                    300,
                    48,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap()
            };
            let host = create();
            let child = create();
            hook_window(child);
            assert!(embed(child, host));
            assert_eq!(parent_of(child), Some(host));
            let before = screen_rect(child);
            SetWindowPos(
                host,
                None,
                100,
                160,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            let after = screen_rect(child);
            assert_eq!(after.top - before.top, 60);
            DestroyWindow(host).unwrap();
            assert!(!IsWindow(Some(child)).as_bool());

            let child = create();
            hook_window(child);
            // NULL is not a usable embedding host. Use the destroyed handle
            // after creating the child, so it cannot refer to that child.
            let stale_host = create();
            DestroyWindow(stale_host).unwrap();
            assert!(!embed(child, stale_host));
            assert_eq!(parent_of(child), None);
            assert_eq!(style(child) & WS_CHILD, 0);
            assert_eq!(HOST.load(std::sync::atomic::Ordering::Relaxed), 0);
            DestroyWindow(child).unwrap();
        }
    }

    fn bar(left: i32, right: i32, tray_left: i32) -> Bar {
        Bar {
            hwnd: HWND(std::ptr::null_mut()),
            rect: RECT {
                left,
                top: 1392,
                right,
                bottom: 1440,
            },
            tray_left,
            monitor: String::new(),
        }
    }

    #[test]
    fn docks_left_of_the_tray_and_stays_on_the_bar() {
        let b = bar(0, 3440, 3100);
        assert_eq!(dock_xy(&b, 150, 48, 8), (2942, 1392));
        assert_eq!(dock_xy(&b, 150, 48, 0), (2950, 1392));
        // An offset larger than the bar pins the pill to the bar's left edge.
        assert_eq!(dock_xy(&b, 150, 48, 99_999), (0, 1392));
        // A negative offset cannot push it over the tray.
        assert_eq!(dock_xy(&b, 150, 48, -50), (2950, 1392));
        // A shorter pill is centered on the bar's height.
        assert_eq!(dock_xy(&b, 150, 40, 0).1, 1396);
    }

    #[test]
    fn overlap() {
        let a = RECT {
            left: 0,
            top: 0,
            right: 100,
            bottom: 48,
        };
        assert_eq!(overlap_ratio(a, a), 1.0);
        assert_eq!(
            overlap_ratio(
                a,
                RECT {
                    left: 50,
                    top: 0,
                    right: 500,
                    bottom: 48
                }
            ),
            0.5
        );
        assert_eq!(
            overlap_ratio(
                a,
                RECT {
                    left: 0,
                    top: 24,
                    right: 100,
                    bottom: 72
                }
            ),
            0.5
        );
        assert_eq!(
            overlap_ratio(
                a,
                RECT {
                    left: 200,
                    top: 0,
                    right: 300,
                    bottom: 48
                }
            ),
            0.0
        );
    }
}
