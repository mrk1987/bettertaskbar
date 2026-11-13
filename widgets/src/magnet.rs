//! One hub owns docked positions, so independent widget timers cannot fight.
use crate::{
    app::Kind,
    config::{self, DockSide, Placement, SharedConfig},
    occupancy, shell,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Mutex};
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow},
};

#[derive(Default, Serialize, Deserialize)]
struct Status {
    ready: Vec<String>,
    problems: BTreeMap<String, String>,
}
static LAST: Mutex<String> = Mutex::new(String::new());
fn status_path() -> std::path::PathBuf {
    config::dir().join("dock-status.json")
}
fn publish(status: &Status) {
    let Ok(text) = serde_json::to_string(status) else {
        return;
    };
    let mut last = LAST.lock().unwrap();
    if *last == text {
        return;
    }
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(config::dir())?;
        let path = status_path();
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &text)?;
        std::fs::rename(tmp, path)
    };
    if write().is_ok() {
        *last = text;
    }
}
pub fn reset() {
    LAST.lock().unwrap().clear();
    publish(&Status::default());
}
fn read() -> Status {
    std::fs::read(status_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
pub fn ready(kind: Kind) -> bool {
    read().ready.iter().any(|k| k == kind.key())
}
pub fn message(kind: Kind) -> Option<String> {
    read().problems.remove(kind.key())
}

/// Split each remaining free interval as it is used. Right groups fill from
/// the right; left groups fill from the left. Overflow uses the next free gap.
fn pack(slots: &[(i32, i32)], items: &[(i32, DockSide)], gap: i32) -> Vec<Option<i32>> {
    let mut free = slots.to_vec();
    let mut result = vec![None; items.len()];
    for side in [DockSide::Right, DockSide::Left] {
        for (i, &(width, wanted)) in items.iter().enumerate() {
            if wanted != side || width <= 0 {
                continue;
            }
            let index = match side {
                DockSide::Right => free.iter().rposition(|&(l, r)| r - l >= width),
                DockSide::Left => free.iter().position(|&(l, r)| r - l >= width),
            };
            if let Some(index) = index {
                let (left, right) = free[index];
                let x = match side {
                    DockSide::Right => right - width,
                    DockSide::Left => left,
                };
                result[i] = Some(x);
                match side {
                    DockSide::Right => free[index].1 = (x - gap).max(left),
                    DockSide::Left => free[index].0 = (x + width + gap).min(right),
                }
            }
        }
    }
    result
}

fn side_at(slots: &[(i32, i32)], x: i32, width: i32) -> DockSide {
    let usable: Vec<_> = slots
        .iter()
        .copied()
        .filter(|(l, r)| r - l >= width)
        .collect();
    if usable.len() < 2 {
        return DockSide::Right;
    }
    let center = x + width / 2;
    let index = usable
        .iter()
        .enumerate()
        .min_by_key(|item| {
            let (l, r) = *item.1;
            if center < l {
                l - center
            } else if center > r {
                center - r
            } else {
                0
            }
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    if index == 0 {
        DockSide::Left
    } else {
        DockSide::Right
    }
}
pub fn side_for_drop(bar: &shell::Bar, x: i32, width: i32) -> Option<DockSide> {
    let mut taken = occupancy::occupied(bar.hwnd, bar.rect)?;
    taken.push(windows::Win32::Foundation::RECT {
        left: bar.tray_left,
        right: bar.rect.right,
        ..bar.rect
    });
    Some(side_at(
        &occupancy::free_slots(bar.rect, &taken, bar.rect.top, bar.rect.bottom),
        x,
        width,
    ))
}

fn owned_window(kind: Kind, pid: u32) -> Option<HWND> {
    // Searching every application's entire child tree for every widget on
    // every supervisor tick is expensive. Keep verified handles until exit.
    static WINDOWS: Mutex<BTreeMap<&'static str, (u32, isize)>> = Mutex::new(BTreeMap::new());
    let mut cache = WINDOWS.lock().unwrap();
    if let Some(&(cached_pid, raw)) = cache.get(kind.key()) {
        let hwnd = HWND(raw as *mut _);
        let mut actual = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut actual));
        }
        if cached_pid == pid && actual == pid && unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Some(hwnd);
        }
        cache.remove(kind.key());
    }
    let hwnd = shell::find_by_title(kind.title())?;
    let mut actual = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut actual));
    }
    if actual != pid {
        return None;
    }
    cache.insert(kind.key(), (pid, hwnd.0 as isize));
    Some(hwnd)
}

