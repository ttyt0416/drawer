//! Native-RGB channel separation, seeded film tones and alpha-safe pencil shading.
use crate::{color_precision::Sample, document::Document};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Operation {
    Aberration {
        offset: [i16; 2],
        strength: f64,
    },
    Film {
        strength: f64,
        contrast: f64,
        fade: f64,
        temperature: f64,
        grain: f64,
        seed: u64,
    },
    Pencil {
        strength: f64,
        outline: f64,
        shading: f64,
        radius: u8,
    },
}
impl Operation {
    pub fn defaults() -> [Self; 3] {
        [
            Self::Aberration {
                offset: [2, 0],
                strength: 1.,
            },
            Self::Film {
                strength: 1.,
                contrast: 0.35,
                fade: 0.15,
                temperature: 0.2,
                grain: 0.1,
                seed: 13,
            },
            Self::Pencil {
                strength: 1.,
                outline: 1.,
                shading: 0.7,
                radius: 1,
            },
        ]
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Aberration { .. } => "Chromatic aberration",
            Self::Film { .. } => "Film tones and grain",
            Self::Pencil { .. } => "Pencil contours and shading",
        }
    }
    pub fn valid(self) -> bool {
        let range = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        match self {
            Self::Aberration { offset, strength } => {
                offset.iter().all(|v| (-32..=32).contains(v)) && range(strength, 0., 1.)
            }
            Self::Film {
                strength,
                contrast,
                fade,
                temperature,
                grain,
                ..
            } => {
                [strength, contrast, fade, grain]
                    .into_iter()
                    .all(|v| range(v, 0., 1.))
                    && range(temperature, -1., 1.)
            }
            Self::Pencil {
                strength,
                outline,
                shading,
                radius,
            } => {
                range(strength, 0., 1.)
                    && range(outline, 0., 2.)
                    && range(shading, 0., 1.)
                    && (1..=16).contains(&radius)
            }
        }
    }
    pub fn identity(self) -> bool {
        match self {
            Self::Aberration { offset, strength } => strength == 0. || offset == [0; 2],
            Self::Film {
                strength,
                contrast,
                fade,
                temperature,
                grain,
                ..
            } => strength == 0. || contrast == 0. && fade == 0. && temperature == 0. && grain == 0.,
            Self::Pencil { strength, .. } => strength == 0.,
        }
    }
    pub fn padding(self) -> [u32; 2] {
        match self {
            Self::Aberration { offset, .. } => offset.map(|v| v.unsigned_abs() as u32),
            _ => [0; 2],
        }
    }
    pub fn padding_options(
        self,
        options: crate::filter_options::Options,
        e: crate::document::PlacedRaster,
        size: [u32; 2],
    ) -> [u32; 2] {
        let Self::Aberration { offset, .. } = self else {
            return self.padding();
        };
        let length = (offset[0] as f64).hypot(offset[1] as f64);
        match options.aberration {
            crate::filter_options::Aberration::Offset => self.padding(),
            crate::filter_options::Aberration::Lateral => {
                let (s, c) = options.angle.to_radians().sin_cos();
                [c, s].map(|v| (v.abs() * length).ceil() as u32)
            }
            crate::filter_options::Aberration::Radial => {
                let c = options.center.unwrap_or(size.map(|v| v as f64 / 2.));
                let scale = length / (size[0].max(size[1]) as f64 / 2.);
                [0, 1].map(|k| {
                    let lo = e.offset[k] as f64;
                    let hi = lo + if k == 0 { e.width } else { e.height } as f64;
                    ((lo - c[k]).abs().max((hi - c[k]).abs()) * scale).ceil() as u32
                })
            }
        }
    }
    pub fn cost(self) -> usize {
        match self {
            Self::Aberration { .. } => 9,
            Self::Film { .. } => 1,
            Self::Pencil { .. } => 9,
        }
    }
    pub fn preserves_alpha(self) -> bool {
        !matches!(self, Self::Aberration { .. })
    }
}
fn luminance(p: Sample) -> f64 {
    let rgb = p.rgb();
    ((rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722) * p.alpha() + 1. - p.alpha())
        .clamp(0., 1.)
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
    op: Operation,
    options: crate::filter_options::Options,
) -> Result<Sample, String> {
    if !op.valid() || !options.valid() {
        return Err("Invalid stylized filter options".into());
    }
    let sampler = crate::filter_options::Sampler {
        document: d,
        layer,
        boundary: options.boundary,
    };
    match op {
        Operation::Aberration { offset, strength } => {
            let shift = match options.aberration {
                crate::filter_options::Aberration::Offset => offset.map(|v| v as f64),
                crate::filter_options::Aberration::Lateral => {
                    let (s, c) = options.angle.to_radians().sin_cos();
                    let length = (offset[0] as f64).hypot(offset[1] as f64);
                    [length * c, length * s]
                }
                crate::filter_options::Aberration::Radial => {
                    let c = options
                        .center
                        .unwrap_or([d.width as f64 / 2., d.height as f64 / 2.]);
                    let scale = (offset[0] as f64).hypot(offset[1] as f64)
                        / (d.width.max(d.height) as f64 / 2.);
                    [0, 1].map(|k| (xy[k] as f64 + 0.5 - c[k]) * scale)
                }
            };
            let channels = [
                sampler.bilinear([xy[0] as f64 - shift[0], xy[1] as f64 - shift[1]]),
                before,
                sampler.bilinear([xy[0] as f64 + shift[0], xy[1] as f64 + shift[1]]),
            ];
            let alpha = channels.iter().map(|p| p.alpha()).fold(0., f64::max);
            let mapped = if alpha == 0. {
                Sample::default()
            } else {
                Sample::new([
                    channels[0].rgb()[0] * channels[0].alpha() / alpha,
                    channels[1].rgb()[1] * channels[1].alpha() / alpha,
                    channels[2].rgb()[2] * channels[2].alpha() / alpha,
                    alpha,
                ])?
            };
            before.mix(mapped, strength)
        }
        Operation::Film {
            strength,
            contrast,
            fade,
            temperature,
            grain,
            seed,
        } => {
            if before.alpha() == 0. {
                return Ok(before);
            }
            let noise = crate::pixel_effects::noise(seed, xy, 0) * grain * 0.2;
            let rgb = before.rgb();
            let mapped = [0, 1, 2].map(|k| {
                let v = rgb[k];
                let curve = v * v * (3. - 2. * v);
                let tone =
                    (v * (1. - contrast) + curve * contrast) * (1. - fade * 0.2) + fade * 0.1;
                let tint = match k {
                    0 => temperature * 0.08,
                    2 => -temperature * 0.08,
                    _ => 0.,
                };
                (tone + tint + noise).clamp(0., 1.)
            });
            let mut mapped = mapped;
            use crate::filter_options::Film;
            if matches!(
                options.film,
                Film::Sepia | Film::SepiaSlant | Film::SepiaAll
            ) {
                let [r, g, b] = mapped;
                mapped = [
                    (0.393 * r + 0.769 * g + 0.189 * b).min(1.),
                    (0.349 * r + 0.686 * g + 0.168 * b).min(1.),
                    (0.272 * r + 0.534 * g + 0.131 * b).min(1.),
                ];
            }
            if matches!(
                options.film,
                Film::Slant | Film::All | Film::SepiaSlant | Film::SepiaAll
            ) {
                let u = (xy[0] as f64 + 0.5) / d.width as f64;
                let v = (xy[1] as f64 + 0.5) / d.height as f64;
                let leak = if matches!(options.film, Film::Slant | Film::SepiaSlant) {
                    ((u + v) * 0.5).powi(4)
                } else {
                    ((u - 0.5).abs().max((v - 0.5).abs()) * 2.).powi(4)
                };
                for (k, c) in mapped.iter_mut().enumerate() {
                    *c = (*c + leak * [0.55, 0.18, 0.04][k]).min(1.);
                }
            }
            before.mix(before.with_rgb(mapped)?, strength)
        }
        Operation::Pencil {
            strength,
            outline,
            shading,
            radius,
        } => {
            if before.alpha() == 0. {
                return Ok(before);
            }
            let r = radius as i64;
            let mut values = [[0.; 3]; 3];
            for (y, row) in values.iter_mut().enumerate() {
                for (x, v) in row.iter_mut().enumerate() {
                    *v = luminance(
                        sampler.at(xy[0] + (x as i64 - 1) * r, xy[1] + (y as i64 - 1) * r),
                    );
                }
            }
            let gx = values[0][2] + 2. * values[1][2] + values[2][2]
                - values[0][0]
                - 2. * values[1][0]
                - values[2][0];
            let gy = values[2][0] + 2. * values[2][1] + values[2][2]
                - values[0][0]
                - 2. * values[0][1]
                - values[0][2];
            let edge = ((gx.abs() + gy.abs()) / 4. * outline).clamp(0., 1.);
            let rgb = before.rgb();
            let gray = ((rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722) * shading + 1.
                - shading)
                .clamp(0., 1.);
            before.mix(before.with_rgb([gray * (1. - edge); 3])?, strength)
        }
    }
}
