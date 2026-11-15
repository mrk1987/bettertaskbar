//! One line of pill text that scrolls when it is wider than its room: it
//! rests at the start, scrolls left until the end is in view, rests there,
//! snaps back to the start and goes round again. Text that fits never moves.

use std::time::Duration;

use eframe::egui;

/// Rest at the start before scrolling, also after each snap back.
const REST_START: f32 = 3.0;
/// Rest with the end in view before snapping back.
const REST_END: f32 = 1.5;
/// Scrolling speed in points per second.
const SPEED: f32 = 30.0;
/// Frame interval while the text moves. The pills are drawn on the CPU, so
/// this stays well under the hub's 60 fps ceiling.
const STEP: Duration = Duration::from_millis(40);

/// How far the text is scrolled `elapsed` seconds into the cycle, for text
/// `overflow` points wider than its room, and how long until it next moves.
pub fn offset_at(elapsed: f32, overflow: f32) -> (f32, Option<Duration>) {
    if overflow <= 0.5 {
        return (0.0, None);
    }
    let scroll = overflow / SPEED;
    let t = elapsed.max(0.0) % (REST_START + scroll + REST_END);
    if t < REST_START {
        (0.0, Some(Duration::from_secs_f32(REST_START - t)))
    } else if t < REST_START + scroll {
        ((t - REST_START) * SPEED, Some(STEP))
    } else {
        (
            overflow,
            Some(Duration::from_secs_f32(REST_START + scroll + REST_END - t)),
        )
    }
}

/// Whether the text has been seen in full once: it fits, or one cycle (rest,
/// scroll to the end, rest there) is over.
pub fn shown_once(elapsed: f32, overflow: f32) -> bool {
    overflow <= 0.5 || elapsed >= REST_START + overflow / SPEED + REST_END
}

/// Show `text` on one line in the room left in `ui`, scrolling if it does not
/// fit. `salt` tells the lines of one pill apart; a new text starts its cycle
/// from the beginning. Returns whether the whole text has been shown once,
/// so a rotating pill can wait for that before moving on.
pub fn line(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>, salt: &str) -> bool {
    let galley = text.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let room = ui.available_width().max(0.0);
    let size = galley.size();
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(size.x.min(room), size.y), egui::Sense::hover());
    let overflow = size.x - room;
    let mut shown = true;
    let offset = if overflow > 0.5 {
        let now = ui.input(|i| i.time);
        let id = ui.scope_id().with(("marquee", salt));
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            galley.text().hash(&mut h);
            h.finish()
        };
        let start = ui.data_mut(|d| {
            let entry = d.get_temp_mut_or_insert_with(id, || (key, now));
            if entry.0 != key {
                *entry = (key, now);
            }
            entry.1
        });
        let elapsed = (now - start) as f32;
        shown = shown_once(elapsed, overflow);
        let (offset, next) = offset_at(elapsed, overflow);
        if let Some(next) = next {
            ui.ctx().request_repaint_after(next);
        }
        offset
    } else {
        0.0
    };
    let fallback = ui.visuals().text_color();
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .galley(rect.min - egui::vec2(offset, 0.0), galley, fallback);
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_never_moves() {
        assert_eq!(offset_at(5.0, 0.0), (0.0, None));
        assert_eq!(offset_at(5.0, -20.0), (0.0, None));
    }

    #[test]
    fn long_text_rests_scrolls_rests_and_snaps_back() {
        // 60 points too wide: 2 s of scrolling in a 6.5 s cycle.
        let (o, next) = offset_at(1.0, 60.0);
        assert_eq!(o, 0.0);
        assert_eq!(next, Some(Duration::from_secs(2)));
        let (o, next) = offset_at(4.0, 60.0);
        assert!((o - 30.0).abs() < 1e-3);
        assert_eq!(next, Some(STEP));
        let (o, _) = offset_at(5.5, 60.0);
        assert_eq!(o, 60.0, "rests with the end in view");
        let (o, _) = offset_at(6.6, 60.0);
        assert_eq!(o, 0.0, "snapped back for the next cycle");
    }

    #[test]
    fn text_counts_as_shown_after_one_full_cycle() {
        assert!(shown_once(0.0, 0.0), "text that fits is shown at once");
        assert!(!shown_once(4.0, 60.0), "still scrolling");
        assert!(!shown_once(6.0, 60.0), "resting at the end");
        assert!(shown_once(6.5, 60.0));
    }
}
