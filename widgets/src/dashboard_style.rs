//! Dashboard palette and controls adapted from CodeZeno's MIT-licensed UI.
//! See vendor/codezeno-usage-core/LICENSE and THIRD_PARTY_NOTICES.md.
use eframe::egui::{self, Color32};
pub const SURFACE: Color32 = Color32::from_rgb(32, 32, 32);
pub const SECTION: Color32 = Color32::from_rgb(35, 35, 35);
pub const BORDER: Color32 = Color32::from_rgb(54, 54, 54);
pub const ACCENT: Color32 = Color32::from_rgb(217, 119, 87);
pub const MUTED: Color32 = Color32::from_rgb(143, 146, 156);
pub const DANGER: Color32 = Color32::from_rgb(232, 119, 95);

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(root) = std::env::var_os("WINDIR") {
        if let Ok(bytes) = std::fs::read(std::path::PathBuf::from(root).join("Fonts/segoeui.ttf")) {
            fonts
                .font_data
                .insert("segoe-ui".into(), egui::FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "segoe-ui".into());
        }
    }
    crate::provider_icons::install(&mut fonts);
    ctx.set_fonts(fonts);
}

pub fn configure(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = SURFACE;
    visuals.window_fill = Color32::from_gray(38);
    visuals.widgets.inactive.bg_fill = Color32::from_gray(48);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_gray(48);
    visuals.widgets.hovered.bg_fill = Color32::from_gray(58);
    visuals.widgets.active.bg_fill = Color32::from_rgb(65, 54, 51);
    visuals.selection.bg_fill = Color32::from_rgb(76, 78, 84);
    visuals.extreme_bg_color = Color32::from_gray(48);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(5);
    }
    visuals.menu_corner_radius = egui::CornerRadius::same(6);
    ctx.set_visuals(visuals);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = egui::vec2(9.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.indent = 16.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    });
}
pub fn group<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(SECTION)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(12)
        .inner_margin(egui::Margin::symmetric(20, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
        .inner
}
pub fn heading(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.label(egui::RichText::new(title).size(25.0).strong());
    ui.label(egui::RichText::new(detail).color(MUTED));
    ui.add_space(12.0);
}
pub fn nav(ui: &mut egui::Ui, selected: bool, title: &str) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), selected, title)
    });
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            4,
            if selected {
                Color32::from_gray(49)
            } else {
                Color32::from_gray(42)
            },
        );
    }
    if selected {
        ui.painter()
            .with_clip_rect(egui::Rect::from_min_max(
                rect.min,
                egui::pos2(rect.left() + 6.0, rect.bottom()),
            ))
            .rect_filled(rect, 4, ACCENT);
    }
    ui.painter().text(
        egui::pos2(rect.left() + 18.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(16.0),
        Color32::from_gray(245),
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            4,
            egui::Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
pub fn toggle(ui: &mut egui::Ui, value: &mut bool) -> egui::Response {
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(112.0, 30.0), egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *value, "Enabled")
    });
    let pill = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 22.0, rect.center().y),
        egui::vec2(40.0, 22.0),
    );
    ui.painter().rect_filled(
        pill,
        11,
        if *value {
            ACCENT
        } else {
            Color32::from_gray(72)
        },
    );
    ui.painter().circle_filled(
        egui::pos2(
            if *value {
                pill.right() - 11.0
            } else {
                pill.left() + 11.0
            },
            pill.center().y,
        ),
        8.0,
        Color32::from_gray(245),
    );
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        if *value { "Enabled" } else { "Disabled" },
        egui::FontId::proportional(14.0),
        Color32::from_gray(218),
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            5,
            egui::Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response
}
pub fn field(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .password(password)
            .desired_width(ui.available_width()),
    );
}
