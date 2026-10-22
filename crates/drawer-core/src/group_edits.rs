//! Explicit linked transforms and ink filters; unsupported members reject the whole command.
use crate::{
    document::{Document, PlacedRaster},
    layer_catalog as c,
    layer_groups::{self as groups, Operation},
    layer_properties::Target,
    layer_search::Key,
    scene::{Affine, PageObject, Point},
    sizing::DocumentState,
};
pub fn transform(d: &Document, selected: &[Key], matrix: Affine) -> Result<DocumentState, String> {
    if !matrix.valid() {
        return Err("Invalid group transform".into());
    }
    let keys = groups::expanded(d, selected, Operation::Transform)?;
    groups::preflight(d, &keys, Operation::Transform)?;
    if matrix.is_identity() {
        return Ok(d.snapshot());
    }
    let members = groups::content_members(d, &groups::roots(d, &keys))?;
    let original = d
        .scene
        .transform
        .then(matrix)
        .then(d.scene.transform.inverse());
    let a = [original.0[0], original.0[1]];
    let b = [original.0[2], original.0[3]];
    let scale = a[0].hypot(a[1]);
    let similarity = (scale - b[0].hypot(b[1])).abs() < 1e-8 * scale.max(1.)
        && (a[0] * b[0] + a[1] * b[1]).abs() < 1e-8 * scale.max(1.).powi(2);
    for key in &members {
        if d.scene.file_objects.get(*key).is_some() {
            continue;
        }
        match c::target(d, *key).ok_or("Missing transform member")? {
            Target::Raster(n) => {
                if crate::layer_effects::adjustment(d, Target::Raster(n)).is_some() {
                    return Err(
                        "Rasterize corrections with their backdrop before transforming a group"
                            .into(),
                    );
                }
            }
            Target::Folder(_) => {}
            Target::Vector(_) => {
                if !similarity {
                    return Err("This page transform would distort vector stroke widths; rasterize this vector before applying the transform".into());
                }
            }
            Target::Gradient(_) => {}
            Target::Object(id) => {
                let n = c::source_index(d, Key::Object(id)).ok_or("Missing object")?;
                if !matches!(
                    d.scene.objects[n],
                    PageObject::Material(_) | PageObject::Line { .. }
                ) || !similarity
                {
                    return Err(
                        "Rasterize text/tone/shadow objects before transforming this group".into(),
                    );
                }
            }
            _ => {
                return Err("Rasterize 3D/animation sources before transforming this group".into());
            }
        }
    }
    let mut state = d.snapshot();
    state.scene.layer_ids.initialize(d.layers.len())?;
    let mut total = 0usize;
    for key in &members {
        let t = c::target(d, *key).ok_or("Missing transform member")?;
        if let Some(link) = state
            .scene
            .file_objects
            .entries
            .iter_mut()
            .find(|l| l.target == *key)
        {
            link.cache.transform = link.cache.transform.then(original);
        } else {
            match t {
                Target::Raster(id) => {
                    let n = crate::layers::index(d, id).ok_or("Missing raster")?;
                    let old = &d.layers[n];
                    let e = old.extent(d.width, d.height);
                    let corners = [
                        e.offset.map(|v| v as f64),
                        [(e.offset[0] + e.width as i64) as f64, e.offset[1] as f64],
                        [
                            (e.offset[0] + e.width as i64) as f64,
                            (e.offset[1] + e.height as i64) as f64,
                        ],
                        [e.offset[0] as f64, (e.offset[1] + e.height as i64) as f64],
                    ]
                    .map(|p| matrix.map(p));
                    let min: [f64; 2] = std::array::from_fn(|k| {
                        corners
                            .iter()
                            .map(|p| p[k])
                            .fold(f64::INFINITY, f64::min)
                            .floor()
                    });
                    let max: [f64; 2] = std::array::from_fn(|k| {
                        corners
                            .iter()
                            .map(|p| p[k])
                            .fold(f64::NEG_INFINITY, f64::max)
                            .ceil()
                    });
                    if min
                        .iter()
                        .chain(&max)
                        .any(|v| !v.is_finite() || v.abs() > 1e9)
                        || (0..2).any(|k| !(1. ..=32768.).contains(&(max[k] - min[k])))
                    {
                        return Err("Transformed raster extent limit".into());
                    }
                    let e = PlacedRaster {
                        offset: min.map(|v| v as i64),
                        width: (max[0] - min[0]) as u32,
                        height: (max[1] - min[1]) as u32,
                    };
                    let count = e.width as usize * e.height as usize;
                    total = total.saturating_add(count);
                    if total > 4 * 1024 * 1024 {
                        return Err("Group resampling exceeds 4 million pixels".into());
                    }
                    let inverse = matrix.inverse();
                    let mut pixels = Vec::new();
                    pixels.try_reserve_exact(count).map_err(|e| e.to_string())?;
                    let mut precise = (d.color.depth == crate::color_precision::Depth::Sixteen)
                        .then(|| Vec::with_capacity(count));
                    for y in 0..e.height {
                        for x in 0..e.width {
                            let p = inverse.map([
                                e.offset[0] as f64 + x as f64 + 0.5,
                                e.offset[1] as f64 + y as f64 + 0.5,
                            ]);
                            if d.color != crate::color_storage::Config::legacy() {
                                let sample = old.sample(
                                    d.width,
                                    d.height,
                                    p[0].floor() as i64,
                                    p[1].floor() as i64,
                                );
                                pixels.push(sample.rgba8());
                                if let Some(precise) = &mut precise {
                                    precise.push(sample.rgba16());
                                }
                            } else {
                                pixels.push(old.pixel(
                                    d.width,
                                    d.height,
                                    p[0].floor() as i64,
                                    p[1].floor() as i64,
                                ));
                            }
                        }
                    }
                    let prior = d.scene.layer_groups.frame(*key).unwrap_or_default();
                    state.scene.layer_groups.set_frame(*key, prior.then(matrix));
                    state.layers[n].pixels = pixels;
                    state.layers[n].precise = precise;
                    state.layers[n].placement = Some(e);
                }
                Target::Vector(id) => {
                    for path in &mut state
                        .scene
                        .lines
                        .layers
                        .iter_mut()
                        .find(|l| l.id == id)
                        .unwrap()
                        .paths
                    {
                        if let Some(svg) = &mut path.svg {
                            svg.transform(original);
                        }
                        for p in &mut path.points {
                            *p = original.map(*p);
                        }
                        path.width *= scale;
                        if let Some(def) = &mut path.definition {
                            def.creation = original.inverse().then(def.creation);
                        }
                    }
                }
                Target::Gradient(id) => {
                    let g = state
                        .scene
                        .gradients
                        .layers
                        .iter_mut()
                        .find(|g| g.id == id)
                        .unwrap();
                    g.creation = original.inverse().then(g.creation);
                }
                Target::Object(id) => {
                    let n = c::source_index(d, Key::Object(id)).unwrap();
                    match &mut state.scene.objects[n] {
                        PageObject::Material(m) => m.transform = m.transform.then(original),
                        PageObject::Line {
                            start,
                            end,
                            width_px,
                            ..
                        } => {
                            *start = original.map(*start);
                            *end = original.map(*end);
                            *width_px *= scale;
                        }
                        _ => unreachable!(),
                    }
                }
                Target::Folder(_) => {}
                _ => unreachable!(),
            }
        }
        let effect = match t {
            Target::Raster(id) => state.scene.layer_effects.rasters.get_mut(&id),
            Target::Vector(id) => state.scene.layer_effects.vectors.get_mut(&id),
            Target::Folder(id) => state.scene.layer_effects.folders.get_mut(&id),
            _ => None,
        };
        if let Some(mask) = effect.and_then(|e| e.mask.as_mut()).filter(|m| m.linked) {
            mask.space = mask.space.then(original);
        }
        let paper = match t {
            Target::Raster(id) => {
                crate::layers::index(d, id).and_then(|n| state.scene.paper.layers.get_mut(&n))
            }
            Target::Folder(id) => Some(&mut state.scene.paper.folders[id as usize].paper),
            _ => None,
        };
        if let Some(paper) = paper
            .filter(|p| p.pattern.effective() || p.blend != crate::brush_composite::Mode::Normal)
        {
            paper.space = paper.space.then(original);
        }
        let style = match t {
            Target::Raster(id) => state.scene.appearance.rasters.get_mut(&id),
            Target::Vector(id) => state.scene.appearance.vectors.get_mut(&id),
            _ => None,
        };
        if let Some(style) = style {
            style.scope_space = style.scope_space.then(original);
        }
    }
    for guide in &mut state.scene.rulers.guides {
        let linked = guide.link.is_some_and(|t| match t {
            crate::line_tools::Target::Raster(n) => {
                members.contains(&Key::Layer(crate::layer_properties::raster(d, n)))
            }
            crate::line_tools::Target::Vector(id) => {
                members.contains(&Key::Layer(Target::Vector(id)))
            }
            _ => false,
        });
        let scoped = matches!(guide.scope,crate::ruler_scope::Scope::Folder(n)if members.contains(&Key::Layer(Target::Folder(n))));
        if linked || scoped {
            guide.space = guide.space.then(original);
        }
    }
    groups::finish(d, &state)?;
    Ok(state)
}
pub fn matrix(pivot: Point, angle: f64, scale: f64, flip_x: bool) -> Result<Affine, String> {
    if !angle.is_finite()
        || !(0.0625..=16.).contains(&scale)
        || pivot.iter().any(|v| !v.is_finite() || v.abs() > 1e6)
    {
        return Err("Transform pivot, angle or scale limit".into());
    }
    let (s, c) = crate::texture::rotation(angle);
    let x = if flip_x { -scale } else { scale };
    let m = Affine([c * x, s * x, -s * scale, c * scale, 0., 0.]);
    let p = m.map(pivot);
    Ok(Affine([
        m.0[0],
        m.0[1],
        m.0[2],
        m.0[3],
        pivot[0] - p[0],
        pivot[1] - p[1],
    ]))
}
pub fn filter(
    d: &Document,
    selected: &[Key],
    filter: crate::layer_effects::Adjustment,
) -> Result<DocumentState, String> {
    if !filter.valid() {
        return Err("Invalid linked ink filter".into());
    }
    let keys = groups::expanded(d, selected, Operation::Filter)?;
    groups::preflight(d, &keys, Operation::Filter)?;
    let members = groups::content_members(d, &groups::roots(d, &keys))?;
    for k in &members {
        let t = c::target(d, *k).ok_or("Missing filter member")?;
        if !matches!(t, Target::Raster(_) | Target::Vector(_) | Target::Folder(_))
            || d.scene.file_objects.get(*k).is_some()
            || crate::layer_effects::adjustment(d, t).is_some()
        {
            return Err("Linked ink filters require editable image/vector layers; rasterize other members first".into());
        }
    }
    if filter.identity() {
        return Ok(d.snapshot());
    }
    let mut state = d.snapshot();
    for k in members {
        match c::target(d, k).unwrap() {
            Target::Raster(id) => {
                let n = crate::layers::index(d, id).unwrap();
                if d.color != crate::color_storage::Config::legacy() {
                    let space = crate::color_profile::WorkingSpace::new(&d.color.profile)?;
                    crate::color_storage::map_layer(
                        &mut state.layers[n],
                        &d.color,
                        d.width,
                        d.height,
                        |value| space.adjusted(value, filter),
                    )?;
                } else {
                    for p in &mut state.layers[n].pixels {
                        *p = filter.pixel(*p);
                    }
                }
            }
            Target::Vector(id) => {
                for p in &mut state
                    .scene
                    .lines
                    .layers
                    .iter_mut()
                    .find(|l| l.id == id)
                    .unwrap()
                    .paths
                {
                    p.color = filter.pixel(p.color);
                    if let Some(fill) = &mut p.fill {
                        *fill = filter.pixel(*fill);
                    }
                }
            }
            _ => {}
        }
    }
    groups::finish(d, &state)?;
    Ok(state)
}

