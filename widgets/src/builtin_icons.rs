//! The widgets' own icons, drawn in code rather than loaded from files.
//!
//! Each icon is designed on a 26 × 26 tile and can be painted at any size,
//! so a Theme Studio theme shows the same icon as the default pill. Themes
//! name an icon by key (`builtin:calendar`); only the keys below exist, so a
//! theme cannot ask the renderer for anything else.

use eframe::egui;
use egui::emath::TSTransform;
use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind};

/// The size every icon is designed at, in points.
pub const TILE: f32 = 26.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Calendar,
    Mail,
    News,
    Music { paused: bool },
    Transit(Transit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transit {
    Bus,
    Tram,
    Metro,
    Train,
    Boat,
    /// Unknown mode or no departure: a gray bus.
    Other,
}

const ALL: [(&str, Icon); 11] = [
    ("builtin:calendar", Icon::Calendar),
    ("builtin:mail", Icon::Mail),
    ("builtin:news", Icon::News),
    ("builtin:music", Icon::Music { paused: false }),
    ("builtin:music-paused", Icon::Music { paused: true }),
    ("builtin:bus", Icon::Transit(Transit::Bus)),
    ("builtin:tram", Icon::Transit(Transit::Tram)),
    ("builtin:metro", Icon::Transit(Transit::Metro)),
    ("builtin:train", Icon::Transit(Transit::Train)),
    ("builtin:boat", Icon::Transit(Transit::Boat)),
    ("builtin:transit", Icon::Transit(Transit::Other)),
];

impl Icon {
    /// The value a widget puts in its `icon` binding.
    pub fn key(self) -> &'static str {
        ALL.iter()
            .find(|(_, icon)| *icon == self)
            .map(|(key, _)| *key)
            .expect("every icon has a key")
    }
}

pub fn from_key(key: &str) -> Option<Icon> {
    ALL.iter().find(|(k, _)| *k == key).map(|(_, icon)| *icon)
}

/// Allocates a tile in `ui` and draws `icon` on it, as the default pills do.
pub fn tile(ui: &mut egui::Ui, icon: Icon) {
    let (rect, _) = ui.allocate_exact_size(vec2(TILE, TILE), egui::Sense::hover());
    paint(ui.painter(), rect, icon);
}

/// Draws `icon` as large as fits in `rect`, centered.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon) {
    painter.extend(shapes(rect, icon));
}

fn shapes(rect: Rect, icon: Icon) -> Vec<Shape> {
    let mut pen = Pen(Vec::new());
    let tile = Rect::from_min_size(Pos2::ZERO, vec2(TILE, TILE));
    match icon {
        Icon::Calendar => proton(&mut pen, tile, true),
        Icon::Mail => proton(&mut pen, tile, false),
        Icon::News => news(&mut pen, tile),
        Icon::Music { paused } => music(&mut pen, tile, paused),
        Icon::Transit(mode) => transit(&mut pen, tile, mode),
    }
    let scale = rect.width().min(rect.height()) / TILE;
    let to = TSTransform::new(
        rect.center().to_vec2() - vec2(TILE, TILE) * 0.5 * scale,
        scale,
    );
    pen.0
        .into_iter()
        .map(|mut shape| {
            shape.transform(to);
            shape
        })
        .collect()
}

/// Collects shapes with the same calls a `Painter` takes, so they can be
/// scaled before they are painted.
struct Pen(Vec<Shape>);

impl Pen {
    fn rect_filled(&mut self, rect: Rect, radius: f32, fill: Color32) {
        self.0.push(Shape::rect_filled(rect, radius, fill));
    }
    fn rect_stroke(&mut self, rect: Rect, radius: f32, stroke: Stroke) {
        self.0
            .push(Shape::rect_stroke(rect, radius, stroke, StrokeKind::Middle));
    }
    fn line_segment(&mut self, points: [Pos2; 2], stroke: Stroke) {
        self.0.push(Shape::line_segment(points, stroke));
    }
    fn circle_filled(&mut self, center: Pos2, radius: f32, fill: Color32) {
        self.0.push(Shape::circle_filled(center, radius, fill));
    }
    fn circle_stroke(&mut self, center: Pos2, radius: f32, stroke: Stroke) {
        self.0.push(Shape::circle_stroke(center, radius, stroke));
    }
    fn add(&mut self, shape: Shape) {
        self.0.push(shape);
    }
}

const WHITE: Color32 = Color32::WHITE;

