//! Separable finite-support Gaussian filtering of native premultiplied RGB/alpha.
use crate::{
    color_precision::{Depth, Sample},
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    sizing::DocumentState,
};
pub fn prepare(
    d: &Document,
    layer: usize,
    radius: u8,
    selection: bool,
) -> Result<DocumentState, String> {
    crate::layer_properties::paintable(d, layer)?;
    if radius > 64 || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Gaussian radius0–64 and snapshot limit required".into());
    }
    let source = &d.layers[layer];
    if radius == 0
        || selection
            && d.scene
                .selection_mask
                .as_ref()
                .is_some_and(|m| m.runs.is_empty())
        || (0..source.pixels.len()).all(|i| source.sample_at(i).alpha() == 0.)
    {
        return Ok(d.snapshot());
    }
    let locked = crate::clipping::alpha_locked(d, layer);
    let old = source.extent(d.width, d.height);
    let pad = if locked { 0 } else { radius as u32 };
    let extent = PlacedRaster {
        width: old.width + 2 * pad,
        height: old.height + 2 * pad,
        offset: [old.offset[0] - pad as i64, old.offset[1] - pad as i64],
    };
    let count = extent.width as usize * extent.height as usize;
    let horizontal = extent.width as usize * old.height as usize;
    let work = (horizontal + count).saturating_mul(2 * radius as usize + 1);
    if !extent.valid()
        || count > 4 * 1024 * 1024
        || work > 64 * 1024 * 1024
        || d.resident_bytes()
            .saturating_add(d.memory_bytes().saturating_mul(2))
            .saturating_add(horizontal.saturating_mul(32))
            .saturating_add(count.saturating_mul(d.color.depth.bytes() * 2))
            > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Gaussian extent4M pixels, work64M or workspace memory limit; reduce the radius or source size".into());
    }
    let sigma = (radius as f64 / 3.).max(0.5);
    let mut kernel = (-(radius as i64)..=radius as i64)
        .map(|x| (-0.5 * (x as f64 / sigma).powi(2)).exp())
        .collect::<Vec<_>>();
    let sum: f64 = kernel.iter().sum();
    for w in &mut kernel {
        *w /= sum;
    }
    let mut rows = vec![[0f64; 4]; horizontal];
    for y in 0..old.height {
        for x in 0..extent.width {
            let mut sum = [0.; 4];
            for (i, w) in kernel.iter().enumerate() {
                let p = source.sample(
                    d.width,
                    d.height,
                    extent.offset[0] + x as i64 + i as i64 - radius as i64,
                    old.offset[1] + y as i64,
                );
                let a = p.alpha() * w;
                let rgb = p.rgb();
                for k in 0..3 {
                    sum[k] += rgb[k] * a;
                }
                sum[3] += a;
            }
            rows[y as usize * extent.width as usize + x as usize] = sum;
        }
    }
    let mut pixels = Vec::with_capacity(count);
    let mut precise = (d.color.depth == Depth::Sixteen).then(|| Vec::with_capacity(count));
    let mut changed = false;
    for y in 0..extent.height {
        for x in 0..extent.width {
            let point = [extent.offset[0] + x as i64, extent.offset[1] + y as i64];
            let old_index = source.pixel_index(d.width, d.height, point[0], point[1]);
            let before = source.sample(d.width, d.height, point[0], point[1]);
            let weight = if selection {
                d.scene.selection_coverage(point.map(|v| v as f64 + 0.5)) as f64 / 255.
            } else {
                1.
            };
            let mut sum = [0.; 4];
            if weight > 0. {
                for (i, w) in kernel.iter().enumerate() {
                    let row = point[1] + i as i64 - radius as i64 - old.offset[1];
                    if row >= 0 && row < old.height as i64 {
                        let p = rows[row as usize * extent.width as usize + x as usize];
                        for k in 0..4 {
                            sum[k] += p[k] * w;
                        }
                    }
                }
            }
            if weight == 0.
                || locked && before.alpha() == 0.
                || sum[3] == 0. && before.alpha() == 0.
            {
                pixels.push(old_index.map_or([0; 4], |i| source.pixels[i]));
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
            let result = if weight == 1. {
                blurred
            } else {
                before.mix(blurred, weight)?
            };
            let mut value = result.store(d.color.depth, d.color.dither, point);
            if locked {
                value[3] = if d.color.depth == Depth::Sixteen {
                    before.rgba16()[3]
                } else {
                    old_index.map_or(0, |i| source.pixels[i][3] as u16)
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
                low != old_index.map_or([0; 4], |i| source.pixels[i])
            };
            pixels.push(low);
        }
    }
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
    crate::layer_groups::finish(d, &state)?;
    Ok(state)
}
