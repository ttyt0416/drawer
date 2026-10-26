//! Compound frame unions, equal strips and coincident-border visibility.
use crate::{
    comic_object::{Frame, contains as inside, distance, polygon_bounds},
    comics::{Kind, Object},
    document::Document,
    layer_search::Key,
    scene::{PageObject, Point, Rect},
    sizing::DocumentState,
};
pub fn rings(f: &Frame) -> impl Iterator<Item = &Vec<Point>> {
    std::iter::once(&f.polygon).chain(&f.compound)
}
pub fn contains(f: &Frame, p: Point) -> bool {
    rings(f).enumerate().any(|(n, r)| {
        inside(r, p)
            && f.component_masks
                .get(n)
                .is_none_or(|masks| masks.iter().all(|mask| inside(mask, p)))
    })
}
pub fn bounds(f: &Frame) -> Rect {
    let mut result: Option<Rect> = None;
    for (n, r) in rings(f).enumerate() {
        let mut area = polygon_bounds(r);
        if let Some(masks) = f.component_masks.get(n) {
            for mask in masks {
                area = area.intersection(polygon_bounds(mask));
            }
        }
        if area.max[0] > area.min[0] && area.max[1] > area.min[1] {
            result = Some(result.map_or(area, |old| Rect {
                min: [old.min[0].min(area.min[0]), old.min[1].min(area.min[1])],
                max: [old.max[0].max(area.max[0]), old.max[1].max(area.max[1])],
            }));
        }
    }
    result.unwrap_or(Rect {
        min: [0.; 2],
        max: [0.; 2],
    })
}
fn edges(poly: &[Point]) -> impl Iterator<Item = [Point; 2]> + '_ {
    poly.iter()
        .copied()
        .zip(poly.iter().copied().cycle().skip(1))
        .take(poly.len())
        .map(|(a, b)| [a, b])
}
pub fn edge_distance(f: &Frame, p: Point) -> f64 {
    if f.outline.is_empty() {
        crate::comic_object::edge_distance(&f.polygon, p)
    } else {
        f.outline
            .iter()
            .map(|e| distance(p, e[0], e[1]))
            .fold(f64::INFINITY, f64::min)
    }
}
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn point(a: Point, v: Point, t: f64) -> Point {
    [a[0] + v[0] * t, a[1] + v[1] * t]
}
fn boundary(frame: &Frame) -> Result<Vec<[Point; 2]>, String> {
    let all = rings(frame)
        .chain(frame.component_masks.iter().flatten())
        .flat_map(|p| edges(p))
        .collect::<Vec<_>>();
    if all.len() > 2048 {
        return Err("Compound frame edge limit2048".into());
    }
    let mut out = vec![];
    for [a, b] in &all {
        let v = sub(*b, *a);
        let length = v[0].hypot(v[1]);
        if length < 1e-9 {
            continue;
        }
        let mut ts = vec![0., 1.];
        for [c, d] in &all {
            let u = sub(*d, *c);
            let den = cross(v, u);
            if den.abs() > 1e-10 {
                let delta = sub(*c, *a);
                let t = cross(delta, u) / den;
                let q = cross(delta, v) / den;
                if (0. ..=1.).contains(&t) && (0. ..=1.).contains(&q) {
                    ts.push(t);
                }
            } else if cross(sub(*c, *a), v).abs() < 1e-7 {
                for p in [c, d] {
                    let t = ((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / (length * length);
                    if (0. ..=1.).contains(&t) {
                        ts.push(t);
                    }
                }
            }
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|a, b| (*a - *b).abs() < 1e-8);
        for t in ts.windows(2) {
            if t[1] - t[0] < 1e-8 {
                continue;
            }
            let m = point(*a, v, (t[0] + t[1]) / 2.);
            let normal = [-v[1] / length * 1e-6, v[0] / length * 1e-6];
            let contains = |p| self::contains(frame, p);
            if contains([m[0] + normal[0], m[1] + normal[1]])
                != contains([m[0] - normal[0], m[1] - normal[1]])
            {
                out.push([point(*a, v, t[0]), point(*a, v, t[1])]);
                if out.len() > 4096 {
                    return Err("Compound outline limit4096".into());
                }
            }
        }
    }
    Ok(out)
}
pub fn refresh(f: &mut Frame) -> Result<(), String> {
    if f.compound.is_empty() && f.component_masks.is_empty() {
        f.outline.clear();
    } else {
        f.outline = boundary(f)?;
        if f.outline.is_empty() {
            return Err("Frame clipping leaves no visible area".into());
        }
    }
    Ok(())
}
fn selected<'a>(d: &'a Document, ids: &[u64]) -> Result<Vec<(usize, &'a Object)>, String> {
    if ids.is_empty()
        || ids.len() > 32
        || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len()
    {
        return Err("Choose 1..32 distinct frames".into());
    }
    let keys = ids.iter().map(|id| Key::Object(*id)).collect::<Vec<_>>();
    crate::layer_groups::preflight(d, &keys, crate::layer_groups::Operation::Transform)?;
    ids.iter()
        .map(|id| {
            let n = d
                .scene
                .catalog
                .objects
                .index(*id, d.scene.objects.len())
                .ok_or("Frame removed")?;
            if let PageObject::Comic(o) = &d.scene.objects[n]
                && matches!(o.kind, Kind::Frame(_))
            {
                Ok((n, o.as_ref()))
            } else {
                Err("Choose frame folders".into())
            }
        })
        .collect()
}
fn finish(d: &Document, mut s: DocumentState) -> Result<DocumentState, String> {
    crate::layer_catalog::reconcile(d, &mut s)?;
    crate::comics::prune(&mut s);
    crate::render_order::reconcile(&mut s);
    crate::layer_groups::finish(d, &s)?;
    Ok(s)
}
pub fn merge(d: &Document, ids: &[u64]) -> Result<DocumentState, String> {
    let chosen = selected(d, ids)?;
    if chosen.len() < 2 || chosen.len() > 32 {
        return Err("Choose 2..32 frames to combine".into());
    }
    let mut object = chosen[0].1.clone();
    let inv = object.transform.inverse();
    let mut polygons = vec![];
    let mut component_masks = vec![];
    let mut children = vec![];
    for (index, o) in &chosen {
        let Kind::Frame(f) = &o.kind else {
            unreachable!()
        };
        let matrix = o.transform.then(inv);
        let id = d.scene.catalog.objects.id(*index);
        let own = d
            .scene
            .comics
            .masks
            .iter()
            .filter(|m| m.target == Key::Object(id))
            .map(|m| {
                m.polygon
                    .iter()
                    .map(|p| m.space.then(inv).map(*p))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for (n, poly) in rings(f).enumerate() {
            polygons.push(poly.iter().map(|p| matrix.map(*p)).collect::<Vec<_>>());
            let mut masks = f.component_masks.get(n).map_or(vec![], |m| {
                m.iter()
                    .map(|m| m.iter().map(|p| matrix.map(*p)).collect())
                    .collect()
            });
            masks.extend(own.iter().cloned());
            component_masks.push(masks);
        }
        for k in &f.children {
            if !children.contains(k) {
                children.push(*k);
            }
        }
    }
    if polygons.len() > 32 {
        return Err("Combined contour limit32".into());
    }
    let Kind::Frame(f) = &mut object.kind else {
        unreachable!()
    };
    f.polygon = polygons.remove(0);
    f.compound = polygons;
    f.component_masks = component_masks;
    f.children = children;
    f.hidden_edges.clear();
    refresh(f)?;
    let keep = ids[0];
    let mut s = d.snapshot();
    s.scene
        .comics
        .masks
        .retain(|m| !matches!(m.target,Key::Object(id)if ids.contains(&id)));
    s.scene.catalog.objects.initialize(s.scene.objects.len())?;
    s.scene.objects[chosen[0].0] = PageObject::Comic(Box::new(object));
    let mut removed = chosen.iter().skip(1).map(|(n, _)| *n).collect::<Vec<_>>();
    removed.sort_unstable();
    for n in removed.into_iter().rev() {
        s.scene.catalog.objects.remove(n, s.scene.objects.len())?;
        s.scene.objects.remove(n);
    }
    for o in &mut s.scene.objects {
        if let PageObject::Comic(o) = o
            && let Kind::Frame(f) = &mut o.kind
        {
            for k in &mut f.children {
                if matches!(k,Key::Object(n) if ids.contains(n) && *n!=keep) {
                    *k = Key::Object(keep);
                }
            }
            let mut seen = std::collections::BTreeSet::new();
            f.children.retain(|k| seen.insert(*k));
        }
    }
    finish(d, s)
}
pub fn divide(d: &Document, id: u64, count: u32, vertical: bool) -> Result<DocumentState, String> {
    if !(2..=32).contains(&count) {
        return Err("Choose 2..32 equal frame divisions".into());
    }
    let chosen = selected(d, &[id])?;
    let (n, o) = chosen[0];
    let Kind::Frame(frame) = &o.kind else {
        unreachable!()
    };
    let b = bounds(frame);
    let axis = if vertical { 0 } else { 1 };
    let mut s = d.snapshot();
    s.scene.catalog.objects.initialize(s.scene.objects.len())?;
    for i in 0..count {
        let lo = b.min[axis] + (b.max[axis] - b.min[axis]) * i as f64 / count as f64;
        let hi = b.min[axis] + (b.max[axis] - b.min[axis]) * (i + 1) as f64 / count as f64;
        let mut masks = frame.component_masks.clone();
        if masks.is_empty() {
            masks = vec![vec![]; 1 + frame.compound.len()];
        }
        let mut strip = b;
        strip.min[axis] = lo;
        strip.max[axis] = hi;
        let polygon = vec![
            strip.min,
            [strip.max[0], strip.min[1]],
            strip.max,
            [strip.min[0], strip.max[1]],
        ];
        let own = d
            .scene
            .comics
            .masks
            .iter()
            .filter(|m| m.target == Key::Object(id))
            .map(|m| {
                m.polygon
                    .iter()
                    .map(|p| m.space.then(o.transform.inverse()).map(*p))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for group in &mut masks {
            group.extend(own.iter().cloned());
            group.push(polygon.clone());
        }
        let mut child = o.clone();
        let Kind::Frame(f) = &mut child.kind else {
            unreachable!()
        };
        f.component_masks = masks;
        f.family = if frame.family == 0 { id } else { frame.family };
        f.hidden_edges.clear();
        refresh(f)?;
        child.name = format!("{} {}", o.name.chars().take(56).collect::<String>(), i + 1);
        if i == 0 {
            s.scene.objects[n] = PageObject::Comic(Box::new(child));
        } else {
            s.scene.objects.push(PageObject::Comic(Box::new(child)));
            s.scene.catalog.objects.append(s.scene.objects.len())?;
        }
    }
    s.scene.comics.masks.retain(|m| m.target != Key::Object(id));
    finish(d, s)
}
pub fn hide_overlaps(d: &Document, ids: &[u64], hide: bool) -> Result<DocumentState, String> {
    let chosen = selected(d, ids)?;
    let mut s = d.snapshot();
    for (n, o) in &chosen {
        let Kind::Frame(f) = &o.kind else {
            unreachable!()
        };
        let mut hidden = vec![];
        if hide {
            let inv = o.transform.inverse();
            let own = if f.outline.is_empty() {
                edges(&f.polygon).collect::<Vec<_>>()
            } else {
                f.outline.clone()
            };
            for [a, b] in own {
                let v = sub(b, a);
                let len = v[0] * v[0] + v[1] * v[1];
                for (other, object) in &chosen {
                    if n == other {
                        continue;
                    }
                    let Kind::Frame(frame) = &object.kind else {
                        unreachable!()
                    };
                    let matrix = object.transform.then(inv);
                    let other_edges = if frame.outline.is_empty() {
                        edges(&frame.polygon).collect::<Vec<_>>()
                    } else {
                        frame.outline.clone()
                    };
                    for [c, d] in other_edges {
                        let c = matrix.map(c);
                        let d = matrix.map(d);
                        if cross(v, sub(c, a)).abs() > 1e-6 || cross(v, sub(d, a)).abs() > 1e-6 {
                            continue;
                        }
                        let project =
                            |p: Point| ((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / len;
                        let (c, d) = (project(c), project(d));
                        let lo = c.min(d).max(0.);
                        let hi = c.max(d).min(1.);
                        if hi - lo > 1e-8 {
                            hidden.push([point(a, v, lo), point(a, v, hi)]);
                        }
                    }
                }
            }
        }
        if let PageObject::Comic(o) = &mut s.scene.objects[*n]
            && let Kind::Frame(f) = &mut o.kind
        {
            f.hidden_edges = hidden;
        }
    }
    finish(d, s)
}
pub fn requires94(s: &crate::scene::Scene) -> bool {
    s.objects.iter().any(|o|matches!(o,PageObject::Comic(o)if matches!(&o.kind,Kind::Frame(f)if !f.compound.is_empty() || !f.component_masks.is_empty() || !f.outline.is_empty() || !f.hidden_edges.is_empty())))
}
