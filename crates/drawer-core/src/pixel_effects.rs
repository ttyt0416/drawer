//! Local, deterministic pixel blocks, grain, connected specks and alpha morphology.
use crate::{
    color_precision::{Depth, Sample},
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DustTarget {
    Dark,
    Light,
    Ink,
}
impl DustTarget {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::Ink];
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark specks",
            Self::Light => "Light specks",
            Self::Ink => "Isolated ink on transparency",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Operation {
    Warp(crate::warp_filters::Operation),
    Style(crate::stylized_effects::Operation),
    Mosaic {
        cell: u16,
        origin: [i64; 2],
    },
    Noise {
        amount: f64,
        seed: u64,
        monochrome: bool,
    },
    Dust {
        max_area: u32,
        threshold: f64,
        target: DustTarget,
    },
    LineWidth {
        delta: i8,
    },
}
impl Operation {
    pub fn all() -> Vec<Self> {
        Self::defaults()
            .into_iter()
            .chain(crate::stylized_effects::Operation::defaults().map(Self::Style))
            .chain(crate::warp_filters::Operation::defaults().map(Self::Warp))
            .collect()
    }
    pub fn same_kind(self, other: Self) -> bool {
        match (self, other) {
            (Self::Warp(a), Self::Warp(b)) => {
                std::mem::discriminant(&a) == std::mem::discriminant(&b)
            }
            (Self::Style(a), Self::Style(b)) => {
                std::mem::discriminant(&a) == std::mem::discriminant(&b)
            }
            (a, b) => std::mem::discriminant(&a) == std::mem::discriminant(&b),
        }
    }
    pub fn preserves_alpha(self) -> bool {
        match self {
            Self::Style(op) => op.preserves_alpha(),
            Self::Noise { .. }
            | Self::Dust {
                target: DustTarget::Dark | DustTarget::Light,
                ..
            } => true,
            _ => false,
        }
    }
    pub fn defaults() -> [Self; 4] {
        [
            Self::Mosaic {
                cell: 4,
                origin: [0; 2],
            },
            Self::Noise {
                amount: 0.1,
                seed: 1,
                monochrome: true,
            },
            Self::Dust {
                max_area: 4,
                threshold: 0.25,
                target: DustTarget::Dark,
            },
            Self::LineWidth { delta: 1 },
        ]
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Warp(op) => op.label(),
            Self::Style(op) => op.label(),
            Self::Mosaic { .. } => "Mosaic",
            Self::Noise { .. } => "Generate noise",
            Self::Dust { .. } => "Remove specks",
            Self::LineWidth { .. } => "Adjust line width",
        }
    }
    pub fn valid(self) -> bool {
        match self {
            Self::Warp(op) => op.valid(),
            Self::Style(op) => op.valid(),
            Self::Mosaic { cell, origin } => {
                (1..=256).contains(&cell) && origin.iter().all(|v| v.unsigned_abs() <= 1_000_000)
            }
            Self::Noise { amount, .. } => amount.is_finite() && (0. ..=1.).contains(&amount),
            Self::Dust {
                max_area,
                threshold,
                ..
            } => max_area <= 4096 && threshold.is_finite() && (0. ..=1.).contains(&threshold),
            Self::LineWidth { delta } => (-16..=16).contains(&delta),
        }
    }
    pub fn identity(self) -> bool {
        match self {
            Self::Warp(op) => op.identity(),
            Self::Style(op) => op.identity(),
            Self::Mosaic { cell, .. } => cell == 1,
            Self::Noise { amount, .. } => amount == 0.,
            Self::Dust { max_area, .. } => max_area == 0,
            Self::LineWidth { delta } => delta == 0,
        }
    }
}
fn hash(mut v: u64) -> u64 {
    v = v.wrapping_add(0x9e3779b97f4a7c15);
    v = (v ^ (v >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    v = (v ^ (v >> 27)).wrapping_mul(0x94d049bb133111eb);
    v ^ (v >> 31)
}
pub fn noise(seed: u64, xy: [i64; 2], channel: usize) -> f64 {
    let n = hash(
        seed ^ hash(xy[0] as u64)
            ^ hash((xy[1] as u64).wrapping_add(0x123456789abcdef0))
            ^ hash(channel as u64),
    );
    ((n >> 11) as f64 / (1u64 << 53) as f64) * 2. - 1.
}
struct Blocks {
    cols: usize,
    base: [i64; 2],
    cell: i64,
    values: Vec<([f64; 4], usize)>,
}
impl Blocks {
    fn index(&self, p: [i64; 2]) -> usize {
        let cell = p.map(|v| v.div_euclid(self.cell));
        (cell[1] - self.base[1]) as usize * self.cols + (cell[0] - self.base[0]) as usize
    }
    fn create(
        d: &Document,
        layer: usize,
        cell: u16,
        origin: [i64; 2],
        tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
    ) -> Result<Self, String> {
        let src = &d.layers[layer];
        let e = src.extent(d.width, d.height);
        let c = cell as i64;
        let lo = [e.offset[0] - origin[0], e.offset[1] - origin[1]];
        let base = lo.map(|v| v.div_euclid(c));
        let cols = ((lo[0] + e.width as i64 - 1).div_euclid(c) - base[0] + 1) as usize;
        let rows = ((lo[1] + e.height as i64 - 1).div_euclid(c) - base[1] + 1) as usize;
        let mut blocks = Self {
            cols,
            base,
            cell: c,
            values: vec![([0.; 4], 0); cols * rows],
        };
        for i in 0..src.pixels.len() {
            if i.is_multiple_of(128) {
                tick(i, src.pixels.len() * 2)?;
            }
            let p = [
                lo[0] + (i % e.width as usize) as i64,
                lo[1] + (i / e.width as usize) as i64,
            ];
            let index = blocks.index(p);
            let (sum, n) = &mut blocks.values[index];
            let sample = src.sample_at(i);
            let a = sample.alpha();
            for (k, v) in sample.rgb().into_iter().enumerate() {
                sum[k] += v * a;
            }
            sum[3] += a;
            *n += 1;
        }
        Ok(blocks)
    }
    fn sample(&self, p: [i64; 2], origin: [i64; 2]) -> Sample {
        let (sum, n) = self.values[self.index([p[0] - origin[0], p[1] - origin[1]])];
        if sum[3] == 0. {
            Sample::default()
        } else {
            Sample::new([
                sum[0] / sum[3],
                sum[1] / sum[3],
                sum[2] / sum[3],
                sum[3] / n as f64,
            ])
            .unwrap()
        }
    }
}
fn specks(
    d: &Document,
    layer: usize,
    max_area: u32,
    threshold: f64,
    target: DustTarget,
    tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Vec<Option<Sample>>, String> {
    let src = &d.layers[layer];
    let e = src.extent(d.width, d.height);
    let count = src.pixels.len();
    let width = e.width as usize;
    let mut eligible = vec![false; count];
    for (i, value) in eligible.iter_mut().enumerate() {
        if i.is_multiple_of(128) {
            tick(i, count * 3)?;
        }
        let p = src.sample_at(i);
        *value = p.alpha() > 0.
            && match target {
                DustTarget::Ink => p.alpha() >= threshold,
                DustTarget::Dark => p.luminance() <= threshold,
                DustTarget::Light => p.luminance() >= threshold,
            };
    }
    let mut visited = vec![false; count];
    let mut result = vec![None; count];
    let mut queue = Vec::new();
    let mut processed = 0;
    for start in 0..count {
        if start.is_multiple_of(128) {
            tick(count + processed, count * 3)?;
        }
        if visited[start] || !eligible[start] {
            continue;
        }
        visited[start] = true;
        queue.push(start);
        let mut members = Vec::new();
        let mut size = 0;
        let mut border = false;
        let mut sum = [0.; 4];
        while let Some(i) = queue.pop() {
            if processed.is_multiple_of(128) {
                tick(count + processed, count * 3)?;
            }
            processed += 1;
            size += 1;
            if members.len() <= max_area as usize {
                members.push(i);
            }
            let x = i % width;
            let y = i / width;
            border |= x == 0 || x + 1 == width || y == 0 || y + 1 == e.height as usize;
            for neighbor in [
                x.checked_sub(1).map(|_| i - 1),
                (x + 1 < width).then_some(i + 1),
                y.checked_sub(1).map(|_| i - width),
                (y + 1 < e.height as usize).then_some(i + width),
            ]
            .into_iter()
            .flatten()
            {
                if eligible[neighbor] {
                    if !visited[neighbor] {
                        visited[neighbor] = true;
                        queue.push(neighbor);
                    }
                } else {
                    let p = src.sample_at(neighbor);
                    let a = p.alpha();
                    for (k, v) in p.rgb().into_iter().enumerate() {
                        sum[k] += v * a;
                    }
                    sum[3] += a;
                }
            }
        }
        if border || size > max_area as usize || target != DustTarget::Ink && sum[3] == 0. {
            continue;
        }
        for i in members {
            result[i] = Some(if target == DustTarget::Ink {
                Sample::default()
            } else {
                src.sample_at(i)
                    .with_rgb([sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3]])?
            });
        }
    }
    Ok(result)
}
fn width_sample(
    d: &Document,
    layer: usize,
    xy: [i64; 2],
    before: Sample,
    delta: i8,
    tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
    progress: [usize; 2],
) -> Result<Sample, String> {
    let src = &d.layers[layer];
    let r = delta.unsigned_abs() as i64;
    let mut best = before;
    let mut distance = 0i64;
    let mut n = 0usize;
    for y in -r..=r {
        for x in -r..=r {
            if n.is_multiple_of(128) {
                tick(progress[0], progress[1])?;
            }
            n += 1;
            let p = src.sample(d.width, d.height, xy[0] + x, xy[1] + y);
            let dist = x * x + y * y;
            let wins = if delta > 0 {
                p.alpha() > best.alpha() || p.alpha() == best.alpha() && dist < distance
            } else {
                p.alpha() < best.alpha()
            };
            if wins {
                best = p;
                distance = dist;
            }
        }
    }
    if best.alpha() == before.alpha() {
        Ok(before)
    } else if delta < 0 || before.alpha() > 0. {
        Sample::new([
            before.rgb()[0],
            before.rgb()[1],
            before.rgb()[2],
            best.alpha(),
        ])
    } else {
        Ok(best)
    }
}
pub fn prepare(
    d: &Document,
    layer: usize,
    op: Operation,
    selection: bool,
    budget: usize,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<DocumentState, String> {
    tick(0, 1)?;
    crate::layer_properties::paintable(d, layer)?;
    if !op.valid() {
        return Err("Mosaic cell1–256/origin±1M, noise0–1, speck area0–4096/threshold0–1 or width−16–16 required".into());
    }
    let src = &d.layers[layer];
    let count = src.pixels.len();
    if count > 4 * 1024 * 1024 || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Pixel effect source4M/snapshot32MiB limit".into());
    }
    if op.identity()
        || selection
            && d.scene
                .selection_mask
                .as_ref()
                .is_some_and(|m| m.runs.is_empty())
    {
        return Ok(d.snapshot());
    }
    let locked = crate::clipping::alpha_locked(d, layer);
    if locked
        && matches!(
            op,
            Operation::LineWidth { .. }
                | Operation::Dust {
                    target: DustTarget::Ink,
                    ..
                }
        )
    {
        return Err(
            "Unlock layer transparency before changing line width or erasing isolated ink".into(),
        );
    }
    let old = src.extent(d.width, d.height);
    let pad = match op {
        Operation::LineWidth { delta } if delta > 0 => [delta as u32; 2],
        Operation::Style(op) if !locked => op.padding(),
        _ => [0; 2],
    };
    let extent = PlacedRaster {
        width: old.width + 2 * pad[0],
        height: old.height + 2 * pad[1],
        offset: [old.offset[0] - pad[0] as i64, old.offset[1] - pad[1] as i64],
    };
    let output = extent.width as usize * extent.height as usize;
    let (work, scratch) = match op {
        Operation::Warp(_) => (output.saturating_mul(8), 0),
        Operation::Style(op) => (output.saturating_mul(op.cost()), 0),
        Operation::Mosaic { cell, .. } => {
            let cols = old.width as usize / cell as usize + 2;
            let rows = old.height as usize / cell as usize + 2;
            (
                count.saturating_mul(2),
                cols.saturating_mul(rows).saturating_mul(40),
            )
        }
        Operation::Noise { .. } => (count, 0),
        Operation::Dust { .. } => (count.saturating_mul(8), count.saturating_mul(64)),
        Operation::LineWidth { delta } => (
            output.saturating_mul((2 * delta.unsigned_abs() as usize + 1).pow(2)),
            0,
        ),
    };
    if !extent.valid()
        || output > 4 * 1024 * 1024
        || work > 64 * 1024 * 1024
        || d.resident_bytes()
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(output.saturating_mul(d.color.depth.bytes() * 3))
            .saturating_add(scratch)
            > budget.min(DEFAULT_DOCUMENT_BUDGET)
    {
        return Err(
            "Pixel effect extent4M, work64M or workspace memory limit; reduce amount/source".into(),
        );
    }
    let blocks = if let Operation::Mosaic { cell, origin } = op {
        Some(Blocks::create(d, layer, cell, origin, &mut tick)?)
    } else {
        None
    };
    let dust = if let Operation::Dust {
        max_area,
        threshold,
        target,
    } = op
    {
        Some(specks(d, layer, max_area, threshold, target, &mut tick)?)
    } else {
        None
    };
    let mut pixels = Vec::with_capacity(output);
    let mut precise = (d.color.depth == Depth::Sixteen).then(|| Vec::with_capacity(output));
    let mut changed = false;
    for i in 0..output {
        if i.is_multiple_of(128) {
            tick(i, output)?;
        }
        let xy = [
            extent.offset[0] + (i % extent.width as usize) as i64,
            extent.offset[1] + (i / extent.width as usize) as i64,
        ];
        let index = src.pixel_index(d.width, d.height, xy[0], xy[1]);
        let before = src.sample(d.width, d.height, xy[0], xy[1]);
        let weight = if selection {
            d.scene.selection_coverage(xy.map(|v| v as f64 + 0.5)) as f64 / 255.
        } else {
            1.
        };
        let mapped = if weight == 0. || locked && before.alpha() == 0. {
            before
        } else {
            match op {
                Operation::Warp(op) => crate::warp_filters::sample(d, layer, xy, op)?,
                Operation::Style(op) => crate::stylized_effects::sample(d, layer, xy, before, op)?,
                Operation::Mosaic { origin, .. } => {
                    let p = blocks.as_ref().unwrap().sample(xy, origin);
                    if locked { before.with_rgb(p.rgb())? } else { p }
                }
                Operation::Noise {
                    amount,
                    seed,
                    monochrome,
                } => {
                    if before.alpha() == 0. {
                        before
                    } else {
                        before.with_rgb([0, 1, 2].map(|k| {
                            (before.rgb()[k]
                                + amount * noise(seed, xy, if monochrome { 0 } else { k }))
                            .clamp(0., 1.)
                        }))?
                    }
                }
                Operation::Dust { .. } => index
                    .and_then(|n| dust.as_ref().unwrap()[n])
                    .unwrap_or(before),
                Operation::LineWidth { delta } => {
                    width_sample(d, layer, xy, before, delta, &mut tick, [i, output])?
                }
            }
        };
        let mapped = if locked || op.preserves_alpha() {
            before.with_rgb(mapped.rgb())?
        } else {
            mapped
        };
        if mapped.alpha() == 0. && before.alpha() == 0.
            || mapped
                .components()
                .iter()
                .zip(before.components())
                .all(|(a, b)| (a - b).abs() <= 1e-12)
        {
            pixels.push(index.map_or([0; 4], |n| src.pixels[n]));
            if let Some(high) = &mut precise {
                high.push(before.rgba16());
            }
            continue;
        }
        let result = before.mix(mapped, weight)?;
        let mut stored = result.store(d.color.depth, d.color.dither, xy);
        if locked || op.preserves_alpha() {
            stored[3] = if d.color.depth == Depth::Sixteen {
                before.rgba16()[3]
            } else {
                index.map_or(0, |n| src.pixels[n][3] as u16)
            };
        }
        let low = if let Some(high) = &mut precise {
            high.push(stored);
            Sample::from16(stored).rgba8()
        } else {
            stored.map(|v| v as u8)
        };
        changed |= if d.color.depth == Depth::Sixteen {
            stored != before.rgba16()
        } else {
            low != index.map_or([0; 4], |n| src.pixels[n])
        };
        pixels.push(low);
    }
    tick(1, 1)?;
    if !changed {
        return Ok(d.snapshot());
    }
    let mut state = d.snapshot();
    state.layers[layer].pixels = pixels;
    state.layers[layer].precise = precise;
    state.layers[layer].placement = if extent == old {
        src.placement
    } else {
        Some(extent)
    };
    state.scene.last_detail_filter = Some(crate::detail_filter::Settings {
        operation: crate::detail_filter::Operation::Pixels(op),
        selection,
    });
    crate::layer_groups::finish(d, &state)?;
    tick(1, 1)?;
    Ok(state)
}
