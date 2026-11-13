//! Minimal tray icon + popup menu on a dedicated message-loop thread.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{Placement, SharedConfig};

const TRAY_MSG: u32 = WM_APP + 1;
const ID_WEATHER: usize = 1;
const ID_CALENDAR: usize = 2;
const ID_MAIL: usize = 3;
const ID_PIN_ALL: usize = 4;
const ID_DETACH_ALL: usize = 5;
const ID_DESKTOP_ALL: usize = 6;
const ID_QUIT: usize = 9;
const ID_DASHBOARD: usize = 10;

struct TrayState {
    cfg: SharedConfig,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn state(hwnd: HWND) -> &'static TrayState {
    unsafe { &*(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const TrayState) }
}

fn command(hwnd: HWND, id: usize) {
    if id == ID_DASHBOARD {
        crate::dashboard::open(crate::dashboard::Page::Widgets);
        return;
    }
    if id == ID_QUIT {
        crate::quit_all();
        return;
    }
    // `update` re-reads the file first, so the hub never writes back a copy
    // that predates a drag or an edit made elsewhere.
    crate::config::update(&state(hwnd).cfg, |c| match id {
        ID_WEATHER => {
            let state = &mut c.weather_widget;
            if !state.enabled {
                state.enabled = true;
                state.visible = true;
            } else {
                state.visible = !state.visible;
            }
        }
        ID_CALENDAR => {
            let state = &mut c.calendar_widget;
            if !state.enabled {
                state.enabled = true;
                state.visible = true;
            } else {
                state.visible = !state.visible;
            }
        }
        ID_MAIL => {
            let state = &mut c.mail_widget;
            if !state.enabled {
                state.enabled = true;
                state.visible = true;
            } else {
                state.visible = !state.visible;
            }
        }
        ID_PIN_ALL | ID_DESKTOP_ALL | ID_DETACH_ALL => {
            let placement = match id {
                ID_PIN_ALL => Placement::Taskbar,
                ID_DESKTOP_ALL => Placement::Desktop,
                _ => Placement::Floating,
            };
            for kind in crate::app::Kind::ALL {
                kind.state_mut(c).placement = placement;
            }
        }
        _ => {}
    });
}

fn show_menu(hwnd: HWND) {
    let st = state(hwnd);
    crate::config::refresh(&st.cfg);
    let (w, c, m) = {
        let cfg = st.cfg.lock().unwrap();
        (
            cfg.weather_widget.visible,
            cfg.calendar_widget.visible,
            cfg.mail_widget.visible,
        )
    };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let check = |on: bool| {
            if on {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            }
        };
        let l_dashboard = wide("Dashboard…");
        let _ = AppendMenuW(menu, MF_STRING, ID_DASHBOARD, PCWSTR(l_dashboard.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let l_weather = wide("Weather");
        let l_calendar = wide("Calendar");
        let l_mail = wide("Mail");
        let l_pin = wide("Pin all to taskbar");
        let l_desktop = wide("Pin all to desktop");
        let l_detach = wide("Float all on top");
        let l_quit = wide("Quit");
        let _ = AppendMenuW(menu, check(w), ID_WEATHER, PCWSTR(l_weather.as_ptr()));
        let _ = AppendMenuW(menu, check(c), ID_CALENDAR, PCWSTR(l_calendar.as_ptr()));
        let _ = AppendMenuW(menu, check(m), ID_MAIL, PCWSTR(l_mail.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_PIN_ALL, PCWSTR(l_pin.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_DESKTOP_ALL, PCWSTR(l_desktop.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_DETACH_ALL, PCWSTR(l_detach.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, PCWSTR(l_quit.as_ptr()));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        m if m == TRAY_MSG => {
            if lparam.0 as u32 == WM_RBUTTONUP {
                show_menu(hwnd);
            } else if lparam.0 as u32 == WM_LBUTTONUP {
                crate::dashboard::open(crate::dashboard::Page::Widgets);
            }
            LRESULT(0)
        }
        m if m == taskbar_created() => {
            // Explorer restarted and lost every tray icon.
            add_icon(hwnd);
            LRESULT(0)
        }
        WM_COMMAND => {
            command(hwnd, wparam.0 & 0xFFFF);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // The last message the window gets: free the state set in `spawn`.
            let state = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut TrayState;
            if !state.is_null() {
                drop(Box::from_raw(state));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            // Remove the icon now; otherwise it lingers until the mouse passes over it.
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 32x32 alpha icon: purple rounded tile with three white dots (the widget trio).
fn make_tray_icon() -> HICON {
    unsafe {
        const N: i32 = 32;
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = N;
        bmi.bmiHeader.biHeight = -N; // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(color) = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            return LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
        };
        let px = std::slice::from_raw_parts_mut(bits as *mut u8, (N * N * 4) as usize);
        let dots = [(10.0f32, 16.0f32), (16.0, 16.0), (22.0, 16.0)];
        for y in 0..N {
            for x in 0..N {
                let cx = x as f32 + 0.5;
                let cy = y as f32 + 0.5;
                // Rounded-box signed distance: half-size 14, corner radius 7, centered at 16.
                let qx = (cx - 16.0).abs() - 7.0;
                let qy = (cy - 16.0).abs() - 7.0;
                let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.min(0.0).max(qy.min(0.0)) - 7.0;
                let tile_a = (0.5 - d).clamp(0.0, 1.0);
                let i = ((y * N + x) * 4) as usize;
                if tile_a <= 0.0 {
                    px[i] = 0;
                    px[i + 1] = 0;
                    px[i + 2] = 0;
                    px[i + 3] = 0;
                    continue;
                }
                let mut dot_a = 0.0f32;
                for &(dx, dy) in &dots {
                    dot_a = dot_a.max((3.1 - (cx - dx).hypot(cy - dy)).clamp(0.0, 1.0));
                }
                let r = 0x6D as f32 * (1.0 - dot_a) + 255.0 * dot_a;
                let g = 0x4A as f32 * (1.0 - dot_a) + 255.0 * dot_a;
                let b = 0xFF as f32 * (1.0 - dot_a) + 255.0 * dot_a;
                px[i] = b as u8;
                px[i + 1] = g as u8;
                px[i + 2] = r as u8;
                px[i + 3] = (tile_a * 255.0) as u8;
            }
        }
        // All-zero AND mask: Windows uses the color bitmap's per-pixel alpha.
        let mask_bits = [0u8; 128];
        let mask = CreateBitmap(
            N,
            N,
            1,
            1,
            Some(mask_bits.as_ptr() as *const core::ffi::c_void),
        );
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon.unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default())
    }
}

/// The message Explorer broadcasts when it has (re)created the taskbar.
fn taskbar_created() -> u32 {
    static ID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

fn add_icon(hwnd: HWND) {
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = TRAY_MSG;
        nid.hIcon = make_tray_icon();
        let tip = wide("BetterTaskbar Widgets");
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

pub fn spawn(cfg: SharedConfig) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = w!("BTWidgetsTray");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        // Hidden top-level window (not message-only) so other processes can
        // find it by title and request shutdown with WM_CLOSE.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!("BetterTaskbar Widgets"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
        .unwrap();
        let st = Box::new(TrayState { cfg });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);

        add_icon(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    })
}