pub fn arrange(cfg: &SharedConfig, children: &[(Kind, Option<std::process::Child>)]) {
    // Never reposition a widget under a press/drag, or another widget while
    // the user is deciding where to drop it.
    if shell::primary_button_down() {
        return;
    }
    let settings = cfg.lock().unwrap().clone();
    let mut status = Status::default();
    let windows: Vec<_> = children
        .iter()
        .filter_map(|(kind, child)| Some((*kind, owned_window(*kind, child.as_ref()?.id())?)))
        .collect();
    if !settings.magnetized {
        occupancy::set_magnetic(false);
        for (kind, hwnd) in windows {
            shell::set_visible(hwnd, kind.state(&settings).visible);
        }
        publish(&status);
        return;
    }
    if !windows.iter().any(|(kind, _)| {
        let state = kind.state(&settings);
        state.visible && state.placement == Placement::Taskbar
    }) {
        occupancy::set_magnetic(false);
        publish(&status);
        return;
    }
    occupancy::set_magnetic(true);
    let mut inferred = Vec::new();
    for bar in shell::taskbars() {
        let mut docked = Vec::new();
        let Some(mut taken) = occupancy::occupied(bar.hwnd, bar.rect) else {
            for (kind, _) in &windows {
                let state = kind.state(&settings);
                if state.placement == Placement::Taskbar
                    && state.visible
                    && shell::bar_for(state.monitor.as_deref())
                        .is_some_and(|target| target.hwnd == bar.hwnd)
                {
                    status
                        .problems
                        .insert(kind.key().into(), "Reading taskbar layout…".into());
                }
            }
            continue;
        };
        taken.push(windows::Win32::Foundation::RECT {
            left: bar.tray_left,
            right: bar.rect.right,
            ..bar.rect
        });
        for &(kind, hwnd) in &windows {
            let state = kind.state(&settings);
            let target = shell::bar_for(state.monitor.as_deref());
            if state.visible
                && state.placement == Placement::Taskbar
                && target
                    .as_ref()
                    .is_some_and(|target| target.hwnd == bar.hwnd)
                && shell::parent_of(hwnd) == Some(bar.hwnd)
            {
                docked.push((kind, hwnd, shell::screen_rect(hwnd)));
            } else if let Some(rect) = shell::visible_rect(hwnd) {
                taken.push(rect);
            }
        }
        let slots = occupancy::free_slots(bar.rect, &taken, bar.rect.top, bar.rect.bottom);
        let items: Vec<_> = docked
            .iter()
            .map(|(kind, _, rect)| {
                let state = kind.state(&settings);
                let width = rect.right - rect.left;
                let side = state.dock_side.unwrap_or_else(|| {
                    let x = state
                        .manual_bar_offset
                        .map(|offset| bar.rect.left + offset)
                        .or_else(|| {
                            state
                                .tray_offset
                                .map(|offset| bar.tray_left - offset - width)
                        })
                        .unwrap_or(rect.left);
                    let side = if state.manual_bar_offset.is_none()
                        && state.tray_offset.is_none()
                        && shell::intersection(*rect, bar.rect).is_none()
                    {
                        DockSide::Right
                    } else {
                        side_at(&slots, x, width)
                    };
                    inferred.push((*kind, side, state.tray_offset, state.manual_bar_offset));
                    side
                });
                (
                    if rect.bottom - rect.top > bar.rect.bottom - bar.rect.top {
                        0
                    } else {
                        width
                    },
                    side,
                )
            })
            .collect();
        // Two logical pixels between neighboring widgets, including at DPI
        // scale. Each docked pill adds its own 4 px inset on top.
        let gap = ((bar.rect.bottom - bar.rect.top) as f32 / 48.0 * 2.0)
            .round()
            .max(1.0) as i32;
        let positions = pack(&slots, &items, gap);
        for ((kind, hwnd, rect), x) in docked.into_iter().zip(positions) {
            match x {
                Some(x) => {
                    let y = bar.rect.top
                        + (bar.rect.bottom - bar.rect.top - (rect.bottom - rect.top)) / 2;
                    if (rect.left, rect.top) != (x, y) {
                        shell::place(hwnd, x, y);
                    }
                    shell::set_visible(hwnd, true);
                    status.ready.push(kind.key().into());
                    status.problems.remove(kind.key());
                }
                None => {
                    shell::set_visible(hwnd, false);
                    status.problems.insert(kind.key().into(),"Not enough free taskbar space. Move this widget, reduce its theme size, or unmagnetise.".into());
                }
            }
        }
    }
    publish(&status);
    if !inferred.is_empty() {
        config::update(cfg, |c| {
            if !c.magnetized {
                return;
            }
            for (kind, side, offset, manual) in inferred {
                let state = kind.state_mut(c);
                if state.dock_side.is_none()
                    && state.tray_offset == offset
                    && state.manual_bar_offset == manual
                {
                    state.dock_side = Some(side);
                }
            }
        });
    }
}

