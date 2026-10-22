//! Alpha-preserving native-RGB sharpening and isolated luma/chroma noise reduction.
use crate::{
    color_precision::{Depth, Sample},
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Operation {
    Pixels(crate::pixel_effects::Operation),
    Sharpen {
        strength: f64,
    },
    Unsharp {
        radius: u8,
        amount: f64,
        threshold: f64,
    },
    Denoise {
        luminance: f64,
        chroma: f64,
        detail: f64,
    },
}
impl Operation {
    pub fn same_kind(self, other: Self) -> bool {
        match (self, other) {
            (Self::Pixels(a), Self::Pixels(b)) => a.same_kind(b),
            (a, b) => std::mem::discriminant(&a) == std::mem::discriminant(&b),
        }
    }
    pub fn all() -> Vec<Self> {
        Self::defaults()
            .into_iter()
            .chain(
                crate::pixel_effects::Operation::all()
                    .into_iter()
                    .map(Self::Pixels),
            )
            .collect()
    }
    pub fn defaults() -> [Self; 3] {
        [
            Self::Sharpen { strength: 0.5 },
            Self::Unsharp {
                radius: 3,
                amount: 1.,
                threshold: 0.01,
            },
            Self::Denoise {
                luminance: 0.5,
                chroma: 0.5,
                detail: 0.03,
            },
        ]
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Pixels(op) => op.label(),
            Self::Sharpen { .. } => "Sharpen",
            Self::Unsharp { .. } => "Unsharp mask",
            Self::Denoise { .. } => "Reduce noise",
        }
    }
    pub fn valid(self) -> bool {
        let range = |v: f64, hi: f64| v.is_finite() && (0. ..=hi).contains(&v);
        match self {
            Self::Pixels(op) => op.valid(),
            Self::Sharpen { strength } => range(strength, 5.),
            Self::Unsharp {
                radius,
                amount,
                threshold,
            } => radius <= 32 && range(amount, 5.) && range(threshold, 1.),
            Self::Denoise {
                luminance,
                chroma,
                detail,
            } => [luminance, chroma, detail]
                .into_iter()
                .all(|v| range(v, 1.)),
        }
    }
    fn identity(self) -> bool {
        match self {
            Self::Pixels(op) => op.identity(),
            Self::Sharpen { strength } => strength == 0.,
            Self::Unsharp { radius, amount, .. } => radius == 0 || amount == 0.,
            Self::Denoise {
                luminance, chroma, ..
            } => luminance == 0. && chroma == 0.,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub operation: Operation,
    #[serde(default)]
    pub selection: bool,
}
impl Settings {
    pub fn valid(self) -> bool {
        self.operation.valid()
    }
}
fn components(rgb: [f64; 3]) -> [f64; 3] {
    [
        rgb[0] * 0.25 + rgb[1] * 0.5 + rgb[2] * 0.25,
        (rgb[0] - rgb[2]) * 0.5,
        -rgb[0] * 0.25 + rgb[1] * 0.5 - rgb[2] * 0.25,
    ]
}
fn from_components(v: [f64; 3]) -> [f64; 3] {
    [v[0] + v[1] - v[2], v[0] + v[2], v[0] - v[1] - v[2]].map(|v| v.clamp(0., 1.))
}
fn sample(d: &Document, layer: usize, x: i64, y: i64) -> Sample {
    let src = &d.layers[layer];
    let e = src.extent(d.width, d.height);
    src.sample(
        d.width,
        d.height,
        x.clamp(e.offset[0], e.offset[0] + e.width as i64 - 1),
        y.clamp(e.offset[1], e.offset[1] + e.height as i64 - 1),
    )
}
fn median(values: &mut [(f64, f64)]) -> f64 {
    values.sort_by(|a, b| a.0.total_cmp(&b.0));
    let half = values.iter().map(|(_, w)| w).sum::<f64>() / 2.;
    let mut sum = 0.;
    for (v, w) in values.iter() {
        sum += w;
        if sum >= half {
            return *v;
        }
    }
    values.last().unwrap().0
}
fn denoise(
    d: &Document,
    layer: usize,
    xy: [i64; 2],
    before: Sample,
    luminance: f64,
    chroma: f64,
    detail: f64,
) -> [f64; 3] {
    // Four opposite pairs preserve continuous one-pixel lines, including diagonals.
    let offsets = [
        [-1, -1],
        [0, -1],
        [1, -1],
        [-1, 0],
        [1, 0],
        [-1, 1],
        [0, 1],
        [1, 1],
    ];
    let neighbors = offsets.map(|v| {
        let p = sample(d, layer, xy[0] + v[0], xy[1] + v[1]);
        (components(p.rgb()), p.alpha())
    });
    let old = components(before.rgb());
    let mut out = old;
    for k in 0..3 {
        let amount = if k == 0 { luminance } else { chroma };
        let line = (0..4).any(|n| {
            let (a, aa) = neighbors[n];
            let (b, ba) = neighbors[7 - n];
            aa > 0. && ba > 0. && (a[k] - old[k]).abs() <= detail && (b[k] - old[k]).abs() <= detail
        });
        if line || amount == 0. {
            continue;
        }
        let mut values = [(0., 0.); 8];
        let mut n = 0;
        for (v, a) in &neighbors {
            if *a > 0. {
                values[n] = (v[k], *a);
                n += 1;
            }
        }
        if n > 0 {
            out[k] = old[k] * (1. - amount) + median(&mut values[..n]) * amount;
        }
    }
    from_components(out)
}
fn gaussian(
    d: &Document,
    layer: usize,
    radius: u8,
    tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Vec<[f64; 3]>, String> {
    let src = &d.layers[layer];
    let e = src.extent(d.width, d.height);
    let count = src.pixels.len();
    let r = radius as i64;
    let sigma = (radius as f64 / 3.).max(0.5);
    let mut weights = (-r..=r)
        .map(|v| (-0.5 * (v as f64 / sigma).powi(2)).exp())
        .collect::<Vec<_>>();
    let total: f64 = weights.iter().sum();
    for v in &mut weights {
        *v /= total;
    }
    let mut rows = vec![[0.; 4]; count];
    for (i, out) in rows.iter_mut().enumerate() {
        if i.is_multiple_of(128) {
            tick(i, count * 3)?;
        }
        let xy = [
            e.offset[0] + (i % e.width as usize) as i64,
            e.offset[1] + (i / e.width as usize) as i64,
        ];
        for (n, w) in weights.iter().enumerate() {
            let p = sample(d, layer, xy[0] + n as i64 - r, xy[1]);
            let a = p.alpha() * w;
            let rgb = p.rgb();
            for k in 0..3 {
                out[k] += rgb[k] * a;
            }
            out[3] += a;
        }
    }
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        if i.is_multiple_of(128) {
            tick(count + i, count * 3)?;
        }
        let x = i % e.width as usize;
        let y = i / e.width as usize;
        let mut sum = [0.; 4];
        for (n, w) in weights.iter().enumerate() {
            let row = (y as i64 + n as i64 - r).clamp(0, e.height as i64 - 1) as usize;
            for (k, v) in rows[row * e.width as usize + x].into_iter().enumerate() {
                sum[k] += v * w;
            }
        }
        result.push(if sum[3] > 0. {
            [sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3]]
        } else {
            src.sample_at(i).rgb()
        });
    }
    Ok(result)
}
pub fn prepare(
    d: &Document,
    layer: usize,
    s: Settings,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<DocumentState, String> {
    prepare_with_budget(d, layer, s, DEFAULT_DOCUMENT_BUDGET, &mut tick)
}
pub fn prepare_with_budget(
    d: &Document,
    layer: usize,
    s: Settings,
    budget: usize,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<DocumentState, String> {
    if let Operation::Pixels(op) = s.operation {
        return crate::pixel_effects::prepare(d, layer, op, s.selection, budget, tick);
    }
    tick(0, 1)?;
    crate::layer_properties::paintable(d, layer)?;
    if !s.valid() {
        return Err("Sharpen/amount0–5, unsharp radius0–32/threshold0–1, noise/detail0–1 and finite values required".into());
    }
    let src = &d.layers[layer];
    let count = src.pixels.len();
    if count > 4 * 1024 * 1024 || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Detail filter source4M pixels/snapshot32MiB limit".into());
    }
    if s.operation.identity()
        || s.selection
            && d.scene
                .selection_mask
                .as_ref()
                .is_some_and(|m| m.runs.is_empty())
    {
        return Ok(d.snapshot());
    }
    let (radius, work) = match s.operation {
        Operation::Unsharp { radius, .. } => {
            (radius, count.saturating_mul(2 * (2 * radius as usize + 1)))
        }
        _ => (0, count.saturating_mul(9)),
    };
    let buffers = if radius > 0 {
        count.saturating_mul(56)
    } else {
        0
    };
    if work > 64 * 1024 * 1024
        || d.resident_bytes()
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(buffers)
            > budget.min(DEFAULT_DOCUMENT_BUDGET)
    {
        return Err(
            "Detail filter64M samples or workspace memory limit; reduce radius/source".into(),
        );
    }
    let blurred = if radius > 0 {
        Some(gaussian(d, layer, radius, &mut tick)?)
    } else {
        None
    };
    let mut state = d.snapshot();
    let out = &mut state.layers[layer];
    let e = src.extent(d.width, d.height);
    let mut changed = false;
    for i in 0..count {
        if i.is_multiple_of(128) {
            tick(
                if radius > 0 { count * 2 + i } else { i },
                if radius > 0 { count * 3 } else { count },
            )?;
        }
        let before = src.sample_at(i);
        if before.alpha() == 0. {
            continue;
        }
        let xy = [
            e.offset[0] + (i % e.width as usize) as i64,
            e.offset[1] + (i / e.width as usize) as i64,
        ];
        let weight = if s.selection {
            d.scene.selection_coverage(xy.map(|v| v as f64 + 0.5)) as f64 / 255.
        } else {
            1.
        };
        if weight == 0. {
            continue;
        }
        let rgb = before.rgb();
        let mapped = match s.operation {
            Operation::Pixels(_) => unreachable!("Pixel operations use their own planner"),
            Operation::Sharpen { strength } => {
                let mut lap = [0.; 3];
                for offset in [[-1, 0], [1, 0], [0, -1], [0, 1]] {
                    let p = sample(d, layer, xy[0] + offset[0], xy[1] + offset[1]);
                    let factor = p.alpha().min(before.alpha()) / before.alpha();
                    for k in 0..3 {
                        lap[k] += (rgb[k] - p.rgb()[k]) * factor;
                    }
                }
                [0, 1, 2].map(|k| (rgb[k] + strength * lap[k]).clamp(0., 1.))
            }
            Operation::Unsharp {
                amount, threshold, ..
            } => {
                let b = blurred.as_ref().unwrap()[i];
                [0, 1, 2].map(|k| {
                    let diff = rgb[k] - b[k];
                    if diff.abs() <= threshold {
                        rgb[k]
                    } else {
                        (rgb[k] + amount * diff).clamp(0., 1.)
                    }
                })
            }
            Operation::Denoise {
                luminance,
                chroma,
                detail,
            } => denoise(d, layer, xy, before, luminance, chroma, detail),
        };
        if mapped.iter().zip(rgb).all(|(a, b)| (a - b).abs() <= 1e-12) {
            continue;
        }
        let result = before.mix(before.with_rgb(mapped)?, weight)?;
        let mut stored = result.store(d.color.depth, d.color.dither, xy);
        stored[3] = if d.color.depth == Depth::Sixteen {
            before.rgba16()[3]
        } else {
            src.pixels[i][3] as u16
        };
        if d.color.depth == Depth::Sixteen {
            if stored == before.rgba16() {
                continue;
            }
            if out.precise.is_none() {
                out.precise = Some((0..count).map(|i| src.sample_at(i).rgba16()).collect());
            }
            out.precise.as_mut().unwrap()[i] = stored;
            out.pixels[i] = Sample::from16(stored).rgba8();
        } else {
            let low = stored.map(|v| v as u8);
            if low == src.pixels[i] {
                continue;
            }
            out.pixels[i] = low;
        }
        changed = true;
    }
    tick(1, 1)?;
    if !changed {
        return Ok(d.snapshot());
    }
    state.scene.last_detail_filter = Some(s);
    crate::layer_groups::finish(d, &state)?;
    tick(1, 1)?;
    Ok(state)
}