/// Proton's purple tile with a calendar or an envelope.
fn proton(p: &mut Pen, rect: Rect, calendar: bool) {
    p.rect_filled(rect, 6.0, Color32::from_rgb(0x6D, 0x4A, 0xFF));
    let c = rect.center();
    let stroke = Stroke::new(1.6, WHITE);
    if calendar {
        let body = Rect::from_center_size(c + vec2(0.0, 1.0), vec2(14.0, 12.0));
        p.rect_stroke(body, 2.0, stroke);
        p.line_segment(
            [
                pos2(body.left(), body.top() + 3.0),
                pos2(body.right(), body.top() + 3.0),
            ],
            stroke,
        );
    } else {
        let body = Rect::from_center_size(c, vec2(14.0, 10.0));
        p.rect_stroke(body, 2.0, stroke);
        p.line_segment(
            [pos2(body.left(), body.top()), pos2(c.x, c.y + 1.0)],
            stroke,
        );
        p.line_segment(
            [pos2(body.right(), body.top()), pos2(c.x, c.y + 1.0)],
            stroke,
        );
    }
}

/// A newspaper page: a headline block and three lines of text.
fn news(p: &mut Pen, rect: Rect) {
    p.rect_filled(rect, 6.0, Color32::from_rgb(0x2F, 0x6F, 0xD6));
    let page = Rect::from_center_size(rect.center(), vec2(15.0, 13.0));
    p.rect_stroke(page, 1.5, Stroke::new(1.4, WHITE));
    p.rect_filled(
        Rect::from_min_size(page.min + vec2(2.5, 2.5), vec2(4.5, 3.5)),
        0.5,
        WHITE,
    );
    for (y, x0) in [(3.2, 8.5), (6.3, 2.5), (9.4, 2.5)] {
        p.line_segment(
            [
                pos2(page.left() + x0, page.top() + y),
                pos2(page.right() - 2.5, page.top() + y),
            ],
            Stroke::new(1.1, WHITE),
        );
    }
}

/// An eighth note, with a pause badge when the player is not playing.
fn music(p: &mut Pen, rect: Rect, paused: bool) {
    p.rect_filled(rect, 6.0, Color32::from_rgb(0xD9, 0x4F, 0x70));
    let c = rect.center();
    let head = c + vec2(-2.5, 4.0);
    p.circle_filled(head, 3.2, WHITE);
    let top = pos2(head.x + 2.8, c.y - 7.0);
    p.line_segment([pos2(top.x, head.y), top], Stroke::new(1.6, WHITE));
    p.line_segment([top, top + vec2(4.5, 3.5)], Stroke::new(1.8, WHITE));
    if paused {
        let badge = Rect::from_center_size(rect.right_bottom() - vec2(4.5, 4.5), vec2(9.0, 9.0));
        p.rect_filled(badge, 2.0, Color32::from_rgb(0x20, 0x24, 0x2c));
        for dx in [-1.6, 1.6] {
            p.line_segment(
                [
                    badge.center() + vec2(dx, -2.5),
                    badge.center() + vec2(dx, 2.5),
                ],
                Stroke::new(1.4, WHITE),
            );
        }
    }
}

