//! Bounded CPU kernels shared by live rendering and raster confirmation.
use crate::{
    color_precision::Sample,
    document::PlacedRaster,
    surface_effects::{Image, Operation, point},
};

fn scalar_sample(v: &[f32], w: usize, h: usize, x: i64, y: i64) -> f32 {
    if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
        0.
    } else {
        v[y as usize * w + x as usize]
    }
}
fn blur(
    v: &[f32],
    w: usize,
    h: usize,
    r: usize,
    tick: &mut impl FnMut(usize) -> Result<(), String>,
) -> Result<Vec<f32>, String> {
    let mut a = v.to_vec();
    if r == 0 {
        return Ok(a);
    }
    // Three separable box passes, zero outside the allocated padded extent.
    for _ in 0..3 {
        for horizontal in [true, false] {
            let mut b = vec![0.; a.len()];
            let (lines, len) = if horizontal { (h, w) } else { (w, h) };
            let index = |line: usize, i: usize| {
                if horizontal {
                    line * w + i
                } else {
                    i * w + line
                }
            };
            for line in 0..lines {
                if line % 16 == 0 {
                    tick(line)?;
                }
                let mut sum = 0f64;
                for i in 0..=r.min(len - 1) {
                    sum += a[index(line, i)] as f64;
                }
                for i in 0..len {
                    b[index(line, i)] = (sum / (2 * r + 1) as f64) as f32;
                    if i >= r {
                        sum -= a[index(line, i - r)] as f64;
                    }
                    if i + r + 1 < len {
                        sum += a[index(line, i + r + 1)] as f64;
                    }
                }
            }
            a = b;
        }
    }
    Ok(a)
}
fn dilate(
    v: &[f32],
    w: usize,
    h: usize,
    r: usize,
    tick: &mut impl FnMut(usize) -> Result<(), String>,
) -> Result<Vec<f32>, String> {
    let mut a = v.to_vec();
    if r == 0 {
        return Ok(a);
    }
    for horizontal in [true, false] {
        let mut b = vec![0.; a.len()];
        let (lines, len) = if horizontal { (h, w) } else { (w, h) };
        let index = |line: usize, i: usize| {
            if horizontal {
                line * w + i
            } else {
                i * w + line
            }
        };
        for line in 0..lines {
            if line % 16 == 0 {
                tick(line)?;
            }
            let mut q = std::collections::VecDeque::<usize>::new();
            let mut next = 0;
            for i in 0..len {
                while next <= i.saturating_add(r).min(len - 1) {
                    while q
                        .back()
                        .is_some_and(|&j| a[index(line, j)] <= a[index(line, next)])
                    {
                        q.pop_back();
                    }
                    q.push_back(next);
                    next += 1;
                }
                while q.front().is_some_and(|&j| j < i.saturating_sub(r)) {
                    q.pop_front();
                }
                b[index(line, i)] = a[index(line, *q.front().unwrap())];
            }
        }
        a = b;
    }
    Ok(a)
}
fn shifted(v: &[f32], w: usize, h: usize, p: [f64; 2]) -> f64 {
    let x = p[0].floor() as i64;
    let y = p[1].floor() as i64;
    let f = [p[0] - x as f64, p[1] - y as f64];
    (0..2)
        .flat_map(|dy| (0..2).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| {
            scalar_sample(v, w, h, x + dx, y + dy) as f64
                * if dx == 0 { 1. - f[0] } else { f[0] }
                * if dy == 0 { 1. - f[1] } else { f[1] }
        })
        .sum()
}
pub(crate) fn appearance(
    image: &Image,
    style: &crate::appearance::Style,
    mut tick: impl FnMut(usize) -> Result<(), String>,
) -> Result<Vec<[f32; 4]>, String> {
    let w = image.extent.width as usize;
    let h = image.extent.height as usize;
    let alpha = image.pixels.iter().map(|p| p[3]).collect::<Vec<_>>();
    let expanded = dilate(&alpha, w, h, style.border as usize, &mut tick)?;
    let mut out = image.pixels.clone();
    for (i, pixel) in out.iter_mut().enumerate() {
        if i % 128 == 0 {
            tick(i)?;
        }
        let raw = Sample::new(image.pixels[i].map(f64::from))?;
        let x = (i % w) as i64;
        let y = (i / w) as i64;
        let neighbor = [
            scalar_sample(&alpha, w, h, x - 1, y),
            scalar_sample(&alpha, w, h, x + 1, y),
            scalar_sample(&alpha, w, h, x, y - 1),
            scalar_sample(&alpha, w, h, x, y + 1),
        ]
        .into_iter()
        .fold(f32::INFINITY, f32::min);
        let edge = if raw.alpha() > 0. {
            (1. - neighbor as f64 / raw.alpha()).max(0.)
        } else {
            0.
        };
        let ink = raw.with_rgb(raw.rgb().map(|v| v * (1. - style.rim * edge)))?;
        let border =
            Sample::from8(style.border_color).coverage((expanded[i] - alpha[i]).max(0.) as f64)?;
        let after = border.over(ink, Default::default(), 1.)?;
        *pixel = raw
            .mix(
                after,
                style.weight(point(image.extent, i).map(|v| v as f64 + 0.5)),
            )?
            .components()
            .map(|v| v as f32);
    }
    Ok(out)
}
pub(crate) fn apply(
    image: &Image,
    source: PlacedRaster,
    op: Operation,
    mut tick: impl FnMut(usize) -> Result<(), String>,
) -> Result<Vec<[f32; 4]>, String> {
    let w = image.extent.width as usize;
    let h = image.extent.height as usize;
    let mut out = image.pixels.clone();
    match op {
        Operation::Shadow {
            blur: r,
            direction,
            distance,
            color,
            opacity,
        } => {
            let alpha = image.pixels.iter().map(|v| v[3]).collect::<Vec<_>>();
            let alpha = blur(&alpha, w, h, r as usize, &mut tick)?;
            let (s, c) = crate::texture::rotation(direction);
            let delta = [c * distance, s * distance];
            for (i, o) in out.iter_mut().enumerate() {
                if i % 128 == 0 {
                    tick(i)?;
                }
                let a = shifted(
                    &alpha,
                    w,
                    h,
                    [(i % w) as f64 - delta[0], (i / w) as f64 - delta[1]],
                ) * opacity;
                let shadow = Sample::from8([color[0], color[1], color[2], 255]).coverage(a)?;
                *o = shadow
                    .over(
                        Sample::new(image.pixels[i].map(f64::from))?,
                        Default::default(),
                        1.,
                    )?
                    .components()
                    .map(|v| v as f32);
            }
        }
        Operation::Glow {
            intensity,
            spread,
            blur: r,
            color,
        } => {
            let light = image
                .pixels
                .iter()
                .map(|p| {
                    let s = Sample::new(p.map(f64::from)).unwrap();
                    (s.luminance().clamp(0., 1.) * s.alpha()) as f32
                })
                .collect::<Vec<_>>();
            let expanded = dilate(&light, w, h, spread as usize, &mut tick)?;
            let light = blur(&expanded, w, h, r as usize, &mut tick)?;
            for (i, o) in out.iter_mut().enumerate() {
                if i % 128 == 0 {
                    tick(i)?;
                }
                let glow = Sample::from8([color[0], color[1], color[2], 255])
                    .coverage((light[i] as f64 * intensity).clamp(0., 1.))?;
                *o = glow
                    .over(
                        Sample::new(image.pixels[i].map(f64::from))?,
                        Default::default(),
                        1.,
                    )?
                    .components()
                    .map(|v| v as f32);
            }
        }
        Operation::Emboss {
            shadow,
            gloss,
            height,
            smoothing,
            light,
        } => {
            let heights = image
                .pixels
                .iter()
                .map(|p| {
                    let s = Sample::new(p.map(f64::from)).unwrap();
                    (s.alpha() * s.luminance().clamp(0., 1.) * height) as f32
                })
                .collect::<Vec<_>>();
            let heights = blur(&heights, w, h, smoothing as usize, &mut tick)?;
            let lnorm = (light.x * light.x + light.y * light.y + 1.).sqrt();
            let l = [light.x / lnorm, light.y / lnorm, 1. / lnorm];
            for (i, o) in out.iter_mut().enumerate() {
                if i % 128 == 0 {
                    tick(i)?;
                }
                let p = image.pixels[i];
                if p[3] == 0. {
                    continue;
                }
                let x = (i % w) as i64;
                let y = (i / w) as i64;
                let dx = (scalar_sample(&heights, w, h, x + 1, y)
                    - scalar_sample(&heights, w, h, x - 1, y)) as f64
                    / 2.;
                let dy = (scalar_sample(&heights, w, h, x, y + 1)
                    - scalar_sample(&heights, w, h, x, y - 1)) as f64
                    / 2.;
                let norm = (dx * dx + dy * dy + 1.).sqrt();
                let normal = [-dx / norm, -dy / norm, 1. / norm];
                let diffuse = normal.iter().zip(l).map(|(a, b)| a * b).sum::<f64>();
                let slope = (1. - normal[2]).clamp(0., 1.);
                let specular = (2. * diffuse * normal[2] - l[2]).max(0.).powi(16) * gloss * slope;
                let factor = (1. + shadow * (diffuse - l[2])).max(0.);
                *o = Sample::new(p.map(f64::from))?
                    .with_rgb([0, 1, 2].map(|k| (p[k] as f64 * factor + specular).clamp(0., 1.)))?
                    .components()
                    .map(|v| v as f32);
            }
        }
        Operation::Crystal {
            cell,
            irregularity,
            offset,
            tiling,
            seed,
        } => {
            let size = [source.width as f64, source.height as f64];
            let counts = size.map(|v| (v / cell as f64).ceil().max(1.) as i64);
            let step = if tiling {
                [size[0] / counts[0] as f64, size[1] / counts[1] as f64]
            } else {
                [cell as f64; 2]
            };
            for (i, o) in out.iter_mut().enumerate() {
                if i % 128 == 0 {
                    tick(i)?;
                }
                let p = point(image.extent, i).map(|v| v as f64 + 0.5);
                let q = [
                    p[0] - source.offset[0] as f64 - offset[0],
                    p[1] - source.offset[1] as f64 - offset[1],
                ];
                let base = [
                    (q[0] / step[0]).floor() as i64,
                    (q[1] / step[1]).floor() as i64,
                ];
                let mut best = (f64::INFINITY, [0.; 2]);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let index = [base[0] + dx, base[1] + dy];
                        let hash = if tiling {
                            [
                                index[0].rem_euclid(counts[0]),
                                index[1].rem_euclid(counts[1]),
                            ]
                        } else {
                            index
                        };
                        let site = [0, 1].map(|k| {
                            (index[k] as f64
                                + 0.5
                                + crate::pixel_effects::noise(seed, hash, k) * 0.45 * irregularity)
                                * step[k]
                        });
                        let dist = (q[0] - site[0]).powi(2) + (q[1] - site[1]).powi(2);
                        if dist < best.0 {
                            best = (dist, site);
                        }
                    }
                }
                let pos = [0, 1].map(|k| {
                    let v = best.1[k] + offset[k];
                    source.offset[k]
                        + if tiling {
                            v.rem_euclid(size[k]).floor() as i64
                        } else {
                            v.floor() as i64
                        }
                });
                *o = image.sample(pos).components().map(|v| v as f32);
            }
        }
    }
    Ok(out)
}
