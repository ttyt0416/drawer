//! Document-space width and pivot policies; viewing never changes stored geometry.
use crate::{
    document::Document,
    layer_search::Key,
    scene::{Point, Rect},
};
pub fn center(points: impl IntoIterator<Item = Point>) -> Result<Point, String> {
    let mut r = Rect {
        min: [f64::INFINITY; 2],
        max: [f64::NEG_INFINITY; 2],
    };
    for p in points {
        for (n, v) in p.into_iter().enumerate() {
            r.min[n] = r.min[n].min(v);
            r.max[n] = r.max[n].max(v);
        }
    }
    if !r.valid() {
        return Err("No finite selected geometry".into());
    }
    Ok([0, 1].map(|n| (r.min[n] + r.max[n]) * 0.5))
}
pub fn group_center(d: &Document, key: Key) -> Result<Point, String> {
    let keys = crate::layer_groups::expanded(d, &[key], crate::layer_groups::Operation::Transform)?;
    let members = crate::layer_groups::content_members(d, &crate::layer_groups::roots(d, &keys))?;
    center(
        members
            .into_iter()
            .filter_map(|k| crate::layer_catalog::bounds(d, k))
            .flat_map(crate::transform_handles::corners),
    )
}
pub fn frame(d: &Document, key: Key) -> crate::scene::Affine {
    if let Some(frame) = d.scene.layer_groups.frame(key) {
        return frame;
    }
    if matches!(key, Key::Layer(crate::layer_properties::Target::Vector(_))) {
        return d.scene.transform;
    }
    if let Some(m) = d.scene.file_objects.get(key).map(|l| &l.cache).or_else(|| {
        matches!(key, Key::Object(_))
            .then(|| crate::layer_catalog::source_index(d, key))
            .flatten()
            .and_then(|n| d.scene.objects.get(n))
            .and_then(|o| {
                if let crate::scene::PageObject::Material(m) = o {
                    Some(&**m)
                } else {
                    None
                }
            })
    }) {
        return m.transform.then(d.scene.transform);
    }
    crate::scene::Affine::default()
}
pub fn oriented(d: &Document, key: Key) -> Result<(Rect, f64), String> {
    let frame = frame(d, key);
    let angle = frame.0[1].atan2(frame.0[0]).to_degrees();
    let inverse = crate::transform_handles::basis(angle).inverse();
    let r = crate::layer_catalog::bounds(d, key).ok_or("Missing transform bounds")?;
    let mut points = crate::transform_handles::corners(r)
        .map(|p| inverse.map(p))
        .to_vec();
    if let Key::Layer(crate::layer_properties::Target::Vector(id)) = key {
        let to_local = d.scene.transform.then(inverse);
        let geometry = d
            .scene
            .lines
            .layers
            .iter()
            .filter(|l| l.id == id)
            .flat_map(|l| &l.paths)
            .flat_map(|path| {
                let radius = path.maximum_width() * 0.5;
                let extent = [
                    radius * to_local.0[0].hypot(to_local.0[2]),
                    radius * to_local.0[1].hypot(to_local.0[3]),
                ];
                path.points.iter().flat_map(move |p| {
                    let p = to_local.map(*p);
                    crate::transform_handles::corners(Rect {
                        min: [0, 1].map(|k| p[k] - extent[k]),
                        max: [0, 1].map(|k| p[k] + extent[k]),
                    })
                })
            })
            .collect::<Vec<_>>();
        if !geometry.is_empty() {
            points = geometry;
        }
    } else if let Some(m) = d.scene.file_objects.get(key).map(|l| &l.cache).or_else(|| {
        matches!(key, Key::Object(_))
            .then(|| crate::layer_catalog::source_index(d, key))
            .flatten()
            .and_then(|n| d.scene.objects.get(n))
            .and_then(|o| {
                if let crate::scene::PageObject::Material(m) = o {
                    Some(&**m)
                } else {
                    None
                }
            })
    }) {
        points =
            m.mapping
                .warp
                .points()
                .into_iter()
                .map(|p| {
                    inverse.map(d.scene.transform.map(m.transform.map(
                        [0, 1].map(|k| m.area.min[k] + p[k] * (m.area.max[k] - m.area.min[k])),
                    )))
                })
                .collect();
    }
    let mut bounds = Rect {
        min: [f64::INFINITY; 2],
        max: [f64::NEG_INFINITY; 2],
    };
    for p in points {
        for (n, v) in p.into_iter().enumerate() {
            bounds.min[n] = bounds.min[n].min(v);
            bounds.max[n] = bounds.max[n].max(v);
        }
    }
    if !bounds.valid() {
        return Err("Missing transform geometry".into());
    }
    for n in 0..2 {
        if bounds.max[n] - bounds.min[n] < 1e-9 {
            bounds.min[n] -= 0.5;
            bounds.max[n] += 0.5;
        }
    }
    Ok((bounds, angle))
}
