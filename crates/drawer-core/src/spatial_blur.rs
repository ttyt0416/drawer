//! Cancellable native-RGB trajectory/aperture blurs, baked from an immutable raster.
use crate::{
    color_precision::{Depth, Sample},
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    scene::{Point, Rect},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Motion,
    Radial,
    Spin,
    Lens,
}
impl Mode {
    pub const ALL: [Self; 4] = [Self::Motion, Self::Radial, Self::Spin, Self::Lens];
    pub fn index(self) -> usize {
        match self {
            Self::Motion => 0,
            Self::Radial => 1,
            Self::Spin => 2,
            Self::Lens => 3,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Motion => "Motion blur",
            Self::Radial => "Radial blur",
            Self::Spin => "Spin blur",
            Self::Lens => "Lens blur",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    #[default]
    Both,
    Forward,
    Backward,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kernel {
    #[default]
    Box,
    Smooth,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    pub length: f64,
    pub radius: f64,
    pub angle: f64,
    pub center: Point,
    pub sides: u8,
    pub roundness: f64,
    pub highlight: f64,
    pub amount: f64,
    pub selection: bool,
    pub boundary: crate::filter_options::Boundary,
    pub direction: Direction,
    pub kernel: Kernel,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Motion,
            length: 8.,
            radius: 8.,
            angle: 0.,
            center: [0.; 2],
            sides: 6,
            roundness: 0.,
            highlight: 0.,
            amount: 1.,
            selection: false,
            boundary: Default::default(),
            direction: Default::default(),
            kernel: Default::default(),
        }
    }
}
impl Settings {
    pub fn options_default(self) -> bool {
        self.boundary == Default::default()
            && self.direction == Default::default()
            && self.kernel == Default::default()
    }
    pub fn valid(self) -> bool {
        self.length.is_finite()
            && (0. ..=128.).contains(&self.length)
            && self.radius.is_finite()
            && (0. ..=64.).contains(&self.radius)
            && self.angle.is_finite()
            && self.angle.abs()
                <= if self.mode == Mode::Spin {
                    360.
                } else {
                    36000.
                }
            && self.center.iter().all(|v| v.is_finite() && v.abs() <= 1e6)
            && (3..=8).contains(&self.sides)
            && [self.roundness, self.amount]
                .iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            && self.highlight.is_finite()
            && (0. ..=4.).contains(&self.highlight)
    }
    pub fn identity(self) -> bool {
        self.amount == 0.
            || match self.mode {
                Mode::Motion => self.length == 0.,
                Mode::Radial | Mode::Lens => self.radius == 0.,
                Mode::Spin => self.angle == 0.,
            }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Presets {
    pub values: [Option<Settings>; 4],
}
impl Presets {
    pub fn valid(&self) -> bool {
        self.values
            .iter()
            .enumerate()
            .all(|(n, v)| v.is_none_or(|s| s.valid() && s.mode.index() == n))
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn get(&self, mode: Mode, size: [u32; 2]) -> Settings {
        self.values[mode.index()]
            .filter(|s| s.valid() && s.mode == mode)
            .unwrap_or(Settings {
                mode,
                center: size.map(|v| v as f64 / 2.),
                angle: if mode == Mode::Spin { 15. } else { 0. },
                ..Default::default()
            })
    }
}
fn rect(e: PlacedRaster) -> Rect {
    Rect {
        min: e.offset.map(|v| v as f64),
        max: [
            (e.offset[0] + e.width as i64) as f64,
            (e.offset[1] + e.height as i64) as f64,
        ],
    }
}
fn bounds(points: impl IntoIterator<Item = Point>) -> Rect {
    let mut b = Rect {
        min: [f64::INFINITY; 2],
        max: [f64::NEG_INFINITY; 2],
    };
    for p in points {
        for (k, v) in p.into_iter().enumerate() {
            b.min[k] = b.min[k].min(v);
            b.max[k] = b.max[k].max(v);
        }
    }
    b
}
fn rotate(p: Point, center: Point, angle: f64) -> Point {
    let (s, c) = crate::texture::rotation(angle);
    let v = [p[0] - center[0], p[1] - center[1]];
    [
        center[0] + v[0] * c - v[1] * s,
        center[1] + v[0] * s + v[1] * c,
    ]
}
fn output_extent(old: PlacedRaster, s: Settings, locked: bool) -> Result<PlacedRaster, String> {
    if locked {
        return Ok(old);
    }
    let mut r = rect(old);
    match s.mode {
        Mode::Motion => {
            let (sin, cos) = crate::texture::rotation(s.angle);
            for (k, v) in [cos, sin].into_iter().enumerate() {
                let pad = (v
                    * s.length
                    * if s.direction == Direction::Both {
                        0.5
                    } else {
                        1.
                    })
                .abs()
                .ceil()
                    + 1.;
                r.min[k] -= pad;
                r.max[k] += pad;
            }
        }
        Mode::Radial | Mode::Lens => {
            let pad = (s.radius
                / if s.mode == Mode::Radial && s.direction == Direction::Both {
                    2.
                } else {
                    1.
                })
            .ceil()
                + 1.;
            for k in 0..2 {
                r.min[k] -= pad;
                r.max[k] += pad;
            }
        }
        Mode::Spin => {
            for k in 0..2 {
                r.min[k] -= 1.;
                r.max[k] += 1.;
            }
            let half = s.angle.abs()
                * if s.direction == Direction::Both {
                    0.5
                } else {
                    1.
                };
            let mut points = Vec::with_capacity(24);
            for p in crate::deform::corners(r) {
                points.extend([-half, 0., half].map(|angle| rotate(p, s.center, angle)));
                let base = (p[1] - s.center[1]).atan2(p[0] - s.center[0]).to_degrees();
                for n in -6..=6 {
                    let angle = n as f64 * 90. - base;
                    if angle >= -half && angle <= half {
                        points.push(rotate(p, s.center, angle));
                    }
                }
            }
            r = bounds(points);
        }
    }
    let lo = r.min.map(f64::floor);
    let hi = r.max.map(f64::ceil);
    if lo
        .iter()
        .chain(&hi)
        .any(|v| !v.is_finite() || v.abs() > 1e6)
        || (0..2).any(|k| !(1. ..=32768.).contains(&(hi[k] - lo[k])))
        || (hi[0] - lo[0]) * (hi[1] - lo[1]) > 4. * 1024. * 1024.
    {
        return Err("Blur extent exceeds4M pixels,32768 per axis or coordinate limit".into());
    }
    Ok(PlacedRaster {
        offset: lo.map(|v| v as i64),
        width: (hi[0] - lo[0]) as u32,
        height: (hi[1] - lo[1]) as u32,
    })
}
/// Exact normalized polygon-to-circle radial interpolation. Rotation is periodic by360/sides.
pub fn aperture_contains(s: Settings, p: Point) -> bool {
    let distance = p[0].hypot(p[1]);
    if distance == 0. {
        return true;
    }
    if s.radius == 0. {
        return false;
    }
    let period = 360. / s.sides as f64;
    let angle = if s.roundness == 1. {
        0.
    } else {
        s.angle.rem_euclid(period)
    };
    let half = std::f64::consts::PI / s.sides as f64;
    let sector = (p[1].atan2(p[0]) - angle.to_radians()).rem_euclid(2. * half) - half;
    let polygon = half.cos() / sector.cos();
    distance <= s.radius * (polygon * (1. - s.roundness) + s.roundness) + 1e-12
}
fn kernel(
    s: Settings,
    extent: PlacedRaster,
    tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Vec<(Point, f64)>, String> {
    if s.mode == Mode::Lens {
        let r = s.radius.ceil() as i32;
        let mut values = Vec::new();
        let mut total = 0.;
        for y in -r..=r {
            tick(0, 1)?;
            for x in -r..=r {
                let mut count = 0.;
                for sy in 0..4 {
                    for sx in 0..4 {
                        if aperture_contains(
                            s,
                            [
                                x as f64 + (sx as f64 + 0.5) / 4. - 0.5,
                                y as f64 + (sy as f64 + 0.5) / 4. - 0.5,
                            ],
                        ) {
                            count += 1.;
                        }
                    }
                }
                if count > 0. {
                    values.push(([x as f64, y as f64], count));
                    total += count;
                }
            }
        }
        if values.is_empty() {
            return Ok(vec![([0.; 2], 1.)]);
        }
        for (_, w) in &mut values {
            *w /= total;
        }
        return Ok(values);
    }
    let travel = match s.mode {
        Mode::Motion => s.length,
        Mode::Radial => s.radius,
        Mode::Spin => {
            crate::deform::corners(rect(extent))
                .iter()
                .map(|p| (p[0] - s.center[0]).hypot(p[1] - s.center[1]))
                .fold(0., f64::max)
                * s.angle.abs().to_radians()
        }
        Mode::Lens => unreachable!(),
    };
    let intervals = (travel * 2.).ceil().clamp(2., 256.) as usize;
    let mut values: Vec<_> = (0..=intervals)
        .map(|n| {
            let u = n as f64 / intervals as f64;
            let t = match s.direction {
                Direction::Both => u - 0.5,
                Direction::Forward => -u,
                Direction::Backward => u,
            };
            let endpoint = if n == 0 || n == intervals { 0.5 } else { 1. };
            let shape = if s.kernel == Kernel::Smooth {
                1. - (u * 2. - 1.).abs()
            } else {
                1.
            };
            ([t, 0.], endpoint * shape)
        })
        .collect();
    let total: f64 = values.iter().map(|(_, w)| w).sum();
    if total == 0. {
        return Ok(vec![([0.; 2], 1.)]);
    }
    for (_, w) in &mut values {
        *w /= total;
    }
    Ok(values)
}
pub fn prepare(
    d: &Document,
    layer: usize,
    s: Settings,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<DocumentState, String> {
    tick(0, 1)?;
    crate::layer_properties::paintable(d, layer)?;
    if !s.valid() || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Invalid blur settings or snapshot limit".into());
    }
    let source = &d.layers[layer];
    if s.identity()
        || s.selection
            && d.scene
                .selection_mask
                .as_ref()
                .is_some_and(|m| m.runs.is_empty())
    {
        return Ok(d.snapshot());
    }
    let mut visible = false;
    for i in 0..source.pixels.len() {
        if i.is_multiple_of(128) {
            tick(0, 1)?;
        }
        if source.sample_at(i).alpha() > 0. {
            visible = true;
            break;
        }
    }
    if !visible {
        return Ok(d.snapshot());
    }
    let locked = crate::clipping::alpha_locked(d, layer);
    let extent = output_extent(source.extent(d.width, d.height), s, locked)?;
    let taps = kernel(s, extent, &mut tick)?;
    let count = extent.width as usize * extent.height as usize;
    if count.saturating_mul(taps.len()) > 64 * 1024 * 1024
        || d.resident_bytes()
            .saturating_add(d.memory_bytes().saturating_mul(3))
            .saturating_add(count.saturating_mul(d.color.depth.bytes() * 3))
            > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Blur processing64M samples or workspace memory limit; reduce radius/angle or source size".into());
    }
    let mut pixels = Vec::with_capacity(count);
    let mut precise = (d.color.depth == Depth::Sixteen).then(|| Vec::with_capacity(count));
    let mut changed = false;
    let (dir_s, dir_c) = crate::texture::rotation(s.angle);
    for y in 0..extent.height {
        for x in 0..extent.width {
            let done = y as usize * extent.width as usize + x as usize;
            if done.is_multiple_of(128) {
                tick(done, count)?;
            }
            let pos = [extent.offset[0] + x as i64, extent.offset[1] + y as i64];
            let point = pos.map(|v| v as f64 + 0.5);
            let before = source.sample(d.width, d.height, pos[0], pos[1]);
            let old = source.pixel_index(d.width, d.height, pos[0], pos[1]);
            let w = s.amount
                * if s.selection {
                    d.scene.selection_coverage(point) as f64 / 255.
                } else {
                    1.
                };
            let mut sum = [0.; 4];
            if w > 0. && (!locked || before.alpha() > 0.) {
                let v = [point[0] - s.center[0], point[1] - s.center[1]];
                let distance = v[0].hypot(v[1]);
                for (n, (offset, weight)) in taps.iter().enumerate() {
                    if n.is_multiple_of(128) {
                        tick(done, count)?;
                    }
                    let q = match s.mode {
                        Mode::Motion => [
                            point[0] + offset[0] * s.length * dir_c,
                            point[1] + offset[0] * s.length * dir_s,
                        ],
                        Mode::Radial => {
                            if distance == 0. {
                                point
                            } else {
                                [
                                    point[0] + v[0] / distance * offset[0] * s.radius,
                                    point[1] + v[1] / distance * offset[0] * s.radius,
                                ]
                            }
                        }
                        Mode::Spin => rotate(point, s.center, offset[0] * s.angle.abs()),
                        Mode::Lens => [point[0] - offset[0], point[1] - offset[1]],
                    };
                    let sample = crate::filter_options::Sampler {
                        document: d,
                        layer,
                        boundary: s.boundary,
                    }
                    .bilinear(q.map(|v| v - 0.5));
                    let a = sample.alpha() * weight;
                    let rgb = sample.rgb();
                    let gain = if s.mode == Mode::Lens {
                        1. + s.highlight
                            * ((rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722 - 0.5) * 2.)
                                .max(0.)
                    } else {
                        1.
                    };
                    for k in 0..3 {
                        sum[k] += rgb[k] * a * gain;
                    }
                    sum[3] += a;
                }
            }
            if w == 0. || locked && before.alpha() == 0. || sum[3] == 0. && before.alpha() == 0. {
                pixels.push(old.map_or([0; 4], |i| source.pixels[i]));
                if let Some(high) = &mut precise {
                    high.push(before.rgba16());
                }
                continue;
            }
            let blurred = if sum[3] > 0. {
                Sample::new([
                    (sum[0] / sum[3]).clamp(0., 1.),
                    (sum[1] / sum[3]).clamp(0., 1.),
                    (sum[2] / sum[3]).clamp(0., 1.),
                    sum[3].clamp(0., 1.),
                ])?
            } else {
                Sample::default()
            };
            let blurred = if locked {
                before.with_rgb(blurred.rgb())?
            } else {
                blurred
            };
            let result = if w == 1. {
                blurred
            } else {
                before.mix(blurred, w)?
            };
            let mut value = result.store(d.color.depth, d.color.dither, pos);
            if locked {
                value[3] = if d.color.depth == Depth::Sixteen {
                    before.rgba16()[3]
                } else {
                    old.map_or(0, |i| source.pixels[i][3] as u16)
                };
            }
            let low = if let Some(high) = &mut precise {
                high.push(value);
                Sample::from16(value).rgba8()
            } else {
                value.map(|v| v as u8)
            };
            changed |= if d.color.depth == Depth::Sixteen {
                value != before.rgba16()
            } else {
                low != old.map_or([0; 4], |i| source.pixels[i])
            };
            pixels.push(low);
        }
    }
    tick(count, count)?;
    if !changed {
        return Ok(d.snapshot());
    }
    let mut state = d.snapshot();
    state.layers[layer].pixels = pixels;
    state.layers[layer].precise = precise;
    state.layers[layer].placement = if locked {
        source.placement
    } else {
        Some(extent)
    };
    state.scene.last_spatial_blur = Some(s);
    crate::layer_groups::finish(d, &state)?;
    tick(count, count)?;
    Ok(state)
}
