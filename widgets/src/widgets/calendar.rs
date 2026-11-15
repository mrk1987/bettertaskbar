//! CalDAV next-event pill: Proton Calendar icon + "Now" / "Next" lines.

use std::sync::mpsc::{self, Receiver};

use eframe::egui;

use crate::caldav::Lines;
use crate::config::{CalendarCfg, Config, SharedConfig};
use crate::icons::IconCache;
use crate::widgets::{Update, Widget};

pub struct CalendarWidget {
    refresh: crate::widgets::RefreshRequest,
    rx: Receiver<Update<Lines>>,
    lines: Option<Lines>,
    /// Shown on the second line instead of "Next" while the server is failing
    /// or the widget is not set up.
    notice: Option<String>,
}

fn pick(c: &Config) -> (CalendarCfg, u64, bool) {
    (
        c.calendar.clone(),
        c.calendar.refresh_secs,
        !c.calendar.username.is_empty(),
    )
}

fn fetch(cfg: &CalendarCfg) -> Result<(Lines, String), String> {
    let f = crate::caldav::fetch(cfg)?;
    let mut summary = format!(
        "{} calendars, {} events in the next 14 days",
        f.calendars, f.events
    );
    if f.bad_rules > 0 {
        summary.push_str(&format!(
            ", {} recurrence rules not understood",
            f.bad_rules
        ));
    }
    Ok((f.lines, summary))
}

impl CalendarWidget {
    pub fn new(cfg: SharedConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let refresh = crate::widgets::poll("calendar", cfg, tx, pick, fetch);
        Self {
            refresh,
            rx,
            lines: None,
            notice: Some("Loading…".into()),
        }
    }
}

pub(crate) fn proton_tile(ui: &mut egui::Ui, kind: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, egui::Color32::from_rgb(0x6D, 0x4A, 0xFF));
    let c = rect.center();
    let stroke = egui::Stroke::new(1.6, egui::Color32::WHITE);
    if kind == "calendar" {
        let body = egui::Rect::from_center_size(c + egui::vec2(0.0, 1.0), egui::vec2(14.0, 12.0));
        p.rect_stroke(body, 2.0, stroke, egui::StrokeKind::Middle);
        p.line_segment(
            [
                egui::pos2(body.left(), body.top() + 3.0),
                egui::pos2(body.right(), body.top() + 3.0),
            ],
            stroke,
        );
    } else {
        let body = egui::Rect::from_center_size(c, egui::vec2(14.0, 10.0));
        p.rect_stroke(body, 2.0, stroke, egui::StrokeKind::Middle);
        p.line_segment(
            [
                egui::pos2(body.left(), body.top()),
                egui::pos2(c.x, c.y + 1.0),
            ],
            stroke,
        );
        p.line_segment(
            [
                egui::pos2(body.right(), body.top()),
                egui::pos2(c.x, c.y + 1.0),
            ],
            stroke,
        );
    }
}

/// Two lines of text at most `width` wide. Text too long for it scrolls.
/// Returns whether both lines have been shown in full once.
pub(crate) fn two_lines(ui: &mut egui::Ui, a: &str, b: &str, width: f32) -> bool {
    ui.vertical(|ui| {
        ui.set_max_width(width);
        lines(ui, a, b)
    })
    .inner
}

/// Two lines of text exactly `width` wide, whatever they say, so the pill
/// keeps one size. Text too long for it scrolls.
pub(crate) fn two_lines_fixed(ui: &mut egui::Ui, a: &str, b: &str, width: f32) -> bool {
    ui.vertical(|ui| {
        ui.set_width(width);
        lines(ui, a, b)
    })
    .inner
}

fn lines(ui: &mut egui::Ui, a: &str, b: &str) -> bool {
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
    // Too long for the pill: scroll, so the whole text can be read.
    let first = crate::widgets::marquee::line(ui, egui::RichText::new(a).size(12.0), "first");
    let second =
        crate::widgets::marquee::line(ui, egui::RichText::new(b).size(11.0).weak(), "second");
    first && second
}

/// The text width of the compact Mail and Calendar pills: just room for
/// "No unread mail", so the two pills are the same size and anything longer
/// scrolls.
pub(crate) fn compact_width(ui: &egui::Ui) -> f32 {
    ui.ctx()
        .fonts_mut(|f| {
            f.layout_no_wrap(
                "No unread mail".into(),
                egui::FontId::proportional(12.0),
                egui::Color32::WHITE,
            )
            .size()
            .x
        })
        .ceil()
}

impl Widget for CalendarWidget {
    /// The minimum; the pill is as wide as its tile and `compact_width`.
    fn size(&self) -> (f32, f32) {
        (48.0, 48.0)
    }

    fn refresh(&mut self) {
        self.refresh.request();
        self.notice = Some("Refresh requested…".into());
    }

    fn values(&self) -> crate::theme::Values {
        let lines = self.lines.clone().unwrap_or_default();
        [
            (
                "primary".into(),
                lines
                    .now
                    .clone()
                    .or(lines.next.clone())
                    .unwrap_or_else(|| "No upcoming events".into()),
            ),
            (
                "secondary".into(),
                self.notice
                    .clone()
                    .or(if lines.now.is_some() {
                        lines.next
                    } else {
                        Some("Free now".into())
                    })
                    .unwrap_or_default(),
            ),
        ]
        .into_iter()
        .collect()
    }
    fn tick(&mut self) {
        while let Ok(update) = self.rx.try_recv() {
            match update {
                Update::Data(lines) => {
                    self.lines = Some(lines);
                    self.notice = None;
                }
                // Keep the last known lines on screen; they are usually still right.
                Update::Failed(reason) => self.notice = Some(reason),
                Update::Unconfigured => {
                    self.lines = None;
                    self.notice = Some("Set calendar login in widgets.json".into());
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _icons: &mut IconCache) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
            proton_tile(ui, "calendar");
            let lines = self.lines.clone().unwrap_or_default();
            let (first, second) = match (&self.notice, lines.now, lines.next) {
                (Some(notice), now, next) => (
                    now.or(next).unwrap_or_else(|| "Calendar".into()),
                    notice.clone(),
                ),
                (None, Some(now), next) => {
                    (now, next.unwrap_or_else(|| "Nothing after this".into()))
                }
                // Nothing on right now: lead with what comes next.
                (None, None, Some(next)) => (next, "Free now".into()),
                (None, None, None) => ("No upcoming events".into(), "Next 14 days".into()),
            };
            two_lines_fixed(ui, &first, &second, compact_width(ui));
        });
    }
}

#[cfg(test)]
impl CalendarWidget {
    /// Fixed sample data for the README banner (`single.rs`, `readme_banner`):
    /// no thread, no network, nothing from the user's own accounts.
    pub fn sample() -> Self {
        Self {
            refresh: Default::default(),
            rx: mpsc::channel().1,
            lines: Some(Lines {
                now: None,
                next: Some("Next: Lunch · 12:00".into()),
            }),
            notice: None,
        }
    }
}
