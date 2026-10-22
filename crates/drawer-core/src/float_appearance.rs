use super::*;
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, PlacedRaster},
    layer_search::Key,
};
pub(super) struct Cache {
    extent: PlacedRaster,
    pixels: Vec<[f32; 4]>,
}
impl Cache {
    pub(super) fn bytes(&self) -> usize {
        self.pixels.len() * 16
    }
    fn sample(&self, p: [i64; 2]) -> Sample {
        let x = p[0] - self.extent.offset[0];
        let y = p[1] - self.extent.offset[1];
        if x < 0 || y < 0 || x >= self.extent.width as i64 || y >= self.extent.height as i64 {
            return Sample::default();
        }
        Sample::new(
            self.pixels[y as usize * self.extent.width as usize + x as usize].map(f64::from),
        )
        .unwrap()
    }
}
fn maxima(values: &[f32], w: usize, h: usize, r: usize, horizontal: bool) -> Vec<f32> {
    if r == 0 {
        return values.to_vec();
    }
    let mut out = vec![0.; values.len()];
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let index = |line: usize, i: usize| {
        if horizontal {
            line * w + i
        } else {
            i * w + line
        }
    };
    for line in 0..lines {
        let mut queue = std::collections::VecDeque::<usize>::new();
        let mut next = 0;
        for i in 0..len {
            let end = i.saturating_add(r).min(len - 1);
            while next <= end {
                while queue
                    .back()
                    .is_some_and(|n| values[index(line, *n)] <= values[index(line, next)])
                {
                    queue.pop_back();
                }
                queue.push_back(next);
                next += 1;
            }
            let start = i.saturating_sub(r);
            while queue.front().is_some_and(|n| *n < start) {
                queue.pop_front();
            }
            out[index(line, i)] = values[index(line, *queue.front().unwrap())];
        }
    }
    out
}
impl Renderer<'_> {
    pub(super) fn cached_display(&self, target: Target, point: Point) -> Option<Sample> {
        self.plan()
            .caches
            .get(&target)
            .map(|c| c.sample(point.map(|v| v.floor() as i64)))
    }
    pub(super) fn build_display_caches(&mut self) -> Result<(), String> {
        let d = self.document;
        let targets = d
            .scene
            .appearance
            .rasters
            .iter()
            .map(|(id, s)| (Target::Raster(*id), s.clone()))
            .chain(
                d.scene
                    .appearance
                    .vectors
                    .iter()
                    .map(|(id, s)| (Target::Vector(*id), s.clone())),
            )
            .collect::<Vec<_>>();
        let mut total = 0usize;
        for (target, style) in targets {
            if !style.enabled
                || style.border == 0 && style.rim == 0.
                || d.scene.file_objects.get(Key::Layer(target)).is_some()
            {
                continue;
            }
            let extent = match target {
                Target::Raster(id) => {
                    crate::layers::index(d, id).map(|n| d.layers[n].extent(d.width, d.height))
                }
                Target::Vector(id) => {
                    let l = d
                        .scene
                        .lines
                        .layers
                        .iter()
                        .find(|l| l.id == id)
                        .ok_or("Missing vector display source")?;
                    crate::appearance::vector_extent(&d.scene, l)?
                }
                _ => None,
            };
            let Some(mut extent) = extent else {
                continue;
            };
            let r = style.border as usize;
            let w = extent.width as usize + 2 * r;
            let h = extent.height as usize + 2 * r;
            let n = w.saturating_mul(h);
            total = total.saturating_add(n);
            let points = if let Target::Vector(id) = target {
                d.scene
                    .lines
                    .layers
                    .iter()
                    .find(|l| l.id == id)
                    .unwrap()
                    .paths
                    .iter()
                    .map(|p| p.points.len())
                    .sum::<usize>()
            } else {
                0
            };
            let existing = self
                .plan()
                .caches
                .values()
                .map(|c| c.pixels.len().saturating_mul(16))
                .sum::<usize>();
            let pending = self
                .stroke
                .map_or(0, |s| s.changed_pixels().saturating_mul(256));
            if w > 32768
                || h > 32768
                || n > 4 * 1024 * 1024
                || total > 16 * 1024 * 1024
                || n.saturating_mul(
                    points
                        + 8
                        + style.scope.as_ref().map_or(0, |s| {
                            s.mask
                                .as_ref()
                                .map_or(1, |m| m.runs.len().max(1).ilog2() as usize + 1)
                        }),
                ) > 64 * 1024 * 1024
                || d.resident_bytes()
                    .saturating_add(pending)
                    .saturating_add(existing)
                    .saturating_add(n.saturating_mul(28))
                    .saturating_add(8 * 1024 * 1024)
                    > DEFAULT_DOCUMENT_BUDGET
            {
                return Err(
                    "Display effects exceed float cache extent, work or memory budget".into(),
                );
            }
            extent.offset[0] -= r as i64;
            extent.offset[1] -= r as i64;
            extent.width = w as u32;
            extent.height = h as u32;
            let mut pixels = Vec::new();
            pixels.try_reserve_exact(n).map_err(|e| e.to_string())?;
            for y in 0..h {
                for x in 0..w {
                    let p = [extent.offset[0] + x as i64, extent.offset[1] + y as i64];
                    let point = p.map(|v| v as f64 + 0.5);
                    let original = d.scene.transform.inverse().map(point);
                    let raw = match target {
                        Target::Raster(id) => {
                            self.raster_raw(crate::layers::index(d, id).unwrap(), p, original)?
                        }
                        Target::Vector(id) => self.vector_raw(id, original)?,
                        _ => unreachable!(),
                    };
                    pixels.push(self.expression(raw, target)?.components().map(|v| v as f32));
                }
            }
            let alpha = pixels.iter().map(|p| p[3]).collect::<Vec<_>>();
            let horizontal = maxima(&alpha, w, h, r, true);
            let expanded = maxima(&horizontal, w, h, r, false);
            drop(horizontal);
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let raw = Sample::new(pixels[i].map(f64::from))?;
                    let mut ink = raw;
                    if ink.alpha() > 0. {
                        if let Some(c) = style.color {
                            ink = ink.with_rgb(c.map(|v| v as f64 / 255.))?;
                        }
                        if style.rim > 0. {
                            let neighbor = [
                                x.checked_sub(1).map(|a| y * w + a),
                                (x + 1 < w).then_some(y * w + x + 1),
                                y.checked_sub(1).map(|a| a * w + x),
                                (y + 1 < h).then_some((y + 1) * w + x),
                            ]
                            .into_iter()
                            .map(|n| n.map_or(0., |n| alpha[n]))
                            .fold(f32::INFINITY, f32::min);
                            let edge = (1. - neighbor as f64 / ink.alpha()).max(0.);
                            let source = self.plan().space.to_srgb(ink)?;
                            ink =
                                self.plan().space.from_srgb(source.with_rgb(
                                    source.rgb().map(|v| v * (1. - style.rim * edge)),
                                )?)?;
                        }
                    }
                    let border = Sample::from8(style.border_color)
                        .coverage((expanded[i] - alpha[i]).max(0.) as f64)?;
                    let after = self.plan().space.compose(
                        border,
                        ink,
                        crate::layer_blend::Mode::Normal,
                        1.,
                    )?;
                    let original = d.scene.transform.inverse().map([
                        extent.offset[0] as f64 + x as f64 + 0.5,
                        extent.offset[1] as f64 + y as f64 + 0.5,
                    ]);
                    pixels[i] = self
                        .plan()
                        .space
                        .mix(
                            raw,
                            after,
                            style.weight(original),
                            crate::color_mix::Space::Standard,
                            0.,
                        )?
                        .components()
                        .map(|v| v as f32);
                }
            }
            self.insert_cache(target, Cache { extent, pixels })?;
        }
        Ok(())
    }
}
