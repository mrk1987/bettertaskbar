//! Which parts of a taskbar are taken (Start, pinned and running apps, the
//! notification area), so a dragged pill only docks in a real gap.
//!
//! Windows 11 draws the taskbar with XAML, so its buttons are not child
//! windows; UI Automation is the only way to get their rectangles. That is
//! slow and can block on Explorer. Magnetic layout reuses a sample until the
//! set of application windows changes, the taskbar moves, or it is clicked.
//! A slow fallback catches shell changes that have no native window event.
//! The approach follows Claude-Code-Usage-Monitor
//! (CodeZeno, MIT), `src/window/taskbar_collision.rs`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongPtrW, IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

use crate::shell;

/// A sample older than this says nothing about the bar as it is now.
const MAX_AGE: Duration = Duration::from_secs(65);
const SAMPLE_EVERY: Duration = Duration::from_millis(400);
const IDLE_REFRESH: Duration = Duration::from_secs(30);
const CHANGE_REFRESH: Duration = Duration::from_secs(1);

struct Sample {
    bar: isize,
    bounds: RECT,
    occupied: Vec<RECT>,
    at: Instant,
}

static SAMPLES: Mutex<Vec<Sample>> = Mutex::new(Vec::new());
static WANTED: AtomicBool = AtomicBool::new(false);
static MAGNETIC: AtomicBool = AtomicBool::new(false);
static START: Once = Once::new();

/// Turn sampling on or off. The reader thread starts on first use.
pub fn set_active(active: bool) {
    WANTED.store(active, Ordering::Relaxed);
    if active {
        START.call_once(|| {
            std::thread::spawn(reader);
        });
    }
}

/// The hub needs layout changes, not a continuous accessibility-tree scan.
pub fn set_magnetic(active: bool) {
    MAGNETIC.store(active, Ordering::Relaxed);
    if active {
        START.call_once(|| {
            std::thread::spawn(reader);
        });
    }
}

fn refresh_due(elapsed: Duration, dragging: bool, changed: bool) -> bool {
    elapsed
        >= if dragging {
            SAMPLE_EVERY
        } else if changed {
            CHANGE_REFRESH
        } else {
            IDLE_REFRESH
        }
}

/// Enumerate handles only. Never traverse other applications' child trees.
fn application_windows() -> Vec<isize> {
    unsafe extern "system" fn collect(hwnd: HWND, param: LPARAM) -> windows::core::BOOL {
        if IsWindowVisible(hwnd).as_bool()
            && GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 == 0
        {
            (&mut *(param.0 as *mut Vec<isize>)).push(hwnd.0 as isize);
        }
        windows::core::BOOL(1)
    }
    let mut handles = Vec::<isize>::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    // Z-order changes do not change the space used by taskbar buttons.
    handles.sort_unstable();
    handles
}

/// Occupied rectangles on a taskbar, or `None` when there is no fresh sample
/// (reader still starting, Explorer not answering, bar moved since).
pub fn occupied(bar: HWND, bounds: RECT) -> Option<Vec<RECT>> {
    let samples = SAMPLES.lock().unwrap_or_else(|e| e.into_inner());
    samples
        .iter()
        .find(|s| s.bar == bar.0 as isize && s.bounds == bounds && s.at.elapsed() <= MAX_AGE)
        .map(|s| s.occupied.clone())
}

/// Gaps along a horizontal bar that `occupied` leaves free, as (left, right).
/// Only rectangles that share the pill's row count.
pub fn free_slots(bounds: RECT, occupied: &[RECT], top: i32, bottom: i32) -> Vec<(i32, i32)> {
    let mut taken: Vec<(i32, i32)> = occupied
        .iter()
        .filter(|r| r.top < bottom && top < r.bottom)
        .map(|r| {
            (
                r.left.clamp(bounds.left, bounds.right),
                r.right.clamp(bounds.left, bounds.right),
            )
        })
        .collect();
    taken.sort_unstable();
    let mut gaps = Vec::new();
    let mut cursor = bounds.left;
    for (left, right) in taken
        .into_iter()
        .chain(std::iter::once((bounds.right, bounds.right)))
    {
        if left > cursor {
            gaps.push((cursor, left));
        }
        cursor = cursor.max(right);
    }
    gaps
}

/// One taskbar and what could be read from it.
pub type BarRead = (shell::Bar, Result<Option<Vec<RECT>>, String>);

/// Read every taskbar once, on the calling thread. For `--check=taskbar`.
pub fn read_now() -> Vec<BarRead> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    shell::taskbars()
        .into_iter()
        .map(|bar| {
            let read = read(bar.hwnd, bar.rect)
                .map(|s| s.map(|s| s.occupied))
                .map_err(|e| e.to_string());
            (bar, read)
        })
        .collect()
}