/// Read-only proof of the actual taskbar geometry, without account data.
pub fn diagnostics(settings: &config::Config) -> Result<String, String> {
    let layouts = occupancy::read_now();
    let mut visible = Vec::new();
    let mut hidden = 0;
    let mut native_overlaps = 0;
    for kind in Kind::ALL {
        let state = kind.state(settings);
        if !state.enabled || !state.visible || state.placement != Placement::Taskbar {
            continue;
        }
        let hwnd = shell::find_by_title(kind.title())
            .ok_or_else(|| format!("{} window not found", kind.label()))?;
        let Some(rect) = shell::visible_rect(hwnd) else {
            hidden += 1;
            continue;
        };
        let bar = shell::bar_for(state.monitor.as_deref()).ok_or("Taskbar not found")?;
        if shell::parent_of(hwnd) != Some(bar.hwnd) {
            return Err(format!("{} is not attached to the taskbar", kind.label()));
        }
        let (_, reading) = layouts
            .iter()
            .find(|(b, _)| b.hwnd == bar.hwnd)
            .ok_or("Taskbar layout unavailable")?;
        let occupied = reading
            .as_ref()
            .map_err(|_| "Taskbar layout read failed")?
            .as_ref()
            .ok_or("Taskbar layout unavailable")?;
        native_overlaps += occupied
            .iter()
            .filter(|r| shell::intersection(rect, **r).is_some())
            .count();
        if rect.right > bar.tray_left {
            native_overlaps += 1;
        }
        visible.push((kind, rect));
    }
    let mut widget_overlaps = 0;
    for (i, (_, a)) in visible.iter().enumerate() {
        for (_, b) in visible.iter().skip(i + 1) {
            if shell::intersection(*a, *b).is_some() {
                widget_overlaps += 1;
            }
        }
    }
    let mut summary = format!(
        "{} visible docked widgets, {} hidden; {} widget overlaps, {} taskbar-control overlaps",
        visible.len(),
        hidden,
        widget_overlaps,
        native_overlaps
    );
    visible.sort_by_key(|(_, r)| r.left);
    let mut previous: Option<i32> = None;
    for (kind, r) in &visible {
        summary.push_str(&format!(
            "\n  {:<10} x {}..{} (width {})",
            kind.key(),
            r.left,
            r.right,
            r.right - r.left
        ));
        if let Some(end) = previous {
            summary.push_str(&format!(", gap {}", r.left - end));
        }
        previous = Some(r.right);
    }
    if widget_overlaps + native_overlaps == 0 {
        Ok(summary)
    } else {
        Err(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_sides_pack_against_their_outer_edge() {
        let out = pack(
            &[(0, 400), (600, 1000)],
            &[
                (100, DockSide::Right),
                (120, DockSide::Right),
                (80, DockSide::Left),
                (50, DockSide::Left),
            ],
            4,
        );
        assert_eq!(out, vec![Some(900), Some(776), Some(0), Some(84)]);
    }
    #[test]
    fn overflow_never_overlaps_icons_or_other_widgets() {
        let slots = [(-1200, -800), (-500, -100)];
        let items = vec![(172, DockSide::Right); 6];
        let out = pack(&slots, &items, 4);
        assert_eq!(out.iter().flatten().count(), 4);
        for (i, x) in out.iter().enumerate() {
            if let Some(x) = x {
                assert!(slots.iter().any(|(l, r)| *l <= *x && x + 172 <= *r));
                for y in out.iter().skip(i + 1).flatten() {
                    assert!(*x + 172 <= *y || *y + 172 <= *x);
                }
            }
        }
    }
    #[test]
    fn a_single_gap_beside_left_aligned_icons_counts_as_the_right_side() {
        assert_eq!(side_at(&[(600, 1800)], 650, 172), DockSide::Right);
        assert_eq!(side_at(&[(0, 600), (1000, 1800)], 200, 172), DockSide::Left);
        assert_eq!(
            side_at(&[(0, 600), (1000, 1800)], 1100, 172),
            DockSide::Right
        );
    }
}
