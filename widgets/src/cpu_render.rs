//! A small software rasterizer for egui's output, so a pill can be drawn
//! without a GPU device. It does the same math as egui-wgpu on a gamma
//! framebuffer: vertex color times texel, both premultiplied and in gamma
//! space, blended "source over" with premultiplied alpha.

use std::collections::HashMap;

use eframe::egui::epaint::textures::{TextureFilter, TexturesDelta};
use eframe::egui::epaint::{ClippedPrimitive, ImageData, Primitive, TextureId, Vertex};
use eframe::egui::Color32;

struct Texture {
    size: [usize; 2],
    pixels: Vec<Color32>,
    nearest: bool,
}

/// The textures egui has asked the painter to keep.
#[derive(Default)]
pub struct Textures(HashMap<TextureId, Texture>);

impl Textures {
    /// Apply new and changed textures. Call before painting.
    pub fn set(&mut self, delta: &TexturesDelta) {
        for (id, changes) in &delta.set {
            for change in changes {
                let ImageData::Color(image) = &change.image;
                let nearest = change.options.magnification == TextureFilter::Nearest;
                match change.pos {
                    None => {
                        self.0.insert(
                            *id,
                            Texture {
                                size: image.size,
                                pixels: image.pixels.clone(),
                                nearest,
                            },
                        );
                    }
                    Some([left, top]) => {
                        let Some(texture) = self.0.get_mut(id) else {
                            continue;
                        };
                        let [w, h] = image.size;
                        for y in 0..h {
                            for x in 0..w {
                                let (tx, ty) = (left + x, top + y);
                                if tx < texture.size[0] && ty < texture.size[1] {
                                    texture.pixels[ty * texture.size[0] + tx] =
                                        image.pixels[y * w + x];
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Drop freed textures and mark the delta handled. Call after painting.
    pub fn free(&mut self, mut delta: TexturesDelta) {
        for id in &delta.free {
            self.0.remove(id);
        }
        delta.clear();
    }
}

/// Premultiplied RGBA pixels, row by row.
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[u8; 4]>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![[0; 4]; width * height],
        }
    }
}

/// Draw tessellated egui output onto `canvas`. Positions are in points and
/// are scaled by `ppp` to pixels.
pub fn paint(canvas: &mut Canvas, primitives: &[ClippedPrimitive], textures: &Textures, ppp: f32) {
    for primitive in primitives {
        let Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        let Some(texture) = textures.0.get(&mesh.texture_id) else {
            continue;
        };
        // Same rounding as egui-wgpu's scissor rectangle.
        let clip = primitive.clip_rect;
        let bound = |v: f32, max: usize| (v * ppp).round().clamp(0.0, max as f32) as usize;
        let clip = [
            bound(clip.min.x, canvas.width),
            bound(clip.min.y, canvas.height),
            bound(clip.max.x, canvas.width),
            bound(clip.max.y, canvas.height),
        ];
        if clip[0] >= clip[2] || clip[1] >= clip[3] {
            continue;
        }
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let vertex = |i: u32| mesh.vertices.get(i as usize);
            if let (Some(a), Some(b), Some(c)) = (
                vertex(triangle[0]),
                vertex(triangle[1]),
                vertex(triangle[2]),
            ) {
                triangle_fill(canvas, [a, b, c], texture, ppp, clip);
            }
        }
    }
}

type Point = (f32, f32);

/// Twice the signed area of (a, b, p): positive when p is on the inner side
/// of the edge a→b for a triangle wound so its own area is positive.
fn edge(a: Point, b: Point, p: Point) -> f32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

/// Top-left fill rule: a pixel centre exactly on an edge belongs to the
/// triangle only for top and left edges, so shared edges are drawn once.
fn covers(w: f32, a: Point, b: Point) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    w > 0.0 || (w == 0.0 && ((dy == 0.0 && dx > 0.0) || dy < 0.0))
}

fn triangle_fill(
    canvas: &mut Canvas,
    v: [&Vertex; 3],
    texture: &Texture,
    ppp: f32,
    clip: [usize; 4],
) {
    let mut v = v;
    let mut p = v.map(|v| (v.pos.x * ppp, v.pos.y * ppp));
    let mut area = edge(p[0], p[1], p[2]);
    if !area.is_finite() || area.abs() < 1e-6 {
        return;
    }
    if area < 0.0 {
        v.swap(1, 2);
        p.swap(1, 2);
        area = -area;
    }
    let min = |f: fn(&Point) -> f32| p.iter().map(f).fold(f32::INFINITY, f32::min);
    let max = |f: fn(&Point) -> f32| p.iter().map(f).fold(f32::NEG_INFINITY, f32::max);
    let x0 = (min(|q| q.0).floor().max(clip[0] as f32)) as usize;
    let y0 = (min(|q| q.1).floor().max(clip[1] as f32)) as usize;
    let x1 = (max(|q| q.0).ceil().min(clip[2] as f32)).max(0.0) as usize;
    let y1 = (max(|q| q.1).ceil().min(clip[3] as f32)).max(0.0) as usize;
    let colors = v.map(|v| v.color.to_array().map(f32::from));
    for y in y0..y1 {
        let py = y as f32 + 0.5;
        for x in x0..x1 {
            let q = (x as f32 + 0.5, py);
            let (w0, w1, w2) = (
                edge(p[1], p[2], q),
                edge(p[2], p[0], q),
                edge(p[0], p[1], q),
            );
            if !(covers(w0, p[1], p[2]) && covers(w1, p[2], p[0]) && covers(w2, p[0], p[1])) {
                continue;
            }
            let (l0, l1, l2) = (w0 / area, w1 / area, w2 / area);
            let u = l0 * v[0].uv.x + l1 * v[1].uv.x + l2 * v[2].uv.x;
            let t = l0 * v[0].uv.y + l1 * v[1].uv.y + l2 * v[2].uv.y;
            let texel = sample(texture, u, t);
            let dst = &mut canvas.pixels[y * canvas.width + x];
            let mut src = [0.0f32; 4];
            for (i, s) in src.iter_mut().enumerate() {
                let color = l0 * colors[0][i] + l1 * colors[1][i] + l2 * colors[2][i];
                *s = color * texel[i] / 255.0;
            }
            let keep = 1.0 - src[3] / 255.0;
            for i in 0..4 {
                dst[i] = (src[i] + dst[i] as f32 * keep).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// The texel at normalized (u, v), bilinear unless the texture asks for
/// nearest, clamped at the edges.
fn sample(texture: &Texture, u: f32, v: f32) -> [f32; 4] {
    let [w, h] = texture.size;
    if w == 0 || h == 0 {
        return [0.0; 4];
    }
    let at = |x: usize, y: usize| texture.pixels[y * w + x].to_array().map(f32::from);
    if texture.nearest {
        let x = ((u * w as f32).floor().max(0.0) as usize).min(w - 1);
        let y = ((v * h as f32).floor().max(0.0) as usize).min(h - 1);
        return at(x, y);
    }
    let fx = (u * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let fy = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    let mut out = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * tx;
        let bottom = c[i] + (d[i] - c[i]) * tx;
        out[i] = top + (bottom - top) * ty;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, epaint::Mesh, pos2, Rect};

    fn white_texture() -> Textures {
        let mut textures = Textures::default();
        textures.0.insert(
            TextureId::default(),
            Texture {
                size: [1, 1],
                pixels: vec![Color32::WHITE],
                nearest: false,
            },
        );
        textures
    }

    fn rect_mesh(rect: Rect, color: Color32) -> ClippedPrimitive {
        let mut mesh = Mesh::default();
        mesh.add_colored_rect(rect, color);
        ClippedPrimitive {
            clip_rect: Rect::EVERYTHING,
            primitive: Primitive::Mesh(mesh),
        }
    }

    #[test]
    fn a_rectangle_fills_exactly_its_pixels_and_shared_edges_once() {
        let mut canvas = Canvas::new(8, 4);
        // Half-transparent: a pixel drawn twice would come out darker.
        let color = Color32::from_rgba_premultiplied(100, 0, 0, 128);
        let shapes = [
            rect_mesh(Rect::from_min_max(pos2(1.0, 1.0), pos2(4.0, 3.0)), color),
            rect_mesh(Rect::from_min_max(pos2(4.0, 1.0), pos2(6.0, 3.0)), color),
        ];
        paint(&mut canvas, &shapes, &white_texture(), 1.0);
        for y in 0..4 {
            for x in 0..8 {
                let inside = (1..6).contains(&x) && (1..3).contains(&y);
                let want = if inside { [100, 0, 0, 128] } else { [0; 4] };
                assert_eq!(canvas.pixels[y * 8 + x], want, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn source_over_blends_premultiplied_colors() {
        let mut canvas = Canvas::new(1, 1);
        let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let shapes = [
            rect_mesh(full, Color32::from_rgb(0, 0, 200)),
            rect_mesh(full, Color32::from_rgba_premultiplied(100, 0, 0, 128)),
        ];
        paint(&mut canvas, &shapes, &white_texture(), 1.0);
        // 100 + 0 * 0.498; 200 * (1 - 128/255) = 99.6; alpha stays opaque.
        assert_eq!(canvas.pixels[0], [100, 0, 100, 255]);
    }

    #[test]
    fn clip_rectangles_and_scale_are_applied_in_pixels() {
        let mut canvas = Canvas::new(8, 8);
        let mut shape = rect_mesh(
            Rect::from_min_max(pos2(0.0, 0.0), pos2(4.0, 4.0)),
            Color32::WHITE,
        );
        shape.clip_rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.5, 4.0));
        paint(&mut canvas, &[shape], &white_texture(), 2.0);
        let lit: Vec<usize> = (0..8).filter(|x| canvas.pixels[*x][3] != 0).collect();
        assert_eq!(lit, [0, 1, 2]);
        assert!(canvas.pixels[7 * 8][3] != 0, "the scaled rect covers row 7");
    }

    #[test]
    fn textures_are_sampled_and_patched() {
        let mut textures = Textures::default();
        let id = TextureId::Managed(7);
        let mut delta = TexturesDelta::default();
        let image = egui::ColorImage::new([2, 1], vec![Color32::RED, Color32::GREEN]);
        delta.set.insert(
            id,
            [egui::epaint::ImageDelta::full(
                image,
                egui::TextureOptions::NEAREST,
            )]
            .into(),
        );
        textures.set(&delta);
        delta.clear();
        let mut patch = TexturesDelta::default();
        let blue = egui::ColorImage::new([1, 1], vec![Color32::BLUE]);
        patch.set.insert(
            id,
            [egui::epaint::ImageDelta::partial(
                [1, 0],
                blue,
                egui::TextureOptions::NEAREST,
            )]
            .into(),
        );
        textures.set(&patch);
        patch.clear();
        let mut mesh = Mesh::with_texture(id);
        mesh.add_rect_with_uv(
            Rect::from_min_max(pos2(0.0, 0.0), pos2(2.0, 1.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let mut canvas = Canvas::new(2, 1);
        let shape = ClippedPrimitive {
            clip_rect: Rect::EVERYTHING,
            primitive: Primitive::Mesh(mesh),
        };
        paint(&mut canvas, &[shape], &textures, 1.0);
        assert_eq!(canvas.pixels, [[255, 0, 0, 255], [0, 0, 255, 255]]);
        let mut free = TexturesDelta::default();
        free.free.insert(id);
        textures.free(free);
        assert!(textures.0.is_empty());
    }
}