fn reader() {
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            crate::log::line(
                "taskbar",
                "UI Automation unavailable (COM init failed); docking will not avoid app icons",
            );
            return;
        }
    }
    let mut reported = false;
    let mut last_read: Option<Instant> = None;
    let mut previous_windows = Vec::new();
    let mut previous_bars = Vec::new();
    let mut changed = false;
    let mut retry_needed = false;
    let mut pressed_on_bar = false;
    loop {
        let dragging = WANTED.load(Ordering::Relaxed);
        if !dragging && !MAGNETIC.load(Ordering::Relaxed) {
            last_read = None;
            std::thread::sleep(Duration::from_millis(150));
            continue;
        }
        let bars = shell::taskbars();
        let geometry: Vec<_> = bars
            .iter()
            .map(|b| (b.hwnd.0 as isize, b.rect, b.tray_left))
            .collect();
        changed |= geometry != previous_bars;
        let windows = application_windows();
        changed |= windows != previous_windows;
        previous_windows = windows;
        previous_bars = geometry;
        let cursor = shell::cursor();
        let down = shell::primary_button_down();
        if down
            && bars.iter().any(|b| {
                cursor.x >= b.rect.left
                    && cursor.x < b.rect.right
                    && cursor.y >= b.rect.top
                    && cursor.y < b.rect.bottom
            })
        {
            pressed_on_bar = true;
        }
        if !down && pressed_on_bar {
            changed = true;
            pressed_on_bar = false;
        }
        if last_read.is_some_and(|at| {
            if retry_needed {
                at.elapsed() < Duration::from_secs(5)
            } else {
                !refresh_due(at.elapsed(), dragging, changed)
            }
        }) {
            std::thread::sleep(Duration::from_millis(150));
            continue;
        }
        let expected = bars.len();
        let next: Vec<Sample> = bars
            .into_iter()
            .filter_map(|bar| match read(bar.hwnd, bar.rect) {
                Ok(sample) => sample,
                Err(e) => {
                    if !reported {
                        reported = true;
                        crate::log::line("taskbar", &format!("cannot read taskbar layout: {e}"));
                    }
                    None
                }
            })
            .collect();
        retry_needed = next.len() != expected;
        *SAMPLES.lock().unwrap_or_else(|e| e.into_inner()) = next;
        last_read = Some(Instant::now());
        changed = false;
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Control types that take up room on the bar. Roles rather than names or
/// class names, which differ between Windows builds and languages.
fn takes_space(role: UIA_CONTROLTYPE_ID) -> bool {
    [
        UIA_ButtonControlTypeId,
        UIA_SplitButtonControlTypeId,
        UIA_CheckBoxControlTypeId,
        UIA_RadioButtonControlTypeId,
        UIA_ListItemControlTypeId,
        UIA_TabItemControlTypeId,
        UIA_MenuItemControlTypeId,
        UIA_EditControlTypeId,
    ]
    .contains(&role)
}

fn read(bar: HWND, bounds: RECT) -> windows::core::Result<Option<Sample>> {
    unsafe {
        if !IsWindowVisible(bar).as_bool() {
            return Ok(None);
        }
        let automation: IUIAutomation2 =
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        automation.SetConnectionTimeout(500)?;
        automation.SetTransactionTimeout(1000)?;
        let request = automation.CreateCacheRequest()?;
        request.SetTreeScope(TreeScope_Element)?;
        request.SetAutomationElementMode(AutomationElementMode_None)?;
        for property in [
            UIA_BoundingRectanglePropertyId,
            UIA_IsOffscreenPropertyId,
            UIA_ControlTypePropertyId,
        ] {
            request.AddProperty(property)?;
        }
        let everything = automation.CreateTrueCondition()?;
        let root = automation.ElementFromHandle(bar)?;
        let elements = root.FindAllBuildCache(TreeScope_Descendants, &everything, &request)?;

        let mut occupied = Vec::new();
        for i in 0..elements.Length()? {
            let element = elements.GetElement(i)?;
            if !takes_space(element.CachedControlType()?) || element.CachedIsOffscreen()?.as_bool()
            {
                continue;
            }
            if let Some(rect) = shell::intersection(element.CachedBoundingRectangle()?, bounds) {
                occupied.push(rect);
            }
        }
        // No controls at all means the tree could not be read, not that the
        // bar is empty.
        if occupied.is_empty() {
            return Ok(None);
        }
        // Discard a sample taken while the bar was moving (auto-hide, DPI).
        if shell::rect_of(bar) != Some(bounds) {
            return Ok(None);
        }
        Ok(Some(Sample {
            bar: bar.0 as isize,
            bounds,
            occupied,
            at: Instant::now(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_magnetic_layout_does_not_scan_continuously() {
        assert!(!refresh_due(Duration::from_secs(29), false, false));
        assert!(refresh_due(Duration::from_secs(30), false, false));
        assert!(!refresh_due(Duration::from_millis(900), false, true));
        assert!(refresh_due(Duration::from_secs(1), false, true));
        assert!(refresh_due(Duration::from_millis(400), true, false));
    }

    fn r(left: i32, right: i32) -> RECT {
        RECT {
            left,
            top: 0,
            right,
            bottom: 48,
        }
    }

    #[test]
    fn gaps_on_both_sides_of_centred_apps_and_before_the_tray() {
        let bar = r(0, 3440);
        // Widgets button, centred app group (overlapping rects), tray.
        let occupied = [r(0, 180), r(1200, 1700), r(1650, 2240), r(3100, 3440)];
        assert_eq!(
            free_slots(bar, &occupied, 0, 48),
            vec![(180, 1200), (2240, 3100)]
        );
    }

    #[test]
    fn empty_bar_is_one_gap_and_full_bar_none() {
        assert_eq!(free_slots(r(0, 1000), &[], 0, 48), vec![(0, 1000)]);
        assert_eq!(free_slots(r(0, 1000), &[r(-50, 1200)], 0, 48), vec![]);
    }

    #[test]
    fn other_rows_do_not_count() {
        let above = RECT {
            left: 100,
            top: -200,
            right: 900,
            bottom: -100,
        };
        assert_eq!(free_slots(r(0, 1000), &[above], 0, 48), vec![(0, 1000)]);
    }

    #[test]
    fn secondary_monitor_origin() {
        let bar = RECT {
            left: -1920,
            top: 1032,
            right: 0,
            bottom: 1080,
        };
        let occupied = [RECT {
            left: -1100,
            top: 1032,
            right: -800,
            bottom: 1080,
        }];
        assert_eq!(
            free_slots(bar, &occupied, 1032, 1080),
            vec![(-1920, -1100), (-800, 0)]
        );
    }
}