/// A colored square with a white pictogram of the mode.
fn transit(p: &mut Pen, rect: Rect, mode: Transit) {
    let fill = match mode {
        Transit::Bus => Color32::from_rgb(0xD2, 0x3C, 0x3C),
        Transit::Tram => Color32::from_rgb(0x1E, 0x88, 0xE5),
        Transit::Metro => Color32::from_rgb(0xE8, 0x6E, 0x10),
        Transit::Train => Color32::from_rgb(0x2E, 0x9E, 0x5B),
        Transit::Boat => Color32::from_rgb(0x12, 0x8F, 0xA8),
        Transit::Other => Color32::from_rgb(0x5A, 0x63, 0x73),
    };
    p.rect_filled(rect, 6.0, fill);
    let c = rect.center();
    let w = WHITE;
    let s = Stroke::new(1.4, w);
    let r = |x: f32, y: f32, ww: f32, hh: f32| Rect::from_min_size(c + vec2(x, y), vec2(ww, hh));
    match mode {
        Transit::Bus | Transit::Other => {
            // Front of a bus: body, windscreen, wheels.
            p.rect_stroke(r(-6.0, -7.5, 12.0, 13.0), 2.5, s);
            p.rect_filled(r(-4.5, -5.5, 9.0, 4.5), 1.0, w);
            p.circle_filled(c + vec2(-3.5, 2.5), 1.1, w);
            p.circle_filled(c + vec2(3.5, 2.5), 1.1, w);
            p.rect_filled(r(-5.5, 5.5, 2.5, 2.5), 0.5, w);
            p.rect_filled(r(3.0, 5.5, 2.5, 2.5), 0.5, w);
        }
        Transit::Tram => {
            // Car with a pantograph to the overhead wire.
            p.line_segment([c + vec2(-5.0, -9.0), c + vec2(5.0, -9.0)], s);
            p.line_segment([c + vec2(0.0, -9.0), c + vec2(-2.5, -5.5)], s);
            p.line_segment([c + vec2(-2.5, -5.5), c + vec2(0.0, -4.5)], s);
            p.rect_stroke(r(-6.0, -4.5, 12.0, 11.0), 2.0, s);
            p.rect_filled(r(-4.5, -3.0, 9.0, 4.0), 1.0, w);
            p.circle_filled(c + vec2(-3.0, 3.5), 1.0, w);
            p.circle_filled(c + vec2(3.0, 3.5), 1.0, w);
            p.line_segment([c + vec2(-7.0, 8.5), c + vec2(7.0, 8.5)], s);
        }
        Transit::Metro => {
            // Round metro sign with an M.
            p.circle_stroke(c, 8.5, Stroke::new(1.6, w));
            let m = [
                c + vec2(-4.0, 4.0),
                c + vec2(-4.0, -4.0),
                c + vec2(0.0, 0.5),
                c + vec2(4.0, -4.0),
                c + vec2(4.0, 4.0),
            ];
            p.add(Shape::line(m.to_vec(), Stroke::new(1.7, w)));
        }
        Transit::Train => {
            // Locomotive front with a sloped nose on rails.
            let body = vec![
                c + vec2(-5.0, -8.0),
                c + vec2(5.0, -8.0),
                c + vec2(6.5, 4.0),
                c + vec2(-6.5, 4.0),
            ];
            p.add(Shape::closed_line(body, s));
            p.rect_filled(r(-3.8, -6.0, 7.6, 4.0), 1.0, w);
            p.circle_filled(c + vec2(-3.3, 1.5), 1.0, w);
            p.circle_filled(c + vec2(3.3, 1.5), 1.0, w);
            p.line_segment([c + vec2(-4.0, 4.0), c + vec2(-6.0, 8.5)], s);
            p.line_segment([c + vec2(4.0, 4.0), c + vec2(6.0, 8.5)], s);
            p.line_segment([c + vec2(-5.0, 6.5), c + vec2(5.0, 6.5)], s);
        }
        Transit::Boat => {
            // Hull, cabin and a wave.
            let hull = vec![
                c + vec2(-8.0, 0.5),
                c + vec2(8.0, 0.5),
                c + vec2(5.5, 4.5),
                c + vec2(-5.5, 4.5),
            ];
            p.add(Shape::convex_polygon(hull, w, Stroke::NONE));
            p.rect_stroke(r(-4.0, -4.5, 7.0, 5.0), 1.0, s);
            p.line_segment([c + vec2(1.0, -4.5), c + vec2(1.0, -8.0)], s);
            let wave: Vec<_> = (0..=8)
                .map(|i| {
                    let x = -8.0 + i as f32 * 2.0;
                    c + vec2(x, 7.5 + if i % 2 == 0 { 0.0 } else { 1.2 })
                })
                .collect();
            p.add(Shape::line(wave, Stroke::new(1.1, w)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_round_trips_and_nothing_else_is_an_icon() {
        for (key, icon) in ALL {
            assert_eq!(from_key(key), Some(icon));
            assert_eq!(icon.key(), key);
        }
        for key in [
            "",
            "builtin:",
            "builtin:Calendar",
            "provider:claude",
            "C:\\x.png",
        ] {
            assert_eq!(from_key(key), None, "{key}");
        }
    }

    #[test]
    fn icons_fill_the_rect_they_are_given() {
        let background = |rect| match shapes(rect, Icon::Calendar).remove(0) {
            Shape::Rect(bg) => bg,
            other => panic!("background is a rect, got {other:?}"),
        };
        // At the design size nothing moves: the default pills look as before.
        let native = Rect::from_min_size(pos2(4.0, 11.0), vec2(TILE, TILE));
        assert_eq!(background(native).rect, native);
        // Twice the size: twice the corner radius, filling the rect.
        let big = Rect::from_min_size(pos2(100.0, 50.0), vec2(52.0, 52.0));
        let bg = background(big);
        assert_eq!(bg.rect, big);
        assert_eq!(bg.corner_radius, egui::CornerRadius::same(12));
        // A wide rect: a square tile, centered.
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(80.0, 26.0));
        assert_eq!(
            background(wide).rect,
            Rect::from_center_size(wide.center(), vec2(TILE, TILE))
        );
    }
}
