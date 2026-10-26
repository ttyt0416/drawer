//! A cut is a simple open polyline with exactly two frame boundary crossings.
use crate::{
    comics::*,
    document::Document,
    layer_search::Key,
    scene::{PageObject, Point},
    sizing::DocumentState,
};
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
pub fn simple(poly: &[Point]) -> bool {
    if poly.len() < 3 || area(poly).abs() < 1e-6 {
        return false;
    }
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        if sub(a, b)[0].hypot(sub(a, b)[1]) < 1e-9 {
            return false;
        }
        for j in i + 1..poly.len() {
            if j == i + 1 || (i == 0 && j == poly.len() - 1) {
                continue;
            }
            let c = poly[j];
            let d = poly[(j + 1) % poly.len()];
            let orient = |p: Point, q: Point, r: Point| cross(sub(q, p), sub(r, p));
            if orient(a, b, c) * orient(a, b, d) < 0. && orient(c, d, a) * orient(c, d, b) < 0. {
                return false;
            }
        }
    }
    true
}
pub fn prepare(d: &Document, id: u64, path: &[Point]) -> Result<DocumentState, String> {
    if path.len() < 2 || path.len() > 64 || path.iter().flatten().any(|v| !v.is_finite()) {
        return Err("Cut needs 2–64 finite points".into());
    }
    let n = d
        .scene
        .catalog
        .objects
        .index(id, d.scene.objects.len())
        .ok_or("Frame removed")?;
    let PageObject::Comic(object) = &d.scene.objects[n] else {
        return Err("Select a frame folder".into());
    };
    let Kind::Frame(frame) = &object.kind else {
        return Err("Select a frame folder".into());
    };
    if !frame.compound.is_empty() || !frame.component_masks.is_empty() {
        return Err(
            "Use equal division for a compound frame, or edit its component polygons first".into(),
        );
    }
    crate::layer_groups::preflight(
        d,
        &[Key::Object(id)],
        crate::layer_groups::Operation::Transform,
    )?;
    if contains(&frame.polygon, path[0]) || contains(&frame.polygon, *path.last().unwrap()) {
        return Err("Start and end the cut outside the frame".into());
    }
    let mut hits = vec![];
    for (segment, line) in path.windows(2).enumerate() {
        let a = line[0];
        let u = sub(line[1], a);
        if u[0].hypot(u[1]) < 1e-8 {
            return Err("Zero-length cut segment".into());
        }
        for (edge, (b, c)) in frame
            .polygon
            .iter()
            .zip(frame.polygon.iter().cycle().skip(1))
            .take(frame.polygon.len())
            .enumerate()
        {
            let v = sub(*c, *b);
            let den = cross(u, v);
            if den.abs() < 1e-10 {
                continue;
            }
            let delta = sub(*b, a);
            let t = cross(delta, v) / den;
            let q = cross(delta, u) / den;
            if (0. ..=1.).contains(&t) && (0. ..=1.).contains(&q) {
                hits.push((segment as f64 + t, edge, [a[0] + t * u[0], a[1] + t * u[1]]));
            }
        }
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    hits.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-8);
    if hits.len() != 2 {
        return Err("The cut must cross the frame exactly twice".into());
    }
    let (a, b) = (hits[0], hits[1]);
    let mut cut = vec![a.2];
    for (i, p) in path.iter().enumerate() {
        if i as f64 > a.0 && (i as f64) < b.0 {
            cut.push(*p);
        }
    }
    cut.push(b.2);
    let boundary = |from: (f64, usize, Point), to: (f64, usize, Point)| {
        let mut out = vec![from.2];
        let mut edge = from.1;
        let mut steps = (to.1 + frame.polygon.len() - from.1) % frame.polygon.len();
        if steps == 0 {
            let a = frame.polygon[from.1];
            let b = frame.polygon[(from.1 + 1) % frame.polygon.len()];
            let v = sub(b, a);
            let parameter = |p: Point| (p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1];
            if parameter(from.2) > parameter(to.2) {
                steps = frame.polygon.len();
            }
        }
        for _ in 0..steps {
            edge = (edge + 1) % frame.polygon.len();
            out.push(frame.polygon[edge]);
        }
        out.push(to.2);
        out
    };
    let mut left = boundary(a, b);
    left.extend(
        cut.iter()
            .rev()
            .skip(1)
            .take(cut.len().saturating_sub(2))
            .copied(),
    );
    let mut right = boundary(b, a);
    right.extend(
        cut.iter()
            .skip(1)
            .take(cut.len().saturating_sub(2))
            .copied(),
    );
    if !simple(&left)
        || !simple(&right)
        || (area(&left).abs() + area(&right).abs() - area(&frame.polygon).abs()).abs()
            > 1e-5 * area(&frame.polygon).abs().max(1.)
    {
        return Err("Cut creates overlapping or empty panels".into());
    }
    let mut state = d.snapshot();
    state
        .scene
        .catalog
        .objects
        .initialize(state.scene.objects.len())?;
    for (i, polygon) in [left, right].into_iter().enumerate() {
        let mut child = (**object).clone();
        let Kind::Frame(f) = &mut child.kind else {
            unreachable!()
        };
        f.polygon = polygon;
        f.hidden_edges.clear();
        f.outline.clear();
        f.family = if frame.family == 0 { id } else { frame.family };
        child.name = format!(
            "{} {}",
            object.name.chars().take(60).collect::<String>(),
            i + 1
        );
        if i == 0 {
            state.scene.objects[n] = PageObject::Comic(Box::new(child));
        } else {
            state.scene.objects.push(PageObject::Comic(Box::new(child)));
            let count = state.scene.objects.len();
            state.scene.catalog.objects.append(count)?;
        }
    }
    crate::layer_catalog::reconcile(d, &mut state)?;
    crate::render_order::reconcile(&mut state);
    crate::layer_groups::finish(d, &state)?;
    Ok(state)
}
