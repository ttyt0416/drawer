//! Independent height, abstraction, block/ringing reduction and inverse-map filters.
use crate::{
    color_precision::Sample,
    document::{Document, PlacedRaster},
    scene::Point,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Operation {
    Normal {
        strength: f64,
        y_up: bool,
    },
    Abstract {
        outline: f64,
        levels: u8,
    },
    Jpeg {
        block: f64,
        ring: f64,
    },
    Polar {
        inverse: bool,
        center: Option<Point>,
        radius: f64,
    },
    Smoothing {
        radius: u8,
        edge: f64,
        amount: f64,
    },
    Blur,
    BlurStrong,
    SharpenMore,
    Pinch {
        strength: f64,
        center: Option<Point>,
        radius: f64,
    },
    Curved {
        strength: f64,
        cylinder: bool,
        center: Option<Point>,
        radius: f64,
    },
    Zigzag {
        amplitude: f64,
        period: f64,
        center: Option<Point>,
        radius: f64,
    },
    Ripple {
        amplitude: f64,
        period: f64,
        center: Option<Point>,
        radius: f64,
    },
}
impl Operation {
    pub const COMMANDS: [&'static str; 12] = [
        "normal-map",
        "line-color-abstraction",
        "jpeg-noise-reduction",
        "polar-coordinates",
        "smoothing-filter",
        "blur-filter",
        "blur-strong",
        "sharpen-more",
        "pinch-filter",
        "curved-surface",
        "zigzag-filter",
        "ripple-filter",
    ];
    pub fn defaults() -> [Self; 12] {
        [
            Self::Normal {
                strength: 1.,
                y_up: true,
            },
            Self::Abstract {
                outline: 1.,
                levels: 6,
            },
            Self::Jpeg {
                block: 0.5,
                ring: 0.25,
            },
            Self::Polar {
                inverse: false,
                center: None,
                radius: 64.,
            },
            Self::Smoothing {
                radius: 2,
                edge: 0.8,
                amount: 1.,
            },
            Self::Blur,
            Self::BlurStrong,
            Self::SharpenMore,
            Self::Pinch {
                strength: 0.5,
                center: None,
                radius: 64.,
            },
            Self::Curved {
                strength: 0.5,
                cylinder: false,
                center: None,
                radius: 64.,
            },
            Self::Zigzag {
                amplitude: 4.,
                period: 16.,
                center: None,
                radius: 64.,
            },
            Self::Ripple {
                amplitude: 4.,
                period: 16.,
                center: None,
                radius: 64.,
            },
        ]
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal { .. } => "Normal map",
            Self::Abstract { .. } => "Color and line abstraction",
            Self::Jpeg { .. } => "JPEG block / ringing reduction",
            Self::Polar { .. } => "Polar coordinates",
            Self::Smoothing { .. } => "Smoothing",
            Self::Blur => "Blur",
            Self::BlurStrong => "Blur (strong)",
            Self::SharpenMore => "Sharpen more",
            Self::Pinch { .. } => "Pinch",
            Self::Curved { .. } => "Curved surface",
            Self::Zigzag { .. } => "ZigZag",
            Self::Ripple { .. } => "Ripple",
        }
    }
    pub fn valid(self) -> bool {
        let unit = |v: f64| v.is_finite() && (0. ..=1.).contains(&v);
        let center =
            |v: Option<Point>| v.is_none_or(|p| p.iter().all(|v| v.is_finite() && v.abs() <= 1e6));
        let radius = |v: f64| (0. ..=32768.).contains(&v);
        match self {
            Self::Normal { strength, .. } => (0. ..=32.).contains(&strength),
            Self::Abstract { outline, levels } => {
                (0. ..=4.).contains(&outline) && (2..=32).contains(&levels)
            }
            Self::Jpeg { block, ring } => unit(block) && unit(ring),
            Self::Polar {
                center: c,
                radius: r,
                ..
            } => center(c) && r > 0. && radius(r),
            Self::Smoothing {
                radius,
                edge,
                amount,
            } => radius <= 8 && unit(edge) && unit(amount),
            Self::Pinch {
                strength,
                center: c,
                radius: r,
            }
            | Self::Curved {
                strength,
                center: c,
                radius: r,
                ..
            } => (-1. ..=1.).contains(&strength) && center(c) && radius(r),
            Self::Zigzag {
                amplitude,
                period,
                center: c,
                radius: r,
            }
            | Self::Ripple {
                amplitude,
                period,
                center: c,
                radius: r,
            } => {
                (0. ..=1024.).contains(&amplitude)
                    && (0.25..=32768.).contains(&period)
                    && center(c)
                    && radius(r)
            }
            _ => true,
        }
    }
    pub fn identity(self) -> bool {
        match self {
            Self::Jpeg { block, ring } => block == 0. && ring == 0.,
            Self::Smoothing { radius, amount, .. } => radius == 0 || amount == 0.,
            Self::Pinch {
                strength, radius, ..
            }
            | Self::Curved {
                strength, radius, ..
            } => strength == 0. || radius == 0.,
            Self::Zigzag {
                amplitude, radius, ..
            }
            | Self::Ripple {
                amplitude, radius, ..
            } => amplitude == 0. || radius == 0.,
            _ => false,
        }
    }
    pub fn preserves_alpha(self) -> bool {
        !matches!(
            self,
            Self::Polar { .. }
                | Self::Pinch { .. }
                | Self::Curved { .. }
                | Self::Zigzag { .. }
                | Self::Ripple { .. }
        )
    }
    pub fn cost(self) -> usize {
        match self {
            Self::Smoothing { radius, .. } => (2 * radius as usize + 1).pow(2),
            Self::BlurStrong => 49,
            Self::Blur | Self::SharpenMore => 9,
            Self::Jpeg { .. } => 20,
            _ => 8,
        }
    }
    pub fn map(self, p: Point, e: PlacedRaster) -> Point {
        let auto = [
            e.offset[0] as f64 + e.width as f64 / 2.,
            e.offset[1] as f64 + e.height as f64 / 2.,
        ];
        let (center, radius) = match self {
            Self::Polar { center, radius, .. }
            | Self::Pinch { center, radius, .. }
            | Self::Curved { center, radius, .. }
            | Self::Zigzag { center, radius, .. }
            | Self::Ripple { center, radius, .. } => (center.unwrap_or(auto), radius),
            _ => return p,
        };
        if self.identity() {
            return p;
        }
        let v = [p[0] - center[0], p[1] - center[1]];
        let r = v[0].hypot(v[1]);
        let q = r / radius;
        if let Self::Polar { inverse, .. } = self {
            if inverse {
                let u = (p[0] - e.offset[0] as f64) / e.width as f64;
                let v = (p[1] - e.offset[1] as f64) / e.height as f64;
                let (s, c) = (u * std::f64::consts::TAU).sin_cos();
                return [center[0] + radius * v * c, center[1] + radius * v * s];
            }
            return [
                e.offset[0] as f64
                    + v[1].atan2(v[0]).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU
                        * e.width as f64,
                e.offset[1] as f64 + q * e.height as f64,
            ];
        }
        if r <= 1e-12 || q >= 1. {
            return p;
        }
        match self {
            Self::Pinch { strength, .. } => {
                let f = q.max(1e-8).powf(strength * 2.);
                [0, 1].map(|k| center[k] + v[k] * f)
            }
            Self::Curved {
                strength, cylinder, ..
            } => {
                if cylinder {
                    let x = (v[0] / radius).clamp(-1., 1.);
                    [
                        p[0] + strength * (x.asin() / std::f64::consts::FRAC_PI_2 * radius - v[0]),
                        p[1],
                    ]
                } else {
                    let f = q.asin() / std::f64::consts::FRAC_PI_2 / q;
                    [0, 1].map(|k| center[k] + v[k] * (1. + strength * (f - 1.)))
                }
            }
            Self::Ripple {
                amplitude, period, ..
            } => {
                let delta = amplitude * (r / period * std::f64::consts::TAU).sin() * (1. - q);
                [0, 1].map(|k| p[k] + v[k] / r * delta)
            }
            Self::Zigzag {
                amplitude, period, ..
            } => {
                let phase = (r / period).rem_euclid(1.);
                let triangle = 1. - 4. * (phase - 0.5).abs();
                let delta = amplitude * triangle * (1. - q);
                [p[0] - v[1] / r * delta, p[1] + v[0] / r * delta]
            }
            _ => p,
        }
    }
}
fn luma(p: Sample) -> f64 {
    let c = p.rgb();
    c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
}
fn mean(
    sampler: &crate::filter_options::Sampler<'_>,
    xy: [i64; 2],
    radius: i64,
    edge: f64,
    before: Sample,
) -> [f64; 3] {
    let mut sum = [0.; 3];
    let mut total = 0.;
    for y in -radius..=radius {
        for x in -radius..=radius {
            let p = sampler.at(xy[0] + x, xy[1] + y);
            let diff = p
                .rgb()
                .into_iter()
                .zip(before.rgb())
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>();
            let w = p.alpha() * (-diff * edge * 64.).exp();
            for (k, v) in p.rgb().into_iter().enumerate() {
                sum[k] += v * w;
            }
            total += w;
        }
    }
    if total > 0. {
        sum.map(|v| v / total)
    } else {
        before.rgb()
    }
}
pub fn sample(
    d: &Document,
    layer: usize,
    xy: [i64; 2],
    before: Sample,
    op: Operation,
) -> Result<Sample, String> {
    sample_options(d, layer, xy, before, op, Default::default())
}
pub fn sample_options(
    d: &Document,
    layer: usize,
    xy: [i64; 2],
    before: Sample,
    mut op: Operation,
    mut options: crate::filter_options::Options,
) -> Result<Sample, String> {
    if !op.valid() || !options.valid() {
        return Err("Invalid finishing filter settings".into());
    }
    if op.identity() && options.area != crate::filter_options::Area::EntireSelection
        || op.preserves_alpha() && before.alpha() == 0.
    {
        return Ok(before);
    }
    let sampler = crate::filter_options::Sampler {
        document: d,
        layer,
        boundary: options.boundary,
    };
    if !op.preserves_alpha() {
        let e = options.region(d, d.layers[layer].extent(d.width, d.height));
        let auto = [
            e.offset[0] as f64 + e.width as f64 / 2.,
            e.offset[1] as f64 + e.height as f64 / 2.,
        ];
        if options.area == crate::filter_options::Area::EntireSelection {
            options.shape = e.width as f64 / e.height as f64;
            match &mut op {
                Operation::Pinch { center, radius, .. }
                | Operation::Curved { center, radius, .. }
                | Operation::Zigzag { center, radius, .. }
                | Operation::Ripple { center, radius, .. } => {
                    *center = Some(auto);
                    *radius = e.height as f64 / 2.;
                }
                _ => {}
            }
        }
        let (center, radius) = match op {
            Operation::Polar { center, radius, .. }
            | Operation::Pinch { center, radius, .. }
            | Operation::Curved { center, radius, .. }
            | Operation::Zigzag { center, radius, .. }
            | Operation::Ripple { center, radius, .. } => (center.unwrap_or(auto), radius),
            _ => (auto, 1.),
        };
        let point = xy.map(|v| v as f64 + 0.5);
        let shaped = options.ellipse(point, center);
        let mapped = if matches!(op, Operation::Polar { .. }) && options.spherize {
            let v = [shaped[0] - center[0], shaped[1] - center[1]];
            let r = v[0].hypot(v[1]);
            let q = r / radius;
            if q > 0. && q < 1. {
                let f = q.asin() / std::f64::consts::FRAC_PI_2 / q;
                [center[0] + v[0] * f, center[1] + v[1] * f]
            } else {
                shaped
            }
        } else {
            op.map(shaped, e)
        };
        return Ok(sampler.bilinear([
            point[0] + (mapped[0] - shaped[0]) * options.shape - 0.5,
            point[1] + mapped[1] - shaped[1] - 0.5,
        ]));
    }
    let at = |x, y| sampler.at(xy[0] + x, xy[1] + y);
    let height = |x, y| {
        let p = at(x, y);
        if p.alpha() == 0. {
            luma(before)
        } else {
            luma(p)
        }
    };
    let rgb = match op {
        Operation::Normal { strength, y_up } => {
            let dx = (height(1, 0) - height(-1, 0)) * 0.5 * strength;
            let dy = (height(0, 1) - height(0, -1)) * 0.5 * strength;
            let len = (dx * dx + dy * dy + 1.).sqrt();
            [
                0.5 - dx / len * 0.5,
                0.5 + if y_up { dy } else { -dy } / len * 0.5,
                0.5 + 0.5 / len,
            ]
        }
        Operation::Abstract { outline, levels } => {
            let gx = height(1, 0) - height(-1, 0);
            let gy = height(0, 1) - height(0, -1);
            let ink = (gx.hypot(gy) * outline).clamp(0., 1.);
            let color = before
                .rgb()
                .map(|v| (v * (levels - 1) as f64).round() / (levels - 1) as f64);
            match options.abstraction {
                crate::filter_options::Abstraction::ColorAndLines => color.map(|v| v * (1. - ink)),
                crate::filter_options::Abstraction::ColorOnly => color,
                crate::filter_options::Abstraction::LinesOnly => [1. - ink; 3],
            }
        }
        Operation::Jpeg { block, ring } => {
            let mut out = before.rgb();
            for axis in 0..2 {
                let m = xy[axis].rem_euclid(8);
                if m == 0 || m == 7 {
                    let mut delta = [0; 2];
                    delta[axis] = if m == 0 { -1 } else { 1 };
                    let p = at(delta[0], delta[1]);
                    if p.alpha() > 0. {
                        for (k, v) in out.iter_mut().enumerate() {
                            *v += block * (p.rgb()[k] - *v) * 0.5;
                        }
                    }
                }
            }
            if ring > 0. && xy.iter().any(|v| matches!(v.rem_euclid(8),0..=2|5..=7)) {
                let soft = mean(&sampler, xy, 1, 0.5, before);
                for (k, v) in out.iter_mut().enumerate() {
                    *v += ring * (soft[k] - *v).clamp(-0.15, 0.15);
                }
            }
            out
        }
        Operation::Smoothing {
            radius,
            edge,
            amount,
        } => {
            let soft = mean(&sampler, xy, radius as i64, edge, before);
            [0, 1, 2].map(|k| before.rgb()[k] + amount * (soft[k] - before.rgb()[k]))
        }
        Operation::Blur => mean(&sampler, xy, 1, 0., before),
        Operation::BlurStrong => mean(&sampler, xy, 3, 0., before),
        Operation::SharpenMore => {
            let soft = mean(&sampler, xy, 1, 0., before);
            [0, 1, 2].map(|k| (before.rgb()[k] + 3. * (before.rgb()[k] - soft[k])).clamp(0., 1.))
        }
        _ => before.rgb(),
    };
    before.with_rgb(rgb)
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Presets {
    pub values: Vec<(String, crate::detail_filter::Settings)>,
}
impl Presets {
    pub fn valid(&self) -> bool {
        self.values.len() <= 32
            && self.values.iter().enumerate().all(|(n, (name, s))| {
                !name.trim().is_empty()
                    && name.chars().count() <= 64
                    && !name.chars().any(char::is_control)
                    && s.valid()
                    && !self.values[..n].iter().any(|(v, _)| v == name)
            })
    }
    pub fn is_default(&self) -> bool {
        self.values.is_empty()
    }
}
pub fn active(scene: &crate::scene::Scene) -> bool {
    let extra = |s: crate::detail_filter::Settings| {
        matches!(
            s.operation,
            crate::detail_filter::Operation::Pixels(crate::pixel_effects::Operation::Extended(_))
        )
    };
    scene.last_detail_filter.is_some_and(extra)
        || scene
            .last_batch_filter
            .as_ref()
            .is_some_and(|r| matches!(r.task,crate::batch_filters::Task::Detail(s)if extra(s)))
}
