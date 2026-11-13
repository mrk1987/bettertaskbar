//! Visual layout editor; the live widgets use this same Theme renderer.
use crate::{
    app::Kind,
    config::{self, Config, SharedConfig},
    dashboard_style as style,
    icons::IconCache,
    theme::{self, Element, ElementKind, Theme},
};
use eframe::egui;

pub struct Studio {
    kind: Kind,
    draft: Theme,
    saved: Theme,
    custom: bool,
    selected: Option<usize>,
    zoom: f32,
    grid: bool,
    snap: bool,
    history: Vec<Theme>,
    redo: Vec<Theme>,
    icons: IconCache,
    message: Option<String>,
}
impl Studio {
    pub fn new(kind: Kind, cfg: &Config) -> Self {
        let custom = kind.state(cfg).theme.is_some();
        let draft = kind
            .state(cfg)
            .theme
            .clone()
            .unwrap_or_else(|| Theme::starter(kind));
        Self {
            kind,
            saved: draft.clone(),
            draft,
            custom,
            selected: None,
            zoom: 1.5,
            grid: true,
            snap: true,
            history: vec![],
            redo: vec![],
            icons: IconCache::default(),
            message: None,
        }
    }
    pub fn dirty(&self) -> bool {
        self.draft != self.saved
    }
    pub fn discard(&mut self) {
        self.draft = self.saved.clone();
        self.history.clear();
        self.redo.clear();
        self.message = None;
    }
    pub fn select(&mut self, kind: Kind, cfg: &Config) {
        if kind == self.kind {
            return;
        }
        if self.dirty() {
            self.message =
                Some("Apply or discard the current theme before switching widgets.".into());
            return;
        }
        *self = Self::new(kind, cfg);
    }
    pub fn save(&mut self, cfg: &SharedConfig) -> bool {
        if !self.dirty() {
            return true;
        }
        self.apply(cfg)
    }
    fn apply(&mut self, cfg: &SharedConfig) -> bool {
        if let Err(e) = self.draft.validate() {
            self.message = Some(e);
            return false;
        }
        let theme = self.draft.clone();
        match config::try_update(cfg, |c| self.kind.state_mut(c).theme = Some(theme)) {
            Ok(()) => {
                self.saved = self.draft.clone();
                self.custom = true;
                self.message = Some("Theme applied to this widget.".into());
                true
            }
            Err(e) => {
                self.message = Some(e);
                false
            }
        }
    }
    fn remember(&mut self, previous: Theme) {
        if self.draft != previous {
            self.history.push(previous);
            if self.history.len() > 64 {
                self.history.remove(0);
            }
            self.redo.clear();
        }
    }
    fn undo(&mut self) {
        if let Some(theme) = self.history.pop() {
            self.redo.push(std::mem::replace(&mut self.draft, theme));
            self.selected = None;
        }
    }
    fn redo(&mut self) {
        if let Some(theme) = self.redo.pop() {
            self.history.push(std::mem::replace(&mut self.draft, theme));
            self.selected = None;
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, cfg: &SharedConfig) {
        style::heading(ui,"Theme Studio","Each widget has its own canvas. Drag layers to move them; use the corner handle to resize.");
        ui.horizontal(|ui| {
            ui.label("Widget");
            let mut kind = self.kind;
            egui::ComboBox::from_id_salt("theme-widget")
                .selected_text(self.kind.label())
                .show_ui(ui, |ui| {
                    for item in Kind::ALL {
                        ui.selectable_value(&mut kind, item, item.label());
                    }
                });
            if kind != self.kind {
                self.select(kind, &cfg.lock().unwrap());
            }
            if ui
                .add_enabled(
                    self.dirty() || !self.custom,
                    egui::Button::new("Apply theme").fill(style::ACCENT),
                )
                .clicked()
            {
                self.apply(cfg);
            }
            if ui
                .add_enabled(self.dirty(), egui::Button::new("Discard"))
                .clicked()
            {
                self.discard();
            }
            if ui
                .button("Use original")
                .on_hover_text("Restore this widget’s original built-in appearance")
                .clicked()
            {
                match config::try_update(cfg, |c| self.kind.state_mut(c).theme = None) {
                    Ok(()) => {
                        let kind = self.kind;
                        *self = Self::new(kind, &cfg.lock().unwrap());
                    }
                    Err(e) => self.message = Some(e),
                }
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.history.is_empty(), egui::Button::new("Undo"))
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo"))
                .clicked()
            {
                self.redo();
            }
            if ui.button("Import…").clicked() {
                self.import();
            }
            if ui.button("Export…").clicked() {
                self.export();
            }
            ui.separator();
            ui.label("Zoom");
            ui.add(
                egui::Slider::new(&mut self.zoom, 0.5..=4.0)
                    .step_by(0.25)
                    .show_value(false),
            );
            ui.label(format!("{:.0}%", self.zoom * 100.0));
            ui.checkbox(&mut self.grid, "Grid");
            ui.checkbox(&mut self.snap, "Snap");
        });
        if let Some(message) = &self.message {
            ui.colored_label(style::MUTED, message);
        }
        if !self.custom && !self.dirty() {
            ui.colored_label(
                style::MUTED,
                "Editing a starter layout. Apply it to replace this widget’s original appearance.",
            );
        }
        let before = self.draft.clone();
        let height = (ui.available_height() - 24.0).max(320.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(158.0, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(158.0);
                    self.layers(ui, height);
                },
            );
            ui.separator();
            let inspector = 238.0;
            let width = (ui.available_width() - inspector - 20.0).max(180.0);
            ui.allocate_ui_with_layout(
                egui::vec2(width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(width);
                    self.canvas(ui, height);
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(inspector, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(inspector);
                    egui::ScrollArea::vertical()
                        .id_salt("theme-inspector")
                        .max_height(height)
                        .show(ui, |ui| self.inspector(ui));
                },
            );
        });
        self.remember(before);
        if !ui.ctx().egui_wants_keyboard_input() {
            let modifiers = egui::Modifiers::COMMAND;
            if ui.input_mut(|i| i.consume_key(modifiers, egui::Key::Z)) {
                self.undo();
            }
            if ui.input_mut(|i| i.consume_key(modifiers, egui::Key::Y)) {
                self.redo();
            }
        }
    }
    fn layers(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.label(egui::RichText::new("Layers").size(16.0).strong());
        ui.menu_button("Add layer", |ui| {
            for (name, kind) in [
                ("Text", ElementKind::Text),
                ("Rectangle", ElementKind::Rectangle),
                ("Progress bar", ElementKind::Progress),
                ("Icon", ElementKind::Icon),
            ] {
                if ui
                    .add_enabled(self.draft.elements.len() < 64, egui::Button::new(name))
                    .clicked()
                {
                    let mut element = Element::new(kind);
                    element.width = element.width.min(self.draft.width - 12.0);
                    element.height = element.height.min(self.draft.height - 8.0);
                    self.draft.elements.push(element);
                    self.selected = Some(self.draft.elements.len() - 1);
                    ui.close();
                }
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("theme-layers")
            .max_height(height - 145.0)
            .show(ui, |ui| {
                for (i, element) in self.draft.elements.iter().enumerate().rev() {
                    if ui
                        .selectable_label(self.selected == Some(i), &element.name)
                        .clicked()
                    {
                        self.selected = Some(i);
                    }
                }
            });
        if let Some(i) = self.selected.filter(|i| *i < self.draft.elements.len()) {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(i + 1 < self.draft.elements.len(), egui::Button::new("Up"))
                    .clicked()
                {
                    self.draft.elements.swap(i, i + 1);
                    self.selected = Some(i + 1);
                }
                if ui.add_enabled(i > 0, egui::Button::new("Down")).clicked() {
                    self.draft.elements.swap(i, i - 1);
                    self.selected = Some(i - 1);
                }
            });
            if ui.button("Duplicate").clicked() && self.draft.elements.len() < 64 {
                let mut e = self.draft.elements[i].clone();
                e.name = format!("{} copy", e.name);
                self.draft.elements.push(e);
                self.selected = Some(self.draft.elements.len() - 1);
            }
            if ui.button("Delete layer").clicked() {
                self.draft.elements.remove(i);
                self.selected = None;
            }
        }
    }
    fn canvas(&mut self, ui: &mut egui::Ui, height: f32) -> egui::Rect {
        ui.label(egui::RichText::new("Preview").size(16.0).strong());
        ui.colored_label(
            style::MUTED,
            "Sample data. Your account details never enter a theme.",
        );
        egui::ScrollArea::both()
            .id_salt("theme-canvas-scroll")
            .max_height(height - 60.0)
            .show(ui, |ui| {
                let size = egui::vec2(self.draft.width, self.draft.height) * self.zoom;
                let (rect, _) =
                    ui.allocate_exact_size(size + egui::vec2(48.0, 80.0), egui::Sense::hover());
                let origin = rect.min + egui::vec2(24.0, 40.0);
                let canvas = egui::Rect::from_min_size(origin, size);
                ui.painter()
                    .rect_filled(canvas, 0, egui::Color32::from_gray(47));
                if self.grid {
                    for x in (0..self.draft.width as usize).step_by(8) {
                        for y in (0..self.draft.height as usize).step_by(8) {
                            ui.painter().circle_filled(
                                origin + egui::vec2(x as f32, y as f32) * self.zoom,
                                0.7,
                                egui::Color32::from_gray(77),
                            );
                        }
                    }
                }
                self.draft.paint(
                    ui,
                    origin,
                    self.zoom,
                    &theme::sample(self.kind),
                    &mut self.icons,
                );
                ui.painter().rect_stroke(
                    canvas,
                    0,
                    egui::Stroke::new(1.0, style::BORDER),
                    egui::StrokeKind::Outside,
                );
                let count = self.draft.elements.len();
                for i in 0..count {
                    let e = &self.draft.elements[i];
                    let r = egui::Rect::from_min_size(
                        origin + egui::vec2(e.x, e.y) * self.zoom,
                        egui::vec2(e.width, e.height) * self.zoom,
                    );
                    let response = ui.interact(
                        r,
                        ui.scope_id().with(("layer", i)),
                        egui::Sense::click_and_drag(),
                    );
                    if response.clicked() || response.drag_started() {
                        self.selected = Some(i);
                    }
                    if response.dragged() {
                        let delta = ui.input(|input| input.pointer.delta()) / self.zoom;
                        let e = &mut self.draft.elements[i];
                        e.x = (e.x + delta.x).clamp(0.0, (self.draft.width - e.width).max(0.0));
                        e.y = (e.y + delta.y).clamp(0.0, (self.draft.height - e.height).max(0.0));
                    }
                    if response.drag_stopped() && self.snap {
                        let e = &mut self.draft.elements[i];
                        e.x = ((e.x / 4.0).round() * 4.0)
                            .clamp(0.0, (self.draft.width - e.width).max(0.0));
                        e.y = ((e.y / 4.0).round() * 4.0)
                            .clamp(0.0, (self.draft.height - e.height).max(0.0));
                    }
                    if self.selected == Some(i) {
                        ui.painter().rect_stroke(
                            r,
                            0,
                            egui::Stroke::new(1.5, style::ACCENT),
                            egui::StrokeKind::Outside,
                        );
                        let handle =
                            egui::Rect::from_center_size(r.right_bottom(), egui::vec2(10.0, 10.0));
                        ui.painter().rect_filled(handle, 2, style::ACCENT);
                        let resize = ui
                            .interact(
                                handle,
                                ui.scope_id().with(("resize", i)),
                                egui::Sense::drag(),
                            )
                            .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
                        if resize.dragged() {
                            let delta = ui.input(|input| input.pointer.delta()) / self.zoom;
                            let e = &mut self.draft.elements[i];
                            e.width =
                                (e.width + delta.x).clamp(1.0, (self.draft.width - e.x).max(1.0));
                            e.height =
                                (e.height + delta.y).clamp(1.0, (self.draft.height - e.y).max(1.0));
                        }
                    }
                }
                canvas
            })
            .inner
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Canvas").size(16.0).strong());
        ui.horizontal(|ui| {
            ui.label("Width");
            ui.add(egui::DragValue::new(&mut self.draft.width).range(48.0..=1600.0));
            ui.label("Height");
            ui.add(egui::DragValue::new(&mut self.draft.height).range(24.0..=256.0));
        });
        ui.horizontal(|ui| {
            ui.label("Background");
            ui.color_edit_button_srgba_unmultiplied(&mut self.draft.background);
        });
        ui.add_space(14.0);
        let Some(e) = self.selected.and_then(|i| self.draft.elements.get_mut(i)) else {
            ui.colored_label(style::MUTED, "Select a layer to edit its properties.");
            return;
        };
        ui.label(egui::RichText::new("Layer").size(16.0).strong());
        style::field(ui, "Name", &mut e.name, false);
        egui::Grid::new("theme-properties")
            .num_columns(2)
            .show(ui, |ui| {
                for (label, value) in [
                    ("X", &mut e.x),
                    ("Y", &mut e.y),
                    ("Width", &mut e.width),
                    ("Height", &mut e.height),
                ] {
                    ui.label(label);
                    ui.add(egui::DragValue::new(value).speed(0.5).range(0.0..=1600.0));
                    ui.end_row();
                }
            });
        if matches!(e.kind, ElementKind::Text | ElementKind::Icon) {
            style::field(ui, "Text / data template", &mut e.text, false);
            ui.horizontal(|ui| {
                ui.label("Font size");
                ui.add(egui::DragValue::new(&mut e.font_size).range(6.0..=96.0));
            });
        }
        if matches!(e.kind, ElementKind::Progress | ElementKind::Icon) {
            style::field(ui, "Data binding", &mut e.binding, false);
        }
        ui.horizontal(|ui| {
            ui.label("Text / track color");
            ui.color_edit_button_srgba_unmultiplied(&mut e.color);
        });
        ui.horizontal(|ui| {
            ui.label("Fill");
            ui.color_edit_button_srgba_unmultiplied(&mut e.fill);
        });
        ui.horizontal(|ui| {
            ui.label("Corner radius");
            ui.add(egui::DragValue::new(&mut e.radius).range(0.0..=128.0));
        });
        ui.add_space(14.0);
        ui.label(egui::RichText::new("Available data").strong());
        for (key, value) in theme::sample(self.kind) {
            if key != "icon" {
                ui.label(format!("{{{key}}}  {value}"));
            }
        }
        ui.label("Icons use the icon binding. Progress bars use numeric data from 0 to 100.");
    }
    fn import(&mut self) {
        let Some(path) = crate::theme_file::choose(false) else {
            return;
        };
        let result = crate::theme_file::read(&path);
        match result {
            Ok(theme) => {
                let previous = std::mem::replace(&mut self.draft, theme);
                self.remember(previous);
                self.selected = None;
                self.message = Some("Theme imported. Apply it to update this widget.".into());
            }
            Err(e) => self.message = Some(e),
        }
    }
    fn export(&mut self) {
        if let Err(e) = self.draft.validate() {
            self.message = Some(e);
            return;
        }
        if let Some(path) = crate::theme_file::choose(true) {
            self.message = Some(match crate::theme_file::write(&path, &self.draft) {
                Ok(()) => "Theme exported. Account settings are not included.".into(),
                Err(e) => e,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn switching_widgets_preserves_unsaved_work_and_undo_redo() {
        let cfg = Config::default();
        let mut studio = Studio::new(Kind::Weather, &cfg);
        let before = studio.draft.clone();
        studio.draft.elements[1].x = 50.0;
        studio.remember(before);
        studio.select(Kind::Mail, &cfg);
        assert_eq!(studio.kind, Kind::Weather);
        assert_eq!(studio.draft.elements[1].x, 50.0);
        studio.undo();
        assert!(!studio.dirty());
        studio.redo();
        assert!(studio.dirty());
        assert_eq!(studio.draft.elements[1].x, 50.0);
        studio.discard();
        studio.select(Kind::Mail, &cfg);
        assert_eq!(studio.kind, Kind::Mail);
        assert!(!studio.dirty());
    }
    #[test]
    fn every_widget_loads_its_own_theme_without_changing_others() {
        let mut cfg = Config::default();
        let mut theme = Theme::starter(Kind::Mail);
        theme.width = 300.0;
        Kind::Mail.state_mut(&mut cfg).theme = Some(theme.clone());
        assert_eq!(Studio::new(Kind::Mail, &cfg).draft, theme);
        assert_eq!(
            Studio::new(Kind::Weather, &cfg).draft,
            Theme::starter(Kind::Weather)
        );
        assert!(Kind::Weather.state(&cfg).theme.is_none());
    }
    #[test]
    fn dragging_a_resize_handle_changes_size_without_moving_the_layer() {
        let ctx = egui::Context::default();
        let mut studio = Studio::new(Kind::Weather, &Config::default());
        studio.selected = Some(0);
        let mut bounds = egui::Rect::NOTHING;
        let mut frame = |studio: &mut Studio, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                bounds = studio.canvas(ui, 350.0);
            });
            output.textures_delta.clear();
            bounds
        };
        let canvas = frame(&mut studio, vec![]);
        let before = studio.draft.elements[0].clone();
        let at = canvas.min
            + egui::vec2(before.x + before.width, before.y + before.height) * studio.zoom
            - egui::vec2(2.0, 2.0);
        frame(&mut studio, vec![egui::Event::PointerMoved(at)]);
        frame(
            &mut studio,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let end = at + egui::vec2(12.0, 0.0);
        frame(&mut studio, vec![egui::Event::PointerMoved(end)]);
        frame(
            &mut studio,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let after = &studio.draft.elements[0];
        assert_eq!((after.x, after.y), (before.x, before.y));
        assert!(after.width > before.width);
    }
}
