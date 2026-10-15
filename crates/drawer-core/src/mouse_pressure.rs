//! Frozen mouse distance/time input, reconstructed from the unchanged document.
use crate::{
    brush_cursor::Trace,
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    painting::{Stroke, Style},
    scene::Point,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    #[default]
    Normal,
    Blend,
    Smudge,
    Spray,
    Wet,
}
impl Effect {
    pub const ALL: [Self; 5] = [
        Self::Normal,
        Self::Blend,
        Self::Smudge,
        Self::Spray,
        Self::Wet,
    ];
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub effect: Effect,
    #[serde(default = "default_strength")]
    pub strength: f64,
    #[serde(default = "default_period")]
    pub spray_period_micros: u32,
    #[serde(default = "default_dabs")]
    pub spray_dabs: u32,
    pub enabled: bool,
    pub start_length: f64,
    pub end_length: f64,
    pub start_curve: [f64; 5],
    pub end_curve: [f64; 5],
    pub speed: bool,
    pub speed_full: f64,
    pub speed_minimum: f64,
    pub seed: u32,
}
fn default_strength() -> f64 {
    0.5
}
fn default_period() -> u32 {
    20000
}
fn default_dabs() -> u32 {
    4
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            effect: Effect::Normal,
            strength: 0.5,
            spray_period_micros: 20000,
            spray_dabs: 4,
            enabled: false,
            start_length: 12.0,
            end_length: 12.0,
            start_curve: [0.0, 0.25, 0.5, 0.75, 1.0],
            end_curve: [0.0, 0.25, 0.5, 0.75, 1.0],
            speed: false,
            speed_full: 2000.0,
            speed_minimum: 0.2,
            seed: 0,
        }
    }
}
impl Settings {
    pub fn active(self) -> bool {
        self.enabled || self.speed || self.seed != 0 || self.effect != Effect::Normal
    }
    pub fn valid(self) -> bool {
        self.strength.is_finite()
            && (0.0..=1.0).contains(&self.strength)
            && (1000..=1000000).contains(&self.spray_period_micros)
            && (1..=64).contains(&self.spray_dabs)
            && [self.start_length, self.end_length]
                .iter()
                .all(|v| v.is_finite() && (0.0..=1e6).contains(v))
            && [self.start_curve, self.end_curve].iter().all(|c| {
                c[0] == 0.0
                    && c[4] == 1.0
                    && c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    && c.windows(2).all(|v| v[0] <= v[1])
            })
            && self.speed_full.is_finite()
            && (0.01..=1e6).contains(&self.speed_full)
            && self.speed_minimum.is_finite()
            && (0.0..=1.0).contains(&self.speed_minimum)
    }
    fn pressure(self, distance: f64, total: f64, finalizing: bool, speed: f64) -> f64 {
        let mut p = 1.0;
        if self.enabled && total > 1e-9 {
            if self.start_length > 0.0 {
                p *= curve(self.start_curve, distance / self.start_length);
            }
            if finalizing && self.end_length > 0.0 {
                p *= curve(self.end_curve, (total - distance) / self.end_length);
            }
        }
        p * speed
    }
}
fn curve(c: [f64; 5], position: f64) -> f64 {
    let p = position.clamp(0.0, 1.0) * 4.0;
    let n = (p as usize).min(3);
    c[n] + (c[n + 1] - c[n]) * (p - n as f64)
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub point: Point,
    pub micros: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub layer: usize,
    pub settings: Settings,
    pub style: Style,
    pub count: u32,
    #[serde(default)]
    pub ends: crate::stroke_ends::Settings,
    #[serde(default)]
    pub native_pressure: bool,
}
impl Metadata {
    pub fn valid(&self) -> bool {
        self.layer < 256
            && self.settings.valid()
            && self.style.valid()
            && self.count > 0
            && self.ends.valid()
            && (!self.native_pressure || !self.settings.active())
    }
}
pub struct Replay {
    document: u64,
    revision: u64,
    layer: usize,
    settings: Settings,
    style: Style,
    samples: Vec<Sample>,
    clocks: Vec<bool>,
    budget: usize,
    ends: crate::stroke_ends::Settings,
    native_pressure: bool,
    inputs: Vec<[f64; 2]>,
}
impl Replay {
    pub fn begin(
        doc: &Document,
        layer: usize,
        style: Style,
        settings: Settings,
    ) -> Result<Self, String> {
        if !settings.valid() || !style.valid() {
            return Err("Invalid mouse stroke settings".into());
        }
        if !doc.scene.valid()
            || doc
                .layers
                .get(layer)
                .is_none_or(|l| l.locked || !l.valid(doc.width, doc.height))
        {
            return Err("Select an unlocked raster layer for mouse reconstruction".into());
        }
        let budget = (DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(doc.resident_bytes())
            .saturating_sub(16 * 1024 * 1024)
            / 3)
        .min(DEFAULT_DOCUMENT_BUDGET / 4);
        if budget < 4096 {
            return Err("Not enough memory to preserve and reconstruct a mouse stroke".into());
        }
        Ok(Self {
            document: doc.id,
            revision: doc.revision,
            layer,
            settings,
            style,
            samples: vec![],
            clocks: vec![],
            budget,
            ends: Default::default(),
            native_pressure: false,
            inputs: vec![],
        })
    }
    pub fn configure_input(
        mut self,
        ends: crate::stroke_ends::Settings,
        native_pressure: bool,
    ) -> Result<Self, String> {
        if !ends.valid() || (native_pressure && self.settings.active()) {
            return Err("Native pressure must stay independent of mouse virtual pressure".into());
        }
        self.ends = ends;
        self.native_pressure = native_pressure;
        Ok(self)
    }
    pub fn push_input(&mut self, sample: Sample, pressure: f64, angle: f64) -> Result<(), String> {
        if !(0.0..=1.0).contains(&pressure) || !angle.is_finite() || angle.abs() > 360.0 {
            return Err("Invalid native stroke pressure/angle".into());
        }
        self.push(sample)?;
        *self.inputs.last_mut().unwrap() = [pressure, angle];
        Ok(())
    }
    pub fn push(&mut self, sample: Sample) -> Result<(), String> {
        self.push_sample(sample, false)
    }
    pub fn push_clock(&mut self, sample: Sample) -> Result<(), String> {
        self.push_sample(sample, true)
    }
    fn push_sample(&mut self, sample: Sample, clock: bool) -> Result<(), String> {
        if self.samples.len() >= 4096
            || sample
                .point
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 1e12)
        {
            return Err("Mouse stroke input/resource limit; pending stroke cancelled".into());
        }
        self.samples
            .try_reserve(1)
            .map_err(|_| "Mouse input recording allocation failed")?;
        self.clocks
            .try_reserve(1)
            .map_err(|_| "Mouse clock recording allocation failed")?;
        self.inputs
            .try_reserve(1)
            .map_err(|_| "Stroke attribute recording allocation failed")?;
        self.inputs.push([1.0, 0.0]);
        self.samples.push(sample);
        self.clocks.push(clock);
        Ok(())
    }
    /// A real release timestamp is authoritative over provisional frame clocks.
    pub fn trim_clocks(&mut self, end: u64) {
        let keep: Vec<_> = self
            .samples
            .iter()
            .zip(&self.clocks)
            .map(|(sample, clock)| !clock || sample.micros.is_some_and(|t| t <= end))
            .collect();
        let mut n = 0;
        self.samples.retain(|_| {
            let result = keep[n];
            n += 1;
            result
        });
        let mut n = 0;
        self.inputs.retain(|_| {
            let result = keep[n];
            n += 1;
            result
        });
        let mut n = 0;
        self.clocks.retain(|_| {
            let result = keep[n];
            n += 1;
            result
        });
    }
    pub fn metadata(&self) -> Metadata {
        Metadata {
            layer: self.layer,
            settings: self.settings,
            style: self.style,
            count: 1,
            ends: self.ends,
            native_pressure: self.native_pressure,
        }
    }
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }
    pub fn spray_period(&self) -> Option<u32> {
        (self.settings.effect == Effect::Spray).then_some(self.settings.spray_period_micros)
    }
    pub fn render(&self, doc: &Document, finalizing: bool) -> Result<Stroke, String> {
        self.render_cancel(doc, finalizing, || false)
    }
    pub fn render_cancel(
        &self,
        doc: &Document,
        finalizing: bool,
        cancel: impl Fn() -> bool,
    ) -> Result<Stroke, String> {
        if cancel() {
            return Err("Mouse reconstruction cancelled".into());
        }
        if doc.id != self.document || doc.revision != self.revision {
            return Err("The original drawing changed; mouse stroke cancelled".into());
        }
        let mut trace = Trace::default();
        let mut points = Vec::new();
        points
            .try_reserve_exact(self.samples.len())
            .map_err(|_| "Mouse reconstruction allocation failed")?;
        let mut distance = 0.0;
        let mut last: Option<Point> = None;
        for (n, sample) in self.samples.iter().enumerate() {
            if n % 256 == 0 && cancel() {
                return Err("Mouse reconstruction cancelled".into());
            }
            let point = trace
                .receive(sample.point, self.style.smoothing)
                .map(|(_, p)| p)
                .or(last)
                .unwrap_or(sample.point);
            // Release catches the filter up to the actual final sample, never prediction.
            let point = if finalizing && n + 1 == self.samples.len() {
                sample.point
            } else {
                point
            };
            if let Some(previous) = last {
                distance += (point[0] - previous[0]).hypot(point[1] - previous[1]);
            }
            points.push((point, distance, sample.micros));
            last = Some(point);
        }
        if distance > 131072.0 {
            return Err(
                "Mouse reconstruction path budget exceeded; pending stroke cancelled".into(),
            );
        }
        let mut style = self.style;
        style.smoothing = 0.0;
        if self.settings.effect == Effect::Spray {
            style.shape.size = (style.shape.size * 0.15).max(0.1);
        }
        let mut stroke = Stroke::with_budget(doc, self.layer, style, self.budget)?;
        stroke.set_seed(self.settings.seed);
        if let Some((point, _, _)) = points.first() {
            stroke.material(doc, self.settings.effect, self.settings.strength, *point)?;
        }
        if (style.color[3] == 0
            && !style.material.pigment()
            && !matches!(self.settings.effect, Effect::Blend | Effect::Smudge))
            || style.opacity == 0.0
        {
            stroke.restore_input_trace(trace, self.style.smoothing);
            return Ok(stroke);
        }
        if self.settings.effect == Effect::Spray {
            self.spray(doc, &mut stroke, &points, distance, finalizing, &cancel)?;
            if cancel() {
                return Err("Timed mouse spray cancelled".into());
            }
            if finalizing {
                stroke.finish_material(doc, &cancel)?;
            }
            stroke.restore_input_trace(trace, self.style.smoothing);
            return Ok(stroke);
        }
        let area = style.shape.size.min(doc.width as f64)
            * (style.shape.size * style.shape.aspect).min(doc.height as f64);
        let work = area * (distance / style.step(1.0) + 2.0);
        if work > 64.0 * 1024.0 * 1024.0 {
            return Err(
                "Mouse reconstruction pixel-work budget exceeded; pending stroke cancelled".into(),
            );
        }
        let mut previous: Option<(Point, f64, Option<u64>)> = None;
        let mut steps = 0usize;
        for (index, (point, at, micros)) in points.into_iter().enumerate() {
            if let Some((from, start, time)) = previous {
                let speed = if self.settings.speed {
                    match (time, micros) {
                        (Some(a), Some(b)) if b > a => {
                            let v = (at - start) / ((b - a) as f64 / 1e6);
                            1.0 - (v / self.settings.speed_full).clamp(0.0, 1.0)
                                * (1.0 - self.settings.speed_minimum)
                        }
                        _ => 1.0,
                    }
                } else {
                    1.0
                };
                let length = at - start;
                let count = (length / 0.5).ceil().max(1.0) as usize;
                steps = steps.saturating_add(count);
                if steps > 262144 {
                    return Err(
                        "Mouse reconstruction work budget exceeded; pending stroke cancelled"
                            .into(),
                    );
                }
                // Add curve knots as well as arc samples, retaining very short tapers.
                let mut positions: Vec<f64> = (1..=count)
                    .map(|n| start + length * n as f64 / count as f64)
                    .collect();
                if self.settings.enabled && length > 1e-9 {
                    for n in 0..=4 {
                        for knot in [
                            self.settings.start_length * n as f64 / 4.0,
                            distance - self.settings.end_length * n as f64 / 4.0,
                        ] {
                            if knot > start && knot < at {
                                positions.push(knot);
                            }
                        }
                    }
                    positions.sort_by(f64::total_cmp);
                    positions.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
                }
                if self.ends.active() && length > 1e-9 {
                    for n in 0..=4 {
                        for knot in [
                            self.ends.start * n as f64 / 4.0,
                            distance - self.ends.end * n as f64 / 4.0,
                        ] {
                            if knot > start && knot < at {
                                positions.push(knot);
                            }
                        }
                    }
                    positions.sort_by(f64::total_cmp);
                    positions.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
                }
                steps = steps.saturating_add(positions.len().saturating_sub(count));
                if steps > 262144 {
                    return Err(
                        "Mouse reconstruction knot-work budget exceeded; pending stroke cancelled"
                            .into(),
                    );
                }
                for d in positions {
                    if cancel() {
                        return Err("Mouse reconstruction cancelled".into());
                    }
                    let t = if length > 1e-9 {
                        (d - start) / length
                    } else {
                        1.0
                    };
                    let previous_input = self.inputs[index.saturating_sub(1)];
                    let current_input = self.inputs[index];
                    let p = if self.native_pressure {
                        previous_input[0] + (current_input[0] - previous_input[0]) * t
                    } else {
                        self.settings.pressure(d, distance, finalizing, speed)
                    };
                    let angle = previous_input[1]
                        + ((current_input[1] - previous_input[1] + 180.0).rem_euclid(360.0)
                            - 180.0)
                            * t;
                    stroke.set_angle_offset((angle + 180.0).rem_euclid(360.0) - 180.0)?;
                    stroke.set_geometry_scale(self.ends.scale(d, distance, finalizing))?;
                    stroke.sample_pressure(
                        doc,
                        [
                            from[0] + (point[0] - from[0]) * t,
                            from[1] + (point[1] - from[1]) * t,
                        ],
                        p,
                    )?;
                }
            } else {
                stroke.set_angle_offset(self.inputs[index][1])?;
                stroke.set_geometry_scale(self.ends.scale(at, distance, finalizing))?;
                stroke.sample_pressure(
                    doc,
                    point,
                    if self.native_pressure {
                        self.inputs[index][0]
                    } else {
                        self.settings.pressure(at, distance, finalizing, 1.0)
                    },
                )?;
            }
            previous = Some((point, at, micros));
        }
        stroke.finish_sample(doc)?;
        if finalizing {
            stroke.finish_material(doc, &cancel)?;
        }
        if cancel() {
            return Err("Mouse reconstruction cancelled before publication".into());
        }
        stroke.restore_input_trace(trace, self.style.smoothing);
        Ok(stroke)
    }
    fn spray(
        &self,
        doc: &Document,
        stroke: &mut Stroke,
        points: &[(Point, f64, Option<u64>)],
        total: f64,
        finalizing: bool,
        cancel: impl Fn() -> bool,
    ) -> Result<(), String> {
        let Some(&(first, first_distance, mut clock)) = points.first() else {
            return Ok(());
        };
        let mut elapsed = 0u64;
        let period = self.settings.spray_period_micros as u64;
        let mut next = period;
        let mut tick = 0u64;
        let mut emit = |point: Point, distance: f64, speed: f64, tick: u64| -> Result<(), String> {
            if cancel() {
                return Err("Timed mouse spray cancelled".into());
            }
            let count = (tick + 1)
                .checked_mul(self.settings.spray_dabs as u64)
                .ok_or("Spray time limit")?;
            let area = (self.style.shape.size * 0.15).max(0.1).powi(2);
            if count > 262144 || area * count as f64 > 64.0 * 1024.0 * 1024.0 {
                return Err("Timed spray work budget exceeded; pending stroke cancelled".into());
            }
            let pressure = self.settings.pressure(distance, total, finalizing, speed);
            let geometry = self.ends.scale(distance, total, finalizing);
            stroke.set_geometry_scale(geometry)?;
            let scale = if self.style.pressure.size {
                self.style.pressure.response.map(pressure)?
            } else {
                1.0
            };
            let (sin, cos) = self.style.shape.angle.to_radians().sin_cos();
            for n in 0..self.settings.spray_dabs {
                let key = tick * self.settings.spray_dabs as u64 + n as u64;
                let radius = self.style.shape.size
                    * 0.5
                    * scale
                    * geometry
                    * random(self.settings.seed, key * 2).sqrt();
                let angle = std::f64::consts::TAU * random(self.settings.seed, key * 2 + 1);
                let x = radius * angle.cos();
                let y = radius * angle.sin() * self.style.shape.aspect;
                stroke.direct_dab(
                    doc,
                    [point[0] + x * cos - y * sin, point[1] + x * sin + y * cos],
                    point,
                    pressure,
                )?;
            }
            Ok(())
        };
        emit(first, first_distance, 1.0, tick)?;
        for pair in points.windows(2) {
            let (from, d0, _) = pair[0];
            let (to, d1, time) = pair[1];
            let dt = match (clock, time) {
                (Some(a), Some(b)) if b > a => b - a,
                _ => 0,
            };
            if let Some(time) = time {
                clock = Some(clock.map_or(time, |old| old.max(time)));
            }
            let previous = elapsed;
            elapsed = elapsed.checked_add(dt).ok_or("Spray clock limit")?;
            let speed = if self.settings.speed && dt > 0 {
                1.0 - ((d1 - d0) / (dt as f64 / 1e6) / self.settings.speed_full).clamp(0.0, 1.0)
                    * (1.0 - self.settings.speed_minimum)
            } else {
                1.0
            };
            while next <= elapsed {
                tick += 1;
                let t = if dt > 0 {
                    (next - previous) as f64 / dt as f64
                } else {
                    1.0
                };
                emit(
                    [
                        from[0] + (to[0] - from[0]) * t,
                        from[1] + (to[1] - from[1]) * t,
                    ],
                    d0 + (d1 - d0) * t,
                    speed,
                    tick,
                )?;
                next = next.checked_add(period).ok_or("Spray clock limit")?;
            }
        }
        Ok(())
    }
}
fn random(seed: u32, key: u64) -> f64 {
    let mut x = (seed as u64) ^ key.wrapping_mul(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((x ^ (x >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}
impl crate::scene::Scene {
    pub fn append_mouse(&mut self, record: Metadata) -> Result<(), String> {
        if !record.valid() {
            return Err("Invalid frozen mouse stroke metadata".into());
        }
        if let Some(last) = self.mouse_strokes.last_mut()
            && last.layer == record.layer
            && last.settings == record.settings
            && last.style == record.style
            && last.ends == record.ends
            && last.native_pressure == record.native_pressure
        {
            last.count = last
                .count
                .checked_add(1)
                .ok_or("Mouse stroke metadata count limit")?;
        } else {
            if self.mouse_strokes.len() >= 4096 {
                return Err("Mouse stroke settings history is full".into());
            }
            self.mouse_strokes
                .try_reserve(1)
                .map_err(|_| "Mouse metadata allocation failed")?;
            self.mouse_strokes.push(record);
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024 {
            return Err("Frozen stroke settings exceed document metadata budget".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn metadata_compression_keeps_different_geometry_and_pressure_sources_separate() {
        use super::*;
        let mut scene = crate::scene::Scene::default();
        let first = Metadata {
            layer: 0,
            settings: Settings::default(),
            style: style(BrushTip::Pen),
            count: 1,
            ends: Default::default(),
            native_pressure: false,
        };
        scene.append_mouse(first.clone()).unwrap();
        let mut second = first;
        second.ends.start = 9.0;
        scene.append_mouse(second.clone()).unwrap();
        let mut native = second;
        native.native_pressure = true;
        scene.append_mouse(native.clone()).unwrap();
        scene.append_mouse(native).unwrap();
        assert_eq!(scene.mouse_strokes.len(), 3);
        assert_eq!(scene.mouse_strokes[0].ends.start, 0.0);
        assert_eq!(scene.mouse_strokes[1].ends.start, 9.0);
        assert!(!scene.mouse_strokes[1].native_pressure);
        assert!(scene.mouse_strokes[2].native_pressure);
        assert_eq!(scene.mouse_strokes[2].count, 2);
    }
    #[test]
    fn smoothing_finishes_at_real_endpoint_and_preserves_raw_curve_pressure_tilt_and_cancel() {
        use super::*;
        let d = doc();
        let before = d.snapshot();
        let mut st = style(BrushTip::Pen);
        st.smoothing = 0.95;
        st.shape.size = 4.0;
        st.pressure.size = false;
        st.pressure.opacity = true;
        let mut r = Replay::begin(&d, 0, st, Default::default())
            .unwrap()
            .configure_input(Default::default(), true)
            .unwrap();
        for (point, p, angle) in [
            ([5.5, 20.5], 0.5, 170.0),
            ([40.5, 20.5], 0.5, -170.0),
            ([40.5, 20.5], 0.0, -170.0),
        ] {
            r.push_input(
                Sample {
                    point,
                    micros: None,
                },
                p,
                angle,
            )
            .unwrap();
        }
        let preview = r.render(&d, false).unwrap();
        assert_eq!(preview.trace().raw, Some([40.5, 20.5]));
        assert_ne!(preview.trace().raw, preview.trace().corrected);
        assert_eq!(preview.style().smoothing, 0.95);
        let final_stroke = r.render(&d, true).unwrap();
        let mut result = d;
        final_stroke
            .commit_mouse(&mut result, r.metadata())
            .unwrap();
        assert!(result.composite_pixel(30, 20)[3] > 0);
        assert_eq!(result.composite_pixel(45, 20)[3], 0);
        assert_eq!(r.inputs, vec![[0.5, 170.0], [0.5, -170.0], [0.0, -170.0]]);
        let mut legacy = serde_json::to_value(r.metadata()).unwrap();
        legacy.as_object_mut().unwrap().remove("ends");
        legacy.as_object_mut().unwrap().remove("native_pressure");
        let m: Metadata = serde_json::from_value(legacy).unwrap();
        assert!(!m.native_pressure);
        assert_eq!(m.ends, Default::default());
        assert!(result.undo());
        assert!(result.snapshot() == before);
        assert!(r.render_cancel(&result, true, || true).is_err());
        assert!(
            Replay::begin(
                &result,
                0,
                st,
                Settings {
                    enabled: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .configure_input(Default::default(), true)
            .is_err()
        );
    }
    use super::*;
    #[test]
    fn all_materials_reconstruct_from_original_and_wet_finish_is_applied_once() {
        for effect in [Effect::Normal, Effect::Blend, Effect::Smudge, Effect::Wet] {
            let mut d = doc();
            for y in 0..32 {
                for x in 0..64 {
                    d.layers[0].pixels[y * 64 + x] = if x < 24 {
                        [220, 20, 40, 255]
                    } else {
                        [20, 70, 220, 255]
                    };
                }
            }
            let before = d.snapshot();
            let r = log(
                &d,
                Settings {
                    enabled: true,
                    effect,
                    seed: 19,
                    ..Default::default()
                },
                BrushTip::Wash,
            );
            r.render(&d, false).unwrap();
            assert!(d.snapshot() == before);
            let mut final_stroke = r.render(&d, true).unwrap();
            let first = final_stroke.preview_revision;
            if effect == Effect::Wet {
                final_stroke.diffuse(&d, 0.5, || false).unwrap();
                assert_eq!(final_stroke.preview_revision, first);
            }
            final_stroke.commit_mouse(&mut d, r.metadata()).unwrap();
            let after = d.snapshot();
            assert_eq!(d.history_counts(), (1, 0));
            assert!(after != before, "{effect:?}");
            d.undo();
            assert!(d.snapshot() == before);
            d.redo();
            assert!(d.snapshot() == after);
            let mut direct = doc();
            before.restore(&mut direct);
            let r = log(
                &direct,
                Settings {
                    enabled: true,
                    effect,
                    seed: 19,
                    ..Default::default()
                },
                BrushTip::Wash,
            );
            r.render(&direct, true)
                .unwrap()
                .commit_mouse(&mut direct, r.metadata())
                .unwrap();
            assert_eq!(direct.layers[0].pixels, d.layers[0].pixels, "{effect:?}");
        }
    }
    #[test]
    fn fixed_spray_ticks_and_seed_ignore_frame_segmentation_and_bad_clocks_do_not_backlog() {
        let settings = Settings {
            effect: Effect::Spray,
            spray_period_micros: 20000,
            spray_dabs: 3,
            seed: 42,
            ..Default::default()
        };
        let mut result = Vec::new();
        for times in [vec![0, 100000], vec![0, 20000, 40000, 60000, 80000, 100000]] {
            let mut d = doc();
            let mut r = Replay::begin(&d, 0, style(BrushTip::Pen), settings).unwrap();
            for time in times {
                r.push(Sample {
                    point: [32.5, 16.5],
                    micros: Some(time),
                })
                .unwrap();
            }
            let stroke = r.render(&d, true).unwrap();
            assert_eq!(stroke.dab_count(), 18);
            stroke.commit_mouse(&mut d, r.metadata()).unwrap();
            result.push(d.layers[0].pixels.clone());
        }
        assert_eq!(result[0], result[1]);
        let d = doc();
        let mut r = Replay::begin(&d, 0, style(BrushTip::Pen), settings).unwrap();
        for time in [Some(100000), Some(100000), None, Some(0)] {
            r.push(Sample {
                point: [32.5, 16.5],
                micros: time,
            })
            .unwrap();
        }
        assert_eq!(r.render(&d, true).unwrap().dab_count(), 3);
        r.push(Sample {
            point: [32.5, 16.5],
            micros: Some(u64::MAX),
        })
        .unwrap();
        assert!(r.render(&d, true).is_err());
        assert_eq!(d.history_counts(), (0, 0));
    }
    #[test]
    fn cancellation_during_reconstruction_and_diffusion_keeps_original_pixels() {
        use std::cell::Cell;
        let d = doc();
        let before = d.snapshot();
        let r = log(
            &d,
            Settings {
                enabled: true,
                effect: Effect::Wet,
                ..Default::default()
            },
            BrushTip::Pen,
        );
        let calls = Cell::new(0);
        let result = r.render_cancel(&d, true, || {
            calls.set(calls.get() + 1);
            calls.get() > 8
        });
        assert!(result.is_err());
        assert!(calls.get() > 8);
        assert!(d.snapshot() == before);
        let mut stroke = r.render(&d, false).unwrap();
        assert!(stroke.diffuse(&d, 0.5, || true).is_err());
        assert!(d.snapshot() == before);
    }
    use crate::{
        assets::BrushTip,
        brush_cursor::Shape,
        document::{Background, CanvasSpec},
        pressure::Mapping,
    };
    fn doc() -> Document {
        Document::create(
            CanvasSpec {
                width: 64,
                height: 32,
                dpi: 300.0,
                background: Background::Transparent,
            },
            8192,
        )
        .unwrap()
    }
    fn style(tip: BrushTip) -> Style {
        Style {
            texture: Default::default(),
            material: Default::default(),
            opacity: 1.0,
            hardness: 1.0,
            spacing: 0.15,
            shape: Shape {
                size: 8.0,
                ..Default::default()
            },
            tip,
            color: [30, 80, 160, 255],
            smoothing: 0.0,
            pressure: Mapping {
                opacity: true,
                ..Default::default()
            },
        }
    }
    fn log(doc: &Document, settings: Settings, tip: BrushTip) -> Replay {
        let mut replay = Replay::begin(doc, 0, style(tip), settings).unwrap();
        for (point, time) in [
            ([8.5, 16.5], 0),
            ([24.5, 16.5], 10000),
            ([52.5, 16.5], 20000),
        ] {
            replay
                .push(Sample {
                    point,
                    micros: Some(time),
                })
                .unwrap();
        }
        replay
    }
    #[test]
    fn final_taper_reconstructs_original_instead_of_overpainting_preview_and_undoes_once() {
        let mut d = doc();
        let before = d.snapshot();
        let settings = Settings {
            enabled: true,
            ..Default::default()
        };
        let replay = log(&d, settings, BrushTip::Wash);
        let preview = replay.render(&d, false).unwrap();
        let preview_end = preview.pixel(&d, 0, &d.layers[0], 52, 16).unwrap_or([0; 4]);
        let final_stroke = replay.render(&d, true).unwrap();
        let final_end = final_stroke
            .pixel(&d, 0, &d.layers[0], 52, 16)
            .unwrap_or([0; 4]);
        assert!(preview_end[3] > final_end[3]);
        assert!(d.snapshot() == before);
        final_stroke
            .commit_mouse(&mut d, replay.metadata())
            .unwrap();
        let after = d.snapshot();
        assert_eq!(d.history_counts(), (1, 0));
        assert_eq!(d.scene.mouse_strokes[0].settings, settings);
        d.undo();
        assert!(d.snapshot() == before);
        d.redo();
        assert!(d.snapshot() == after);
        let mut direct = doc();
        let direct_log = log(&direct, settings, BrushTip::Wash);
        direct_log
            .render(&direct, true)
            .unwrap()
            .commit_mouse(&mut direct, direct_log.metadata())
            .unwrap();
        assert_eq!(direct.layers[0].pixels, d.layers[0].pixels);
    }
    #[test]
    fn point_short_overlap_disabled_and_curve_knots_have_explicit_pressure_rules() {
        let settings = Settings {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(settings.pressure(0.0, 0.0, true, 1.0), 1.0);
        assert!((settings.pressure(1.5, 3.0, true, 1.0) - 0.015625).abs() < 1e-12);
        assert_eq!(settings.pressure(44.0, 44.0, false, 1.0), 1.0);
        assert_eq!(settings.pressure(44.0, 44.0, true, 1.0), 0.0);
        assert_eq!(Settings::default().pressure(0.0, 44.0, true, 1.0), 1.0);
        let mut d = doc();
        let mut dot = Replay::begin(&d, 0, style(BrushTip::Pen), settings).unwrap();
        dot.push(Sample {
            point: [12.5, 12.5],
            micros: Some(10),
        })
        .unwrap();
        dot.push(Sample {
            point: [12.5, 12.5],
            micros: Some(10),
        })
        .unwrap();
        dot.render(&d, true)
            .unwrap()
            .commit_mouse(&mut d, dot.metadata())
            .unwrap();
        assert!(d.composite_pixel(12, 12)[3] > 0);
        assert_eq!(d.history_counts(), (1, 0));
    }
    #[test]
    fn speed_factor_is_separate_and_missing_repeated_reverse_and_wrapped_clocks_are_safe() {
        let settings = Settings {
            speed: true,
            ..Default::default()
        };
        let mut alpha = Vec::new();
        for times in [
            [Some(0), Some(1000)],
            [Some(0), Some(1000000)],
            [None, None],
            [Some(10), Some(10)],
            [Some(10), Some(0)],
            [Some(u64::MAX), Some(0)],
        ] {
            let mut d = doc();
            let mut r = Replay::begin(&d, 0, style(BrushTip::Pen), settings).unwrap();
            for (point, micros) in [[8.5, 16.5], [52.5, 16.5]].into_iter().zip(times) {
                r.push(Sample { point, micros }).unwrap();
            }
            r.render(&d, true)
                .unwrap()
                .commit_mouse(&mut d, r.metadata())
                .unwrap();
            alpha.push(d.composite_pixel(32, 16)[3]);
        }
        assert!(alpha[0] < alpha[1]);
        assert!(alpha[1] <= alpha[2]);
        assert!(alpha[2..].iter().all(|v| *v == 255), "{alpha:?}");
    }
    #[test]
    fn seeded_replays_are_deterministic_frozen_and_log_preserves_order_and_times() {
        let d = doc();
        let mut settings = Settings {
            enabled: true,
            seed: 123,
            ..Default::default()
        };
        let replay = log(&d, settings, BrushTip::Pencil);
        settings.seed = 124;
        let another = log(&d, settings, BrushTip::Pencil);
        assert_eq!(replay.metadata().settings.seed, 123);
        assert_eq!(replay.samples()[1].micros, Some(10000));
        let pixels = |r: &Replay| {
            let stroke = r.render(&d, true).unwrap();
            (0..32)
                .flat_map(|y| (0..64).map(move |x| (x, y)))
                .map(|(x, y)| stroke.pixel(&d, 0, &d.layers[0], x, y).unwrap_or([0; 4]))
                .collect::<Vec<_>>()
        };
        assert_eq!(pixels(&replay), pixels(&replay));
        assert_ne!(pixels(&replay), pixels(&another));
        let json = serde_json::to_string(&replay.metadata()).unwrap();
        let read: Metadata = serde_json::from_str(&json).unwrap();
        assert_eq!(read, replay.metadata());
    }
    #[test]
    fn stale_original_invalid_settings_and_record_path_limits_never_commit_partial_pixels() {
        let mut d = doc();
        let settings = Settings {
            enabled: true,
            ..Default::default()
        };
        let mut replay = log(&d, settings, BrushTip::Pen);
        let before = d.snapshot();
        assert!(
            replay
                .push(Sample {
                    point: [f64::NAN, 0.0],
                    micros: None
                })
                .is_err()
        );
        assert!(d.snapshot() == before);
        replay
            .push(Sample {
                point: [1e12, 0.0],
                micros: Some(30000),
            })
            .unwrap();
        assert!(replay.render(&d, true).is_err());
        assert!(d.snapshot() == before);
        let replay = log(&d, settings, BrushTip::Pen);
        d.edit_pixel(0, 0, 0, [255; 4]);
        let external = d.snapshot();
        assert!(replay.render(&d, true).is_err());
        assert!(d.snapshot() == external);
        let mut invalid = settings;
        invalid.start_curve[2] = f64::NAN;
        assert!(Replay::begin(&d, 0, style(BrushTip::Pen), invalid).is_err());
        let mut replay = Replay::begin(&d, 0, style(BrushTip::Pen), settings).unwrap();
        for _ in 0..4096 {
            replay
                .push(Sample {
                    point: [0.0; 2],
                    micros: None,
                })
                .unwrap();
        }
        assert!(
            replay
                .push(Sample {
                    point: [0.0; 2],
                    micros: None
                })
                .is_err()
        );
        assert!(d.snapshot() == external);
    }
    #[test]
    fn metadata_capacity_failure_preserves_pixels_and_compressed_records_follow_undo_redo() {
        let mut d = doc();
        let settings = Settings {
            enabled: true,
            ..Default::default()
        };
        for _ in 0..2 {
            let r = log(&d, settings, BrushTip::Wash);
            r.render(&d, true)
                .unwrap()
                .commit_mouse(&mut d, r.metadata())
                .unwrap();
        }
        assert_eq!(d.scene.mouse_strokes.len(), 1);
        assert_eq!(d.scene.mouse_strokes[0].count, 2);
        assert_eq!(d.history_counts(), (2, 0));
        d.undo();
        assert_eq!(d.scene.mouse_strokes[0].count, 1);
        d.redo();
        assert_eq!(d.scene.mouse_strokes[0].count, 2);
        let template = d.scene.mouse_strokes[0].clone();
        d.scene.mouse_strokes = (0..4096)
            .map(|seed| Metadata {
                settings: Settings { seed, ..settings },
                ..template.clone()
            })
            .collect();
        let before = d.snapshot();
        let r = log(
            &d,
            Settings {
                seed: 9000,
                ..settings
            },
            BrushTip::Wash,
        );
        assert!(
            r.render(&d, true)
                .unwrap()
                .commit_mouse(&mut d, r.metadata())
                .is_err()
        );
        assert!(d.snapshot() == before);
    }
}
