use super::*;
use crate::{
    color_precision::{Depth, Sample},
    color_profile::WorkingSpace,
};
pub(super) struct Floating {
    space: WorkingSpace,
    pixels: Vec<Pixel>,
    carried: Option<Sample>,
}
struct Pixel {
    before: Sample,
    unmasked: Sample,
    after: Sample,
}
pub(super) struct Dab {
    pub offset: usize,
    pub previous: Option<usize>,
    pub amount: f64,
    pub limited: bool,
    pub color: Sample,
    pub point: [i64; 2],
    pub paint_factor: f64,
    pub mask: u8,
    pub special: bool,
}
pub(super) struct ResultPixel {
    before: Sample,
    unmasked: Sample,
    after: Sample,
}
impl ResultPixel {
    pub fn changed(&self) -> bool {
        self.before != self.after
    }
    pub fn preview(&self) -> [u8; 4] {
        self.after.rgba8()
    }
}
impl Floating {
    pub fn new(d: &Document, _style: Style) -> Result<Self, String> {
        Ok(Self {
            space: WorkingSpace::new(&d.color.profile)?,
            pixels: vec![],
            carried: None,
        })
    }
    pub fn compute(
        &self,
        d: &Document,
        layer: usize,
        style: Style,
        dab: Dab,
    ) -> Result<ResultPixel, String> {
        let before = dab.previous.map_or_else(
            || d.layers[layer].sample_at(dab.offset),
            |n| self.pixels[n].before,
        );
        let base = if dab.limited {
            before
        } else {
            dab.previous.map_or(before, |n| self.pixels[n].unmasked)
        };
        let ink = dab.color;
        let composition = style.material.composition;
        let unmasked = if style.clone_stamp.enabled {
            let point = [
                dab.point[0] + style.clone_stamp.offset[0],
                dab.point[1] + style.clone_stamp.offset[1],
            ];
            let source = if point[0] >= 0
                && point[1] >= 0
                && point[0] < d.width as i64
                && point[1] < d.height as i64
            {
                d.layers[layer].sample(d.width, d.height, point[0], point[1])
            } else {
                Sample::default()
            };
            if style.material.alpha_lock && source.alpha() == 0. {
                base
            } else {
                self.space.mix(
                    base,
                    source,
                    dab.amount,
                    crate::color_mix::Space::Standard,
                    0.,
                )?
            }
        } else if style.tonal() {
            self.space.tonal(base, style.material.tonal, dab.amount)?
        } else if style.pixel_overwrite() {
            Sample::from8(style.color)
        } else if style.erases() {
            base.coverage(1. - dab.amount)?
        } else if matches!(
            style.material.mode,
            crate::brush_material::Mode::Smudge | crate::brush_material::Mode::RunningColor
        ) || style.material.colorless
        {
            self.space.mix(
                base,
                ink,
                dab.amount,
                style.material.space,
                style.material.brightness,
            )?
        } else {
            self.space.brush(
                base,
                ink,
                composition.mode,
                dab.amount,
                dab.point,
                composition.seed,
            )?
        };
        let factor = dab.mask as f64 / 255. * dab.paint_factor;
        let after = if style.pixel_overwrite() {
            self.space.mix(
                before,
                ink.coverage(dab.amount * dab.paint_factor)?,
                dab.mask as f64 / 255.,
                crate::color_mix::Space::Standard,
                0.,
            )?
        } else if dab.special {
            self.space.brush(
                before,
                ink,
                composition.mode,
                dab.amount * factor,
                dab.point,
                composition.seed,
            )?
        } else {
            self.space.mix(
                before,
                unmasked,
                factor,
                crate::color_mix::Space::Standard,
                0.,
            )?
        };
        let after = if style.material.alpha_lock {
            let rgb = if after.alpha() == 0. {
                before.rgb()
            } else {
                after.rgb()
            };
            Sample::new([rgb[0], rgb[1], rgb[2], before.alpha()])?
        } else {
            after
        };
        Ok(ResultPixel {
            before,
            unmasked,
            after,
        })
    }
    pub fn record(&mut self, n: usize, value: ResultPixel) -> Result<(), String> {
        let pixel = Pixel {
            before: value.before,
            unmasked: value.unmasked,
            after: value.after,
        };
        if n == self.pixels.len() {
            self.pixels.try_reserve(1).map_err(|e| e.to_string())?;
            self.pixels.push(pixel);
        } else {
            self.pixels[n] = pixel;
        }
        Ok(())
    }
    pub fn sample(&self, n: usize) -> Sample {
        self.pixels[n].after
    }
}
impl Stroke {
    pub(super) fn finish_float_edges(
        &mut self,
        doc: &Document,
        cancel: impl Fn() -> bool,
    ) -> Result<(), String> {
        let extent = doc.layers[self.layer].extent(doc.width, doc.height);
        let f = self.floating.as_ref().unwrap();
        let mut result = Vec::with_capacity(self.changes.len());
        for (n, change) in self.changes.iter().enumerate() {
            if n % 256 == 0 && cancel() {
                return Err("Water edge processing cancelled".into());
            }
            let p = [
                extent.offset[0] + (change.offset % extent.width as usize) as i64,
                extent.offset[1] + (change.offset / extent.width as usize) as i64,
            ];
            let current = &f.pixels[n];
            let boundary = current.after != current.before
                && current.after.alpha() > 0.
                && [[-1, 0], [1, 0], [0, -1], [0, 1]].iter().any(|q| {
                    doc.layers[self.layer]
                        .pixel_index(doc.width, doc.height, p[0] + q[0], p[1] + q[1])
                        .and_then(|offset| self.lookup.get(&offset))
                        .is_none_or(|i| {
                            let v = &f.pixels[*i];
                            v.after == v.before || v.after.alpha() == 0.
                        })
                });
            let factor = if boundary {
                self.style.material.edge
                    * self.style.opacity
                    * self.coverage(doc, [p[0] as f64 + 0.5, p[1] as f64 + 0.5]) as f64
                    / 255.
            } else {
                0.
            };
            let after = if factor > 0. {
                let srgb = f.space.to_srgb(current.after)?;
                f.space
                    .from_srgb(srgb.with_rgb(srgb.rgb().map(|v| v * (1. - 0.5 * factor)))?)?
            } else {
                current.after
            };
            result.push(after);
        }
        let f = self.floating.as_mut().unwrap();
        for (n, p) in result.into_iter().enumerate() {
            f.pixels[n].after = p;
            f.pixels[n].unmasked = p;
            self.changes[n].after = p.rgba8();
            self.unmasked[n] = p.rgba8();
        }
        self.edges_done = true;
        self.direct = true;
        self.preview_revision += 1;
        Ok(())
    }
    pub(super) fn diffuse_float(
        &mut self,
        doc: &Document,
        strength: f64,
        positions: Vec<usize>,
        cancel: impl Fn() -> bool,
    ) -> Result<(), String> {
        let layer = &doc.layers[self.layer];
        let extent = layer.extent(doc.width, doc.height);
        let space = &self.floating.as_ref().unwrap().space;
        let mut pixels = Vec::with_capacity(positions.len());
        let mut changes = Vec::with_capacity(positions.len());
        for (n, offset) in positions.into_iter().enumerate() {
            if n % 256 == 0 && cancel() {
                return Err("Floating diffusion cancelled".into());
            }
            let x = extent.offset[0] + (offset % extent.width as usize) as i64;
            let y = extent.offset[1] + (offset / extent.width as usize) as i64;
            let frozen: &Stroke = self;
            let average = space.average(
                (-1..=1).flat_map(|dy| {
                    (-1..=1).map(move |dx| (frozen.float_candidate(doc, x + dx, y + dy, true), 1.))
                }),
                crate::color_mix::Space::Standard,
            )?;
            let original = layer.sample_at(offset);
            let current = self.float_candidate(doc, x, y, true);
            if self.style.material.alpha_lock && original.alpha() == 0. {
                continue;
            }
            let desired = space.mix(
                current,
                average,
                strength,
                crate::color_mix::Space::Standard,
                0.,
            )?;
            let desired = if self.style.material.keep_density {
                with_alpha(desired, desired.alpha().max(current.alpha()), current)?
            } else {
                desired
            };
            let point = [x as f64 + 0.5, y as f64 + 0.5];
            let factor = self.style.opacity
                * if self.style.advanced_texture() {
                    1.
                } else {
                    self.materials.factor(self.style.texture, point)
                }
                * self.coverage(doc, point) as f64
                / 255.;
            let after = space.mix(
                original,
                desired,
                factor,
                crate::color_mix::Space::Standard,
                0.,
            )?;
            let after = if self.style.material.alpha_lock {
                with_alpha(after, original.alpha(), original)?
            } else {
                after
            };
            if after != original {
                pixels.push(Pixel {
                    before: original,
                    unmasked: after,
                    after,
                });
                changes.push(PixelChange {
                    offset,
                    before: original.rgba8(),
                    after: after.rgba8(),
                });
            }
        }
        self.lookup.clear();
        self.unmasked.clear();
        self.amounts.clear();
        self.lookup
            .try_reserve(changes.len())
            .map_err(|e| e.to_string())?;
        self.unmasked
            .try_reserve(changes.len())
            .map_err(|e| e.to_string())?;
        self.amounts
            .try_reserve(changes.len())
            .map_err(|e| e.to_string())?;
        for (n, c) in changes.iter().enumerate() {
            self.lookup.insert(c.offset, n);
            self.unmasked.push(c.after);
            self.amounts.push(0.);
        }
        self.changes = changes;
        self.floating.as_mut().unwrap().pixels = pixels;
        self.diffused = true;
        self.direct = true;
        self.preview_revision += 1;
        Ok(())
    }
    fn float_candidate(&self, doc: &Document, x: i64, y: i64, raw: bool) -> Sample {
        if x < 0 || y < 0 || x >= doc.width as i64 || y >= doc.height as i64 {
            return Sample::default();
        }
        let layer = &doc.layers[self.layer];
        let Some(offset) = layer.pixel_index(doc.width, doc.height, x, y) else {
            return Sample::default();
        };
        self.lookup.get(&offset).map_or_else(
            || layer.sample_at(offset),
            |n| {
                let p = &self.floating.as_ref().unwrap().pixels[*n];
                if raw { p.unmasked } else { p.after }
            },
        )
    }
    pub(super) fn initialize_float_material(&mut self, doc: &Document, point: Point) {
        let sample =
            self.float_candidate(doc, point[0].floor() as i64, point[1].floor() as i64, false);
        self.floating.as_mut().unwrap().carried = Some(sample);
    }
    pub(super) fn float_pigment(
        &mut self,
        doc: &Document,
        center: Point,
        pressure: f64,
        radii: [f64; 2],
        rotation: [f64; 2],
    ) -> Result<Sample, String> {
        use crate::{brush_material::Mode, mouse_pressure::Effect};
        let f = self.floating.as_ref().unwrap();
        let mut m = self.style.material;
        if m.mode == Mode::Water && pressure < m.supply_threshold {
            m.supply = 0.;
        }
        m.mixing *= m.composition.mixing_pressure.map(Some(pressure));
        let picked = if m.mode == Mode::RunningColor && m.quality == crate::color_mix::Quality::High
        {
            let mut samples = Vec::with_capacity(9);
            let [cos, sin] = rotation;
            for dy in [-0.5, 0., 0.5] {
                for dx in [-0.5, 0., 0.5] {
                    let p = [
                        center[0] + dx * radii[0] * cos - dy * radii[1] * sin,
                        center[1] + dx * radii[0] * sin + dy * radii[1] * cos,
                    ];
                    if p[0] >= 0.
                        && p[1] >= 0.
                        && p[0] < doc.width as f64
                        && p[1] < doc.height as f64
                    {
                        samples.push((
                            self.float_candidate(
                                doc,
                                p[0].floor() as i64,
                                p[1].floor() as i64,
                                false,
                            ),
                            self.coverage(doc, p) as f64 / 255.,
                        ));
                    }
                }
            }
            f.space.average(samples, m.space)?
        } else if m.mode == Mode::RunningColor && self.coverage(doc, center) == 0 {
            Sample::default()
        } else {
            self.float_candidate(
                doc,
                center[0].floor() as i64,
                center[1].floor() as i64,
                false,
            )
        };
        let foreground = Sample::from8(self.dab_color);
        let blend = |a, b, w| f.space.mix(a, b, w, m.space, m.brightness);
        let mut carried = f.carried;
        let color = if !m.pigment() {
            match self.effect {
                Effect::Blend => blend(foreground, picked, self.strength)?,
                Effect::Smudge => {
                    let old = carried.unwrap_or(picked);
                    carried = Some(blend(old, picked, self.strength)?);
                    old
                }
                _ => foreground,
            }
        } else if m.mode == Mode::RunningColor {
            let ink = if m.colorless {
                picked
            } else {
                foreground.coverage(m.supply)?
            };
            let local = blend(ink, picked, m.mixing)?;
            if m.keep_density {
                with_alpha(local, local.alpha().max(picked.alpha()), picked)?
            } else {
                local
            }
        } else if m.mode == Mode::Paint && !m.colorless {
            foreground.coverage(m.supply)?
        } else {
            let previous = carried.unwrap_or(picked);
            let distance = self
                .material_last
                .map_or(0., |last| (center[0] - last[0]).hypot(center[1] - last[1]));
            let sampled = if m.keep_density {
                with_alpha(picked, picked.alpha().max(previous.alpha()), previous)?
            } else {
                picked
            };
            let pickup = if m.smudge_distance == 0. {
                m.mixing * (1. - m.retention)
            } else {
                m.mixing * (1. - m.retention.powf(distance / m.smudge_distance))
            };
            let pigment = blend(previous, sampled, pickup)?;
            let supply = if m.colorless || m.mode == Mode::Smudge {
                0.
            } else {
                m.supply
            };
            let replenish = if self.material_last.is_none() {
                supply
            } else {
                1. - (1. - supply).powf(distance / self.style.shape.size.max(0.1))
            };
            let pigment = blend(pigment, foreground, replenish)?;
            let pigment = if m.keep_density {
                with_alpha(pigment, pigment.alpha().max(previous.alpha()), previous)?
            } else {
                pigment
            };
            carried = Some(pigment);
            pigment
        };
        self.floating.as_mut().unwrap().carried = carried;
        color.coverage(1. - m.dilution)
    }
    pub(super) fn float_dual(
        &self,
        a: f64,
        b: f64,
        primary: Sample,
    ) -> Result<(f64, Sample), String> {
        let space = &self.floating.as_ref().unwrap().space;
        let (coverage, ink) = self.style.variation.dual.ink_float(
            a,
            b,
            space.to_srgb(primary)?,
            self.style.dual_rgb(),
            space.to_srgb(Sample::from8([
                self.style.variation.dual.controls.color[0],
                self.style.variation.dual.controls.color[1],
                self.style.variation.dual.controls.color[2],
                255,
            ]))?,
        )?;
        Ok((coverage, space.from_srgb(ink)?))
    }
    pub fn float_pixel(&self, doc: &Document, layer: usize, x: i64, y: i64) -> Option<Sample> {
        if layer != self.layer || doc.id != self.document || doc.revision != self.revision {
            return None;
        }
        let offset = doc.layers[layer].pixel_index(doc.width, doc.height, x, y)?;
        self.lookup
            .get(&offset)
            .and_then(|n| {
                self.floating.as_ref().and_then(|f| {
                    let p = &f.pixels[*n];
                    (p.before != p.after).then_some(f.sample(*n))
                })
            })
            .or_else(|| {
                self.arms
                    .iter()
                    .find_map(|a| a.stroke.float_pixel(doc, layer, x, y))
            })
    }
    pub(super) fn commit_float(
        self,
        doc: &mut Document,
        metadata: Option<crate::mouse_pressure::Metadata>,
    ) -> Result<bool, String> {
        let mut state = doc.snapshot();
        let target = &mut state.layers[self.layer];
        let mut precise = if doc.color.depth == Depth::Sixteen {
            Some(target.precise.clone().unwrap_or_else(|| {
                target
                    .pixels
                    .iter()
                    .map(|p| Sample::from8(*p).rgba16())
                    .collect()
            }))
        } else {
            None
        };
        let mut seen = std::collections::HashSet::new();
        let mut changed = false;
        let source = std::iter::once(&self).chain(self.arms.iter().map(|a| a.stroke.as_ref()));
        let extent = target.extent(doc.width, doc.height);
        let width = extent.width as usize;
        for stroke in source {
            let floating = stroke
                .floating
                .as_ref()
                .ok_or("Missing frozen floating stroke")?;
            for (change, pixel) in stroke.changes.iter().zip(&floating.pixels) {
                if pixel.before == pixel.after || !seen.insert(change.offset) {
                    continue;
                }
                let n = change.offset;
                let stored = pixel.after.store(
                    doc.color.depth,
                    doc.color.dither,
                    [
                        extent.offset[0] + (n % width) as i64,
                        extent.offset[1] + (n / width) as i64,
                    ],
                );
                changed |= if doc.color.depth == Depth::Sixteen {
                    doc.layers[self.layer].sample_at(n).rgba16() != stored
                } else {
                    target.pixels[n] != stored.map(|v| v as u8)
                };
                if let Some(precise) = &mut precise {
                    precise[n] = stored;
                    target.pixels[n] = Sample::from16(stored).rgba8();
                } else {
                    target.pixels[n] = stored.map(|v| v as u8);
                }
            }
        }
        if !changed {
            return Ok(false);
        }
        target.precise = precise;
        if let Some(metadata) = metadata {
            if metadata.layer != self.layer {
                return Err("Frozen stroke metadata layer mismatch".into());
            }
            state.scene.append_mouse(metadata)?;
        }
        doc.commit_state(state)
    }
}
fn with_alpha(value: Sample, alpha: f64, fallback: Sample) -> Result<Sample, String> {
    let rgb = if value.alpha() == 0. {
        fallback.rgb()
    } else {
        value.rgb()
    };
    Sample::new([rgb[0], rgb[1], rgb[2], alpha])
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        color_storage::Config,
        document::{Background, CanvasSpec},
        pressure::Mapping,
    };
    fn page() -> Document {
        Document::create_with_color(
            CanvasSpec {
                width: 1,
                height: 1,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
            Config::default(),
        )
        .unwrap()
    }
    fn style() -> Style {
        Style {
            clone_stamp: Default::default(),
            variation: Default::default(),
            pattern: Default::default(),
            dynamics: Default::default(),
            sequence: Default::default(),
            filters: Default::default(),
            profile: Default::default(),
            texture: Default::default(),
            material: Default::default(),
            opacity: 1.,
            hardness: 1.,
            spacing: 0.15,
            shape: Shape {
                size: 2.,
                aspect: 1.,
                angle: 0.,
            },
            tip: BrushTip::Marker,
            color: [31, 67, 109, 255],
            smoothing: 0.,
            pressure: Mapping {
                size: false,
                opacity: true,
                ..Default::default()
            },
        }
    }
    #[test]
    fn floating_clone_running_color_diffusion_and_water_edges_keep_low_alpha() {
        let mut d = page();
        let source = [12345, 23456, 34567, 123];
        d.layers[0].precise = Some(vec![source]);
        d.layers[0].pixels = vec![Sample::from16(source).rgba8()];
        let mut s = style();
        s.pressure.opacity = false;
        s.tip = BrushTip::PixelPen;
        s.shape.size = 1.;
        s.profile.pixel.overwrite = false;
        s.clone_stamp = crate::clone_stamp::Stamp {
            enabled: true,
            offset: [0, 0],
        };
        let mut stroke = Stroke::begin(&d, 0, s).unwrap();
        stroke.stamp_single(&d, [0.5; 2], 1., 1.).unwrap();
        assert_eq!(
            crate::float_render::Renderer::preview(&d, Some(&stroke))
                .unwrap()
                .pixel([0.5; 2])
                .unwrap()
                .rgba16(),
            source
        );
        assert!(!stroke.commit(&mut d).unwrap());
        s.clone_stamp = Default::default();
        s.tip = BrushTip::Marker;
        s.material.mode = crate::brush_material::Mode::RunningColor;
        s.material.colorless = true;
        s.material.mixing = 1.;
        s.material.quality = crate::color_mix::Quality::High;
        let mut stroke = Stroke::begin(&d, 0, s).unwrap();
        stroke
            .material(&d, crate::mouse_pressure::Effect::Normal, 1., [0.5; 2])
            .unwrap();
        stroke.stamp_single(&d, [0.5; 2], 1., 1.).unwrap();
        assert_eq!(
            crate::float_render::Renderer::preview(&d, Some(&stroke))
                .unwrap()
                .pixel([0.5; 2])
                .unwrap()
                .rgba16(),
            source
        );
        drop(stroke);
        s.material = Default::default();
        let mut stroke = Stroke::begin(&d, 0, s).unwrap();
        stroke.stamp_single(&d, [0.5; 2], 1., 1.).unwrap();
        let current = stroke.float_pixel(&d, 0, 0, 0).unwrap();
        stroke.diffuse(&d, 0.5, || false).unwrap();
        let rgb = current.rgb();
        let expected = Sample::new([rgb[0], rgb[1], rgb[2], current.alpha() * (0.5 + 0.5 / 9.)])
            .unwrap()
            .rgba16();
        assert_eq!(stroke.float_pixel(&d, 0, 0, 0).unwrap().rgba16(), expected);
        stroke.commit(&mut d).unwrap();
        assert_eq!(d.layers[0].precise.as_ref().unwrap()[0], expected);
        assert!(d.undo());
        s.material = Default::default();
        s.material.edge = 0.8;
        s.material.mode = crate::brush_material::Mode::Water;
        s.material.water = 0.;
        s.color = [31, 67, 109, 255];
        let mut stroke = Stroke::begin(&d, 0, s).unwrap();
        stroke.stamp_single(&d, [0.5; 2], 0.0001, 1.).unwrap();
        let before = stroke.float_pixel(&d, 0, 0, 0).unwrap();
        stroke.finish_material(&d, || false).unwrap();
        let after = stroke.float_pixel(&d, 0, 0, 0).unwrap();
        assert!(after.rgb()[0] < before.rgb()[0]);
        assert_eq!(after.alpha(), before.alpha());
        assert_eq!(d.layers[0].precise.as_ref().unwrap()[0], source);
    }
    #[test]
    fn actual_float_stroke_1000_low_pressure_dabs_low_bits_history_and_native() {
        let mut d = page();
        let original = d.snapshot();
        let mut stroke = Stroke::begin(&d, 0, style()).unwrap();
        for _ in 0..1000 {
            stroke.stamp_single(&d, [0.5; 2], 0.00001, 1.).unwrap();
        }
        let alpha = 1. - (1. - 0.22 * 0.00001f64).powi(1000);
        let expected = Sample::new([31. / 255., 67. / 255., 109. / 255., alpha])
            .unwrap()
            .rgba16();
        assert_eq!(stroke.float_pixel(&d, 0, 0, 0).unwrap().rgba16(), expected);
        assert_eq!(
            crate::float_render::Renderer::preview(&d, Some(&stroke))
                .unwrap()
                .pixel([0.5; 2])
                .unwrap()
                .rgba16(),
            expected
        );
        assert_eq!(d.snapshot(), original);
        assert!(stroke.commit(&mut d).unwrap());
        assert_eq!(d.layers[0].precise.as_ref().unwrap()[0], expected);
        assert_eq!(d.history_counts(), (1, 0));
        let after = d.snapshot();
        assert!(d.undo());
        assert_eq!(d.snapshot(), original);
        assert!(d.redo());
        assert_eq!(d.snapshot(), after);
        let root = crate::test_support::TempDir::new();
        let file = root.0.join("float-stroke.drawer");
        crate::files::save(&mut d, &file).unwrap();
        assert_eq!(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .unwrap()
                .snapshot(),
            after
        );
        let mut eight = Document::create_with_color(
            CanvasSpec {
                width: 1,
                height: 1,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
            Config {
                depth: Depth::Eight,
                ..Default::default()
            },
        )
        .unwrap();
        let mut stroke = Stroke::begin(&eight, 0, style()).unwrap();
        for _ in 0..1000 {
            stroke.stamp_single(&eight, [0.5; 2], 0.00001, 1.).unwrap();
        }
        assert_eq!(eight.layers[0].pixels[0], [0; 4]);
        assert!(stroke.commit(&mut eight).unwrap());
        assert_eq!(eight.layers[0].pixels[0], [31, 67, 109, 1]);
        assert_eq!(eight.history_counts(), (1, 0));
        let path = root.0.join("float-eight.drawer");
        crate::files::save(&mut eight, &path).unwrap();
        assert_eq!(
            crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                .unwrap()
                .snapshot(),
            eight.snapshot()
        );
        let mut tiny = page();
        let mut stroke = Stroke::begin(&tiny, 0, style()).unwrap();
        for _ in 0..10 {
            stroke.stamp_single(&tiny, [0.5; 2], 0.00001, 1.).unwrap();
        }
        assert!(stroke.commit(&mut tiny).unwrap());
        assert_eq!(tiny.layers[0].pixels[0], [0; 4]);
        assert!(tiny.layers[0].precise.as_ref().unwrap()[0][3] > 0);
        let mut zero = page();
        let original = zero.snapshot();
        let mut stroke = Stroke::begin(&zero, 0, style()).unwrap();
        stroke
            .stamp_single(&zero, [0.5; 2], 0.00000001, 1.)
            .unwrap();
        assert!(!stroke.commit(&mut zero).unwrap());
        assert_eq!(zero.snapshot(), original);
        assert_eq!(zero.history_counts(), (0, 0));
    }
    #[test]
    fn low_bit_alpha_protection_and_pixel_locks_cannot_be_bypassed_by_eight_bit_preview() {
        let mut d = page();
        let source = [30000, 40000, 50000, 1];
        d.layers[0].precise = Some(vec![source]);
        d.layers[0].pixels = vec![Sample::from16(source).rgba8()];
        d.scene
            .clipping
            .alpha
            .insert(crate::layer_properties::Target::Raster(1));
        let mut stroke = Stroke::begin(&d, 0, style()).unwrap();
        stroke.stamp_single(&d, [0.5; 2], 1., 1.).unwrap();
        assert!(stroke.commit(&mut d).unwrap());
        assert_eq!(d.layers[0].precise.as_ref().unwrap()[0][3], 1);
        let mut illegal = d.snapshot();
        illegal.layers[0].precise.as_mut().unwrap()[0][3] = 2;
        assert!(d.commit_state(illegal).is_err());
        d.scene
            .layer_properties
            .rasters
            .entry(1)
            .or_default()
            .pixels_locked = true;
        let original = d.snapshot();
        let mut illegal = d.snapshot();
        illegal.layers[0].precise.as_mut().unwrap()[0][0] += 1;
        assert!(d.commit_state(illegal).is_err());
        assert_eq!(d.snapshot(), original);
        assert!(Stroke::begin(&d, 0, style()).is_err());
    }
}
