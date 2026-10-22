//! Non-destructive raster tracing and bounded length-based vector cleanup.
use crate::{
    document::Document,
    scene::{Affine, Point},
    sizing::DocumentState,
    vector_scope::Picked,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trace {
    pub alpha: u8,
    pub brightness: u8,
    pub colors: usize,
    pub error: f64,
}
impl Default for Trace {
    fn default() -> Self {
        Self {
            alpha: 1,
            brightness: 255,
            colors: 8,
            error: 0.,
        }
    }
}
pub struct Prepared {
    pub state: DocumentState,
    pub layer: Option<u64>,
    pub changed: usize,
}
fn tick(work: &mut usize, count: usize) -> Result<(), String> {
    *work = work.saturating_add(count);
    if *work > 64 * 1024 * 1024 {
        Err("Vector conversion work limit64M".into())
    } else {
        Ok(())
    }
}
fn distance(a: [u8; 4], b: [u8; 4]) -> u64 {
    (0..4)
        .map(|i| {
            let a = if i == 3 {
                a[i] as i64 * 255
            } else {
                a[i] as i64 * a[3] as i64
            };
            let b = if i == 3 {
                b[i] as i64 * 255
            } else {
                b[i] as i64 * b[3] as i64
            };
            (a - b).pow(2) as u64
        })
        .sum()
}
fn palette(
    hist: &BTreeMap<[u8; 4], usize>,
    count: usize,
    work: &mut usize,
) -> Result<Vec<[u8; 4]>, String> {
    if hist.len() <= count {
        return Ok(hist.keys().copied().collect());
    }
    let mut colors = vec![*hist.iter().max_by_key(|(c, n)| (**n, **c)).unwrap().0];
    while colors.len() < count {
        tick(work, hist.len() * colors.len())?;
        let next = hist
            .iter()
            .max_by_key(|(c, n)| {
                (
                    colors
                        .iter()
                        .map(|p| distance(**c, *p))
                        .min()
                        .unwrap()
                        .saturating_mul(**n as u64),
                    **c,
                )
            })
            .unwrap()
            .0;
        colors.push(*next);
    }
    for _ in 0..4 {
        let mut sums = vec![[0u64; 5]; count];
        tick(work, hist.len() * count)?;
        for (color, n) in hist {
            let i = nearest(*color, &colors);
            for k in 0..4 {
                sums[i][k] += color[k] as u64 * *n as u64;
            }
            sums[i][4] += *n as u64;
        }
        for (color, sum) in colors.iter_mut().zip(sums) {
            if sum[4] > 0 {
                *color = std::array::from_fn(|k| ((sum[k] + sum[4] / 2) / sum[4]) as u8);
            }
        }
    }
    colors.sort();
    colors.dedup();
    Ok(colors)
}
fn nearest(c: [u8; 4], colors: &[[u8; 4]]) -> usize {
    (0..colors.len())
        .min_by_key(|i| (distance(c, colors[*i]), *i))
        .unwrap()
}
type Grid = [u32; 2];
type Edges = BTreeMap<Grid, Vec<(Grid, u8)>>;
fn loops(mut edges: Edges) -> Result<Vec<Vec<Point>>, String> {
    let mut out = vec![];
    while let Some((&first, _)) = edges.first_key_value() {
        let mut p = first;
        let mut dir = 0;
        let mut points = vec![];
        loop {
            points.push([p[0] as f64, p[1] as f64]);
            let next = edges.get_mut(&p).ok_or("Open raster contour")?;
            let n = (0..next.len())
                .min_by_key(|i| match (next[*i].1 + 4 - dir) % 4 {
                    1 => 0,
                    0 => 1,
                    3 => 2,
                    _ => 3,
                })
                .ok_or("Open raster contour")?;
            let (q, d) = next.remove(n);
            if next.is_empty() {
                edges.remove(&p);
            }
            p = q;
            dir = d;
            if p == first {
                break;
            }
        }
        // Exact removal of collinear grid edges, including the seam.
        let n = points.len();
        let reduced = (0..n)
            .filter(|i| {
                let a = points[(*i + n - 1) % n];
                let b = points[*i];
                let c = points[(*i + 1) % n];
                (b[0] - a[0]) * (c[1] - b[1]) != (b[1] - a[1]) * (c[0] - b[0])
            })
            .map(|i| points[i])
            .collect::<Vec<_>>();
        if reduced.len() < 3 {
            return Err("Degenerate raster contour".into());
        }
        out.push(reduced);
    }
    Ok(out)
}
fn simplify(p: &[Point], error: f64, work: &mut usize) -> Result<Vec<Point>, String> {
    if error == 0. {
        return Ok(p.to_vec());
    }
    let n = p.len();
    let mut keep = vec![false; n + 1];
    keep[0] = true;
    keep[n / 2] = true;
    keep[n] = true;
    let mut stack = vec![(0, n / 2), (n / 2, n)];
    while let Some((a, b)) = stack.pop() {
        tick(work, b - a)?;
        let mut best = (error, None);
        for i in a + 1..b {
            let e = crate::line_tools::distance(p[i % n], p[a % n], p[b % n]).0;
            if e > best.0 {
                best = (e, Some(i));
            }
        }
        if let Some(i) = best.1 {
            keep[i] = true;
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    let q = (0..n)
        .filter(|i| keep[*i])
        .map(|i| p[i])
        .collect::<Vec<_>>();
    if q.len() < 3 { Ok(p.to_vec()) } else { Ok(q) }
}
fn area(p: &[Point]) -> f64 {
    (0..p.len())
        .map(|i| {
            let a = p[i];
            let b = p[(i + 1) % p.len()];
            a[0] * b[1] - a[1] * b[0]
        })
        .sum()
}
fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
    for k in 0..2 {
        if a[k].max(b[k]) < c[k].min(d[k]) || c[k].max(d[k]) < a[k].min(b[k]) {
            return false;
        }
    }
    cross(a, b, c) * cross(a, b, d) <= 0. && cross(c, d, a) * cross(c, d, b) <= 0.
}
fn safe(
    original: &[Vec<Point>],
    candidate: &[Vec<Point>],
    work: &mut usize,
) -> Result<bool, String> {
    for (i, p) in candidate.iter().enumerate() {
        if area(p) * area(&original[i]) <= 0. {
            return Ok(false);
        }
        for a in 0..p.len() {
            for b in a + 1..p.len() {
                if b == a + 1 || a == 0 && b == p.len() - 1 {
                    continue;
                }
                tick(work, 1)?;
                if intersects(p[a], p[(a + 1) % p.len()], p[b], p[(b + 1) % p.len()]) {
                    return Ok(false);
                }
            }
        }
        for j in 0..i {
            let q = &candidate[j];
            let contains =
                |r: &[Point], v| crate::svg_style::contains(r, v, crate::svg_style::Rule::EvenOdd);
            if contains(q, p[0]) != contains(&original[j], original[i][0])
                || contains(p, q[0]) != contains(&original[i], original[j][0])
            {
                return Ok(false);
            }
            for a in 0..p.len() {
                for b in 0..q.len() {
                    tick(work, 1)?;
                    if intersects(p[a], p[(a + 1) % p.len()], q[b], q[(b + 1) % q.len()]) {
                        return Ok(false);
                    }
                }
            }
        }
    }
    Ok(true)
}
fn compound(contours: &[Vec<Point>], inverse: Affine) -> Vec<Point> {
    let mut out = vec![];
    for p in contours {
        let p = p.iter().map(|p| inverse.map(*p)).collect::<Vec<_>>();
        if out.is_empty() {
            out.extend_from_slice(&p);
            out.push(p[0]);
        } else {
            let first = out[0];
            out.extend_from_slice(&p);
            out.push(p[0]);
            out.push(first);
        }
    }
    out
}
pub fn trace(d: &Document, layer: usize, s: Trace) -> Result<Prepared, String> {
    if !(1..=16).contains(&s.colors) || !s.error.is_finite() || !(0. ..=1024.).contains(&s.error) {
        return Err("Trace uses1–16 colors and finite error0–1024 pixels".into());
    }
    let source = d.layers.get(layer).ok_or("Choose a raster layer")?;
    let count = d.width as usize * d.height as usize;
    if count > 1024 * 1024 || d.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Trace snapshot/pixel limit1M".into());
    }
    let mut work = 0;
    let mut hist = BTreeMap::new();
    let eligible = |p: [u8; 4]| {
        p[3] > 0
            && p[3] >= s.alpha
            && (54 * p[0] as u32 + 183 * p[1] as u32 + 19 * p[2] as u32) / 256
                <= s.brightness as u32
    };
    for y in 0..d.height {
        for x in 0..d.width {
            let p = source.pixel(d.width, d.height, x as i64, y as i64);
            if eligible(p) {
                *hist.entry(p).or_insert(0usize) += 1;
                if hist.len() > 65536 {
                    return Err("Trace unique-color limit65536".into());
                }
            }
        }
    }
    if hist.is_empty() {
        return Err("No pixels pass the trace thresholds".into());
    }
    let colors = palette(&hist, s.colors, &mut work)?;
    tick(&mut work, hist.len() * colors.len())?;
    let classes = hist
        .keys()
        .map(|p| (*p, nearest(*p, &colors) as u8))
        .collect::<BTreeMap<_, _>>();
    let mut labels = vec![255u8; count];
    for y in 0..d.height {
        for x in 0..d.width {
            if let Some(i) = classes.get(&source.pixel(d.width, d.height, x as i64, y as i64)) {
                labels[y as usize * d.width as usize + x as usize] = *i;
            }
        }
    }
    let at = |x: i64, y: i64| {
        if x < 0 || y < 0 || x >= d.width as i64 || y >= d.height as i64 {
            255
        } else {
            labels[y as usize * d.width as usize + x as usize]
        }
    };
    let mut edges = vec![Edges::new(); colors.len()];
    let mut edge_count = 0;
    for y in 0..d.height {
        for x in 0..d.width {
            let i = at(x as i64, y as i64);
            if i == 255 {
                continue;
            }
            for (dx, dy, a, b, dir) in [
                (0, -1, [x, y], [x + 1, y], 0),
                (1, 0, [x + 1, y], [x + 1, y + 1], 1),
                (0, 1, [x + 1, y + 1], [x, y + 1], 2),
                (-1, 0, [x, y + 1], [x, y], 3),
            ] {
                if at(x as i64 + dx, y as i64 + dy) != i {
                    edge_count += 1;
                    if edge_count > 65536 {
                        return Err("Trace boundary limit65536; crop or reduce colors first".into());
                    }
                    edges[i as usize].entry(a).or_default().push((b, dir));
                }
            }
        }
    }
    let mut state = d.snapshot();
    let id = state.scene.lines.add_layer("Traced raster outlines")?;
    let mut changed = 0;
    for (color, edges) in colors.into_iter().zip(edges) {
        if edges.is_empty() {
            continue;
        }
        let original = loops(edges)?;
        let proposed = original
            .iter()
            .map(|p| simplify(p, s.error, &mut work))
            .collect::<Result<Vec<_>, _>>()?;
        let contours = if s.error == 0. || safe(&original, &proposed, &mut work)? {
            proposed
        } else {
            original
        };
        let points = compound(&contours, d.scene.transform.inverse());
        let path = state.scene.lines.add_path(id, points, true, 0., color)?;
        let p = state
            .scene
            .lines
            .layers
            .last_mut()
            .unwrap()
            .paths
            .iter_mut()
            .find(|p| p.id == path)
            .unwrap();
        p.stroke = false;
        p.fill = Some(color);
        changed += 1;
    }
    Ok(Prepared {
        state: crate::vector_scope::checked(d, state)?,
        layer: Some(id),
        changed,
    })
}
pub fn all(d: &Document) -> Vec<Picked> {
    d.scene
        .lines
        .layers
        .iter()
        .filter(|l| {
            crate::vector_edit::writable(d, l.id).is_ok()
                && !crate::layer_properties::inherited(
                    d,
                    crate::layer_properties::Target::Vector(l.id),
                )
                .position_locked
        })
        .filter(|l| !l.paths.is_empty())
        .map(|l| Picked {
            layer: l.id,
            selection: crate::vector_edit::Selection {
                paths: l.paths.iter().map(|p| p.id).collect(),
                controls: vec![],
            },
        })
        .collect()
}
pub fn remove_short(d: &Document, scope: &[Picked], threshold: f64) -> Result<Prepared, String> {
    if !threshold.is_finite() || !(0. ..=1e9).contains(&threshold) {
        return Err("Finite nonnegative document length required".into());
    }
    crate::vector_scope::validate(d, scope)?;
    let mut state = d.snapshot();
    let mut changed = 0;
    for selected in scope {
        if crate::layer_properties::inherited(
            d,
            crate::layer_properties::Target::Vector(selected.layer),
        )
        .position_locked
        {
            return Err("Vector positions are locked".into());
        }
        let index = state
            .scene
            .lines
            .layers
            .iter()
            .position(|l| l.id == selected.layer)
            .unwrap();
        let mut paths = vec![];
        for p in state.scene.lines.layers[index].paths.clone() {
            if !selected.selection.path_ids().contains(&p.id) {
                paths.push(p);
                continue;
            }
            let period = if p.closed {
                p.points.len() as f64 / (p.points.len() - 1) as f64
            } else {
                1.
            };
            let mut ranges = crate::vector_clipboard::ranges(&p, &selected.selection)?;
            let mut wrap_end = None;
            if p.closed
                && ranges.len() > 1
                && ranges[0].0 == 0.
                && ranges.last().unwrap().1 == period
            {
                let first = ranges.remove(0);
                wrap_end = Some(first.1);
                let last = ranges.pop().unwrap();
                ranges.push((last.0, period + first.1));
            }
            let mut cuts = vec![];
            for (a, b) in ranges {
                // Length of the rendered curve, never its control polygon or view projection.
                let count = (p.points.len() - 1) as f64;
                let length: f64 = ((a * count).floor() as usize..(b * count).ceil() as usize)
                    .map(|n| {
                        let fraction =
                            ((b * count).min(n as f64 + 1.) - (a * count).max(n as f64)).max(0.);
                        let x = d.scene.transform.map(p.points[n % p.points.len()]);
                        let y = d.scene.transform.map(p.points[(n + 1) % p.points.len()]);
                        (x[0] - y[0]).hypot(x[1] - y[1]) * fraction
                    })
                    .sum();
                if length < threshold {
                    changed += 1;
                    if b > period {
                        cuts.push((0., wrap_end.unwrap()));
                        cuts.push((a, period));
                    } else {
                        cuts.push((a, b));
                    }
                }
            }
            if cuts.is_empty() {
                paths.push(p);
                continue;
            }
            let period = if p.closed {
                p.points.len() as f64 / (p.points.len() - 1) as f64
            } else {
                1.
            };
            if p.fill.is_some() && cuts != [(0., period)] {
                return Err("Filled contours can only be removed as whole paths".into());
            }
            cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut cursor = 0.;
            let mut keeps = vec![];
            for (a, b) in cuts {
                if a > cursor + 1e-12 {
                    keeps.push((cursor, a));
                }
                cursor = b;
            }
            if cursor < period - 1e-12 {
                keeps.push((cursor, period));
            }
            for (n, (a, b)) in keeps.into_iter().enumerate() {
                let part = crate::vector_erase::part(&p, a, b)?;
                let mut q = p.clone();
                q.points = part.points;
                q.pressure = p.fragment_pressure_at(&part.parameters)?;
                q.definition = None;
                q.closed = false;
                q.fill = None;
                if n > 0 {
                    q.id = state.scene.lines.id()?;
                }
                paths.push(q);
            }
            state
                .scene
                .lines
                .selection
                .points
                .retain(|c| c.path != p.id);
            state.scene.lines.selection.paths.retain(|id| *id != p.id);
        }
        state.scene.lines.layers[index].paths = paths;
    }
    state.scene.lines.prune_selection();
    Ok(Prepared {
        state: crate::vector_scope::checked(d, state)?,
        layer: None,
        changed,
    })
}
