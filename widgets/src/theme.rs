//! Per-widget visual documents. Bindings substitute data; they never execute code.
use eframe::egui::{self, Color32, Rect, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Values = BTreeMap<String, String>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Text,
    Rectangle,
    Progress,
    Icon,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub name: String,
    pub kind: ElementKind,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub binding: String,
    pub font_size: f32,
    pub color: [u8; 4],
    pub fill: [u8; 4],
    pub radius: f32,
}

impl Element {
    pub fn new(kind: ElementKind) -> Self {
        Self {
            name: match kind {
                ElementKind::Text => "Text",
                ElementKind::Rectangle => "Rectangle",
                ElementKind::Progress => "Progress",
                ElementKind::Icon => "Icon",
            }
            .into(),
            kind,
            x: 12.0,
            y: 8.0,
            width: 150.0,
            height: 22.0,
            text: "{primary}".into(),
            binding: "usage".into(),
            font_size: 12.0,
            color: [245, 245, 245, 255],
            fill: [217, 119, 87, 255],
            radius: 4.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub version: u32,
    pub width: f32,
    pub height: f32,
    pub background: [u8; 4],
    pub elements: Vec<Element>,
}

impl Theme {
    pub fn starter(kind: crate::app::Kind) -> Self {
        let mut primary = Element::new(ElementKind::Text);
        primary.x = 46.0;
        primary.y = 7.0;
        primary.width = 176.0;
        primary.height = 16.0;
        let mut secondary = primary.clone();
        secondary.name = "Secondary line".into();
        secondary.y = 25.0;
        secondary.text = "{secondary}".into();
        secondary.font_size = 11.0;
        secondary.color = [165, 167, 175, 255];
        let mut icon = Element::new(ElementKind::Icon);
        icon.name = "Widget icon".into();
        icon.x = 10.0;
        icon.y = 11.0;
        icon.width = 26.0;
        icon.height = 26.0;
        icon.binding = "icon".into();
        icon.text = kind.label().chars().take(1).collect();
        Self {
            version: 1,
            width: 232.0,
            height: 48.0,
            background: [0, 0, 0, 0],
            elements: vec![icon, primary, secondary],
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported theme version".into());
        }
        if !self.width.is_finite()
            || !self.height.is_finite()
            || !(48.0..=1600.0).contains(&self.width)
            || !(24.0..=256.0).contains(&self.height)
        {
            return Err("Canvas must be 48–1600 px wide and 24–256 px tall".into());
        }
        if self.elements.len() > 64 {
            return Err("A theme can contain at most 64 layers".into());
        }
        for e in &self.elements {
            if [e.x, e.y, e.width, e.height, e.font_size, e.radius]
                .iter()
                .any(|v| !v.is_finite())
                || e.x < 0.0
                || e.y < 0.0
                || e.width < 1.0
                || e.height < 1.0
                || e.x + e.width > self.width + 0.1
                || e.y + e.height > self.height + 0.1
                || !(6.0..=96.0).contains(&e.font_size)
                || !(0.0..=128.0).contains(&e.radius)
                || e.text.len() > 1024
                || e.name.len() > 128
                || e.binding.len() > 64
            {
                return Err(format!(
                    "Layer '{}' is outside the canvas or has invalid properties",
                    e.name
                ));
            }
        }
        Ok(())
    }

    pub fn paint(
        &self,
        ui: &mut egui::Ui,
        origin: egui::Pos2,
        scale: f32,
        values: &Values,
        icons: &mut crate::icons::IconCache,
    ) {
        let canvas = Rect::from_min_size(origin, Vec2::new(self.width, self.height) * scale);
        let painter = ui
            .painter()
            .with_clip_rect(canvas.intersect(ui.clip_rect()));
        painter.rect_filled(canvas, 0, color(self.background));
        for e in &self.elements {
            let rect = Rect::from_min_size(
                origin + Vec2::new(e.x, e.y) * scale,
                Vec2::new(e.width, e.height) * scale,
            );
            let p = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
            match e.kind {
                ElementKind::Text => {
                    let text = substitute(&e.text, values);
                    p.text(
                        rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        text,
                        egui::FontId::proportional(e.font_size * scale),
                        color(e.color),
                    );
                }
                ElementKind::Rectangle => {
                    p.rect_filled(rect, e.radius * scale, color(e.fill));
                }
                ElementKind::Progress => {
                    p.rect_filled(rect, e.radius * scale, color(e.color));
                    let percent = values
                        .get(&e.binding)
                        .and_then(|v| v.parse::<f32>().ok())
                        .filter(|v| v.is_finite())
                        .unwrap_or(0.0)
                        .clamp(0.0, 100.0);
                    let mut filled = rect;
                    filled.max.x = filled.min.x + rect.width() * percent / 100.0;
                    p.rect_filled(filled, e.radius * scale, color(e.fill));
                }
                ElementKind::Icon => {
                    let value = values.get(&e.binding).map(String::as_str).unwrap_or("");
                    // Only the widget supplies image URLs. Imported themes cannot
                    // turn the renderer into an arbitrary URL/file loader.
                    if let Some(provider) = crate::provider_icons::from_key(value) {
                        let clipped = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                        crate::provider_icons::paint(&clipped, rect, provider);
                    } else if value.starts_with("https://www.yr.no/assets/images/weather-symbols/")
                    {
                        if let Some(texture) = icons.get(ui.ctx(), value, 64) {
                            p.image(
                                texture.id(),
                                rect,
                                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                    } else {
                        p.rect_filled(rect, e.radius * scale, color(e.fill));
                        p.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            &e.text,
                            egui::FontId::proportional(e.font_size * scale),
                            color(e.color),
                        );
                    }
                }
            }
        }
    }
}

pub fn color(rgba: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
}

pub fn substitute(template: &str, values: &Values) -> String {
    let mut rest = template;
    let mut out = String::new();
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start + 1..].find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = &rest[start + 1..start + 1 + end];
        out.push_str(values.get(key).map(String::as_str).unwrap_or("—"));
        rest = &rest[start + end + 2..];
    }
    out.push_str(rest);
    out
}

pub fn sample(kind: crate::app::Kind) -> Values {
    use crate::app::Kind;
    let (primary, secondary) = match kind {
        Kind::Weather => ("8° / 2°", "3 mm"),
        Kind::Calendar => ("Now: Project review", "Next: Team meeting · 14:30"),
        Kind::Mail => ("Design review notes", "5 unread"),
        _ => ("Session: 34% used · 2h 15m", "Weekly: 62% used · 3d 4h"),
    };
    let mut values: Values = [
        ("primary", primary),
        ("secondary", secondary),
        ("title", kind.label()),
        ("icon", ""),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    let extra: &[(&str, &str)] = match kind {
        Kind::Weather => &[("high", "8"), ("low", "2"), ("precip", "3")],
        Kind::Mail => &[("count", "5")],
        Kind::Usage(_) => &[("usage", "34"), ("weekly", "62"), ("reset", "2h 15m")],
        _ => &[],
    };
    values.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    if let Kind::Usage(provider) = kind {
        values.insert(
            "icon".into(),
            format!("provider:{}", provider.descriptor().key),
        );
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_reject_unbounded_or_non_finite_layouts() {
        let mut theme = Theme::starter(crate::app::Kind::Weather);
        assert!(theme.validate().is_ok());
        theme.elements[0].x = f32::NAN;
        assert!(theme.validate().is_err());
        theme.elements[0].x = 4000.0;
        assert!(theme.validate().is_err());
    }
    #[test]
    fn binding_values_are_literal_and_unknowns_are_visible() {
        let values = [("a".into(), "{secret}".into())].into_iter().collect();
        assert_eq!(substitute("{a} {missing}", &values), "{secret} —");
    }
    #[test]
    fn all_widget_starters_round_trip_without_account_settings() {
        for kind in crate::app::Kind::ALL {
            let theme = Theme::starter(kind);
            let json = serde_json::to_string(&theme).unwrap();
            assert_eq!(serde_json::from_str::<Theme>(&json).unwrap(), theme);
            assert!(!json.contains("password"));
            assert!(theme.validate().is_ok());
        }
    }
}
