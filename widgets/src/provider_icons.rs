//! Original CodeZeno provider marks, rather than ambiguous initials.
//! Lobe Icons (MIT) + GitHub mark from Simple Icons (CC0). See third-party notices.
use codezeno_usage_core::providers::ProviderId;
use eframe::egui;
pub const FAMILY: &str = "provider-marks";
pub fn install(fonts: &mut egui::FontDefinitions) {
    fonts.font_data.insert(
        FAMILY.into(),
        egui::FontData::from_static(include_bytes!("../assets/provider-marks.ttf")).into(),
    );
    fonts
        .families
        .insert(egui::FontFamily::Name(FAMILY.into()), vec![FAMILY.into()]);
}
pub fn glyph(provider: ProviderId) -> char {
    match provider {
        ProviderId::Claude => '\u{f001}',
        ProviderId::Codex => '\u{f002}',
        ProviderId::Antigravity => '\u{f003}',
        ProviderId::OpenCode => '\u{f004}',
        ProviderId::Cursor => '\u{f005}',
        ProviderId::Grok => '\u{f006}',
        ProviderId::Copilot => '\u{f007}',
    }
}
pub fn paint(ui: &egui::Ui, rect: egui::Rect, provider: ProviderId) {
    let color = if provider == ProviderId::Claude {
        egui::Color32::from_rgb(217, 119, 87)
    } else {
        egui::Color32::from_gray(240)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph(provider),
        egui::FontId::new(
            rect.height().min(rect.width()),
            egui::FontFamily::Name(FAMILY.into()),
        ),
        color,
    );
}
pub fn from_key(key: &str) -> Option<ProviderId> {
    key.strip_prefix("provider:").and_then(ProviderId::from_key)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_provider_has_a_distinct_real_glyph() {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::default();
        install(&mut fonts);
        ctx.set_fonts(fonts);
        let mut seen = std::collections::HashSet::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for p in ProviderId::ALL {
                assert!(seen.insert(glyph(p)));
                let galley = ui.ctx().fonts_mut(|f| {
                    f.layout_no_wrap(
                        glyph(p).to_string(),
                        egui::FontId::new(18.0, egui::FontFamily::Name(FAMILY.into())),
                        egui::Color32::WHITE,
                    )
                });
                assert!(!galley.rows[0].glyphs[0].uv_rect.is_nothing());
            }
        });
        output.textures_delta.clear();
    }
}
