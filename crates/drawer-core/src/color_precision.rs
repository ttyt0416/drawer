//! Unassociated floating RGB samples; premultiplied arithmetic until final storage.
use crate::layer_blend::Mode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Depth {
    Eight,
    #[default]
    Sixteen,
}
impl Depth {
    pub fn maximum(self) -> u32 {
        match self {
            Self::Eight => 255,
            Self::Sixteen => 65535,
        }
    }
    pub fn bytes(self) -> usize {
        match self {
            Self::Eight => 4,
            Self::Sixteen => 8,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dither {
    pub enabled: bool,
    pub seed: u32,
}
impl Dither {
    fn noise(self, [x, y]: [i64; 2], channel: usize) -> f64 {
        if !self.enabled {
            return 0.;
        }
        let mut value = (x as u64).wrapping_mul(0x9e3779b185ebca87)
            ^ (y as u64).wrapping_mul(0xc2b2ae3d27d4eb4f)
            ^ (self.seed as u64).wrapping_mul(0x165667b19e3779f9)
            ^ (channel as u64).wrapping_mul(0x94d049bb133111eb);
        value ^= value >> 29;
        value = value.wrapping_mul(0x94d049bb133111eb);
        value ^= value >> 31;
        // Stay strictly inside the half-step endpoints, including the zero hash.
        ((value >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * (1. - 2e-9)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample([f64; 4]);
impl Sample {
    pub fn new(mut value: [f64; 4]) -> Result<Self, String> {
        if value.iter().any(|v| !v.is_finite()) {
            return Err("Nonfinite color sample".into());
        }
        value[3] = value[3].clamp(0., 1.);
        if value[3] == 0. {
            value = [0.; 4];
        }
        Ok(Self(value))
    }
    pub fn from8(value: [u8; 4]) -> Self {
        Self::new(value.map(|v| v as f64 / 255.)).unwrap()
    }
    pub fn from16(value: [u16; 4]) -> Self {
        Self::new(value.map(|v| v as f64 / 65535.)).unwrap()
    }
    pub fn components(self) -> [f64; 4] {
        self.0
    }
    pub fn alpha(self) -> f64 {
        self.0[3]
    }
    pub fn rgb(self) -> [f64; 3] {
        [self.0[0], self.0[1], self.0[2]]
    }
    pub fn clamped(self) -> Self {
        Self([
            self.0[0].clamp(0., 1.),
            self.0[1].clamp(0., 1.),
            self.0[2].clamp(0., 1.),
            self.alpha(),
        ])
    }
    pub fn with_rgb(self, rgb: [f64; 3]) -> Result<Self, String> {
        Self::new([rgb[0], rgb[1], rgb[2], self.alpha()])
    }
    pub fn coverage(self, weight: f64) -> Result<Self, String> {
        if !weight.is_finite() {
            return Err("Nonfinite color coverage".into());
        }
        let mut v = self.0;
        v[3] *= weight.clamp(0., 1.);
        Self::new(v)
    }
    pub fn store(self, depth: Depth, dither: Dither, point: [i64; 2]) -> [u16; 4] {
        let maximum = depth.maximum() as f64;
        let alpha = (self.alpha() * maximum + 0.5).floor().clamp(0., maximum) as u16;
        if alpha == 0 {
            return [0; 4];
        }
        let mut out = [0, 0, 0, alpha];
        for (i, v) in out[..3].iter_mut().enumerate() {
            *v = (self.0[i].clamp(0., 1.) * maximum + dither.noise(point, i) + 0.5)
                .floor()
                .clamp(0., maximum) as u16;
        }
        out
    }
    pub fn rgba8(self) -> [u8; 4] {
        self.store(Depth::Eight, Default::default(), [0; 2])
            .map(|v| v as u8)
    }
    pub fn rgba16(self) -> [u16; 4] {
        self.store(Depth::Sixteen, Default::default(), [0; 2])
    }
    /// The premultiplied source-over formula, using the existing encoded sRGB blend functions.
    pub fn over(self, source: Self, mode: Mode, opacity: f64) -> Result<Self, String> {
        if !opacity.is_finite() {
            return Err("Nonfinite layer opacity".into());
        }
        let ab = self.alpha();
        let a = source.alpha() * opacity.clamp(0., 1.);
        let alpha = a + ab * (1. - a);
        if alpha == 0. {
            return Ok(Self::default());
        }
        let blended = if mode.is_normal() {
            source.rgb()
        } else {
            mode.rgb(self.rgb(), source.rgb())
        };
        let rgb = [0, 1, 2].map(|i| {
            ((1. - a) * self.0[i] * ab + (1. - ab) * source.0[i] * a + a * ab * blended[i]) / alpha
        });
        Self::new([rgb[0], rgb[1], rgb[2], alpha])
    }
    pub fn atop(self, source: Self, mode: Mode, opacity: f64) -> Result<Self, String> {
        if !opacity.is_finite() {
            return Err("Nonfinite clipping opacity".into());
        }
        if self.alpha() == 0. {
            return Ok(Self::default());
        }
        let a = source.alpha() * opacity.clamp(0., 1.);
        let blend = if mode.is_normal() {
            source.rgb()
        } else {
            mode.rgb(self.rgb(), source.rgb())
        };
        let rgb = [0, 1, 2].map(|i| self.0[i] * (1. - a) + blend[i] * a);
        Self::new([rgb[0], rgb[1], rgb[2], self.alpha()])
    }
    pub fn brush(
        self,
        source: Self,
        mode: crate::brush_composite::Mode,
        amount: f64,
        point: [i64; 2],
        seed: u32,
    ) -> Result<Self, String> {
        use crate::brush_composite::Mode as Brush;
        if !amount.is_finite() {
            return Err("Nonfinite brush coverage".into());
        }
        let amount = amount.clamp(0., 1.);
        let a = source.alpha() * amount;
        match mode {
            Brush::Normal => self.over(source, Mode::Normal, amount),
            Brush::Erase => self.coverage(1. - a),
            Brush::Background => source.coverage(amount)?.over(self, Mode::Normal, 1.),
            Brush::ReplaceAlpha => Self::new([
                self.0[0],
                self.0[1],
                self.0[2],
                self.alpha() * (1. - amount) + source.alpha() * amount,
            ]),
            Brush::CompareDensity => {
                if a > self.alpha() {
                    source.coverage(amount)
                } else {
                    Ok(self)
                }
            }
            Brush::EraseCompare => {
                if a >= self.alpha() && a > 0. {
                    Ok(Self::default())
                } else {
                    Ok(self)
                }
            }
            Brush::Dissolve => {
                if crate::brush_composite::random(point, seed) < a {
                    Self::new([source.0[0], source.0[1], source.0[2], 1.])
                } else {
                    Ok(self)
                }
            }
            Brush::BlackBurn | Brush::WhiteBurn => {
                let source = if self.alpha() > 0. {
                    source.with_rgb(if mode == Brush::BlackBurn {
                        Mode::ColorBurn.rgb(self.rgb(), source.rgb())
                    } else {
                        Mode::ColorDodge.rgb(self.rgb(), source.rgb())
                    })?
                } else {
                    source
                };
                self.over(source, Mode::Normal, amount)
            }
        }
    }
    /// Coverage interpolation is performed on premultiplied color, not straight RGB.
    pub fn mix(self, other: Self, weight: f64) -> Result<Self, String> {
        if !weight.is_finite() {
            return Err("Nonfinite interpolation weight".into());
        }
        let w = weight.clamp(0., 1.);
        let a = self.alpha() * (1. - w) + other.alpha() * w;
        if a == 0. {
            return Ok(Self::default());
        }
        let rgb = [0, 1, 2]
            .map(|i| (self.0[i] * self.alpha() * (1. - w) + other.0[i] * other.alpha() * w) / a);
        Self::new([rgb[0], rgb[1], rgb[2], a])
    }
    pub fn luminance(self) -> f64 {
        let linear = self.rgb().map(|v| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        });
        let y = linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
        if self.0[0] == self.0[1] && self.0[1] == self.0[2] {
            return self.0[0];
        }
        if y <= 0.0031308 {
            12.92 * y
        } else {
            1.055 * y.powf(1. / 2.4) - 0.055
        }
    }
    pub fn expression(
        self,
        mode: crate::layer_search::Expression,
        brightness: f64,
        alpha: f64,
    ) -> Result<Self, String> {
        if ![brightness, alpha]
            .iter()
            .all(|v| v.is_finite() && (0. ..=1.).contains(v))
        {
            return Err("Invalid expression threshold".into());
        }
        use crate::layer_search::Expression;
        match mode {
            Expression::Color => Ok(self),
            Expression::Gray => self.with_rgb([self.luminance(); 3]),
            Expression::Mono => {
                if self.alpha() == 0. || self.alpha() < alpha {
                    return Ok(Self::default());
                }
                let v = if self.luminance() >= brightness {
                    1.
                } else {
                    0.
                };
                Self::new([v, v, v, 1.])
            }
        }
    }
    pub fn adjusted(self, value: crate::layer_effects::Adjustment) -> Result<Self, String> {
        if !value.valid() {
            return Err("Invalid color adjustment".into());
        }
        if value.identity() || self.alpha() == 0. {
            return Ok(self);
        }
        let contrast = if value.contrast > 0. {
            1. / (1. - value.contrast * 0.99)
        } else {
            1. + value.contrast
        };
        let mut rgb = self
            .rgb()
            .map(|v| ((v - 0.5) * contrast + 0.5 + value.brightness).clamp(0., 1.));
        if value.hue != 0. || value.saturation != 0. {
            let max = rgb[0].max(rgb[1]).max(rgb[2]);
            let min = rgb[0].min(rgb[1]).min(rgb[2]);
            let delta = max - min;
            let mut h = if delta == 0. {
                0.
            } else if max == rgb[0] {
                ((rgb[1] - rgb[2]) / delta).rem_euclid(6.)
            } else if max == rgb[1] {
                (rgb[2] - rgb[0]) / delta + 2.
            } else {
                (rgb[0] - rgb[1]) / delta + 4.
            };
            h = (h + value.hue / 60.).rem_euclid(6.);
            let (c, m) = match value.color_model {
                crate::layer_effects::ColorModel::Hsv => {
                    let s = if max == 0. { 0. } else { delta / max };
                    let c = max * (s * (1. + value.saturation)).clamp(0., 1.);
                    (c, max - c)
                }
                crate::layer_effects::ColorModel::Hsl => {
                    let l = (max + min) / 2.;
                    let scale = 1. - (2. * l - 1.).abs();
                    let s = if delta == 0. || scale == 0. {
                        0.
                    } else {
                        delta / scale
                    };
                    let c = scale * (s * (1. + value.saturation)).clamp(0., 1.);
                    (c, l - c / 2.)
                }
            };
            let x = c * (1. - (h.rem_euclid(2.) - 1.).abs());
            rgb = match h as u32 {
                0 => [c, x, 0.],
                1 => [x, c, 0.],
                2 => [0., c, x],
                3 => [0., x, c],
                4 => [x, 0., c],
                _ => [c, 0., x],
            }
            .map(|v| v + m);
        }
        if value.invert {
            rgb = rgb.map(|v| 1. - v);
        }
        self.with_rgb(rgb)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pixels {
    Eight(Vec<[u8; 4]>),
    Sixteen(Vec<[u16; 4]>),
}
impl Pixels {
    pub fn depth(&self) -> Depth {
        match self {
            Self::Eight(_) => Depth::Eight,
            Self::Sixteen(_) => Depth::Sixteen,
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Eight(v) => v.len(),
            Self::Sixteen(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn memory_bytes(&self) -> usize {
        self.len().saturating_mul(self.depth().bytes())
    }
    pub fn sample(&self, n: usize) -> Option<Sample> {
        match self {
            Self::Eight(v) => v.get(n).copied().map(Sample::from8),
            Self::Sixteen(v) => v.get(n).copied().map(Sample::from16),
        }
    }
    pub fn from_samples(
        values: &[Sample],
        depth: Depth,
        dither: Dither,
        width: usize,
    ) -> Result<Self, String> {
        if width == 0 || values.len() > 4 * 1024 * 1024 {
            return Err("Color pixel buffer exceeds limits".into());
        }
        let stored =
            |n: usize, p: &Sample| p.store(depth, dither, [(n % width) as i64, (n / width) as i64]);
        Ok(match depth {
            Depth::Eight => Self::Eight(
                values
                    .iter()
                    .enumerate()
                    .map(|(n, p)| stored(n, p).map(|v| v as u8))
                    .collect(),
            ),
            Depth::Sixteen => Self::Sixteen(
                values
                    .iter()
                    .enumerate()
                    .map(|(n, p)| stored(n, p))
                    .collect(),
            ),
        })
    }
    pub fn convert(&self, depth: Depth, dither: Dither, width: usize) -> Result<Self, String> {
        if width == 0 || self.len() > 4 * 1024 * 1024 {
            return Err("Color conversion buffer exceeds limits".into());
        }
        if depth == self.depth() {
            return Ok(self.clone());
        }
        let stored = |n: usize| {
            self.sample(n)
                .unwrap()
                .store(depth, dither, [(n % width) as i64, (n / width) as i64])
        };
        Ok(match depth {
            Depth::Eight => Self::Eight(
                (0..self.len())
                    .map(|n| stored(n).map(|v| v as u8))
                    .collect(),
            ),
            Depth::Sixteen => Self::Sixteen((0..self.len()).map(stored).collect()),
        })
    }
}

impl Sample {
    pub fn mix_space(
        self,
        other: Self,
        weight: f64,
        space: crate::color_mix::Space,
        brightness: f64,
    ) -> Result<Self, String> {
        let mixed = if space == crate::color_mix::Space::Standard {
            self.mix(other, weight)?
        } else {
            crate::color_mix::mix_float(space, self, other, weight)?
        };
        crate::lab_mix::correct_float(mixed, self, other, weight, brightness)
    }
    pub fn tonal(self, settings: crate::tonal::Settings, amount: f64) -> Result<Self, String> {
        use crate::tonal::{Band, Mode};
        if !settings.valid() || !amount.is_finite() {
            return Err("Invalid floating tonal adjustment".into());
        }
        if settings.mode == Mode::Off || self.alpha() == 0. {
            return Ok(self);
        }
        let rgb = self.rgb();
        let value = if rgb[0] == rgb[1] && rgb[1] == rgb[2] {
            rgb[0]
        } else {
            (54. * rgb[0] + 183. * rgb[1] + 19. * rgb[2]) / 256.
        };
        let band = if value < 85. / 255. {
            Band::Shadows
        } else if value < 170. / 255. {
            Band::Midtones
        } else {
            Band::Highlights
        };
        if band != settings.band {
            return Ok(self);
        }
        let weight = settings.strength * amount.clamp(0., 1.);
        self.with_rgb(rgb.map(|v| match settings.mode {
            Mode::Dodge => v + (1. - v) * weight,
            Mode::Burn => v * (1. - weight),
            Mode::Off => v,
        }))
    }
}
