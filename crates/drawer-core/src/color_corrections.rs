//! Native document-RGB point corrections. Geometry, alpha and hidden colors are preserved.
use crate::{
    color_precision::{Depth, Sample},
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    sizing::DocumentState,
};
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operation {
    Levels {
        input: [f64; 2],
        output: [f64; 2],
        gamma: f64,
    },
    Balance([f64; 3]),
    Posterize(u16),
    Threshold(f64),
    Invert,
}
impl Operation {
    pub fn defaults() -> [Self; 5] {
        [
            Self::Levels {
                input: [0., 1.],
                output: [0., 1.],
                gamma: 1.,
            },
            Self::Balance([0.; 3]),
            Self::Posterize(8),
            Self::Threshold(0.5),
            Self::Invert,
        ]
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Levels { .. } => "Levels",
            Self::Balance(_) => "Color balance",
            Self::Posterize(_) => "Posterize",
            Self::Threshold(_) => "Threshold",
            Self::Invert => "Invert RGB",
        }
    }
    pub fn valid(self) -> bool {
        match self {
            Self::Levels {
                input,
                output,
                gamma,
            } => {
                input
                    .iter()
                    .chain(&output)
                    .all(|v| v.is_finite() && (0. ..=1.).contains(v))
                    && input[1] - input[0] >= 1e-9
                    && output[1] > output[0]
                    && gamma.is_finite()
                    && (0.01..=10.).contains(&gamma)
            }
            Self::Balance(v) => v.iter().all(|v| v.is_finite() && (-1. ..=1.).contains(v)),
            Self::Posterize(n) => (2..=256).contains(&n),
            Self::Threshold(v) => v.is_finite() && (0. ..=1.).contains(&v),
            Self::Invert => true,
        }
    }
    fn identity(self) -> bool {
        self == Self::Levels {
            input: [0., 1.],
            output: [0., 1.],
            gamma: 1.,
        } || self == Self::Balance([0.; 3])
    }
    pub fn rgb(self, rgb: [f64; 3]) -> Result<[f64; 3], String> {
        if !self.valid()
            || rgb
                .iter()
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        {
            return Err("Invalid native RGB correction parameters/input".into());
        }
        Ok(match self {
            Self::Levels {
                input,
                output,
                gamma,
            } => rgb.map(|v| {
                output[0]
                    + (output[1] - output[0])
                        * ((v - input[0]) / (input[1] - input[0]))
                            .clamp(0., 1.)
                            .powf(1. / gamma)
            }),
            Self::Balance(offset) => [0, 1, 2].map(|k| (rgb[k] + offset[k]).clamp(0., 1.)),
            Self::Posterize(n) => rgb.map(|v| (v * (n - 1) as f64).round() / (n - 1) as f64),
            Self::Threshold(t) => {
                [if rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722 >= t {
                    1.
                } else {
                    0.
                }; 3]
            }
            Self::Invert => rgb.map(|v| 1. - v),
        })
    }
}
pub fn prepare(
    d: &Document,
    layer: usize,
    selection: bool,
    op: Operation,
) -> Result<DocumentState, String> {
    crate::layer_properties::paintable(d, layer)?;
    if !op.valid() {
        return Err("Levels need increasing input/output black-white points, gamma0.01–10; posterize2–256; threshold0–1; finite balance−1–1".into());
    }
    let source = &d.layers[layer];
    if source.pixels.len() > 4 * 1024 * 1024 || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Color correction snapshot/pixel limit4M".into());
    }
    if op.identity() {
        return Ok(d.snapshot());
    }
    let extent = source.extent(d.width, d.height);
    let mut state = d.snapshot();
    let out = &mut state.layers[layer];
    for i in 0..source.pixels.len() {
        let before = source.sample_at(i);
        if before.alpha() == 0. {
            continue;
        }
        let pos = [
            extent.offset[0] + (i % extent.width as usize) as i64,
            extent.offset[1] + (i / extent.width as usize) as i64,
        ];
        let w = if selection {
            d.scene.selection_coverage(pos.map(|v| v as f64 + 0.5)) as f64 / 255.
        } else {
            1.
        };
        if w == 0. {
            continue;
        }
        let mapped = before.with_rgb(op.rgb(before.rgb())?)?;
        let result = if w == 1. {
            mapped
        } else {
            before.mix(mapped, w)?
        };
        let mut stored = result.store(d.color.depth, d.color.dither, pos);
        stored[3] = if d.color.depth == Depth::Sixteen {
            before.rgba16()[3]
        } else {
            source.pixels[i][3] as u16
        };
        if d.color.depth == Depth::Sixteen {
            if stored == before.rgba16() {
                continue;
            }
            if out.precise.is_none() {
                out.precise = Some(
                    (0..source.pixels.len())
                        .map(|i| source.sample_at(i).rgba16())
                        .collect(),
                );
            }
            out.precise.as_mut().unwrap()[i] = stored;
            out.pixels[i] = Sample::from16(stored).rgba8();
        } else {
            out.pixels[i] = stored.map(|v| v as u8);
        }
    }
    crate::layer_groups::finish(d, &state)?;
    Ok(state)
}
