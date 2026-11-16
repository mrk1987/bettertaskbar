//! One hub owns docked positions, so independent widget timers cannot fight.
use crate::i18n::tr;
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

/// The order to `pack` pills in, from their centers: left pills from the left
/// edge inward, right pills from the tray inward. Packing then keeps the
/// order the user placed them in, rather than a fixed order by kind.
fn pack_order(centers: &[(i32, DockSide)]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..centers.len()).collect();
    order.sort_by_key(|&i| match centers[i] {
        (c, DockSide::Left) => (0, c),
        (c, DockSide::Right) => (1, -c),
    });
    order
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

/// The windows of the widget processes the hub started.
pub fn process_windows(children: &[(Kind, Option<std::process::Child>)]) -> Vec<(Kind, HWND)> {
    children
        .iter()
        .filter_map(|(kind, child)| Some((*kind, owned_window(*kind, child.as_ref()?.id())?)))
        .collect()
}

pub fn arrange(cfg: &SharedConfig, windows: &[(Kind, HWND)]) {
    // Never reposition a widget under a press/drag, or another widget while
    // the user is deciding where to drop it.
    if shell::primary_button_down() || crate::app::any_dragging() {
        return;
    }
    let settings = cfg.lock().unwrap().clone();
    let mut status = Status::default();
    let windows = windows.to_vec();
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
    // Packed positions that differ from the saved ones; saved so the order
    // survives a restart and the next drop is compared with the real layout.
    let mut packed = Vec::new();
    // One enumeration of every top-level window per tick, not one per pill.
    let bars = shell::taskbars();
    for bar in bars.iter().cloned() {
        let mut docked = Vec::new();
        let Some(mut taken) = occupancy::occupied(bar.hwnd, bar.rect) else {
            for (kind, _) in &windows {
                let state = kind.state(&settings);
                if state.placement == Placement::Taskbar
                    && state.visible
                    && shell::pick_bar(&bars, state.monitor.as_deref())
                        .is_some_and(|target| target.hwnd == bar.hwnd)
                {
                    status
                        .problems
                        .insert(kind.key().into(), tr("Reading taskbar layout…").into());
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
            let target = shell::pick_bar(&bars, state.monitor.as_deref());
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
        // Where each pill was last put: a dropped pill at its drop point,
        // the others where the previous pack left them.
        let centers: Vec<_> = docked
            .iter()
            .zip(&items)
            .map(|((kind, _, rect), &(width, side))| {
                let left = kind
                    .state(&settings)
                    .manual_bar_offset
                    .map_or(rect.left, |offset| bar.rect.left + offset);
                (left + width / 2, side)
            })
            .collect();
        let order = pack_order(&centers);
        let ordered: Vec<_> = order.iter().map(|&i| items[i]).collect();
        let mut positions = vec![None; items.len()];
        for (x, &i) in pack(&slots, &ordered, gap).into_iter().zip(&order) {
            positions[i] = x;
        }
        for ((kind, hwnd, rect), x) in docked.into_iter().zip(positions) {
            match x {
                Some(x) => {
                    let saved = kind.state(&settings).manual_bar_offset;
                    if saved != Some(x - bar.rect.left) {
                        packed.push((kind, saved, x - bar.rect.left));
                    }
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
                    status.problems.insert(kind.key().into(),tr("Not enough free taskbar space. Move this widget, make its theme smaller, or switch off magnetic placement under General.").into());
                }
            }
        }
    }
    publish(&status);
    if !inferred.is_empty() || !packed.is_empty() {
        config::update(cfg, |c| {
            if !c.magnetized {
                return;
            }
            for (kind, read, offset) in packed {
                if !kind.exists(c) {
                    continue;
                }
                let state = kind.state_mut(c);
                // Skip a pill that was dropped again since this pass read it.
                if state.manual_bar_offset == read && state.placement == Placement::Taskbar {
                    state.manual_bar_offset = Some(offset);
                }
            }
            for (kind, side, offset, manual) in inferred {
                if !kind.exists(c) {
                    continue;
                }
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
    for kind in crate::app::user_facing_kinds(settings) {
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
    fn pills_keep_the_order_they_were_placed_in() {
        // Left side: Weather 0-70, Departures 72-260, Calendar 262-480, and
        // Mail just dropped at 230, between Departures and Calendar.
        let centers = [
            (35, DockSide::Left),
            (230 + 109, DockSide::Left),
            (72 + 94, DockSide::Left),
            (262 + 109, DockSide::Left),
        ];
        assert_eq!(pack_order(&centers), vec![0, 2, 1, 3]);
        // Right side packs from the tray inward: rightmost first.
        let right = [
            (900, DockSide::Right),
            (1000, DockSide::Right),
            (950, DockSide::Right),
        ];
        assert_eq!(pack_order(&right), vec![1, 2, 0]);
        // Packing in that order puts Mail between Departures and Calendar.
        let widths = [70, 218, 188, 218];
        let order = pack_order(&centers);
        let items: Vec<_> = order.iter().map(|&i| (widths[i], DockSide::Left)).collect();
        let packed = pack(&[(0, 2000)], &items, 2);
        let mut x = [0; 4];
        for (slot, &i) in order.iter().enumerate() {
            x[i] = packed[slot].unwrap();
        }
        assert!(x[0] < x[2] && x[2] < x[1] && x[1] < x[3], "{x:?}");
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
