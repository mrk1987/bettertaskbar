//! Minimal tray icon + popup menu on a dedicated message-loop thread.

use crate::i18n::tr;
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

use crate::app::Kind;
use crate::config::{Placement, SharedConfig};

const TRAY_MSG: u32 = WM_APP + 1;
const ID_PIN_ALL: usize = 4;
const ID_DETACH_ALL: usize = 5;
const ID_DESKTOP_ALL: usize = 6;
const ID_QUIT: usize = 9;
const ID_DASHBOARD: usize = 10;
/// Widget toggles are `ID_WIDGET + Kind::index()`.
const ID_WIDGET: usize = 100;

struct TrayState {
    cfg: SharedConfig,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn state(hwnd: HWND) -> &'static TrayState {
    unsafe { &*(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const TrayState) }
}

/// The placement every enabled widget shares, if they all share one.
pub(crate) fn common_placement(c: &crate::config::Config) -> Option<Placement> {
    let mut enabled = Kind::ALL
        .into_iter()
        .map(|k| k.state(c))
        .filter(|s| s.enabled)
        .map(|s| s.placement);
    let first = enabled.next()?;
    enabled.all(|p| p == first).then_some(first)
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
        ID_PIN_ALL | ID_DESKTOP_ALL | ID_DETACH_ALL => {
            let placement = match id {
                ID_PIN_ALL => Placement::Taskbar,
                ID_DESKTOP_ALL => Placement::Desktop,
                _ => Placement::Floating,
            };
            for kind in Kind::ALL {
                kind.state_mut(c).placement = placement;
            }
        }
        _ => {
            let Some(kind) = id.checked_sub(ID_WIDGET).and_then(|i| Kind::ALL.get(i)) else {
                return;
            };
            let state = kind.state_mut(c);
            if !state.enabled {
                state.enabled = true;
                state.visible = true;
            } else {
                state.visible = !state.visible;
            }
        }
    });
}

fn show_menu(hwnd: HWND) {
    let st = state(hwnd);
    crate::config::refresh(&st.cfg);
    let cfg = st.cfg.lock().unwrap().clone();
    crate::i18n::apply(cfg.language);
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let Ok(usage) = CreatePopupMenu() else {
            let _ = DestroyMenu(menu);
            return;
        };
        // Labels must outlive TrackPopupMenu.
        let mut labels: Vec<Vec<u16>> = Vec::new();
        let mut add = |menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str| {
            labels.push(wide(text));
            let _ = AppendMenuW(menu, flags, id, PCWSTR(labels.last().unwrap().as_ptr()));
        };
        let check = |on: bool| {
            if on {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            }
        };
        add(menu, MF_STRING, ID_DASHBOARD, tr("Dashboard…"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        for kind in Kind::ALL {
            let state = kind.state(&cfg);
            let on = check(state.enabled && state.visible);
            let id = ID_WIDGET + kind.index();
            match kind {
                Kind::Usage(_) => add(usage, on, id, kind.label()),
                _ => add(menu, on, id, kind.label()),
            }
        }
        labels.push(wide(tr("AI usage")));
        let _ = AppendMenuW(
            menu,
            MF_POPUP,
            usage.0 as usize,
            PCWSTR(labels.last().unwrap().as_ptr()),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let common = common_placement(&cfg);
        let mut placement = |id: usize, p: Placement, text: &str| {
            labels.push(wide(text));
            let _ = AppendMenuW(
                menu,
                check(common == Some(p)),
                id,
                PCWSTR(labels.last().unwrap().as_ptr()),
            );
            if common == Some(p) {
                let _ = CheckMenuRadioItem(menu, id as u32, id as u32, id as u32, MF_BYCOMMAND.0);
            }
        };
        placement(ID_PIN_ALL, Placement::Taskbar, tr("Pin all to taskbar"));
        placement(ID_DESKTOP_ALL, Placement::Desktop, tr("Pin all to desktop"));
        placement(ID_DETACH_ALL, Placement::Floating, tr("Float all on top"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        labels.push(wide(tr("Quit")));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            ID_QUIT,
            PCWSTR(labels.last().unwrap().as_ptr()),
        );
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        // Destroys the AI usage submenu with it.
        let _ = DestroyMenu(menu);
        drop(labels);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_is_checked_only_when_every_enabled_widget_shares_it() {
        let mut c = crate::config::Config::default();
        for kind in Kind::ALL {
            kind.state_mut(&mut c).placement = Placement::Taskbar;
        }
        assert_eq!(common_placement(&c), Some(Placement::Taskbar));
        // A disabled widget elsewhere does not count.
        Kind::News.state_mut(&mut c).enabled = false;
        Kind::News.state_mut(&mut c).placement = Placement::Floating;
        assert_eq!(common_placement(&c), Some(Placement::Taskbar));
        Kind::Weather.state_mut(&mut c).placement = Placement::Desktop;
        assert_eq!(common_placement(&c), None);
    }
}
