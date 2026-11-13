//! Yr weather icons: download an SVG, rasterize it and keep it as a texture.
//! Downloads run on a thread of their own so a slow server never freezes the
//! widget; until an icon arrives the widget simply has none.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;

/// A failed icon is tried again after this long.
const RETRY_AFTER: Duration = Duration::from_secs(600);

enum Slot {
    Loading,
    Ready(egui::TextureHandle),
    Failed(Instant),
}

type Loaded = (String, Option<egui::ColorImage>);

pub struct IconCache {
    slots: HashMap<String, Slot>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
}

impl Default for IconCache {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            slots: HashMap::new(),
            tx,
            rx,
        }
    }
}

impl IconCache {
    /// The icon at `url`, rasterized at `px` pixels square, or `None` while it
    /// is loading or after it failed. Never blocks.
    pub fn get(&mut self, ctx: &egui::Context, url: &str, px: u32) -> Option<egui::TextureHandle> {
        while let Ok((done, image)) = self.rx.try_recv() {
            let slot = match image {
                Some(image) => {
                    Slot::Ready(ctx.load_texture(&done, image, egui::TextureOptions::LINEAR))
                }
                None => Slot::Failed(Instant::now()),
            };
            self.slots.insert(done, slot);
        }
        match self.slots.get(url) {
            Some(Slot::Ready(tex)) => return Some(tex.clone()),
            Some(Slot::Loading) => return None,
            Some(Slot::Failed(at)) if at.elapsed() < RETRY_AFTER => return None,
            _ => {}
        }
        self.slots.insert(url.to_owned(), Slot::Loading);
        let (tx, ctx, url) = (self.tx.clone(), ctx.clone(), url.to_owned());
        std::thread::spawn(move || {
            // In a child process: see `http::get_isolated`.
            let image = crate::http::get_isolated(&url).and_then(|bytes| raster(&bytes, px));
            if image.is_none() {
                crate::log::line("weather", &format!("cannot load icon {url}"));
            }
            let _ = tx.send((url, image));
            ctx.request_repaint();
        });
        None
    }
}

fn raster(bytes: &[u8], px: u32) -> Option<egui::ColorImage> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    // Fit the SVG viewBox into the square texture and centre it; an identity
    // transform would draw it 1:1 and crop anything larger than `px`.
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    let tx = (px as f32 - size.width() * scale) / 2.0;
    let ty = (px as f32 - size.height() * scale) / 2.0;
    let transform = usvg::Transform::from_row(scale, 0.0, 0.0, scale, tx, ty);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia is premultiplied; egui wants straight alpha.
    let src = pixmap.data();
    let mut out = Vec::with_capacity(src.len());
    for p in src.as_chunks::<4>().0 {
        let a = p[3] as u16;
        let straight = |c: u8| (c as u16 * 255).checked_div(a).unwrap_or(0) as u8;
        out.extend_from_slice(&[straight(p[0]), straight(p[1]), straight(p[2]), p[3]]);
    }
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [px as usize, px as usize],
        &out,
    ))
}
