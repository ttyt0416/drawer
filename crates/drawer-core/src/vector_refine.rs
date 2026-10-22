//! Local outline refinements. Source parameters, never spatial nearest crossings, carry pressure.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    line_tools::Path,
    scene::Point,
    shapes::Curve,
    sizing::DocumentState,
    vector_edit::{Patch, Selection},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ends {
    #[default]
    Both,
    Either,
    Free,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Width {
    #[default]
    Thicken,
    Narrow,
    ScaleUp,
    ScaleDown,
    Fix,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operation {
    Join(f64),
    Simplify(f64),
    Convert(Curve),
    Width(Width, f64, bool),
    Opacity(u8),
    Tip(crate::vector_tip::Preset),
    Split,
}
fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn start(d: &Document, layer: u64, s: &Selection, geometry: bool) -> Result<DocumentState, String> {
    if geometry
        && crate::layer_properties::inherited(d, crate::layer_properties::Target::Vector(layer))
            .position_locked
    {
        return Err("Vector positions are locked".into());
    }
    crate::vector_edit::prepare(d, layer, s, Patch::default())
}
fn finish(
    d: &Document,
    mut state: DocumentState,
    layer: u64,
    ids: Vec<u64>,
) -> Result<DocumentState, String> {
    state.scene.lines.selection = crate::vector_selection::State {
        layer: Some(layer),
        kind: crate::vector_selection::Kind::Strokes,
        paths: ids,
        ..Default::default()
    };
    if !state.scene.valid()
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Refinement exceeds geometry, pressure or Undo limits".into());
    }
    crate::layer_properties::validate_edit(d, &state)?;
    Ok(state)
}
fn raw(path: &mut Path, points: Vec<Point>, parameters: &[f64]) -> Result<(), String> {
    path.pressure = path.fragment_pressure_at(parameters)?;
    path.points = points;
    path.definition = None;
    Ok(())
}
fn simplify(
    path: &mut Path,
    transform: crate::scene::Affine,
    tolerance: f64,
) -> Result<(), String> {
    if tolerance == 0. {
        return Ok(());
    }
    let n = path.points.len();
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0, n - 1)];
    // Closed outlines retain a third anchor; simplifying two arcs also preserves their seam.
    if path.closed {
        keep[n / 2] = true;
        stack = vec![(0, n / 2), (n / 2, n - 1)];
    }
    while let Some((a, b)) = stack.pop() {
        let mut best = (tolerance, None);
        for i in a + 1..b {
            let error = crate::line_tools::distance(
                transform.map(path.points[i]),
                transform.map(path.points[a]),
                transform.map(path.points[b]),
            )
            .0;
            if error > best.0 {
                best = (error, Some(i));
            }
        }
        if let Some(i) = best.1 {
            keep[i] = true;
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    let indices = (0..n).filter(|i| keep[*i]).collect::<Vec<_>>();
    if indices.len() == n {
        return Ok(());
    }
    let points = indices.iter().map(|i| path.points[*i]).collect();
    let parameters = indices
        .iter()
        .map(|i| *i as f64 / (n - 1) as f64)
        .collect::<Vec<_>>();
    raw(path, points, &parameters)
}
fn reversed(mut p: Path) -> Path {
    p.points.reverse();
    p.definition = None;
    p.pressure.device.reverse();
    p.pressure.controls.reverse();
    for positions in [
        &mut p.pressure.device_positions,
        &mut p.pressure.control_positions,
    ] {
        positions.reverse();
        for t in positions {
            *t = 1. - *t;
        }
    }
    p
}
fn knots(count: usize, positions: &[f64]) -> Vec<f64> {
    let mut out = vec![0., 1.];
    if positions.is_empty() {
        if count > 1 {
            out.extend((0..count).map(|n| n as f64 / (count - 1) as f64));
        }
    } else {
        out.extend_from_slice(positions);
    }
    out.sort_by(f64::total_cmp);
    out.dedup();
    out
}
fn join(a: &Path, b: &Path) -> Path {
    let mut out = a.clone();
    out.definition = None;
    let count = a.points.len() + b.points.len() - 1;
    let spans = [
        (a, 0., (a.points.len() - 1) as f64),
        (b, a.points.len() as f64, (b.points.len() - 1) as f64),
    ];
    let mut profile = crate::vector_pressure::Profile {
        device_enabled: a.pressure.device_enabled,
        ..Default::default()
    };
    for (p, offset, scale) in spans {
        if !a.pressure.device.is_empty() || !b.pressure.device.is_empty() {
            for t in knots(p.pressure.device.len(), &p.pressure.device_positions) {
                let exact = if p.pressure.device_positions.is_empty() {
                    p.pressure
                        .device
                        .iter()
                        .enumerate()
                        .find(|(n, _)| {
                            t == if p.pressure.device.len() <= 1 {
                                0.
                            } else {
                                *n as f64 / (p.pressure.device.len() - 1) as f64
                            }
                        })
                        .map(|(_, v)| *v)
                } else {
                    p.pressure
                        .device_positions
                        .iter()
                        .position(|x| *x == t)
                        .map(|n| p.pressure.device[n])
                };
                profile
                    .device
                    .push(exact.unwrap_or_else(|| Some(p.pressure.device_at(t))));
                profile
                    .device_positions
                    .push((offset + scale * t) / count as f64);
            }
        }
        if !a.pressure.controls.is_empty() || !b.pressure.controls.is_empty() {
            for t in knots(p.pressure.controls.len(), &p.pressure.control_positions) {
                profile.controls.push(p.pressure.control_at(t));
                profile
                    .control_positions
                    .push((offset + scale * t) / count as f64);
            }
        }
    }
    out.points.extend_from_slice(&b.points);
    out.pressure = profile;
    out
}
fn curve_points(points: &[Point], curve: Curve, closed: bool) -> Result<Vec<Point>, String> {
    if curve == Curve::StraightLine {
        return Ok(points.to_vec());
    }
    if points.len() > 128 {
        return Err("Curve conversion supports128 anchors; simplify first".into());
    }
    crate::shapes::geometry(
        crate::shapes::Settings {
            kind: crate::shapes::Kind::Bezier,
            curve,
            closed,
            ..Default::default()
        },
        points,
    )
    .map(|g| g.points)
}
pub fn prepare(
    d: &Document,
    layer: u64,
    s: &Selection,
    op: Operation,
) -> Result<DocumentState, String> {
    let geometry = !matches!(
        op,
        Operation::Width(..) | Operation::Opacity(_) | Operation::Tip(_)
    );
    let mut state = start(d, layer, s, geometry)?;
    let ids = s.path_ids();
    let layer_index = state
        .scene
        .lines
        .layers
        .iter()
        .position(|l| l.id == layer)
        .unwrap();
    match op {
        Operation::Join(limit) => {
            if !limit.is_finite() || !(0. ..=1e6).contains(&limit) || ids.len() != 2 {
                return Err(
                    "Select two open strokes and a finite maximum endpoint distance".into(),
                );
            }
            let paths = &state.scene.lines.layers[layer_index].paths;
            let a = paths.iter().find(|p| ids.contains(&p.id)).unwrap();
            let b = paths
                .iter()
                .find(|p| p.id != a.id && ids.contains(&p.id))
                .unwrap();
            if a.closed
                || b.closed
                || a.fill.is_some()
                || b.fill.is_some()
                || a.stroke != b.stroke
                || a.tip != b.tip
                || a.svg != b.svg
                || a.pressure.device_enabled != b.pressure.device_enabled
            {
                return Err(
                    "Join requires open strokes with matching tip and device-pressure mode".into(),
                );
            }
            let mut best = (f64::INFINITY, false, false);
            for ra in [false, true] {
                for rb in [false, true] {
                    let pa = a.points[if ra { 0 } else { a.points.len() - 1 }];
                    let pb = b.points[if rb { b.points.len() - 1 } else { 0 }];
                    let len = distance(d.scene.transform.map(pa), d.scene.transform.map(pb));
                    if len < best.0 {
                        best = (len, ra, rb);
                    }
                }
            }
            if best.0 > limit {
                return Err("Nearest endpoints exceed maximum join distance".into());
            }
            let a = if best.1 {
                reversed(a.clone())
            } else {
                a.clone()
            };
            let b = if best.2 {
                reversed(b.clone())
            } else {
                b.clone()
            };
            if a.width != b.width || a.color != b.color {
                // Keep each source segment, its editable curve and pressure exactly.
                // An independent tapered connector avoids flattening either style.
                let mut selected = vec![a.id, b.id];
                if best.0 > 0. {
                    let id = state.scene.lines.id()?;
                    let mut bridge = a.clone();
                    bridge.id = id;
                    bridge.definition = None;
                    bridge.svg = None;
                    bridge.points = vec![*a.points.last().unwrap(), b.points[0]];
                    let wa = a.width_at(1.);
                    let wb = b.width_at(0.);
                    bridge.width = wa.max(wb).clamp(1e-12, 512.);
                    bridge.pressure = crate::vector_pressure::Profile {
                        device_enabled: false,
                        controls: vec![wa / bridge.width, wb / bridge.width],
                        ..Default::default()
                    };
                    let paths = &mut state.scene.lines.layers[layer_index].paths;
                    let index = paths.iter().position(|p| p.id == a.id).unwrap() + 1;
                    paths.insert(index, bridge);
                    selected.push(id);
                }
                return finish(d, state, layer, selected);
            }
            let joined = join(&a, &b);
            let paths = &mut state.scene.lines.layers[layer_index].paths;
            paths.retain(|p| p.id != b.id);
            *paths.iter_mut().find(|p| p.id == a.id).unwrap() = joined;
            return finish(d, state, layer, vec![a.id]);
        }
        Operation::Split => {
            if ids.len() != 1 || s.controls.len() != 1 {
                return Err("Select one interior control point to split the line".into());
            }
            let control = s.controls[0];
            let new_id = state.scene.lines.id()?;
            let paths = &mut state.scene.lines.layers[layer_index].paths;
            let path = paths.iter_mut().find(|p| p.id == control.path).unwrap();
            if path.fill.is_some() {
                return Err("Split a stroke without a filled interior".into());
            }
            let index = (path.control_parameters()?[control.point as usize]
                * (path.points.len() - 1) as f64)
                .round() as usize;
            let original = path.clone();
            let n = path.points.len();
            if path.closed {
                let indices = (index..=index + n).map(|i| i % n).collect::<Vec<_>>();
                let parameters = (index..=index + n)
                    .map(|i| i as f64 / (n - 1) as f64)
                    .collect::<Vec<_>>();
                raw(
                    path,
                    indices.iter().map(|i| original.points[*i]).collect(),
                    &parameters,
                )?;
                path.closed = false;
                return finish(d, state, layer, vec![original.id]);
            }
            if index == 0 || index + 1 == n {
                return Err("Choose an interior point, not an endpoint".into());
            }
            let mut other = original.clone();
            other.id = new_id;
            raw(
                path,
                original.points[..=index].to_vec(),
                &(0..=index)
                    .map(|i| i as f64 / (n - 1) as f64)
                    .collect::<Vec<_>>(),
            )?;
            raw(
                &mut other,
                original.points[index..].to_vec(),
                &(index..n)
                    .map(|i| i as f64 / (n - 1) as f64)
                    .collect::<Vec<_>>(),
            )?;
            paths.push(other);
            return finish(d, state, layer, vec![original.id, new_id]);
        }
        _ => {}
    }
    for p in state.scene.lines.layers[layer_index]
        .paths
        .iter_mut()
        .filter(|p| ids.contains(&p.id))
    {
        match op {
            Operation::Simplify(t) => {
                if !t.is_finite() || !(0. ..=1e6).contains(&t) {
                    return Err("Tolerance must be finite and nonnegative".into());
                }
                simplify(p, d.scene.transform, t)?;
            }
            Operation::Convert(curve) => {
                let controls = (0..p.control_count())
                    .filter_map(|i| p.control_point(i))
                    .collect::<Vec<_>>();
                let points = curve_points(&controls, curve, p.closed)?;
                let parameters = (0..points.len())
                    .map(|i| i as f64 / (points.len() - 1) as f64)
                    .collect::<Vec<_>>();
                raw(p, points, &parameters)?;
            }
            Operation::Width(mode, value, minimum) => {
                if !value.is_finite()
                    || !(0. ..=512.).contains(&value)
                    || matches!(mode, Width::ScaleUp | Width::ScaleDown) && value < 1.
                {
                    return Err("Width requires0–512; scale factor must be at least1".into());
                }
                let width = match mode {
                    Width::Thicken => p.width + value,
                    Width::Narrow => p.width - value,
                    Width::ScaleUp => p.width * value,
                    Width::ScaleDown => p.width / value,
                    Width::Fix => value,
                };
                // The minimum is the base width in document pixels; pressure remains independent.
                let minimum = if minimum {
                    let inverse = crate::line_tools::maximum_scale(d.scene.transform.inverse());
                    if !inverse.is_finite() {
                        return Err("Invalid width transform".into());
                    }
                    inverse
                } else {
                    0.
                };
                if minimum > 512. {
                    return Err("One document pixel exceeds the stored width limit".into());
                }
                p.width = width.clamp(minimum, 512.);
            }
            Operation::Opacity(alpha) => {
                p.color[3] = alpha;
                if !p.stroke
                    && p.svg.is_some()
                    && let Some(fill) = &mut p.fill
                {
                    fill[3] = alpha;
                }
            }
            Operation::Tip(tip) => p.tip = tip,
            Operation::Join(_) | Operation::Split => unreachable!(),
        }
    }
    finish(d, state, layer, ids.into_iter().collect())
}
fn fixed(ends: Ends, start: Point, a: Point, b: Point) -> (bool, bool) {
    match ends {
        Ends::Both => (true, true),
        Ends::Free => (false, false),
        Ends::Either => {
            let first = distance(start, a) <= distance(start, b);
            (first, !first)
        }
    }
}
pub fn push(
    d: &Document,
    layer: u64,
    s: &Selection,
    center: Point,
    delta: Point,
    radius: f64,
    ends: Ends,
) -> Result<DocumentState, String> {
    if !radius.is_finite()
        || !(0.1..=1e6).contains(&radius)
        || !crate::line_tools::point_valid(center)
        || !crate::line_tools::point_valid(delta)
    {
        return Err("Invalid local push radius or movement".into());
    }
    let mut state = start(d, layer, s, true)?;
    let ids = s.path_ids();
    for p in state
        .scene
        .lines
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .unwrap()
        .paths
        .iter_mut()
        .filter(|p| ids.contains(&p.id))
    {
        let n = p.points.len();
        let (a, b) = fixed(
            ends,
            center,
            d.scene.transform.map(p.points[0]),
            d.scene.transform.map(p.points[n - 1]),
        );
        if delta == [0.; 2] {
            continue;
        }
        let original = p.clone();
        let mut points = vec![];
        let mut parameters = vec![];
        for i in 0..if p.closed { n } else { n - 1 } {
            let from = original.points[i];
            let to = original.points[(i + 1) % n];
            let x = d.scene.transform.map(from);
            let y = d.scene.transform.map(to);
            let length = distance(x, y);
            let mut fractions = vec![0.];
            if length > 0. {
                let unit = [(y[0] - x[0]) / length, (y[1] - x[1]) / length];
                let q = [center[0] - x[0], center[1] - x[1]];
                let along = q[0] * unit[0] + q[1] * unit[1];
                let across = q[0] * unit[1] - q[1] * unit[0];
                if across.abs() < radius {
                    let reach = ((radius - across.abs()) * (radius + across.abs())).sqrt();
                    let begin = ((along - reach) / length).clamp(0., 1.);
                    let end = ((along + reach) / length).clamp(0., 1.);
                    if end > begin {
                        fractions.extend((0..=16).map(|j| begin + (end - begin) * j as f64 / 16.));
                        fractions.push((along / length).clamp(begin, end));
                    }
                }
            }
            fractions.retain(|t| *t < 1.);
            fractions.sort_by(f64::total_cmp);
            fractions.dedup_by(|a, b| (*a - *b).abs() < 1e-14);
            for t in fractions {
                if points.len() >= 4096 {
                    return Err("Local push exceeds4096 outline points; simplify first".into());
                }
                points.push([
                    from[0] + (to[0] - from[0]) * t,
                    from[1] + (to[1] - from[1]) * t,
                ]);
                parameters.push((i as f64 + t) / (n - 1) as f64);
            }
        }
        if !p.closed {
            points.push(*original.points.last().unwrap());
            parameters.push(1.);
        }
        let mut changed = false;
        for (point, t) in points.iter_mut().zip(&parameters) {
            if !p.closed && (*t == 0. && a || *t == 1. && b) {
                continue;
            }
            let weight =
                (1. - distance(d.scene.transform.map(*point), center) / radius).clamp(0., 1.);
            if weight > 1e-12 {
                let v = d
                    .scene
                    .transform
                    .inverse()
                    .vector([delta[0] * weight, delta[1] * weight]);
                point[0] += v[0];
                point[1] += v[1];
                changed = true;
            }
        }
        if changed {
            raw(p, points, &parameters)?;
        }
    }
    finish(d, state, layer, ids.into_iter().collect())
}
pub fn redraw(
    d: &Document,
    layer: u64,
    s: &Selection,
    trace: &[Point],
    ends: Ends,
    curve: Curve,
) -> Result<DocumentState, String> {
    if !(2..=128).contains(&trace.len())
        || trace.iter().any(|p| !crate::line_tools::point_valid(*p))
        || s.path_ids().len() != 1
        || s.controls.is_empty()
    {
        return Err("Redraw needs one path, a start control and2–128 trace points".into());
    }
    let mut state = start(d, layer, s, true)?;
    let id = s.controls[0].path;
    let p = state
        .scene
        .lines
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .unwrap()
        .paths
        .iter_mut()
        .find(|p| p.id == id)
        .unwrap();
    let params = p.control_parameters()?;
    let mut indices = s
        .controls
        .iter()
        .map(|c| c.point as usize)
        .collect::<Vec<_>>();
    if indices.len() == 1 {
        let last = *trace.last().unwrap();
        let near = (0..p.control_count())
            .min_by(|a, b| {
                distance(d.scene.transform.map(p.control_point(*a).unwrap()), last).total_cmp(
                    &distance(d.scene.transform.map(p.control_point(*b).unwrap()), last),
                )
            })
            .unwrap();
        indices.push(near);
    }
    indices.sort_unstable();
    let a = (params[indices[0]] * (p.points.len() - 1) as f64).round() as usize;
    let b = (params[*indices.last().unwrap()] * (p.points.len() - 1) as f64).round() as usize;
    if a == b {
        return Err("Drag to a different control or select two boundary controls".into());
    }
    let original = p.clone();
    let start = trace[0];
    let mut trace = trace
        .iter()
        .map(|v| d.scene.transform.inverse().map(*v))
        .collect::<Vec<_>>();
    // Orient the captured trace to the selected span, including reverse drags.
    if distance(trace[0], p.points[b]) < distance(trace[0], p.points[a]) {
        trace.reverse();
    }
    let (fa, fb) = fixed(
        ends,
        start,
        d.scene.transform.map(p.points[a]),
        d.scene.transform.map(p.points[b]),
    );
    // Interior boundaries always remain attached to the unselected geometry.
    if fa || a > 0 || p.closed {
        trace[0] = p.points[a];
    }
    let last = trace.len() - 1;
    if fb || b + 1 < p.points.len() || p.closed {
        trace[last] = p.points[b];
    }
    let replacement = curve_points(&trace, curve, false)?;
    let mut points = original.points[..a].to_vec();
    points.extend_from_slice(&replacement);
    points.extend_from_slice(&original.points[b + 1..]);
    let count = (original.points.len() - 1) as f64;
    let mut parameters = (0..a).map(|i| i as f64 / count).collect::<Vec<_>>();
    parameters.extend(
        (0..replacement.len()).map(|i| {
            (a as f64 + (b - a) as f64 * i as f64 / (replacement.len() - 1) as f64) / count
        }),
    );
    parameters.extend((b + 1..original.points.len()).map(|i| i as f64 / count));
    raw(p, points, &parameters)?;
    finish(d, state, layer, vec![id])
}
