//! Bounded CPU rendering of egui meshes. Documents still use the same core compositor.
use crate::egui;
use std::{collections::HashMap, sync::Arc};
struct Texture {
    image: Arc<egui::ColorImage>,
    options: egui::TextureOptions,
}
#[derive(Default)]
pub(super) struct Painter {
    textures: HashMap<egui::TextureId, Texture>,
    pub pixels: Vec<u32>,
}
impl Painter {
    pub fn set(&mut self, delta: &egui::TexturesDelta) -> Result<(), String> {
        for (id, changes) in &delta.set {
            for change in changes {
                let egui::ImageData::Color(image) = &change.image;
                if image.size.contains(&0)
                    || image.size[0].checked_mul(image.size[1]) != Some(image.pixels.len())
                    || image.pixels.len() > 16_777_216
                {
                    return Err("Invalid/oversized software texture".into());
                }
                if let Some([x, y]) = change.pos {
                    let texture = self
                        .textures
                        .get_mut(id)
                        .ok_or("Software texture patch has no base")?;
                    if x.checked_add(image.size[0])
                        .is_none_or(|n| n > texture.image.size[0])
                        || y.checked_add(image.size[1])
                            .is_none_or(|n| n > texture.image.size[1])
                    {
                        return Err("Software texture patch outside base".into());
                    }
                    let base = Arc::make_mut(&mut texture.image);
                    for row in 0..image.size[1] {
                        let start = (y + row) * base.size[0] + x;
                        base.pixels[start..start + image.size[0]].copy_from_slice(
                            &image.pixels[row * image.size[0]..(row + 1) * image.size[0]],
                        );
                    }
                    texture.options = change.options;
                } else {
                    if self.textures.len() >= 4096 && !self.textures.contains_key(id) {
                        return Err("Software texture count limit".into());
                    }
                    self.textures.insert(
                        *id,
                        Texture {
                            image: image.clone(),
                            options: change.options,
                        },
                    );
                }
            }
        }
        Ok(())
    }
    pub fn free(&mut self, delta: &egui::TexturesDelta) {
        for id in &delta.free {
            self.textures.remove(id);
        }
    }
    pub fn paint(
        &mut self,
        size: [usize; 2],
        ppp: f32,
        primitives: &[egui::ClippedPrimitive],
        background: egui::Color32,
    ) -> Result<(), String> {
        if !ppp.is_finite() || ppp <= 0. {
            return Err("Invalid software display scale".into());
        }
        let count = size[0]
            .checked_mul(size[1])
            .filter(|n| *n <= 16_777_216)
            .ok_or("Software display limit16M pixels")?;
        self.pixels
            .try_reserve(count.saturating_sub(self.pixels.len()))
            .map_err(|e| e.to_string())?;
        self.pixels.resize(count, 0);
        self.pixels.fill(pack(background.to_array()));
        if size.contains(&0) {
            return Ok(());
        }
        let mut budget = 120_000_000usize;
        for primitive in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
                return Err("This graphics callback has no software implementation".into());
            };
            if !mesh.is_valid() || mesh.indices.len() > 3_000_000 {
                return Err("Invalid software mesh".into());
            }
            let texture = self
                .textures
                .get(&mesh.texture_id)
                .ok_or("Software mesh texture unavailable")?;
            let clip = primitive.clip_rect;
            if [clip.min.x, clip.min.y, clip.max.x, clip.max.y]
                .iter()
                .any(|n| n.is_nan())
            {
                return Err("Nonfinite software clip".into());
            }
            let clip = [
                (clip.min.x * ppp).floor().max(0.) as usize,
                (clip.min.y * ppp).floor().max(0.) as usize,
                ((clip.max.x * ppp).ceil().max(0.) as usize).min(size[0]),
                ((clip.max.y * ppp).ceil().max(0.) as usize).min(size[1]),
            ];
            for indices in mesh.indices.as_chunks::<3>().0 {
                let vertices =
                    [indices[0], indices[1], indices[2]].map(|i| mesh.vertices[i as usize]);
                triangle(
                    &mut self.pixels,
                    size,
                    clip,
                    ppp,
                    vertices,
                    texture,
                    &mut budget,
                )?;
            }
        }
        Ok(())
    }
}
fn pack(p: [u8; 4]) -> u32 {
    u32::from(p[2]) | (u32::from(p[1]) << 8) | (u32::from(p[0]) << 16) | 0xff00_0000
}
fn cross(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
fn top_left(a: [f64; 2], b: [f64; 2]) -> bool {
    b[1] < a[1] || (b[1] == a[1] && b[0] > a[0])
}
fn texel(t: &Texture, uv: [f64; 2]) -> [f64; 4] {
    let [w, h] = t.image.size;
    let index = |n: isize, len: usize| match t.options.wrap_mode {
        egui::TextureWrapMode::ClampToEdge => n.clamp(0, len as isize - 1) as usize,
        egui::TextureWrapMode::Repeat => n.rem_euclid(len as isize) as usize,
        egui::TextureWrapMode::MirroredRepeat => {
            let n = n.rem_euclid(len as isize * 2) as usize;
            if n >= len { 2 * len - 1 - n } else { n }
        }
    };
    let at = |x: isize, y: isize| {
        t.image.pixels[index(y, h) * w + index(x, w)]
            .to_array()
            .map(f64::from)
    };
    if t.options.magnification == egui::TextureFilter::Nearest {
        return at(
            (uv[0] * w as f64).floor() as isize,
            (uv[1] * h as f64).floor() as isize,
        );
    }
    let x = uv[0] * w as f64 - 0.5;
    let y = uv[1] * h as f64 - 0.5;
    let ix = x.floor();
    let iy = y.floor();
    let a = x - ix;
    let b = y - iy;
    let p = [
        at(ix as isize, iy as isize),
        at(ix as isize + 1, iy as isize),
        at(ix as isize, iy as isize + 1),
        at(ix as isize + 1, iy as isize + 1),
    ];
    std::array::from_fn(|c| {
        (p[0][c] * (1. - a) + p[1][c] * a) * (1. - b) + (p[2][c] * (1. - a) + p[3][c] * a) * b
    })
}
fn triangle(
    pixels: &mut [u32],
    size: [usize; 2],
    clip: [usize; 4],
    ppp: f32,
    mut v: [egui::epaint::Vertex; 3],
    texture: &Texture,
    budget: &mut usize,
) -> Result<(), String> {
    if v.iter().any(|v| !v.pos.is_finite() || !v.uv.is_finite()) {
        return Err("Nonfinite software vertex".into());
    }
    let mut p = v.map(|v| [f64::from(v.pos.x * ppp), f64::from(v.pos.y * ppp)]);
    let mut area = cross(p[0], p[1], p[2]);
    if area.abs() < 1e-10 {
        return Ok(());
    }
    if area < 0. {
        p.swap(1, 2);
        v.swap(1, 2);
        area = -area;
    }
    let x0 = (p
        .iter()
        .map(|p| p[0])
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.) as usize)
        .max(clip[0]);
    let y0 = (p
        .iter()
        .map(|p| p[1])
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.) as usize)
        .max(clip[1]);
    let x1 = (p
        .iter()
        .map(|p| p[0])
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .max(0.) as usize)
        .min(clip[2]);
    let y1 = (p
        .iter()
        .map(|p| p[1])
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .max(0.) as usize)
        .min(clip[3]);
    let cost = x1.saturating_sub(x0).saturating_mul(y1.saturating_sub(y0));
    *budget = budget
        .checked_sub(cost)
        .ok_or("Software display work limit; drawing remains in memory")?;
    let colors = v.map(|v| v.color.to_array().map(f64::from));
    for y in y0..y1 {
        for x in x0..x1 {
            let point = [x as f64 + 0.5, y as f64 + 0.5];
            let edges = [
                cross(p[1], p[2], point),
                cross(p[2], p[0], point),
                cross(p[0], p[1], point),
            ];
            let inclusive = [
                top_left(p[1], p[2]),
                top_left(p[2], p[0]),
                top_left(p[0], p[1]),
            ];
            if (0..3).any(|i| edges[i] < 0. || (edges[i] == 0. && !inclusive[i])) {
                continue;
            }
            let weights = edges.map(|n| n / area);
            let uv = [
                (0..3).map(|i| weights[i] * f64::from(v[i].uv.x)).sum(),
                (0..3).map(|i| weights[i] * f64::from(v[i].uv.y)).sum(),
            ];
            let color: [f64; 4] =
                std::array::from_fn(|c| (0..3).map(|i| weights[i] * colors[i][c]).sum());
            let sample = texel(texture, uv);
            let source: [f64; 4] = std::array::from_fn(|c| sample[c] * color[c] / 255.);
            let target = &mut pixels[y * size[0] + x];
            let previous = [
                ((*target >> 16) & 255) as f64,
                ((*target >> 8) & 255) as f64,
                (*target & 255) as f64,
            ];
            let result: [u8; 4] = std::array::from_fn(|c| {
                if c == 3 {
                    255
                } else {
                    (source[c] + previous[c] * (1. - source[3] / 255.))
                        .round()
                        .clamp(0., 255.) as u8
                }
            });
            *target = pack(result);
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn mesh() -> egui::ClippedPrimitive {
        let mut m = egui::Mesh::with_texture(egui::TextureId::Managed(1));
        m.add_rect_with_uv(
            egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(4., 4.)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
            egui::Color32::from_rgba_premultiplied(100, 0, 0, 128),
        );
        egui::ClippedPrimitive {
            clip_rect: egui::Rect::EVERYTHING,
            primitive: egui::epaint::Primitive::Mesh(m),
        }
    }
    #[test]
    fn premultiplied_shared_edge_is_blended_once_and_clipping_is_exact() {
        let mut p = Painter::default();
        p.textures.insert(
            egui::TextureId::Managed(1),
            Texture {
                image: Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::WHITE)),
                options: egui::TextureOptions::NEAREST,
            },
        );
        let mut primitive = mesh();
        primitive.clip_rect = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(4., 4.));
        p.paint([4, 4], 1., &[primitive.clone()], egui::Color32::BLACK)
            .unwrap();
        assert!(p.pixels.iter().all(|p| *p == 0xff64_0000));
        primitive.clip_rect = egui::Rect::from_min_max(egui::pos2(1., 1.), egui::pos2(3., 3.));
        p.paint([4, 4], 1., &[primitive], egui::Color32::BLACK)
            .unwrap();
        assert_eq!(p.pixels.iter().filter(|p| **p == 0xff64_0000).count(), 4);
    }
    #[test]
    fn ordered_partial_texture_updates_wrapping_and_free_are_preserved() {
        let mut p = Painter::default();
        let id = egui::TextureId::Managed(1);
        let mut delta = egui::TexturesDelta::default();
        delta.push(
            id,
            egui::epaint::ImageDelta::full(
                egui::ColorImage::filled([2, 2], egui::Color32::RED),
                egui::TextureOptions::NEAREST,
            ),
        );
        delta.push(
            id,
            egui::epaint::ImageDelta::partial(
                [1, 0],
                egui::ColorImage::filled([1, 1], egui::Color32::GREEN),
                egui::TextureOptions::NEAREST,
            ),
        );
        p.set(&delta).unwrap();
        assert_eq!(p.textures[&id].image.pixels[1], egui::Color32::GREEN);
        assert_eq!(p.textures[&id].image.pixels[0], egui::Color32::RED);
        let t = p.textures.get_mut(&id).unwrap();
        t.options.wrap_mode = egui::TextureWrapMode::Repeat;
        assert_eq!(texel(t, [1.75, 0.25]), [0., 255., 0., 255.]);
        let mut free = egui::TexturesDelta::default();
        free.free(id);
        p.free(&free);
        assert!(!p.textures.contains_key(&id));
        assert!(
            p.set(&{
                let mut d = egui::TexturesDelta::default();
                d.push(
                    id,
                    egui::epaint::ImageDelta::partial(
                        [0, 0],
                        egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
                        egui::TextureOptions::NEAREST,
                    ),
                );
                d
            })
            .is_err()
        );
    }
    #[test]
    fn invalid_scale_and_nonfinite_vertices_reject_before_indexing() {
        let mut p = Painter::default();
        assert!(
            p.paint([1, 1], f32::NAN, &[], egui::Color32::BLACK)
                .is_err()
        );
        assert!(
            p.paint([usize::MAX, 2], 1., &[], egui::Color32::BLACK)
                .is_err()
        );
        p.textures.insert(
            egui::TextureId::Managed(1),
            Texture {
                image: Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::WHITE)),
                options: egui::TextureOptions::NEAREST,
            },
        );
        let mut bad = mesh();
        if let egui::epaint::Primitive::Mesh(m) = &mut bad.primitive {
            m.vertices[0].pos.x = f32::NAN;
        }
        assert!(p.paint([4, 4], 1., &[bad], egui::Color32::BLACK).is_err());
    }
}
