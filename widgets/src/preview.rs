//! Debug-only renderer readback for synthetic dashboard fixtures, not desktop capture.
pub fn capture(ctx: &eframe::egui::Context, frame: usize) {
    if frame != 8 || !std::env::args().any(|a| a == "--preview") {
        return;
    }
    let Some(path) =
        std::env::args().find_map(|a| a.strip_prefix("--preview-shot=").map(str::to_owned))
    else {
        return;
    };
    ctx.request_screenshot(move |image| {
        let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
        let result = image::save_buffer(
            &path,
            &pixels,
            image.size[0] as u32,
            image.size[1] as u32,
            image::ColorType::Rgba8,
        );
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    });
}