/// An exact nearest-sampling geometry proof permits hidden folder children and
/// alpha-protected layers to follow a transform without permitting ink edits.
pub(crate) fn geometry_matches(d: &Document, after: &DocumentState, id: u64) -> bool {
    let key = Key::Layer(Target::Raster(id));
    let old_frame = d.scene.layer_groups.frame(key).unwrap_or_default();
    let Some(new_frame) = after.scene.layer_groups.frame(key) else {
        return false;
    };
    if old_frame == new_frame {
        return false;
    }
    let matrix = old_frame.inverse().then(new_frame);
    if !matrix.valid() {
        return false;
    }
    let Some(n) = crate::layers::index(d, id) else {
        return false;
    };
    let Some(to) = (0..after.layers.len()).find(|n| after.scene.layer_ids.id(*n) == id) else {
        return false;
    };
    let old = &d.layers[n];
    let new = &after.layers[to];
    let a = old.extent(d.width, d.height);
    let b = new.extent(after.width, after.height);
    if !new.valid(after.width, after.height) || new.pixels.len() > 4 * 1024 * 1024 {
        return false;
    }
    let corners = [
        a.offset.map(|v| v as f64),
        [(a.offset[0] + a.width as i64) as f64, a.offset[1] as f64],
        [
            (a.offset[0] + a.width as i64) as f64,
            (a.offset[1] + a.height as i64) as f64,
        ],
        [a.offset[0] as f64, (a.offset[1] + a.height as i64) as f64],
    ]
    .map(|p| matrix.map(p));
    for k in 0..2 {
        let min = corners
            .iter()
            .map(|p| p[k])
            .fold(f64::INFINITY, f64::min)
            .floor();
        let max = corners
            .iter()
            .map(|p| p[k])
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil();
        if min != b.offset[k] as f64 || max != b.offset[k] as f64 + [b.width, b.height][k] as f64 {
            return false;
        }
    }
    let inverse = matrix.inverse();
    new.pixels.iter().enumerate().all(|(n, p)| {
        let q = inverse.map([
            b.offset[0] as f64 + (n % b.width as usize) as f64 + 0.5,
            b.offset[1] as f64 + (n / b.width as usize) as f64 + 0.5,
        ]);
        if d.color != crate::color_storage::Config::legacy()
            || old.precise.is_some()
            || new.precise.is_some()
        {
            new.sample_at(n).rgba16()
                == old
                    .sample(d.width, d.height, q[0].floor() as i64, q[1].floor() as i64)
                    .rgba16()
        } else {
            *p == old.pixel(d.width, d.height, q[0].floor() as i64, q[1].floor() as i64)
        }
    })
}
