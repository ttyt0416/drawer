//! A pending raster stroke is previewed separately, then committed as one pixel history entry.
#[path = "painting_float.rs"]
mod floating;
use crate::{
    assets::BrushTip,
    brush_cursor::{Shape, Trace},
    document::{DEFAULT_DOCUMENT_BUDGET, Document, RasterLayer},
    history::PixelChange,
    scene::Point,
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_STROKE: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    #[serde(default, skip_serializing_if = "crate::clone_stamp::Stamp::is_default")]
    pub clone_stamp: crate::clone_stamp::Stamp,
    #[serde(
        default,
        skip_serializing_if = "crate::brush_variation::Settings::is_default"
    )]
    pub variation: crate::brush_variation::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::brush_pattern::Settings::is_default"
    )]
    pub pattern: crate::brush_pattern::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::brush_dynamics::Settings::is_default"
    )]
    pub dynamics: crate::brush_dynamics::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::tip_sequence::Settings::is_default"
    )]
    pub sequence: crate::tip_sequence::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::stroke_filter::Settings::is_default"
    )]
    pub filters: crate::stroke_filter::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::tip_profile::Settings::is_default"
    )]
    pub profile: crate::tip_profile::Settings,
    #[serde(default, skip_serializing_if = "crate::texture::Pattern::is_default")]
    pub texture: crate::texture::Pattern,
    #[serde(
        default,
        skip_serializing_if = "crate::brush_material::Settings::is_default"
    )]
    pub material: crate::brush_material::Settings,
    #[serde(default = "full_opacity")]
    pub opacity: f64,
    #[serde(default = "full_opacity")]
    pub hardness: f64,
    #[serde(default = "default_spacing")]
    pub spacing: f64,
    pub shape: Shape,
    pub tip: BrushTip,
    pub color: [u8; 4],
    pub smoothing: f64,
    pub pressure: crate::pressure::Mapping,
}
fn full_opacity() -> f64 {
    1.0
}
fn default_spacing() -> f64 {
    0.15
}
impl Style {
    pub fn erases(&self) -> bool {
        self.tip == BrushTip::Eraser || self.profile.transparent
    }
    pub fn tonal(&self) -> bool {
        self.material.tonal.mode != crate::tonal::Mode::Off
            && !self.clone_stamp.enabled
            && !self.pixel_overwrite()
            && !self.erases()
    }
    pub fn advanced_texture(&self) -> bool {
        self.sequence.slots[..if self.sequence.enabled {
            self.sequence.count as usize
        } else {
            1
        }]
            .iter()
            .any(|s| s.texture.is_some())
            || (self.variation.dual.active() && self.variation.dual.controls.texture.is_some())
    }
    pub fn water_edges(&self) -> bool {
        !self.tonal()
            && !self.erases()
            && self.material.mode == crate::brush_material::Mode::Water
            && self.material.edge > 0.0
            && !self.clone_stamp.enabled
            && !self.pixel_overwrite()
            && self.material.composition.mode == crate::brush_composite::Mode::Normal
    }
    pub fn dual_rgb(&self) -> bool {
        !self.tonal()
            && self.variation.dual.active()
            && self.variation.dual.controls.rgb
            && !self.clone_stamp.enabled
            && !self.erases()
            && !self.pixel_overwrite()
    }
    pub fn pixel_overwrite(&self) -> bool {
        !self.profile.transparent
            && self.tip == BrushTip::PixelPen
            && self.profile.pixel.overwrite
            && !self.clone_stamp.enabled
            && self.material.composition.mode == crate::brush_composite::Mode::Normal
    }
    pub fn resolved(mut self) -> Self {
        if self.tip == BrushTip::PixelPen && !self.profile.pixel.pressure {
            self.pressure.size = false;
            self.pressure.opacity = false;
            self.dynamics.pressure.enabled = false;
            self.material.composition.mixing_pressure.enabled = false;
        }
        if self.tip == BrushTip::Eraser && self.material.valid() {
            self.material = crate::brush_material::Settings {
                alpha_lock: self.material.alpha_lock,
                composition: crate::brush_composite::Settings {
                    render_quality: self.material.composition.render_quality,
                    ..Default::default()
                },
                ..Default::default()
            };
        }
        if self.tip == BrushTip::Smudge && self.material.valid() {
            self.material.mode = crate::brush_material::Mode::Smudge;
            self.material.supply = 0.0;
            self.material.colorless = true;
        }
        if self.clone_stamp.enabled && self.material.valid() {
            self.material = crate::brush_material::Settings {
                alpha_lock: self.material.alpha_lock,
                composition: crate::brush_composite::Settings {
                    render_quality: self.material.composition.render_quality,
                    ..Default::default()
                },
                ..Default::default()
            };
        }
        if self.profile.transparent && self.material.valid() {
            self.clone_stamp = Default::default();
            self.material = crate::brush_material::Settings {
                composition: crate::brush_composite::Settings {
                    render_quality: self.material.composition.render_quality,
                    ..Default::default()
                },
                ..Default::default()
            };
        }
        self
    }
    /// Conservative minor-axis spacing also covers a thin tip across its narrow direction.
    pub(crate) fn step(self, pressure_scale: f64) -> f64 {
        (self.pattern.behavior.minimum(self.shape.size)
            * if self.pattern.mode == crate::brush_pattern::Mode::Decoration {
                self.pattern.interval
            } else {
                self.shape.aspect * self.spacing
            }
            * pressure_scale)
            .max(1.0 / 1024.0)
    }
    pub fn valid(self) -> bool {
        self.clone_stamp.valid()
            && self.variation.valid()
            && self.pattern.valid()
            && self.dynamics.valid()
            && self.sequence.valid()
            && self.filters.valid()
            && self.profile.valid()
            && self.shape.valid()
            && self.material.valid()
            && self.texture.valid()
            && (0.0..=1.0).contains(&self.opacity)
            && (0.0..=1.0).contains(&self.hardness)
            && (0.01..=10.0).contains(&self.spacing)
            && self.shape.angle.abs() <= 1e12
            && (0.0..=0.95).contains(&self.smoothing)
            && self.pressure.error().is_none()
    }
}
struct Arm {
    transform: crate::scene::Affine,
    stroke: Box<Stroke>,
}
pub struct Stroke {
    floating: Option<Box<floating::Floating>>,
    ruler: crate::rulers::Runtime,
    arms: Vec<Arm>,
    mirror_tip: bool,
    free_limit: usize,
    pub document: u64,
    pub layer: usize,
    revision: u64,
    pub preview_revision: u64,
    nonce: u64,
    seed: u32,
    effect: crate::mouse_pressure::Effect,
    strength: f64,
    carried: Option<[u8; 4]>,
    material_last: Option<Point>,
    direct: bool,
    diffused: bool,
    edges_done: bool,
    dabs: u32,
    style: Style,
    trace: Trace,
    last: Option<Point>,
    last_stamp: Option<Point>,
    pressure: f64,
    pressure_filter: crate::stroke_filter::Pressure,
    dynamics: crate::brush_dynamics::Input,
    last_dynamics: crate::brush_dynamics::Input,
    heading: Option<f64>,
    particle: Option<Point>,
    dynamic_index: u64,
    pattern_index: u64,
    ribbon: Option<(Point, f64)>,
    dab_color: [u8; 4],
    sequence: crate::tip_sequence::Sequence,
    mask_tip: BrushTip,
    mask_square: bool,
    materials: std::sync::Arc<crate::material_library::Library>,
    last_stamp_pressure: f64,
    scale: f64,
    last_scale: f64,
    last_stamp_scale: f64,
    angle_offset: f64,
    last_stamp_angle: f64,
    remainder: f64,
    changes: Vec<PixelChange>,
    unmasked: Vec<[u8; 4]>,
    amounts: Vec<f64>,
    lookup: HashMap<usize, usize>,
    limit: usize,
    pixel_work: u64,
    region: Option<crate::line_regions::Region>,
    region_prepared: bool,
}
impl Stroke {
    fn prepare_region(&mut self, doc: &Document, point: Point) -> Result<(), String> {
        if !self.region_prepared {
            if doc.scene.lines.settings.clip_paint && !self.style.erases() {
                let region = crate::line_regions::Region::build(doc, point)?;
                let pixel_bytes = if self.floating.is_some() { 256 } else { 96 };
                let remaining = self
                    .limit
                    .saturating_mul(pixel_bytes)
                    .saturating_sub(region.bytes());
                if remaining < pixel_bytes {
                    return Err("Reference mask exceeds stroke memory budget".into());
                }
                self.limit = remaining / pixel_bytes;
                self.region = Some(region);
            }
            self.region_prepared = true;
        }
        Ok(())
    }
    fn coverage(&self, doc: &Document, p: Point) -> u8 {
        if self.region.as_ref().is_some_and(|r| !r.contains(p)) {
            0
        } else {
            doc.scene.selection_coverage(p)
        }
    }
    pub fn set_geometry_scale(&mut self, scale: f64) -> Result<(), String> {
        if !(0.0..=1.0).contains(&scale) {
            return Err("Invalid stroke geometry scale".into());
        }
        for arm in &mut self.arms {
            arm.stroke.set_geometry_scale(scale)?;
        }
        self.scale = scale;
        Ok(())
    }
    pub fn set_dynamic_input(&mut self, input: crate::brush_dynamics::Input) -> Result<(), String> {
        if !input.valid() {
            return Err("Invalid brush dynamic input".into());
        }
        for arm in &mut self.arms {
            let mut mapped = input;
            mapped.bearing = input
                .bearing
                .map(|a| crate::rulers::angle(arm.transform, a));
            arm.stroke.set_dynamic_input(mapped)?;
        }
        self.dynamics = input;
        Ok(())
    }
    fn rotation(&self) -> f64 {
        self.style.dynamics.angle(
            self.heading,
            self.dynamics,
            self.particle,
            self.angle_offset,
        )
    }
    fn dynamic_size(&self, pressure: f64, index: u64) -> Result<f64, String> {
        let base = self.style.shape.size
            * if self.style.pressure.size {
                self.style.profile.size(self.style.pressure.map(pressure)?)
            } else {
                1.0
            };
        Ok(self
            .style
            .pattern
            .behavior
            .minimum(self.style.dynamics.size(
                base,
                self.style.pressure.device_response.map(pressure)?,
                self.dynamics,
                index,
            )))
    }
    fn heading_to(&mut self, point: Point) {
        if let Some(from) = self.last {
            let delta = [point[0] - from[0], point[1] - from[1]];
            if delta[0].hypot(delta[1]) > 1e-9 {
                self.heading = Some(delta[1].atan2(delta[0]).to_degrees());
            }
        }
    }
    pub fn with_materials(
        mut self,
        materials: std::sync::Arc<crate::material_library::Library>,
    ) -> Result<Self, String> {
        if self.last.is_some() || !self.changes.is_empty() || !materials.valid() {
            return Err("Material snapshot must be valid and set before the stroke".into());
        }
        for arm in &mut self.arms {
            arm.stroke.materials = materials.clone();
        }
        self.materials = materials;
        Ok(self)
    }
    pub fn dab_count(&self) -> u32 {
        self.dabs
    }
    fn candidate(&self, doc: &Document, x: i64, y: i64) -> [u8; 4] {
        self.candidate_value(doc, x, y, false)
    }
    fn candidate_value(&self, doc: &Document, x: i64, y: i64, raw: bool) -> [u8; 4] {
        if x < 0 || y < 0 || x >= doc.width as i64 || y >= doc.height as i64 {
            return [0; 4];
        }
        let layer = &doc.layers[self.layer];
        layer
            .pixel_index(doc.width, doc.height, x, y)
            .map_or([0; 4], |offset| {
                self.lookup.get(&offset).map_or(layer.pixels[offset], |n| {
                    if raw {
                        self.unmasked[*n]
                    } else {
                        self.changes[*n].after
                    }
                })
            })
    }
    pub fn material(
        &mut self,
        doc: &Document,
        effect: crate::mouse_pressure::Effect,
        strength: f64,
        point: Point,
    ) -> Result<(), String> {
        self.check(doc)?;
        if !(0.0..=1.0).contains(&strength) || point.iter().any(|v| !v.is_finite()) {
            return Err("Invalid stroke material".into());
        }
        let point = self.ruler.constrain(point);
        self.ruler.check_snapping()?;
        for arm in &mut self.arms {
            arm.stroke
                .material(doc, effect, strength, arm.transform.map(point))?;
        }
        self.effect = if self.style.clone_stamp.enabled || self.style.profile.transparent {
            crate::mouse_pressure::Effect::Normal
        } else {
            effect
        };
        self.strength = strength;
        self.carried = Some(self.candidate(doc, point[0].floor() as i64, point[1].floor() as i64));
        if self.floating.is_some() {
            self.initialize_float_material(doc, point);
        }
        Ok(())
    }
    fn pigment_color(&mut self, picked: [u8; 4], center: Point, pressure: f64) -> [u8; 4] {
        use crate::brush_material::Mode;
        use crate::mouse_pressure::Effect;
        let mut m = self.style.material;
        if m.mode == Mode::Water && pressure < m.supply_threshold {
            m.supply = 0.0;
        }
        m.mixing *= m.composition.mixing_pressure.map(Some(pressure));
        let blend = |a, b, w| {
            let result = if m.space == crate::color_mix::Space::Standard {
                mix(a, b, w)
            } else {
                crate::color_mix::mix(m.space, a, b, w)
            };
            crate::lab_mix::correct(result, a, b, w, m.brightness)
        };
        let mut color = if !m.pigment() {
            match self.effect {
                Effect::Blend => blend(self.dab_color, picked, self.strength),
                Effect::Smudge => {
                    let carried = self.carried.unwrap_or(picked);
                    self.carried = Some(blend(carried, picked, self.strength));
                    carried
                }
                _ => self.dab_color,
            }
        } else if m.mode == Mode::RunningColor {
            let mut foreground = if m.colorless { picked } else { self.dab_color };
            foreground[3] =
                (foreground[3] as f64 * if m.colorless { 1.0 } else { m.supply }).round() as u8;
            let mut local = blend(foreground, picked, m.mixing);
            if m.keep_density {
                local[3] = local[3].max(picked[3]);
            }
            local
        } else if m.mode == Mode::Paint && !m.colorless {
            let mut color = self.dab_color;
            color[3] = (color[3] as f64 * m.supply).round() as u8;
            color
        } else {
            let previous = self.carried.unwrap_or(picked);
            let distance = self
                .material_last
                .map_or(0.0, |last| (center[0] - last[0]).hypot(center[1] - last[1]));
            let mut sampled = picked;
            if m.keep_density && sampled[3] < previous[3] {
                if sampled[3] == 0 {
                    sampled[..3].copy_from_slice(&previous[..3]);
                }
                sampled[3] = previous[3];
            }
            let pickup = if m.smudge_distance == 0.0 {
                m.mixing * (1.0 - m.retention)
            } else {
                m.mixing * (1.0 - m.retention.powf(distance / m.smudge_distance))
            };
            let mut pigment = blend(previous, sampled, pickup);
            let supply = if m.colorless || m.mode == Mode::Smudge {
                0.0
            } else {
                m.supply
            };
            let replenish = if self.material_last.is_none() {
                supply
            } else {
                1.0 - (1.0 - supply).powf(distance / self.style.shape.size.max(0.1))
            };
            pigment = blend(pigment, self.dab_color, replenish);
            if m.keep_density {
                if pigment[3] == 0 && previous[3] > 0 {
                    pigment[..3].copy_from_slice(&previous[..3]);
                }
                pigment[3] = pigment[3].max(previous[3]);
            }
            self.carried = Some(pigment);
            pigment
        };
        self.material_last = Some(center);
        color[3] = (color[3] as f64 * (1.0 - m.dilution)).round() as u8;
        color
    }
    pub fn finish_material(
        &mut self,
        doc: &Document,
        cancel: impl Fn() -> bool,
    ) -> Result<(), String> {
        self.finish_material_inner(doc, &cancel)
    }
    fn finish_material_inner(
        &mut self,
        doc: &Document,
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        for arm in &mut self.arms {
            arm.stroke.finish_material_inner(doc, cancel)?;
        }
        use crate::brush_material::Mode;
        if self.style.profile.transparent
            || self.style.tonal()
            || self.style.clone_stamp.enabled
            || self.style.pixel_overwrite()
            || self.style.material.composition.mode != crate::brush_composite::Mode::Normal
        {
            return self.check(doc);
        }
        let strength = if self.style.material.mode == Mode::Water {
            self.style.material.water
        } else if self.style.material.mode == Mode::Paint
            && self.effect == crate::mouse_pressure::Effect::Wet
        {
            self.strength
        } else {
            0.0
        };
        if strength > 0.0 {
            self.diffuse(doc, strength, cancel)?;
        }
        self.finish_water_edges(doc, cancel)
    }
    fn finish_water_edges(
        &mut self,
        doc: &Document,
        cancel: impl Fn() -> bool,
    ) -> Result<(), String> {
        self.check(doc)?;
        if self.edges_done || !self.style.water_edges() {
            return Ok(());
        }
        if cancel() {
            return Err("Water edge processing cancelled".into());
        }
        self.finish_sample(doc)?;
        self.pixel_work = self
            .pixel_work
            .saturating_add(self.changes.len() as u64 * 4);
        if self.pixel_work > 64 * 1024 * 1024 {
            return Err("Water edge pixel-work budget exceeded".into());
        }
        if self.floating.is_some() {
            return self.finish_float_edges(doc, cancel);
        }
        let layer = &doc.layers[self.layer];
        let extent = layer.extent(doc.width, doc.height);
        let mut rim = Vec::new();
        rim.try_reserve_exact(self.changes.len())
            .map_err(|e| e.to_string())?;
        for (n, change) in self.changes.iter().enumerate() {
            if n % 256 == 0 && cancel() {
                return Err("Water edge processing cancelled".into());
            }
            let p = [
                extent.offset[0] + (change.offset % extent.width as usize) as i64,
                extent.offset[1] + (change.offset / extent.width as usize) as i64,
            ];
            rim.push(
                change.after != change.before
                    && change.after[3] > 0
                    && [[-1, 0], [1, 0], [0, -1], [0, 1]].iter().any(|d| {
                        let q = [p[0] + d[0], p[1] + d[1]];
                        layer
                            .pixel_index(doc.width, doc.height, q[0], q[1])
                            .and_then(|offset| self.lookup.get(&offset))
                            .is_none_or(|i| {
                                self.changes[*i].after == self.changes[*i].before
                                    || self.changes[*i].after[3] == 0
                            })
                    }),
            );
        }
        for (n, boundary) in rim.into_iter().enumerate() {
            if n % 256 == 0 && cancel() {
                return Err("Water edge processing cancelled".into());
            }
            if !boundary {
                continue;
            }
            let change = &self.changes[n];
            let p = [
                extent.offset[0] as f64 + (change.offset % extent.width as usize) as f64 + 0.5,
                extent.offset[1] as f64 + (change.offset / extent.width as usize) as f64 + 0.5,
            ];
            let factor =
                self.style.material.edge * self.style.opacity * self.coverage(doc, p) as f64
                    / 255.0;
            let change = &mut self.changes[n];
            for c in &mut change.after[..3] {
                *c = (*c as f64 * (1.0 - 0.5 * factor)).round() as u8;
            }
            self.unmasked[n] = change.after;
        }
        self.edges_done = true;
        self.direct = true;
        self.preview_revision += 1;
        Ok(())
    }
    pub fn direct_dab(
        &mut self,
        doc: &Document,
        point: Point,
        anchor: Point,
        pressure: f64,
    ) -> Result<(), String> {
        self.check(doc)?;
        if !(0.0..=1.0).contains(&pressure)
            || point
                .iter()
                .chain(anchor.iter())
                .any(|v| !v.is_finite() || v.abs() > 1e12)
        {
            return Err("Invalid timed dab".into());
        }
        let (point, anchor) = self.ruler.constrain_dab(point, anchor);
        self.ruler.check_snapping()?;
        for arm in &mut self.arms {
            arm.stroke.direct_dab(
                doc,
                arm.transform.map(point),
                arm.transform.map(anchor),
                pressure,
            )?;
        }
        self.prepare_region(doc, anchor)?;
        self.heading_to(anchor);
        self.particle = Some([anchor[0] - point[0], anchor[1] - point[1]]);
        self.stamp(doc, point, pressure, self.scale)?;
        self.last = Some(anchor);
        self.pressure = pressure;
        self.direct = true;
        self.dabs += 1;
        Ok(())
    }
    pub fn diffuse(
        &mut self,
        doc: &Document,
        strength: f64,
        cancel: impl Fn() -> bool,
    ) -> Result<(), String> {
        if self.diffused {
            return Ok(());
        }
        self.check(doc)?;
        self.finish_sample(doc)?;
        if !(0.0..=1.0).contains(&strength) {
            return Err("Invalid final diffusion".into());
        }
        if self.changes.is_empty() || strength == 0.0 {
            self.diffused = true;
            self.direct = true;
            return Ok(());
        }
        let advanced = self.style.advanced_texture();
        let layer = &doc.layers[self.layer];
        let extent = layer.extent(doc.width, doc.height);
        let mut positions = std::collections::HashSet::new();
        for change in &self.changes {
            let x = change.offset % extent.width as usize;
            let y = change.offset / extent.width as usize;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let px = extent.offset[0] + x as i64 + dx;
                    let py = extent.offset[1] + y as i64 + dy;
                    if px < 0 || py < 0 || px >= doc.width as i64 || py >= doc.height as i64 {
                        continue;
                    }
                    if let Some(offset) = layer.pixel_index(doc.width, doc.height, px, py) {
                        if !positions.contains(&offset) && positions.len() >= self.limit {
                            return Err("Final diffusion exceeds stroke memory budget".into());
                        }
                        positions.try_reserve(1).map_err(|e| e.to_string())?;
                        positions.insert(offset);
                    }
                }
            }
        }
        let mut positions: Vec<_> = positions.into_iter().collect();
        positions.sort_unstable();
        self.pixel_work = self.pixel_work.saturating_add(
            positions.len() as u64
                * (9 + u64::from(
                    self.materials
                        .get(
                            self.style.texture.source,
                            crate::material_library::Kind::Texture,
                        )
                        .is_some(),
                )),
        );
        if self.pixel_work > 64 * 1024 * 1024 {
            return Err(
                "Final diffusion pixel-work budget exceeded; pending stroke cancelled".into(),
            );
        }
        if self.floating.is_some() {
            return self.diffuse_float(doc, strength, positions, cancel);
        }
        let mut changes = Vec::new();
        changes
            .try_reserve_exact(positions.len())
            .map_err(|e| e.to_string())?;
        for (n, offset) in positions.into_iter().enumerate() {
            if n % 256 == 0 && cancel() {
                return Err("Mouse final diffusion cancelled".into());
            }
            let x = extent.offset[0] + (offset % extent.width as usize) as i64;
            let y = extent.offset[1] + (offset / extent.width as usize) as i64;
            let mut rgb = [0.0; 3];
            let mut alpha = 0.0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let p = self.candidate_value(doc, x + dx, y + dy, true);
                    let a = p[3] as f64 / 255.0;
                    alpha += a;
                    for channel in 0..3 {
                        rgb[channel] += p[channel] as f64 * a;
                    }
                }
            }
            let average = if alpha > 0.0 {
                [
                    (rgb[0] / alpha).round() as u8,
                    (rgb[1] / alpha).round() as u8,
                    (rgb[2] / alpha).round() as u8,
                    (alpha / 9.0 * 255.0).round() as u8,
                ]
            } else {
                [0; 4]
            };
            let original = layer.pixels[offset];
            let current = self.candidate_value(doc, x, y, true);
            if self.style.material.alpha_lock && original[3] == 0 {
                continue;
            }
            let mut desired = mix(current, average, strength);
            if self.style.material.keep_density {
                desired[3] = desired[3].max(current[3]);
            }
            let mut after = mix(
                original,
                desired,
                self.style.opacity
                    * if advanced {
                        1.0
                    } else {
                        self.materials
                            .factor(self.style.texture, [x as f64 + 0.5, y as f64 + 0.5])
                    }
                    * self.coverage(doc, [x as f64 + 0.5, y as f64 + 0.5]) as f64
                    / 255.0,
            );
            if self.style.material.alpha_lock {
                after[3] = original[3];
            }
            if original != after {
                changes.push(PixelChange {
                    offset,
                    before: original,
                    after,
                });
            }
        }
        self.lookup.clear();
        self.lookup
            .try_reserve(changes.len())
            .map_err(|e| e.to_string())?;
        self.unmasked.clear();
        self.amounts.clear();
        self.unmasked
            .try_reserve(changes.len())
            .map_err(|e| e.to_string())?;
        self.amounts
            .try_reserve(changes.len())
            .map_err(|e| e.to_string())?;
        for (n, change) in changes.iter().enumerate() {
            self.lookup.insert(change.offset, n);
            self.unmasked.push(change.after);
            self.amounts.push(0.0);
        }
        self.changes = changes;
        self.diffused = true;
        self.direct = true;
        self.preview_revision += 1;
        Ok(())
    }
    pub fn set_seed(&mut self, seed: u32) {
        for arm in &mut self.arms {
            arm.stroke.set_seed(seed);
        }
        self.seed = seed;
    }
    pub fn anchor(&self) -> Option<(usize, BrushTip, Point)> {
        self.last.map(|point| (self.layer, self.style.tip, point))
    }
    pub fn begin(doc: &Document, layer: usize, style: Style) -> Result<Self, String> {
        let style = style.resolved();
        Self::with_budget(doc, layer, style, DEFAULT_DOCUMENT_BUDGET / 4)
    }
    pub fn with_budget(
        doc: &Document,
        layer: usize,
        style: Style,
        budget: usize,
    ) -> Result<Self, String> {
        Self::with_ruler_budget(doc, layer, style, budget, true)
    }
    fn with_ruler_budget(
        doc: &Document,
        layer: usize,
        style: Style,
        budget: usize,
        assisted: bool,
    ) -> Result<Self, String> {
        let floating = (doc.color != crate::color_storage::Config::legacy())
            .then(|| floating::Floating::new(doc, style.resolved()))
            .transpose()?
            .map(Box::new);
        let pixel_bytes = if floating.is_some() { 256 } else { 96 };
        let ruler = if assisted {
            crate::rulers::Runtime::for_target(doc, crate::line_tools::Target::Raster(layer))?
        } else {
            Default::default()
        };
        let copies = ruler.copies.clone();
        let total_budget = budget
            .min(DEFAULT_DOCUMENT_BUDGET.saturating_sub(doc.resident_bytes()))
            .saturating_sub(ruler.memory_bytes());
        let free_limit =
            total_budget.saturating_sub(std::mem::size_of::<Self>() + 256) / pixel_bytes;
        // Reserve a quarter for the single-history merge of independent arms.
        let budget = if copies.is_empty() {
            total_budget
        } else {
            total_budget / 4 * 3
        } / (copies.len() + 1);
        let mut style = style.resolved();
        if crate::clipping::alpha_locked(doc, layer) {
            style.material.alpha_lock = true;
        }
        crate::layer_properties::paintable(doc, layer)?;
        if doc.layers.get(layer).is_some_and(|l| l.locked) {
            return Err("The raster layer is locked".into());
        }
        if !style.valid()
            || doc
                .layers
                .get(layer)
                .is_none_or(|l| l.locked || !l.valid(doc.width, doc.height))
        {
            return Err("Choose a valid raster layer and brush".into());
        }
        let limit = budget
            .min(DEFAULT_DOCUMENT_BUDGET.saturating_sub(doc.resident_bytes()))
            .saturating_sub(std::mem::size_of::<Self>() + 256)
            / pixel_bytes;
        if limit == 0 {
            return Err("No memory available for a stroke".into());
        }
        let mut stroke = Self {
            floating,
            ruler,
            arms: vec![],
            mirror_tip: false,
            free_limit,
            document: doc.id,
            layer,
            revision: doc.revision,
            preview_revision: 0,
            nonce: NEXT_STROKE.fetch_add(1, Ordering::Relaxed),
            seed: 0,
            effect: crate::mouse_pressure::Effect::Normal,
            strength: 0.5,
            carried: None,
            material_last: None,
            direct: false,
            diffused: false,
            edges_done: false,
            dabs: 0,
            style,
            trace: Default::default(),
            last: None,
            last_stamp: None,
            pressure: 1.0,
            pressure_filter: Default::default(),
            dynamics: Default::default(),
            last_dynamics: Default::default(),
            heading: None,
            particle: None,
            dynamic_index: 0,
            pattern_index: 0,
            ribbon: None,
            dab_color: style.color,
            sequence: Default::default(),
            mask_tip: style.tip,
            mask_square: style.profile.square(style.tip),
            materials: Default::default(),
            last_stamp_pressure: 1.0,
            scale: 1.0,
            last_scale: 1.0,
            last_stamp_scale: 1.0,
            angle_offset: 0.0,
            last_stamp_angle: 0.0,
            remainder: 0.0,
            changes: vec![],
            unmasked: vec![],
            amounts: vec![],
            lookup: HashMap::new(),
            limit,
            pixel_work: 0,
            region: None,
            region_prepared: false,
        };
        for transform in copies {
            let mut child_style = style;
            let sign = if crate::rulers::reflected(transform) {
                -1.0
            } else {
                1.0
            };
            if style.dynamics.direction == crate::brush_dynamics::Direction::Fixed {
                child_style.shape.angle = crate::rulers::angle(transform, style.shape.angle);
                child_style.dynamics.fallback_angle *= sign;
                child_style.dynamics.scatter_angle *= sign;
            } else {
                child_style.shape.angle *= sign;
                child_style.dynamics.fallback_angle =
                    crate::rulers::angle(transform, style.dynamics.fallback_angle);
                child_style.dynamics.scatter_angle =
                    crate::rulers::angle(transform, style.dynamics.scatter_angle);
            }
            let v = transform.map(style.clone_stamp.offset.map(|v| v as f64));
            let o = transform.map([0.0; 2]);
            child_style.clone_stamp.offset =
                [(v[0] - o[0]).round() as i64, (v[1] - o[1]).round() as i64];
            let mut child = Self::with_ruler_budget(doc, layer, child_style, budget, false)?;
            child.mirror_tip = crate::rulers::reflected(transform);
            stroke.arms.push(Arm {
                transform,
                stroke: Box::new(child),
            });
        }
        Ok(stroke)
    }
    pub fn configure_snapping(
        &mut self,
        doc: &Document,
        settings: crate::snapping::Settings,
        scale: f64,
    ) -> Result<(), String> {
        if self.last.is_some() || !self.changes.is_empty() {
            return Err("Configure snapping before stroke input".into());
        }
        if !settings.rulers {
            self.disable_rulers()?;
        }
        let bytes = self.ruler.configure_snapping(doc, settings, scale)?;
        self.limit = self
            .limit
            .checked_sub(bytes.div_ceil(96))
            .filter(|n| *n > 0)
            .ok_or("Guide snapping exceeds stroke memory")?;
        Ok(())
    }
    pub fn set_snap_disabled(&mut self, disabled: bool) {
        self.ruler.set_snap_disabled(disabled);
    }
    /// A session toggle is fixed before input; it never edits the page rulers.
    pub fn disable_rulers(&mut self) -> Result<(), String> {
        if self.last.is_some() || !self.changes.is_empty() {
            return Err("Change ruler snapping before starting a stroke".into());
        }
        self.limit = self.free_limit;
        self.arms.clear();
        self.ruler = Default::default();
        Ok(())
    }
    fn collect_changes(self) -> Result<Vec<PixelChange>, String> {
        if self.arms.is_empty() {
            return Ok(self.changes);
        }
        let count = self.changed_pixels();
        let mut result = Vec::new();
        result
            .try_reserve_exact(count)
            .map_err(|_| "Symmetry commit allocation failed")?;
        let mut seen = std::collections::HashSet::new();
        seen.try_reserve(count)
            .map_err(|_| "Symmetry index allocation failed")?;
        // Primary then increasing arm order: overlapping axis pixels are written once.
        for change in self
            .changes
            .into_iter()
            .chain(self.arms.into_iter().flat_map(|a| a.stroke.changes))
        {
            if change.before != change.after && seen.insert(change.offset) {
                result.push(change);
            }
        }
        Ok(result)
    }
    fn check(&self, doc: &Document) -> Result<(), String> {
        if doc.id != self.document
            || doc.revision != self.revision
            || doc
                .layers
                .get(self.layer)
                .is_none_or(|l| l.locked || !l.valid(doc.width, doc.height))
        {
            return Err("The drawing changed; pending stroke discarded".into());
        }
        Ok(())
    }
    pub fn sample(&mut self, doc: &Document, point: Point) -> Result<(), String> {
        self.sample_pressure(doc, point, 1.0)
    }
    pub fn sample_pressure(
        &mut self,
        doc: &Document,
        point: Point,
        pressure: f64,
    ) -> Result<(), String> {
        self.sample_pressure_inner(doc, point, pressure, false)
    }
    pub(crate) fn sample_processed_pressure(
        &mut self,
        doc: &Document,
        point: Point,
        pressure: f64,
    ) -> Result<(), String> {
        self.sample_pressure_inner(doc, point, pressure, true)
    }
    fn sample_pressure_inner(
        &mut self,
        doc: &Document,
        point: Point,
        pressure: f64,
        processed: bool,
    ) -> Result<(), String> {
        if self.diffused {
            return Err("A finalized wet stroke cannot accept more input".into());
        }
        self.check(doc)?;
        if !(0.0..=1.0).contains(&pressure) {
            return Err("Invalid normalized pressure".into());
        }
        if !point.iter().all(|n| n.is_finite() && n.abs() <= 1e12) {
            return Err("Invalid drawing coordinate".into());
        }
        let point = self.ruler.constrain(point);
        self.ruler.check_snapping()?;
        for arm in &mut self.arms {
            arm.stroke
                .sample_pressure_inner(doc, arm.transform.map(point), pressure, processed)?;
        }
        self.prepare_region(doc, point)?;
        let pressure = if processed {
            pressure
        } else {
            self.pressure_filter.receive(self.style.filters, pressure)?
        };
        let Some((_, to)) = self
            .trace
            .receive(point, self.style.filters.position(self.style.smoothing))
        else {
            if ((self.pressure != pressure
                && (self.style.pressure.size
                    || self.style.pressure.opacity
                    || self.style.material.composition.mixing_pressure.enabled
                    || self.style.dynamics.pressure.enabled))
                || self.last_stamp_angle != self.rotation()
                || self.last_stamp_scale != self.scale
                || (!self.style.dynamics.is_default() && self.last_dynamics != self.dynamics))
                && let Some(last) = self.last
            {
                self.stamp(doc, last, pressure, self.scale)?;
            }
            self.pressure = pressure;
            self.last_scale = self.scale;
            return Ok(());
        };
        self.heading_to(to);
        self.particle = None;
        if let Some(from) = self.last {
            let delta = [to[0] - from[0], to[1] - from[1]];
            let length = delta[0].hypot(delta[1]);
            let pressure_scale = if self.style.pressure.size {
                self.style
                    .profile
                    .size(self.style.pressure.map(self.pressure)?)
                    .min(self.style.profile.size(self.style.pressure.map(pressure)?))
            } else {
                1.0
            };
            let step = if self.style.pattern.mode == crate::brush_pattern::Mode::Ribbon {
                0.5
            } else if self.style.dynamics.is_default() {
                self.style
                    .step(pressure_scale * self.last_scale.min(self.scale))
            } else if self.style.dynamics.spacing_with_size {
                let size = self
                    .dynamic_size(self.pressure, self.dynamic_index)?
                    .min(self.dynamic_size(pressure, self.dynamic_index)?);
                (size
                    * if self.style.pattern.mode == crate::brush_pattern::Mode::Decoration {
                        self.style.pattern.interval
                    } else {
                        self.style.shape.aspect * self.style.spacing
                    }
                    * self.last_scale.min(self.scale))
                .max(1.0 / 1024.0)
            } else {
                self.style.step(1.0)
            };
            if length > 1e-9 {
                let radius = self.style.shape.size.max(self.style.dynamics.minimum_size)
                    * 0.5
                    * if self.style.sequence.enabled || self.style.profile.square(self.style.tip) {
                        1.0 + self.style.shape.aspect
                    } else {
                        1.0
                    }
                    + self.style.shape.size.max(self.style.dynamics.minimum_size)
                        * 0.5
                        * self.style.variation.dual.extra_radius(
                            self.style.shape.size.max(self.style.dynamics.minimum_size),
                        )
                    + 1.0
                    + if self.style.pattern.mode == crate::brush_pattern::Mode::Scatter {
                        self.style.pattern.width * 0.5
                    } else {
                        0.0
                    };
                if let Some([begin, end]) = clip(
                    from,
                    to,
                    [-radius, -radius],
                    [doc.width as f64 + radius, doc.height as f64 + radius],
                ) {
                    let first = step - self.remainder.rem_euclid(step);
                    let start = ((begin * length - first) / step).ceil().max(0.0) as usize;
                    let finish = ((end * length - first) / step).floor();
                    if finish >= start as f64 {
                        let finish = finish as usize;
                        if finish - start > 1_000_000 {
                            return Err("Stroke path is too long".into());
                        }
                        for n in start..=finish {
                            let t = (first + n as f64 * step) / length;
                            self.stamp(
                                doc,
                                [from[0] + delta[0] * t, from[1] + delta[1] * t],
                                self.pressure + (pressure - self.pressure) * t,
                                self.last_scale + (self.scale - self.last_scale) * t,
                            )?;
                        }
                    }
                }
                self.remainder = (self.remainder + length).rem_euclid(step);
            } else if (self.pressure != pressure
                && (self.style.pressure.size
                    || self.style.pressure.opacity
                    || self.style.material.composition.mixing_pressure.enabled
                    || self.style.dynamics.pressure.enabled))
                || self.last_scale != self.scale
            {
                self.stamp(doc, to, pressure, self.scale)?;
            }
        } else {
            self.stamp(doc, to, pressure, self.scale)?;
        }
        self.last = Some(to);
        self.last_scale = self.scale;
        self.pressure = pressure;
        Ok(())
    }
    pub fn finish_sample(&mut self, doc: &Document) -> Result<(), String> {
        for arm in &mut self.arms {
            arm.stroke.finish_sample(doc)?;
        }
        if self.direct {
            return self.check(doc);
        }
        if let Some(last) = self.last
            && self.last_stamp.is_none_or(|p| {
                (last[0] - p[0]).hypot(last[1] - p[1]) > 1e-7
                    || (self.last_stamp_pressure != self.pressure
                        && (self.style.pressure.size
                            || self.style.pressure.opacity
                            || self.style.material.composition.mixing_pressure.enabled
                            || self.style.dynamics.pressure.enabled))
                    || self.last_stamp_angle != self.rotation()
                    || self.last_stamp_scale != self.scale
            })
        {
            self.stamp(doc, last, self.pressure, self.scale)?;
        }
        Ok(())
    }
    pub fn commit(mut self, doc: &mut Document) -> Result<bool, String> {
        self.check(doc)?;
        self.finish_sample(doc)?;
        self.finish_material(doc, || false)?;
        if self.floating.is_some() {
            return self.commit_float(doc, None);
        }
        let layer = self.layer;
        let changes = self.collect_changes()?;
        doc.commit_pixel_changes(layer, changes)
    }
    pub fn commit_mouse(
        mut self,
        doc: &mut Document,
        metadata: crate::mouse_pressure::Metadata,
    ) -> Result<bool, String> {
        self.check(doc)?;
        self.finish_sample(doc)?;
        self.finish_material(doc, || false)?;
        if self.floating.is_some() {
            return self.commit_float(doc, Some(metadata));
        }
        let layer = self.layer;
        let changes = self.collect_changes()?;
        doc.commit_mouse_changes(layer, changes, metadata)
    }
    pub fn pixel(
        &self,
        doc: &Document,
        layer: usize,
        raster: &RasterLayer,
        x: u32,
        y: u32,
    ) -> Option<[u8; 4]> {
        if layer != self.layer || doc.id != self.document || doc.revision != self.revision {
            return None;
        }
        let offset = raster.pixel_index(doc.width, doc.height, x as i64, y as i64)?;
        if self.arms.is_empty() {
            self.lookup.get(&offset).map(|n| self.changes[*n].after)
        } else {
            self.changed_pixel(offset)
        }
    }
    fn changed_pixel(&self, offset: usize) -> Option<[u8; 4]> {
        self.lookup
            .get(&offset)
            .and_then(|n| {
                let change = &self.changes[*n];
                (change.before != change.after).then_some(change.after)
            })
            .or_else(|| {
                self.arms
                    .iter()
                    .find_map(|a| a.stroke.changed_pixel(offset))
            })
    }
    pub fn preview_key(&self) -> (u64, u64) {
        (
            self.nonce,
            self.arms.iter().fold(self.preview_revision, |n, a| {
                n.wrapping_add(a.stroke.preview_revision)
            }),
        )
    }
    pub fn mask_is_square(&self) -> bool {
        self.mask_square
    }
    pub fn mask_tip(&self) -> BrushTip {
        self.mask_tip
    }
    pub fn style(&self) -> Style {
        self.style
    }
    pub fn set_angle_offset(&mut self, angle: f64) -> Result<(), String> {
        if !angle.is_finite() || angle.abs() > 360.0 {
            return Err("Invalid pen tilt angle".into());
        }
        for arm in &mut self.arms {
            arm.stroke
                .set_angle_offset(if crate::rulers::reflected(arm.transform) {
                    -angle
                } else {
                    angle
                })?;
        }
        self.angle_offset = angle;
        Ok(())
    }
    pub fn shape(&self) -> Shape {
        let mut shape = self.style.shape;
        shape.angle += self.rotation();
        shape.size = self
            .dynamic_size(self.pressure, self.dynamic_index.saturating_sub(1))
            .unwrap_or(0.0)
            * self.scale;
        if self.scale > 0.0 {
            shape.size = self.style.pattern.behavior.minimum(shape.size);
        }
        if self.style.pattern.mode == crate::brush_pattern::Mode::Ribbon {
            shape.size *= shape.aspect;
            shape.aspect = 1.0;
        }
        shape
    }
    pub(crate) fn restore_input_trace(&mut self, trace: Trace, smoothing: f64) {
        self.trace = trace;
        self.style.smoothing = smoothing;
    }
    pub fn trace(&self) -> Trace {
        self.trace.clone()
    }
    pub fn changed_pixels(&self) -> usize {
        self.changes.len()
            + self
                .arms
                .iter()
                .map(|a| a.stroke.changed_pixels())
                .sum::<usize>()
    }
    fn stamp(
        &mut self,
        doc: &Document,
        center: Point,
        pressure: f64,
        scale: f64,
    ) -> Result<(), String> {
        if self.style.pattern.mode != crate::brush_pattern::Mode::Scatter {
            return self.stamp_single(doc, center, pressure, scale);
        }
        let pattern = self.style.pattern;
        for particle in 0..pattern.count {
            let tip = if self.style.sequence.enabled {
                self.sequence
                    .clone()
                    .next(self.style.sequence)
                    .map_or(0, |slot| self.style.sequence.tips[slot])
            } else {
                self.style.tip as u32 + 1
            };
            let mut offset = pattern.offset_for_tip(
                self.pattern_index,
                particle,
                tip,
                self.style.shape.angle + self.style.dynamics.scatter_angle,
            );
            if self.mirror_tip {
                let (sin, cos) = (self.style.shape.angle + self.style.dynamics.scatter_angle)
                    .to_radians()
                    .sin_cos();
                let x = offset[0] * cos + offset[1] * sin;
                let y = -offset[0] * sin + offset[1] * cos;
                offset = [x * cos + y * sin, x * sin - y * cos];
            }
            self.particle = Some([-offset[0], -offset[1]]);
            self.stamp_single(
                doc,
                [center[0] + offset[0], center[1] + offset[1]],
                pressure,
                scale,
            )?;
        }
        self.pattern_index = self.pattern_index.saturating_add(1);
        self.last_stamp = Some(center);
        Ok(())
    }
    fn stamp_single(
        &mut self,
        doc: &Document,
        center: Point,
        pressure: f64,
        scale: f64,
    ) -> Result<(), String> {
        if (!self.style.dynamics.is_default()
            || !self.style.variation.is_default()
            || self.style.pattern.mode != crate::brush_pattern::Mode::Dabs)
            && self.dynamic_index >= 262144
        {
            return Err("Brush dynamics dab-work budget exceeded; pending stroke cancelled".into());
        }
        self.last_stamp = Some(center);
        self.last_stamp_pressure = pressure;
        self.last_stamp_scale = scale;
        self.last_stamp_angle = self.rotation();
        self.last_dynamics = self.dynamics;
        self.dab_color = self
            .style
            .variation
            .color(self.style.color, self.dynamic_index);
        let mut shape = self.style.shape;
        shape.size = self.dynamic_size(pressure, self.dynamic_index)? * scale;
        if scale > 0.0 {
            shape.size = self.style.pattern.behavior.minimum(shape.size);
        }
        shape.angle += self.rotation();
        self.dynamic_index = self.dynamic_index.saturating_add(1);
        let ribbon = if self.style.pattern.mode == crate::brush_pattern::Mode::Ribbon {
            shape.size *= shape.aspect;
            shape.aspect = 1.0;
            self.ribbon.replace((center, shape.size * 0.5))
        } else {
            None
        };
        let pressure = self.style.pressure.map(pressure)?;
        let opacity = (if self.style.pressure.opacity {
            self.style.profile.opacity(pressure)
        } else {
            1.0
        }) * self.style.dynamics.opacity_speed.map(self.dynamics.speed);
        if shape.size == 0.0 || opacity == 0.0 || self.style.opacity == 0.0 {
            return Ok(());
        }
        let source = if self.style.sequence.enabled {
            let Some(slot) = self.sequence.next(self.style.sequence) else {
                return Ok(());
            };
            Some((slot, self.style.sequence.tips[slot]))
        } else {
            None
        };
        let slot = self.style.sequence.slots[source.map_or(0, |(n, _)| n)];
        let advanced = self.style.advanced_texture();
        let inherited = crate::tip_texture::Settings {
            pattern: self.style.texture,
            ..Default::default()
        };
        let primary_texture = slot.texture.unwrap_or(inherited);
        let secondary_texture = self
            .style
            .variation
            .dual
            .controls
            .texture
            .unwrap_or(inherited);
        let materials = self.materials.clone();
        let bitmap =
            source.and_then(|(_, id)| materials.get(id, crate::material_library::Kind::Tip));
        let mask_tip = source.map_or(self.style.tip, |(slot, _)| {
            self.style.sequence.tip(slot, self.style.tip)
        });
        let bitmap_square =
            bitmap.is_some() && self.style.profile.image == crate::tip_profile::Image::Native;
        self.mask_tip = mask_tip;
        self.mask_square = bitmap_square || self.style.profile.square(mask_tip);
        let pixel_tip = self.style.tip == BrushTip::PixelPen || mask_tip == BrushTip::PixelPen;
        let (center, radius, minor) = if pixel_tip {
            let width = shape.size.round().max(1.0);
            let height = (width * shape.aspect).round().max(1.0);
            let offset = |n: f64| {
                if (n as u64).is_multiple_of(2) {
                    1.0
                } else {
                    0.5
                }
            };
            let (sin, cos) = shape.angle.to_radians().sin_cos();
            let dimensions = if sin.abs() > cos.abs() {
                [height, width]
            } else {
                [width, height]
            };
            (
                if self.style.profile.pixel.grid || (width == 1.0 && height == 1.0) {
                    [
                        center[0].floor() + offset(dimensions[0]),
                        center[1].floor() + offset(dimensions[1]),
                    ]
                } else {
                    center
                },
                width / 2.0,
                height / 2.0,
            )
        } else {
            (center, shape.size * 0.5, shape.size * 0.5 * shape.aspect)
        };
        let (sin, cos) = shape.angle.to_radians().sin_cos();
        let extra = radius * self.style.variation.dual.extra_radius(shape.size);
        let extent = if bitmap_square || self.style.profile.square(mask_tip) {
            [
                radius * cos.abs() + minor * sin.abs() + 1.0,
                radius * sin.abs() + minor * cos.abs() + 1.0,
            ]
        } else {
            [
                (radius * cos).hypot(minor * sin) + 1.0,
                (radius * sin).hypot(minor * cos) + 1.0,
            ]
        };
        let extent = extent.map(|n| n + extra);
        let layer = &doc.layers[self.layer];
        let storage = layer.extent(doc.width, doc.height);
        let lower = std::array::from_fn::<_, 2, _>(|n| {
            (center[n] - extent[n]).min(ribbon.map_or(f64::INFINITY, |(p, r)| p[n] - r - 1.0))
        });
        let upper = std::array::from_fn::<_, 2, _>(|n| {
            (center[n] + extent[n]).max(ribbon.map_or(f64::NEG_INFINITY, |(p, r)| p[n] + r + 1.0))
        });
        let low = [
            lower[0].floor().max(0.0).max(storage.offset[0] as f64) as i64,
            lower[1].floor().max(0.0).max(storage.offset[1] as f64) as i64,
        ];
        let high = [
            upper[0]
                .ceil()
                .min(doc.width as f64 - 1.0)
                .min((storage.offset[0] + storage.width as i64 - 1) as f64) as i64,
            upper[1]
                .ceil()
                .min(doc.height as f64 - 1.0)
                .min((storage.offset[1] + storage.height as i64 - 1) as f64) as i64,
        ];
        let work = if high[0] >= low[0] && high[1] >= low[1] {
            (high[0] - low[0] + 1) as u64 * (high[1] - low[1] + 1) as u64
        } else {
            0
        };
        let samples = 1
            + if self.style.dual_rgb() { 4 } else { 0 }
            + if self.style.material.composition.render_quality == crate::color_mix::Quality::High
                && !pixel_tip
            {
                4
            } else {
                0
            }
            + bitmap.map_or(0, |asset| asset.sample_cost())
            + if advanced {
                primary_texture.cost()
                    + if self.style.variation.dual.active() {
                        secondary_texture.cost()
                    } else {
                        0
                    }
            } else {
                0
            }
            + if materials
                .get(
                    self.style.texture.source,
                    crate::material_library::Kind::Texture,
                )
                .is_some()
            {
                1
            } else {
                0
            };
        self.pixel_work = self.pixel_work.saturating_add(work.saturating_mul(samples));
        if self.pixel_work > 64 * 1024 * 1024 {
            return Err("Stroke pixel-work budget exceeded; pending stroke cancelled".into());
        }
        let mut changed = false;
        let picked = if self.style.material.mode == crate::brush_material::Mode::RunningColor
            && self.style.material.quality == crate::color_mix::Quality::High
        {
            self.pixel_work = self.pixel_work.saturating_add(9);
            if self.pixel_work > 64 * 1024 * 1024 {
                return Err("Color sampling work budget exceeded".into());
            }
            let mut samples = [([0; 4], 0.0); 9];
            let mut sample_count = 0;
            for dy in [-0.5, 0.0, 0.5] {
                for dx in [-0.5, 0.0, 0.5] {
                    let p = [
                        center[0] + dx * radius * cos - dy * minor * sin,
                        center[1] + dx * radius * sin + dy * minor * cos,
                    ];
                    if p[0] >= 0.0
                        && p[1] >= 0.0
                        && p[0] < doc.width as f64
                        && p[1] < doc.height as f64
                    {
                        let weight = self.coverage(doc, p) as f64 / 255.0;
                        if weight > 0.0 {
                            samples[sample_count] = (
                                self.candidate(doc, p[0].floor() as i64, p[1].floor() as i64),
                                weight,
                            );
                            sample_count += 1;
                        }
                    }
                }
            }
            crate::color_mix::average(self.style.material.space, samples)
        } else if self.style.material.mode == crate::brush_material::Mode::RunningColor
            && (center[0] < 0.0
                || center[1] < 0.0
                || center[0] >= doc.width as f64
                || center[1] >= doc.height as f64
                || self.coverage(doc, center) == 0)
        {
            [0; 4]
        } else {
            self.candidate(doc, center[0].floor() as i64, center[1].floor() as i64)
        };
        let float_paint = if self.floating.is_some() {
            Some(self.float_pigment(doc, center, pressure, [radius, minor], [cos, sin])?)
        } else {
            None
        };
        let paint_color = self.pigment_color(picked, center, pressure);
        for y in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                let point = [x as f64 + 0.5, y as f64 + 0.5];
                let mask = self.coverage(doc, point);
                if mask == 0 {
                    continue;
                }
                let dx = point[0] - center[0];
                let dy = point[1] - center[1];
                let mut raw_q = [
                    (dx * cos + dy * sin) / radius,
                    (-dx * sin + dy * cos) / minor,
                ];
                if self.mirror_tip {
                    raw_q[1] = -raw_q[1];
                }
                let q = slot.point(raw_q);
                let mirror_tip = self.mirror_tip;
                let geometry = |p: Point| {
                    let delta = [p[0] - center[0], p[1] - center[1]];
                    let local = slot.point([
                        (delta[0] * cos + delta[1] * sin) / radius,
                        (-delta[0] * sin + delta[1] * cos) / minor
                            * if mirror_tip { -1.0 } else { 1.0 },
                    ]);
                    if self.style.pattern.mode == crate::brush_pattern::Mode::Ribbon {
                        crate::brush_pattern::ribbon_distance(
                            p,
                            ribbon.unwrap_or((center, radius)),
                            (center, radius),
                        )
                    } else if self.style.pattern.mode == crate::brush_pattern::Mode::Decoration {
                        (self.style.pattern.decoration.distance(local), minor)
                    } else {
                        (
                            if bitmap_square || self.style.profile.square(mask_tip) {
                                local[0].abs().max(local[1].abs())
                            } else {
                                local[0].hypot(local[1])
                            },
                            minor,
                        )
                    }
                };
                let response = |(distance, edge_radius): (f64, f64)| {
                    let edge = ((1.0 - distance) * edge_radius + 0.5).clamp(0.0, 1.0);
                    let softness = if self.style.hardness == 1.0 {
                        1.0
                    } else {
                        ((1.0 - distance) / (1.0 - self.style.hardness)).clamp(0.0, 1.0)
                    };
                    edge * softness
                };
                let (distance, edge_radius) = geometry(point);
                let coverage = if pixel_tip {
                    if distance < 1.0 { 1.0 } else { 0.0 }
                } else if self.style.material.composition.render_quality
                    == crate::color_mix::Quality::High
                {
                    let mut sum = 0.0;
                    for oy in [-0.25, 0.25] {
                        for ox in [-0.25, 0.25] {
                            sum += response(geometry([point[0] + ox, point[1] + oy]));
                        }
                    }
                    sum * 0.25
                } else {
                    response((distance, edge_radius))
                };
                if coverage <= 0.0 && !self.style.variation.dual.active() {
                    continue;
                }
                let amount = match mask_tip {
                    BrushTip::Pen | BrushTip::Eraser | BrushTip::Smudge | BrushTip::PixelPen => {
                        coverage
                    }
                    BrushTip::Marker => coverage * 0.22,
                    BrushTip::Pencil => {
                        let [gx, gy] = if slot.flip.iter().any(|v| *v) {
                            [
                                (center[0] + q[0] * radius * cos - q[1] * minor * sin).floor()
                                    as i64,
                                (center[1] + q[0] * radius * sin + q[1] * minor * cos).floor()
                                    as i64,
                            ]
                        } else {
                            [x, y]
                        };
                        let grain = if self.seed == 0 {
                            ((gx.rem_euclid(23) * 17
                                + gy.rem_euclid(23) * 31
                                + gx.rem_euclid(23) * gy.rem_euclid(23) * 7)
                                .rem_euclid(23)) as f64
                                / 22.0
                        } else {
                            let mut hash = (gx as u64).wrapping_mul(0x9e3779b97f4a7c15)
                                ^ (gy as u64).rotate_left(31)
                                ^ self.seed as u64;
                            hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                            hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
                            ((hash ^ (hash >> 31)) % 23) as f64 / 22.0
                        };
                        coverage * (0.4 + 0.6 * grain)
                    }
                    BrushTip::Wash => {
                        coverage
                            * (1.0 - (distance - 0.5 / minor).max(0.0).powi(2))
                                .max(0.0)
                                .powi(3)
                            * 0.65
                    }
                };
                let amount = amount * bitmap.map_or(1.0, |asset| asset.tip(q));
                let amount = if advanced {
                    primary_texture.apply(amount, point, q, &materials)
                } else {
                    amount
                };
                let mut float_ink = float_paint;
                let mut float_amount = None;
                let (amount, paint_color) = if self.style.variation.dual.active() {
                    let secondary = self.style.variation.dual.mask(
                        raw_q,
                        minor,
                        point,
                        self.style.variation.seed,
                        shape.size,
                    );
                    let secondary = if advanced {
                        secondary_texture.apply(
                            secondary,
                            point,
                            self.style.variation.dual.local(raw_q, shape.size),
                            &materials,
                        )
                    } else {
                        secondary
                    };
                    if let Some(primary) = float_paint {
                        let (a, c) = self.float_dual(amount, secondary, primary)?;
                        float_ink = Some(c);
                        float_amount = Some(a);
                    }
                    self.style.variation.dual.ink(
                        amount,
                        secondary,
                        paint_color,
                        self.style.dual_rgb(),
                    )
                } else {
                    (amount, paint_color)
                };
                let amount = float_amount.unwrap_or(amount);
                if amount <= 0.0 {
                    continue;
                }
                let offset = layer.pixel_index(doc.width, doc.height, x, y).unwrap();
                let old = self.lookup.get(&offset).copied();
                let original = old.map_or(layer.pixels[offset], |n| self.changes[n].before);
                if self.style.material.alpha_lock
                    && if self.floating.is_some() {
                        layer.sample_at(offset).alpha() == 0.
                    } else {
                        original[3] == 0
                    }
                {
                    continue;
                }
                let unmasked = old.map_or(original, |n| self.unmasked[n]);
                let composition = self.style.material.composition;
                let special =
                    !self.style.tonal() && composition.mode != crate::brush_composite::Mode::Normal;
                let limited = self.style.tonal()
                    || special
                    || (self.style.dynamics.opacity_speed.enabled
                        && !self.style.pattern.behavior.continuous)
                    || self.style.clone_stamp.enabled
                    || self.style.pattern.mode == crate::brush_pattern::Mode::Ribbon
                    || self.style.tip == BrushTip::PixelPen
                    || (self.style.pressure.opacity
                        && self.style.tip != BrushTip::Marker
                        && self.style.tip != BrushTip::Smudge
                        && !self.style.pattern.behavior.continuous
                        && !self.style.pattern.behavior.spacing_density);
                let amount = amount * opacity;
                let amount = if limited {
                    amount
                } else {
                    let alpha = if self.style.tonal()
                        || self.style.erases()
                        || matches!(
                            self.style.material.mode,
                            crate::brush_material::Mode::Smudge
                                | crate::brush_material::Mode::RunningColor
                        )
                        || self.style.material.colorless
                    {
                        1.0
                    } else {
                        float_ink.map_or(paint_color[3] as f64 / 255.0, |v| v.alpha())
                    };
                    self.style
                        .pattern
                        .behavior
                        .corrected(amount, alpha, self.style.spacing)
                };
                let amount = if limited {
                    amount.max(old.map_or(0.0, |n| self.amounts[n]))
                } else {
                    amount
                };
                let base = if limited { original } else { unmasked };
                let next = if self.style.clone_stamp.enabled {
                    let source = self.style.clone_stamp.sample(doc, self.layer, [x, y]);
                    if self.style.material.alpha_lock && source[3] == 0 {
                        base
                    } else {
                        mix(base, source, amount)
                    }
                } else if self.style.pixel_overwrite() {
                    self.dab_color
                } else if self.style.tonal() {
                    self.style.material.tonal.apply(base, amount)
                } else if special {
                    composition
                        .mode
                        .compose(base, paint_color, amount, [x, y], composition.seed)
                } else if self.style.erases() {
                    [
                        base[0],
                        base[1],
                        base[2],
                        (base[3] as f64 * (1.0 - amount)).round() as u8,
                    ]
                } else if matches!(
                    self.style.material.mode,
                    crate::brush_material::Mode::Smudge | crate::brush_material::Mode::RunningColor
                ) || self.style.material.colorless
                {
                    mix(base, paint_color, amount)
                } else {
                    over(base, paint_color, amount)
                };
                let paint_factor = self.style.opacity
                    * if advanced {
                        1.0
                    } else {
                        self.materials.factor(self.style.texture, point)
                    };
                let factor = mask as f64 / 255.0 * paint_factor;
                let mut result = if self.style.pixel_overwrite() {
                    let mut ink = self.dab_color;
                    ink[3] = (ink[3] as f64 * amount * paint_factor).round() as u8;
                    // Brush controls determine replacement RGBA; selection blends
                    // that replacement with the original pixel only once.
                    mix(original, ink, mask as f64 / 255.0)
                } else if special {
                    composition.mode.compose(
                        original,
                        paint_color,
                        amount * factor,
                        [x, y],
                        composition.seed,
                    )
                } else {
                    mix(original, next, factor)
                };
                if self.style.material.alpha_lock {
                    result[3] = original[3];
                }
                let precise = self
                    .floating
                    .as_ref()
                    .map(|f| {
                        f.compute(
                            doc,
                            self.layer,
                            self.style,
                            floating::Dab {
                                offset,
                                previous: old,
                                amount,
                                limited,
                                color: if self.style.pixel_overwrite() {
                                    crate::color_precision::Sample::from8(self.dab_color)
                                } else {
                                    float_ink.unwrap_or_else(|| {
                                        crate::color_precision::Sample::from8(paint_color)
                                    })
                                },
                                point: [x, y],
                                paint_factor,
                                mask,
                                special,
                            },
                        )
                    })
                    .transpose()?;
                if let Some(value) = &precise {
                    result = value.preview();
                }
                if old.is_none()
                    && precise
                        .as_ref()
                        .map_or(result == original, |v| !v.changed())
                {
                    continue;
                }
                let index = if let Some(n) = old {
                    n
                } else {
                    if self.changes.len() >= self.limit {
                        return Err(
                            "Stroke exceeds available memory; reduce brush size or free history"
                                .into(),
                        );
                    }
                    self.changes.try_reserve(1).map_err(|e| e.to_string())?;
                    self.unmasked.try_reserve(1).map_err(|e| e.to_string())?;
                    self.amounts.try_reserve(1).map_err(|e| e.to_string())?;
                    self.lookup.try_reserve(1).map_err(|e| e.to_string())?;
                    let n = self.changes.len();
                    self.changes.push(PixelChange {
                        offset,
                        before: original,
                        after: original,
                    });
                    self.unmasked.push(original);
                    self.amounts.push(0.0);
                    self.lookup.insert(offset, n);
                    n
                };
                if let Some(value) = precise {
                    changed |= value.changed();
                    self.floating.as_mut().unwrap().record(index, value)?;
                }
                changed |= self.changes[index].after != result;
                self.changes[index].after = result;
                self.unmasked[index] = next;
                self.amounts[index] = amount;
            }
        }
        if changed {
            self.preview_revision += 1;
        }
        Ok(())
    }
}
fn mix(a: [u8; 4], b: [u8; 4], weight: f64) -> [u8; 4] {
    if weight == 1.0 {
        return b;
    }
    let aa = a[3] as f64 / 255.0;
    let ba = b[3] as f64 / 255.0;
    let alpha = aa * (1.0 - weight) + ba * weight;
    if alpha <= 0.0 {
        return [a[0], a[1], a[2], 0];
    }
    let mut out = [0; 4];
    for c in 0..3 {
        out[c] =
            ((a[c] as f64 * aa * (1.0 - weight) + b[c] as f64 * ba * weight) / alpha).round() as u8;
    }
    out[3] = (alpha * 255.0).round() as u8;
    out
}
fn over(before: [u8; 4], color: [u8; 4], amount: f64) -> [u8; 4] {
    let a = color[3] as f64 / 255.0 * amount;
    let b = before[3] as f64 / 255.0;
    let alpha = a + b * (1.0 - a);
    if alpha <= 0.0 {
        return before;
    }
    let mut out = [0; 4];
    for c in 0..3 {
        out[c] = ((color[c] as f64 * a + before[c] as f64 * b * (1.0 - a)) / alpha).round() as u8;
    }
    out[3] = (alpha * 255.0).round() as u8;
    out
}
fn clip(from: Point, to: Point, min: Point, max: Point) -> Option<[f64; 2]> {
    let mut start: f64 = 0.0;
    let mut end: f64 = 1.0;
    for axis in 0..2 {
        let d = to[axis] - from[axis];
        if d.abs() < 1e-12 {
            if from[axis] < min[axis] || from[axis] > max[axis] {
                return None;
            }
        } else {
            let a = (min[axis] - from[axis]) / d;
            let b = (max[axis] - from[axis]) / d;
            start = start.max(a.min(b));
            end = end.min(a.max(b));
            if start > end {
                return None;
            }
        }
    }
    Some([start, end])
}

/// Deterministic production-path check used by the installed application smoke run.
pub fn verify_spacing() -> Result<(), String> {
    use crate::document::{Background, CanvasSpec};
    for tip in BrushTip::ALL {
        for angle in [0.0, 37.0, 90.0] {
            let mut d = Document::create(
                CanvasSpec {
                    width: 128,
                    height: 64,
                    dpi: 300.0,
                    background: Background::Transparent,
                },
                131072,
            )
            .map_err(|e| e.to_string())?;
            if tip == BrushTip::Eraser {
                d.layers[0].pixels.fill([80, 90, 100, 255]);
            }
            if tip == BrushTip::Smudge {
                for y in 0..64 {
                    for x in 0..5 {
                        d.layers[0].pixels[y * 128 + x] = [210, 70, 20, 255];
                    }
                }
            }
            let before = d.snapshot();
            let style = Style {
                clone_stamp: Default::default(),
                variation: Default::default(),
                pattern: Default::default(),
                dynamics: Default::default(),
                sequence: Default::default(),
                filters: Default::default(),
                profile: Default::default(),
                texture: Default::default(),
                material: crate::brush_material::Settings {
                    keep_density: tip == BrushTip::Smudge,
                    ..crate::basic_brush::material(tip)
                },
                shape: Shape {
                    size: 20.0,
                    aspect: 0.05,
                    angle,
                },
                tip,
                color: [40, 80, 120, 255],
                opacity: 1.0,
                hardness: 1.0,
                spacing: 0.15,
                smoothing: 0.0,
                pressure: crate::pressure::Mapping {
                    size: false,
                    ..Default::default()
                },
            };
            let mut stroke = Stroke::begin(&d, 0, style)?;
            stroke.sample(&d, [4.5, 32.5])?;
            stroke.sample(&d, [122.5, 32.5])?;
            if !stroke.commit(&mut d)? {
                return Err("Sparse stroke failed".into());
            }
            if !(5..122).all(|x| {
                if tip == BrushTip::Eraser {
                    d.composite_pixel(x, 32)[3] < 255
                } else {
                    d.composite_pixel(x, 32)[3] > 0
                }
            }) {
                return Err(format!("Sparse thin-tip gaps: {tip:?}, {angle}"));
            }
            let after = d.snapshot();
            if !d.undo() || d.snapshot() != before || !d.redo() || d.snapshot() != after {
                return Err("Sparse stroke history diverged".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec},
        render::Renderer,
        selection::{Mask, Run},
    };
    fn doc() -> Document {
        Document::create(
            CanvasSpec {
                width: 32,
                height: 12,
                dpi: 300.0,
                background: Background::Transparent,
            },
            4096,
        )
        .unwrap()
    }
    fn style(tip: BrushTip) -> Style {
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
            opacity: 1.0,
            hardness: 1.0,
            spacing: 0.15,
            shape: Shape {
                size: 1.0,
                aspect: 1.0,
                angle: 0.0,
            },
            tip,
            color: [30, 60, 90, 200],
            smoothing: 0.0,
            pressure: crate::pressure::Mapping {
                size: false,
                opacity: false,
                ..Default::default()
            },
        }
    }
    #[test]
    fn symmetry_ignores_noop_primary_when_another_arm_changes_the_pixel() {
        let mut d = doc();
        d.scene
            .rulers
            .add(
                "Mirror",
                crate::rulers::Kind::Mirror {
                    a: [16.0, 0.0],
                    b: [16.0, 12.0],
                },
            )
            .unwrap();
        let before = d.snapshot();
        let mut stroke = Stroke::begin(&d, 0, style(BrushTip::Pen)).unwrap();
        stroke.changes.push(PixelChange {
            offset: 0,
            before: [0; 4],
            after: [0; 4],
        });
        stroke.lookup.insert(0, 0);
        let arm = &mut stroke.arms[0].stroke;
        arm.changes.push(PixelChange {
            offset: 0,
            before: [0; 4],
            after: [20, 40, 60, 128],
        });
        arm.lookup.insert(0, 0);
        assert_eq!(
            stroke.pixel(&d, 0, &d.layers[0], 0, 0),
            Some([20, 40, 60, 128])
        );
        assert!(stroke.commit(&mut d).unwrap());
        assert_eq!(d.composite_pixel(0, 0), [20, 40, 60, 128]);
        assert_eq!(d.history_counts(), (1, 0));
        assert!(d.undo());
        assert_eq!(d.snapshot(), before);
    }
    #[test]
    fn pattern_dab_budget_rejects_without_publishing() {
        let mut d = doc();
        let before = d.snapshot();
        let mut s = style(BrushTip::Pen);
        s.pattern.mode = crate::brush_pattern::Mode::Scatter;
        let mut st = Stroke::begin(&d, 0, s).unwrap();
        st.dynamic_index = 262144;
        assert!(st.direct_dab(&d, [12.5, 6.5], [12.5, 6.5], 1.0).is_err());
        drop(st);
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
        assert!(!d.undo());
    }
    #[test]
    fn running_quality_respects_selection_boundary_alpha_lock_and_empty_edges() {
        use crate::{brush_material::Mode, color_mix::Quality, scene::Rect};
        for alpha_lock in [false, true] {
            let mut d = doc();
            for y in 0..12 {
                for x in 0..32 {
                    d.layers[0].pixels[y * 32 + x] = if x < 16 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 0, 255, 128]
                    };
                }
            }
            d.scene.selection = Some(Rect {
                min: [16.0, 0.0],
                max: [32.0, 12.0],
            });
            let before = d.snapshot();
            let mut s = style(BrushTip::Pen);
            s.shape.size = 10.0;
            s.material = crate::brush_material::Settings {
                mode: Mode::RunningColor,
                quality: Quality::High,
                supply: 0.0,
                mixing: 1.0,
                alpha_lock,
                ..Default::default()
            };
            let mut st = Stroke::begin(&d, 0, s).unwrap();
            st.direct_dab(&d, [16.5, 6.5], [16.5, 6.5], 1.0).unwrap();
            st.commit(&mut d).unwrap();
            assert_eq!(d.composite_pixel(15, 6), [255, 0, 0, 255]);
            assert_eq!(d.composite_pixel(16, 6), [0, 0, 255, 128]);
            assert!(d.snapshot() == before);
        }
        let mut d = doc();
        let mut s = style(BrushTip::Pen);
        s.material.mode = Mode::RunningColor;
        s.material.quality = Quality::High;
        s.material.supply = 0.0;
        s.material.mixing = 1.0;
        let mut st = Stroke::begin(&d, 0, s).unwrap();
        st.direct_dab(&d, [-0.1, 6.5], [-0.1, 6.5], 1.0).unwrap();
        assert!(!st.commit(&mut d).unwrap());
        let mut st = Stroke::begin(&d, 0, s).unwrap();
        st.pixel_work = 64 * 1024 * 1024;
        assert!(st.direct_dab(&d, [4.5, 6.5], [4.5, 6.5], 1.0).is_err());
        assert_eq!(d.history_counts(), (0, 0));
    }
    #[test]
    fn sparse_fast_thin_tips_are_connected_and_replay_history_is_exact() {
        verify_spacing().unwrap();
    }
    #[test]
    fn curves_packet_boundaries_large_motion_pressure_phase_and_budget_are_bounded() {
        fn draw(points: &[[f64; 2]]) -> Document {
            let mut d = Document::create(
                CanvasSpec {
                    width: 128,
                    height: 64,
                    dpi: 300.0,
                    background: Background::Transparent,
                },
                131072,
            )
            .unwrap();
            let mut style = style(BrushTip::Pen);
            style.shape.size = 4.0;
            let mut stroke = Stroke::begin(&d, 0, style).unwrap();
            for p in points {
                stroke.sample(&d, *p).unwrap();
            }
            stroke.commit(&mut d).unwrap();
            d
        }
        let sparse = [[4.5, 12.5], [50.5, 12.5], [95.5, 44.5], [122.5, 20.5]];
        let mut dense = vec![sparse[0]];
        for pair in sparse.windows(2) {
            for n in 1..=32 {
                let t = n as f64 / 32.0;
                dense.push([
                    pair[0][0] + (pair[1][0] - pair[0][0]) * t,
                    pair[0][1] + (pair[1][1] - pair[0][1]) * t,
                ]);
            }
        }
        let a = draw(&sparse);
        let b = draw(&dense);
        assert!(a.snapshot() == b.snapshot());
        for pair in sparse.windows(2) {
            for n in 0..=200 {
                let t = n as f64 / 200.0;
                let p = [
                    pair[0][0] + (pair[1][0] - pair[0][0]) * t,
                    pair[0][1] + (pair[1][1] - pair[0][1]) * t,
                ];
                assert!(a.composite_pixel(p[0].floor() as u32, p[1].floor() as u32)[3] > 0);
            }
        }
        let outside = draw(&[[-1e12, 32.5], [1e12, 32.5]]);
        assert!((0..128).all(|x| outside.composite_pixel(x, 32)[3] > 0));
        let mut d = doc();
        let before = d.snapshot();
        let mut st = style(BrushTip::Pen);
        st.shape.size = 8.0;
        st.pressure.size = true;
        let mut stroke = Stroke::begin(&d, 0, st).unwrap();
        stroke.sample_pressure(&d, [4.5, 6.5], 1.0).unwrap();
        stroke.sample_pressure(&d, [5.6, 6.5], 1.0).unwrap();
        stroke.sample_pressure(&d, [6.0, 6.5], 0.01).unwrap();
        assert!(stroke.last_stamp.unwrap()[0] <= 6.0 + 1e-9);
        assert!(stroke.remainder < st.step(0.01));
        drop(stroke);
        assert!(d.snapshot() == before);
        let mut stroke = Stroke::begin(&d, 0, st).unwrap();
        stroke.pixel_work = 64 * 1024 * 1024;
        assert!(stroke.sample(&d, [10.5, 6.5]).is_err());
        drop(stroke);
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
        // A short segment still receives its real endpoint once; repeated positions add no phantom tail.
        let mut stroke = Stroke::begin(&d, 0, style(BrushTip::Pen)).unwrap();
        stroke.sample(&d, [10.5, 6.5]).unwrap();
        stroke.sample(&d, [10.55, 6.5]).unwrap();
        stroke.sample(&d, [10.55, 6.5]).unwrap();
        stroke.finish_sample(&d).unwrap();
        assert_eq!(stroke.last_stamp, Some([10.55, 6.5]));
        stroke.commit(&mut d).unwrap();
        assert_eq!(d.history_counts(), (1, 0));
    }
    #[test]
    fn brush_parameters_validate_softness_spacing_opacity_and_short_stroke() {
        let mut s = style(BrushTip::Pen);
        for (opacity, hardness, spacing) in [(0.0, 0.0, 0.01), (1.0, 1.0, 10.0)] {
            s.opacity = opacity;
            s.hardness = hardness;
            s.spacing = spacing;
            assert!(s.valid());
        }
        for bad in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
            s = style(BrushTip::Pen);
            s.opacity = bad;
            assert!(!s.valid());
            s = style(BrushTip::Pen);
            s.hardness = bad;
            assert!(!s.valid());
        }
        s = style(BrushTip::Pen);
        s.shape.size = 0.0;
        assert!(Stroke::begin(&doc(), 0, s).is_err());
        for bad in [0.0, 10.01, f64::NAN] {
            s = style(BrushTip::Pen);
            s.spacing = bad;
            assert!(!s.valid());
        }
        let mut hard = doc();
        let mut soft = doc();
        s = style(BrushTip::Pen);
        s.shape.size = 8.0;
        let mut stroke = Stroke::begin(&hard, 0, s).unwrap();
        stroke.sample(&hard, [12.5, 6.5]).unwrap();
        stroke.commit(&mut hard).unwrap();
        s.hardness = 0.0;
        let mut stroke = Stroke::begin(&soft, 0, s).unwrap();
        stroke.sample(&soft, [12.5, 6.5]).unwrap();
        stroke.commit(&mut soft).unwrap();
        assert_eq!(hard.composite_pixel(12, 6), soft.composite_pixel(12, 6));
        assert!(
            soft.composite_pixel(15, 6)[3] > 0
                && soft.composite_pixel(15, 6)[3] < hard.composite_pixel(15, 6)[3]
        );
        let mut d = doc();
        let original = d.snapshot();
        s.opacity = 0.0;
        let mut stroke = Stroke::begin(&d, 0, s).unwrap();
        stroke.sample(&d, [4.5, 6.5]).unwrap();
        stroke.sample(&d, [4.6, 6.5]).unwrap();
        assert!(!stroke.commit(&mut d).unwrap());
        assert!(d.snapshot() == original);
        assert_eq!(d.history_counts(), (0, 0));
        s.hardness = 1.0;
        s.opacity = 0.5;
        s.color[3] = 255;
        s.spacing = 0.01;
        let mut stroke = Stroke::begin(&d, 0, s).unwrap();
        for x in [4.5, 20.5, 4.5, 20.5] {
            stroke.sample(&d, [x, 6.5]).unwrap();
        }
        stroke.commit(&mut d).unwrap();
        assert!(d.layers[0].pixels.iter().all(|p| p[3] <= 128));
        assert_eq!(d.composite_pixel(4, 6)[3], 128);
        let mut sparse = doc();
        s.opacity = 1.0;
        s.spacing = 10.0;
        s.shape.size = 1.0;
        let mut stroke = Stroke::begin(&sparse, 0, s).unwrap();
        stroke.sample(&sparse, [2.5, 6.5]).unwrap();
        stroke.sample(&sparse, [29.5, 6.5]).unwrap();
        stroke.commit(&mut sparse).unwrap();
        assert_eq!(sparse.composite_pixel(6, 6)[3], 0);
        assert!(sparse.composite_pixel(2, 6)[3] > 0);
        assert!(sparse.composite_pixel(29, 6)[3] > 0);
        let mut legacy = serde_json::to_value(s).unwrap();
        for key in ["opacity", "hardness", "spacing"] {
            legacy.as_object_mut().unwrap().remove(key);
        }
        let restored: Style = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            (restored.opacity, restored.hardness, restored.spacing),
            (1.0, 1.0, 0.15)
        );
    }
    #[test]
    fn normalized_pressure_axes_are_independent_zero_half_full_and_bounded() {
        use crate::pressure::Mapping;
        for mapping in [
            Mapping {
                size: false,
                opacity: false,
                ..Default::default()
            },
            Mapping {
                size: true,
                opacity: false,
                ..Default::default()
            },
            Mapping {
                size: false,
                opacity: true,
                ..Default::default()
            },
            Mapping {
                size: true,
                opacity: true,
                ..Default::default()
            },
        ] {
            let mut widths = vec![];
            for pressure in [0.0, 0.5, 1.0] {
                let mut d = Document::create(
                    CanvasSpec {
                        width: 32,
                        height: 32,
                        dpi: 300.0,
                        background: Background::Transparent,
                    },
                    8192,
                )
                .unwrap();
                let before = d.snapshot();
                let mut style = style(BrushTip::Pen);
                style.shape.size = 12.0;
                style.pressure = mapping;
                let mut s = Stroke::begin(&d, 0, style).unwrap();
                s.sample_pressure(&d, [16.5, 16.5], pressure).unwrap();
                let preview = Renderer::preview(&d, Some(&s)).pixel([16.5, 16.5]);
                let changed = s.commit(&mut d).unwrap();
                let expected = if (mapping.size || mapping.opacity) && pressure == 0.0 {
                    0
                } else if mapping.opacity {
                    (200.0 * pressure).round() as u8
                } else {
                    200
                };
                assert_eq!(d.composite_pixel(16, 16)[3], expected);
                assert_eq!(preview, d.composite_pixel(16, 16));
                assert_eq!(changed, expected > 0);
                widths.push((0..32).filter(|x| d.composite_pixel(*x, 16)[3] > 0).count());
                if changed {
                    let after = d.snapshot();
                    assert!(d.undo());
                    assert!(d.snapshot() == before);
                    assert!(d.redo());
                    assert!(d.snapshot() == after);
                }
            }
            if mapping.size {
                assert!(widths[1] < widths[2]);
            } else {
                assert_eq!(widths[1], widths[2]);
            }
        }
        let mut d = doc();
        let mut style = style(BrushTip::Pen);
        style.pressure = Mapping {
            size: false,
            opacity: true,
            ..Default::default()
        };
        let mut s = Stroke::begin(&d, 0, style).unwrap();
        for x in [2.5, 29.5, 2.5, 29.5] {
            s.sample_pressure(&d, [x, 4.5], 0.5).unwrap();
        }
        s.commit(&mut d).unwrap();
        // Diameter-relative 0.15 spacing need not land on this exact pixel centre.
        assert!((95..=100).contains(&d.composite_pixel(12, 4)[3]));
        assert_eq!(d.composite_pixel(2, 4)[3], 100);
        assert!(d.layers[0].pixels.iter().all(|p| p[3] <= 100));
        let before = d.snapshot();
        let mut s = Stroke::begin(&d, 0, style).unwrap();
        for p in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            assert!(s.sample_pressure(&d, [2.5, 4.5], p).is_err());
        }
        drop(s);
        assert!(d.snapshot() == before);
        // A stationary pen can change pressure without needing a fake movement.
        let mut s = Stroke::begin(&d, 0, style).unwrap();
        s.sample_pressure(&d, [2.5, 8.5], 0.0).unwrap();
        s.sample_pressure(&d, [2.5, 8.5], 1.0).unwrap();
        s.commit(&mut d).unwrap();
        assert_eq!(d.composite_pixel(2, 8)[3], 200);
        let mut off = style;
        off.pressure = Mapping {
            size: false,
            opacity: false,
            ..Default::default()
        };
        let mut s = Stroke::begin(&d, 0, off).unwrap();
        for p in [0.0, 0.5, 1.0, 0.0] {
            s.sample_pressure(&d, [25.5, 8.5], p).unwrap();
        }
        s.commit(&mut d).unwrap();
        assert_eq!(d.composite_pixel(25, 8)[3], 200);
    }
    #[test]
    fn pressure_range_curve_changes_new_stroke_only_and_invalid_style_is_rejected() {
        use crate::pressure::Response;
        let mut d = doc();
        d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        let before = d.snapshot();
        let mut style = style(BrushTip::Pen);
        style.shape.size = 4.0;
        style.pressure.opacity = true;
        style.pressure.response = Response {
            minimum: 0.25,
            maximum: 0.75,
            curve: [0.0, 0.1, 0.25, 0.8, 1.0],
        };
        let mut s = Stroke::begin(&d, 0, style).unwrap();
        // Changing the external draft cannot alter a stroke that is already underway.
        style.pressure.response = Response::default();
        s.sample_pressure(&d, [12.5, 6.5], 0.5).unwrap();
        assert_eq!(Renderer::preview(&d, Some(&s)).pixel([12.5, 6.5])[3], 50);
        s.commit(&mut d).unwrap();
        assert_eq!(d.composite_pixel(12, 6)[3], 50);
        assert_eq!(d.composite_pixel(0, 0), [1, 2, 3, 255]);
        assert!(d.undo());
        assert!(d.snapshot() == before);
        style.pressure.size = true;
        style.pressure.opacity = false;
        style.pressure.response = Response {
            minimum: 0.25,
            maximum: 0.75,
            curve: [0.0, 0.1, 0.25, 0.8, 1.0],
        };
        let mut s = Stroke::begin(&d, 0, style).unwrap();
        s.sample_pressure(&d, [12.5, 6.5], 0.5).unwrap();
        assert_eq!(s.shape().size, 1.0);
        s.commit(&mut d).unwrap();
        assert_eq!(d.composite_pixel(12, 6)[3], 200);
        style.pressure.response.minimum = style.pressure.response.maximum;
        assert!(Stroke::begin(&d, 0, style).is_err());
    }
    #[test]
    fn point_and_fast_drag_preview_commit_one_history_and_exact_undo_redo() {
        let mut d = doc();
        d.mark_saved("point.drawer".into());
        let before = d.snapshot();
        let mut s = Stroke::begin(&d, 0, style(BrushTip::Pen)).unwrap();
        s.sample(&d, [2.5, 4.5]).unwrap();
        assert_eq!(d.snapshot(), before);
        assert_eq!(d.history_counts(), (0, 0));
        let preview = Renderer::preview(&d, Some(&s)).pixel([2.5, 4.5]);
        assert!(preview[3] > 0);
        assert!(s.commit(&mut d).unwrap());
        assert_eq!(d.composite_pixel(2, 4), preview);
        assert_eq!(d.history_counts(), (1, 0));
        assert!(d.dirty);
        let after = d.snapshot();
        assert!(d.undo());
        assert_eq!(d.snapshot(), before);
        assert!(!d.dirty);
        assert!(d.redo());
        assert_eq!(d.snapshot(), after);
        let mut a = doc();
        let mut b = doc();
        let mut fast = Stroke::begin(&a, 0, style(BrushTip::Pencil)).unwrap();
        fast.sample(&a, [1.5, 4.5]).unwrap();
        fast.sample(&a, [30.5, 4.5]).unwrap();
        fast.commit(&mut a).unwrap();
        let mut dense = Stroke::begin(&b, 0, style(BrushTip::Pencil)).unwrap();
        for x in 1..=30 {
            dense.sample(&b, [x as f64 + 0.5, 4.5]).unwrap();
        }
        dense.commit(&mut b).unwrap();
        assert_eq!(a.snapshot(), b.snapshot());
        assert_eq!(a.history_counts(), (1, 0));
        assert!((1..=30).all(|x| a.composite_pixel(x, 4)[3] > 0));
    }
    #[test]
    fn eraser_selection_feather_outside_clipping_cancel_limits_and_stale_input() {
        let mut d = doc();
        d.layers[0].pixels.fill([80, 90, 100, 255]);
        d.scene.selection_mask = Some(Mask {
            runs: vec![Run {
                x: 10,
                y: 4,
                length: 10,
                alpha: 128,
            }],
        });
        let before = d.snapshot();
        let mut s = Stroke::begin(&d, 0, style(BrushTip::Eraser)).unwrap();
        s.sample(&d, [-1e12, 4.5]).unwrap();
        s.sample(&d, [1e12, 4.5]).unwrap();
        assert_eq!(d.snapshot(), before);
        s.commit(&mut d).unwrap();
        assert_eq!(d.history_counts(), (1, 0));
        for x in 0..32 {
            assert_eq!(
                d.layers[0].pixels[4 * 32 + x][3],
                if (10..20).contains(&x) { 127 } else { 255 }
            );
        }
        assert!(d.undo());
        assert_eq!(d.snapshot(), before);
        d.scene.selection_mask = Some(Mask::default());
        let mut s = Stroke::begin(&d, 0, style(BrushTip::Pen)).unwrap();
        s.sample(&d, [2.0, 2.0]).unwrap();
        assert!(!s.commit(&mut d).unwrap());
        d.scene.selection_mask = None;
        let before = d.snapshot();
        // Keep five changed-pixel slots after the current stroke's fixed overhead.
        // Adding brush settings must not turn this pixel-limit test into a constructor failure.
        let budget = std::mem::size_of::<Stroke>() + 256 + 5 * 96;
        let mut s = Stroke::with_budget(&d, 0, style(BrushTip::Pen), budget).unwrap();
        assert!(s.sample(&d, [f64::NAN, 0.0]).is_err());
        s.sample(&d, [2.5, 4.5]).unwrap();
        assert!(s.sample(&d, [30.5, 4.5]).is_err());
        drop(s);
        assert_eq!(d.snapshot(), before);
        let mut s = Stroke::begin(&d, 0, style(BrushTip::Pen)).unwrap();
        s.sample(&d, [2.5, 4.5]).unwrap();
        d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        assert!(s.commit(&mut d).is_err());
        assert_eq!(d.layers[0].pixels[0], [1, 2, 3, 255]);
    }
}
