//! Explicit rasterization and content-color edits. Frozen previews commit as one state.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster, RasterLayer},
    layer_properties::{self as p, Target},
    layer_search::{Expression, Key},
    render::Renderer,
    render_order::{Entry, Group},
    scene::Rect,
    sizing::DocumentState,
};
#[derive(Clone)]
pub enum Action {
    Rasterize(Expression),
    Brightness {
        selection: bool,
    },
    Color {
        color: [u8; 4],
        selection: bool,
        paths: Vec<u64>,
    },
}
pub struct Prepared {
    pub state: DocumentState,
    pub selected: Key,
    pub affected: Vec<Key>,
    pub notes: Vec<String>,
    pub maximum_delta: u8,
}
fn finish(
    d: &Document,
    mut state: DocumentState,
    selected: Key,
    affected: Vec<Key>,
    notes: Vec<String>,
    compare: bool,
) -> Result<Prepared, String> {
    crate::layer_catalog::reconcile(d, &mut state)?;
    crate::render_order::reconcile(&mut state);
    crate::file_objects::prune(&mut state);
    if !state.scene.valid()
        || !state.scene.paper.valid_for(state.layers.len())
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Raster conversion scene/Undo budget".into());
    }
    p::validate_edit(d, &state)?;
    let mut maximum_delta = 0;
    if compare {
        let after = crate::layer_merge::preview(&state, d.current_frame)?;
        let a = Renderer::new(d);
        let b = Renderer::new(&after);
        if let Some(e) = a.error().or_else(|| b.error()) {
            return Err(e.into());
        }
        for y in 0..d.height {
            for x in 0..d.width {
                let point = [x as f64 + 0.5, y as f64 + 0.5];
                for (a, b) in a.pixel(point).into_iter().zip(b.pixel(point)) {
                    maximum_delta = maximum_delta.max(a.abs_diff(b));
                }
            }
        }
        if maximum_delta > 1 {
            return Err("Raster conversion could not preserve this composition within one RGBA step; the page is unchanged".into());
        }
    }
    Ok(Prepared {
        state,
        selected,
        affected,
        notes,
        maximum_delta,
    })
}
fn gray(c: [u8; 4]) -> u8 {
    ((54 * c[0] as u32 + 183 * c[1] as u32 + 19 * c[2] as u32 + 128) / 256) as u8
}
fn expression(p: &mut [u8; 4], value: Expression) {
    if p[3] == 0 {
        *p = [0; 4];
        return;
    }
    let value = match value {
        Expression::Color => return,
        Expression::Gray => gray(*p),
        Expression::Mono => {
            if gray(*p) >= 128 {
                255
            } else {
                0
            }
        }
    };
    p[..3].fill(value);
}
fn bounds(d: &Document, key: Key, caches: &crate::appearance::Caches) -> Option<Rect> {
    if d.scene.file_objects.get(key).is_some() {
        return crate::layer_catalog::bounds(d, key);
    }
    if let Key::Layer(Target::Vector(id)) = key
        && let Some(n) = d.scene.lines.layers.iter().position(|l| l.id == id)
        && let Some(c) = caches.vectors[n].as_ref().and_then(|c| c.bounds())
    {
        return Some(c);
    }
    crate::layer_catalog::bounds(d, key)
}
fn extent(bounds: Rect) -> Result<PlacedRaster, String> {
    let min = bounds.min.map(f64::floor);
    let size = [0, 1].map(|n| (bounds.max[n].ceil() - min[n]).max(1.));
    if min.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
        || size.iter().any(|v| !v.is_finite() || *v > 32768.)
        || size[0] * size[1] > 4. * 1024. * 1024.
    {
        return Err("Raster extent exceeds 32768 pixels / 4 million pixels".into());
    }
    Ok(PlacedRaster {
        offset: min.map(|v| v as i64),
        width: size[0] as u32,
        height: size[1] as u32,
    })
}
fn check(d: &Document, t: Target) -> Result<(), String> {
    let flags = crate::file_objects::stored_flags(d, t);
    if flags.locked
        || flags.pixels_locked
        || flags.position_locked
        || flags.hidden
        || d.scene.clipping.solo.is_some()
    {
        return Err(
            "Choose an unlocked visible layer and leave folder solo before converting".into(),
        );
    }
    if !d.scene.valid() || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Invalid document or conversion snapshot limit".into());
    }
    Ok(())
}
pub fn prepare(d: &Document, key: Key, action: Action) -> Result<Prepared, String> {
    let t = crate::layer_catalog::target(d, key).ok_or("Select a layer")?;
    match action {
        Action::Rasterize(value) => {
            check(d, t)?;
            rasterize(d, key, t, value)
        }
        Action::Brightness { selection } => pixels(d, key, t, selection, None),
        Action::Color {
            color,
            selection,
            paths,
        } => match t {
            Target::Raster(_) => pixels(d, key, t, selection, Some(color)),
            Target::Vector(id) => vector_color(d, key, id, &paths, color),
            _ => Err(
                "Recolor a raster or selected vector strokes; rasterize other content first".into(),
            ),
        },
    }
}
fn rasterize(d: &Document, key: Key, t: Target, value: Expression) -> Result<Prepared, String> {
    if d.resident_bytes()
        .saturating_add(d.memory_bytes().saturating_mul(4))
        .saturating_add(128 * 1024 * 1024)
        > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Rasterization render/Undo memory reserve".into());
    }
    let complexity = d.layers.len()
        + d.scene.objects.len() * 4
        + d.scene
            .lines
            .layers
            .iter()
            .flat_map(|l| &l.paths)
            .map(|p| p.points.len())
            .sum::<usize>()
        + d.scene.worlds.iter().map(|w| w.edges.len()).sum::<usize>()
        + d.scene.gradients.run_count();
    if (d.width as usize)
        .saturating_mul(d.height as usize)
        .saturating_mul(complexity.max(1))
        > 64 * 1024 * 1024
    {
        return Err("Rasterization comparison work limit".into());
    }
    if let Target::Folder(_) = t {
        let prepared = crate::layer_merge::prepare(
            d,
            crate::layer_merge::Operation::Selected(vec![t]),
            Default::default(),
        )?;
        let mut state = prepared.state;
        let Target::Raster(id) = prepared.selected else {
            return Err("Folder raster output mismatch".into());
        };
        convert_expression(&mut state, id, value)?;
        return finish(
            d,
            state,
            Key::Layer(Target::Raster(id)),
            vec![key],
            vec!["Folder children and effects become a raster; one Undo restores them.".into()],
            value == Expression::Color,
        );
    }
    if let Target::Raster(id) = t {
        return raster(d, key, id, value);
    }
    if !matches!(
        t,
        Target::Vector(_) | Target::Object(_) | Target::Gradient(_) | Target::World(_)
    ) {
        return Err("Choose raster, vector, text/material/geometry, gradient, 3D or a folder; animation uses current-frame Flatten page".into());
    }
    if d.layers.len() >= 256 {
        return Err("Raster layer limit (256)".into());
    }
    let mut affected = vec![key];
    let mut mode = crate::layer_blend::mode(d, t);
    let mut group = None;
    if let Target::Vector(id) = t {
        let layers = &d.scene.lines.layers;
        let selected = layers
            .iter()
            .position(|l| l.id == id)
            .ok_or("Missing vector")?;
        let base = if crate::clipping::active(d, t) {
            match d.scene.clipping.binding(t) {
                Some(Target::Vector(id)) => {
                    layers.iter().position(|l| l.id == id).unwrap_or(selected)
                }
                _ => selected,
            }
        } else {
            selected
        };
        if d.scene
            .clipping
            .binding(Target::Vector(layers[base].id))
            .is_none()
        {
            affected = vec![Key::Layer(Target::Vector(layers[base].id))];
            let mut n = base + 1;
            while n < layers.len()
                && d.scene
                    .clipping
                    .binding(Target::Vector(layers[n].id))
                    .is_some()
            {
                if d.scene.clipping.binding(Target::Vector(layers[n].id))
                    == Some(Target::Vector(layers[base].id))
                {
                    affected.push(Key::Layer(Target::Vector(layers[n].id)));
                }
                n += 1;
            }
            mode = crate::layer_blend::mode(d, Target::Vector(layers[base].id));
        }
        group = Some(Group::Vector);
    }
    let flags = crate::layer_roles::flags(d, t);
    for k in &affected {
        let t = crate::layer_catalog::target(d, *k).ok_or("Source was removed")?;
        check(d, t)?;
        if crate::layer_roles::flags(d, t) != flags {
            return Err(
                "Give the clipping run common reference/draft roles before rasterizing".into(),
            );
        }
    }
    let caches = crate::appearance::Caches::new(d, None)?;
    let mut rect = Rect {
        min: [f64::INFINITY; 2],
        max: [f64::NEG_INFINITY; 2],
    };
    for key in &affected {
        if let Some(b) = bounds(d, *key, &caches) {
            for n in 0..2 {
                rect.min[n] = rect.min[n].min(b.min[n]);
                rect.max[n] = rect.max[n].max(b.max[n]);
            }
        }
    }
    if !rect.valid() {
        rect = Rect {
            min: [0.; 2],
            max: [1.; 2],
        };
    }
    let placement = extent(rect)?;
    let owner = crate::layer_catalog::target(d, affected[0]).ok_or("Missing source owner")?;
    let owner_properties = p::properties(d, owner);
    drop(caches);
    let mut capture_state = d.snapshot();
    let mut intrinsic = owner_properties;
    intrinsic.opacity = 100;
    p::set_state(d, &mut capture_state, owner, intrinsic)?;
    let capture = crate::layer_merge::preview(&capture_state, d.current_frame)?;
    drop(capture_state);
    let r = Renderer::new(&capture);
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(placement.width as usize * placement.height as usize)
        .map_err(|e| e.to_string())?;
    for y in 0..placement.height {
        for x in 0..placement.width {
            let point = [
                placement.offset[0] as f64 + x as f64 + 0.5,
                placement.offset[1] as f64 + y as f64 + 0.5,
            ];
            let mut pixel = if group.is_some() {
                let original = d.scene.transform.inverse().map(point);
                crate::paper::clipped_vectors(
                    &capture,
                    point,
                    original,
                    [0; 4],
                    |_, l| affected.contains(&Key::Layer(Target::Vector(l.id))),
                    false,
                    |n| Some(r.vector_pixel(n, point)),
                )
            } else {
                r.source_pixel_unbounded(key, point)
            };
            expression(&mut pixel, value);
            pixels.push(pixel);
        }
    }
    let high_samples = if d.color != crate::color_storage::Config::legacy() {
        let renderer = crate::float_render::Renderer::new(&capture)?;
        let mut samples = Vec::with_capacity(pixels.len());
        for y in 0..placement.height {
            for x in 0..placement.width {
                let point = [
                    placement.offset[0] as f64 + x as f64 + 0.5,
                    placement.offset[1] as f64 + y as f64 + 0.5,
                ];
                let sample = if group.is_some() {
                    renderer.vector_run(&affected, point)?
                } else {
                    renderer.source_pixel(key, point)?
                };
                samples.push(renderer.convert_expression(sample, value)?);
            }
        }
        Some(samples)
    } else {
        None
    };
    drop(r);
    drop(capture);
    let selection = crate::layer_search::Selection {
        keys: affected.iter().copied().collect(),
        primary: Some(key),
        ..Default::default()
    };
    let mut state = crate::search_edit::prepare(
        d,
        &Default::default(),
        &selection,
        crate::search_edit::Edit::Delete,
    )?
    .state;
    let order = crate::render_order::normalized(&d.scene);
    let at = order
        .iter()
        .position(|e| affected.contains(&e.key))
        .ok_or("Raster source stack entry is missing")?;
    let group = group.unwrap_or(order[at].group);
    state.scene.layer_ids.initialize(state.layers.len())?;
    let n = state.layers.len();
    let name = crate::layer_catalog::rows(d)?
        .into_iter()
        .find(|r| r.key == key)
        .map_or("Rasterized".into(), |r| {
            r.name
                .chars()
                .filter(|c| !c.is_control())
                .take(48)
                .collect::<String>()
                + " raster"
        });
    state.scene.paper.roots = state.scene.paper.root_nodes(state.layers.len());
    state.scene.paper.roots.push(crate::paper::Node::Raster(n));
    state.layers.push(RasterLayer {
        precise: None,
        locked: false,
        name,
        pixels,
        placement: Some(placement),
    });
    if let Some(samples) = high_samples {
        crate::color_storage::set_samples(&mut state.layers[n], &d.color, placement, samples)?;
    }
    state.scene.layer_ids.append(state.layers.len())?;
    let id = state.scene.layer_ids.id(n);
    let new = Key::Layer(Target::Raster(id));
    let mut props = owner_properties;
    props.blend = mode;
    props.hidden = false;
    state.scene.layer_properties.set(Target::Raster(id), props);
    crate::layer_roles::put_raster(&mut state, n, flags);
    let mut info = d.scene.catalog.info(key).cloned().unwrap_or_default();
    info.name = None;
    info.kind = None;
    info.properties = Default::default();
    info.expression = value;
    info.reference = false;
    info.draft = false;
    state.scene.catalog.set(new, info);
    let mut entries = vec![];
    for (ei, e) in order.into_iter().enumerate() {
        if ei == at {
            entries.push(Entry { key: new, group });
        }
        if !affected.contains(&e.key) {
            entries.push(e);
        }
    }
    state.scene.render_order.entries = entries;
    finish(d,state,new,affected,vec!["Editable sources, masks and display effects become paintable raster pixels at their original compositing position. Vector clipping runs are converted together. One Undo restores the original sources.".into()],value==Expression::Color)
}
fn convert_expression(s: &mut DocumentState, id: u64, value: Expression) -> Result<(), String> {
    if let Some(n) = (0..s.layers.len()).find(|n| s.scene.layer_ids.id(*n) == id) {
        if s.color != crate::color_storage::Config::legacy() {
            let space = crate::color_profile::WorkingSpace::new(&s.color.profile)?;
            crate::color_storage::map_layer(&mut s.layers[n], &s.color, s.width, s.height, |p| {
                if value == Expression::Color {
                    Ok(p)
                } else {
                    space.from_srgb(space.to_srgb(p)?.expression(
                        value,
                        128. / 255.,
                        128. / 255.,
                    )?)
                }
            })?;
        } else {
            for pixel in &mut s.layers[n].pixels {
                expression(pixel, value);
            }
        }
        s.scene.catalog.rasters.entry(id).or_default().expression = value;
    }
    Ok(())
}
fn raster(d: &Document, key: Key, id: u64, value: Expression) -> Result<Prepared, String> {
    let n = crate::layers::index(d, id).ok_or("Missing raster")?;
    if crate::layer_effects::adjustment(d, Target::Raster(id)).is_some() {
        return Err("Merge a correction with its backdrop instead of rasterizing it alone".into());
    }
    let appearance = crate::appearance::Caches::new(d, None)?;
    let extent = crate::file_objects::render_extents(
        d,
        &d.scene.paper,
        &[crate::paper::Node::Raster(n)],
        &appearance,
    )?
    .into_iter()
    .next()
    .ok_or("Missing raster extent")?;
    drop(appearance);
    if extent.width as usize * extent.height as usize > 4 * 1024 * 1024 {
        return Err("Rasterization extent exceeds 4 million pixels".into());
    }
    let r = Renderer::new(d);
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(extent.width as usize * extent.height as usize)
        .map_err(|e| e.to_string())?;
    for y in 0..extent.height {
        for x in 0..extent.width {
            let point = [extent.offset[0] + x as i64, extent.offset[1] + y as i64];
            let mut c = d.scene.paper.intrinsic_node_pixel_with(
                d,
                crate::paper::Node::Raster(n),
                point,
                &mut |n, _| r.raster_pixel(n, point),
            );
            expression(&mut c, value);
            pixels.push(c);
        }
    }
    drop(r);
    let mut state = d.snapshot();
    state.layers[n].pixels = pixels;
    if d.color != crate::color_storage::Config::legacy() {
        let renderer = crate::float_render::Renderer::new(d)?;
        let mut samples = Vec::with_capacity(state.layers[n].pixels.len());
        for y in 0..extent.height {
            for x in 0..extent.width {
                let point = [extent.offset[0] + x as i64, extent.offset[1] + y as i64];
                samples.push(
                    renderer.convert_expression(renderer.intrinsic_raster(n, point)?, value)?,
                );
            }
        }
        crate::color_storage::set_samples(&mut state.layers[n], &d.color, extent, samples)?;
    }
    state.layers[n].placement = Some(extent);
    state.scene.appearance.remove(Target::Raster(id));
    state.scene.layer_effects.remove(Target::Raster(id));
    state.scene.file_objects.entries.retain(|l| l.target != key);
    let mut paper = state
        .scene
        .paper
        .layers
        .get(&n)
        .copied()
        .unwrap_or_default();
    paper.pattern = Default::default();
    paper.origin = [0.; 2];
    state.scene.paper.layers.insert(n, paper);
    state
        .scene
        .catalog
        .rasters
        .entry(id)
        .or_default()
        .expression = value;
    finish(d,state,key,vec![key],vec!["Layer opacity, blending, clipping and roles stay editable; masks, paper texture and display effects become raster pixels.".into()],value==Expression::Color)
}
fn pixels(
    d: &Document,
    key: Key,
    t: Target,
    selection: bool,
    color: Option<[u8; 4]>,
) -> Result<Prepared, String> {
    let Target::Raster(id) = t else {
        return Err("Brightness to opacity requires a raster layer".into());
    };
    let n = crate::layers::index(d, id).ok_or("Missing raster")?;
    p::paintable(d, n)?;
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Color edit snapshot limit".into());
    }
    let mut state = d.snapshot();
    let extent = d.layers[n].extent(d.width, d.height);
    if d.color != crate::color_storage::Config::legacy() {
        let space = crate::color_profile::WorkingSpace::new(&d.color.profile)?;
        let mut samples = Vec::with_capacity(d.layers[n].pixels.len());
        for i in 0..d.layers[n].pixels.len() {
            let original = d.layers[n].sample_at(i);
            let point = [
                extent.offset[0] as f64 + (i % extent.width as usize) as f64 + 0.5,
                extent.offset[1] as f64 + (i / extent.width as usize) as f64 + 0.5,
            ];
            let weight = if selection {
                d.scene.selection_coverage(point) as f64 / 255.
            } else {
                1.
            };
            let after = if original.alpha() == 0. || weight == 0. {
                original
            } else if let Some(c) = color {
                let rgb = c[..3].iter().map(|v| *v as f64 / 255.).collect::<Vec<_>>();
                let desired = crate::color_precision::Sample::new([
                    rgb[0],
                    rgb[1],
                    rgb[2],
                    original.alpha(),
                ])?;
                space.mix(
                    original,
                    desired,
                    weight,
                    crate::color_mix::Space::Standard,
                    0.,
                )?
            } else {
                let source = space.to_srgb(original)?;
                let target = crate::color_precision::Sample::new([
                    0.,
                    0.,
                    0.,
                    source.alpha() * (1. - source.luminance()),
                ])?;
                space.from_srgb(source.mix(target, weight)?)?
            };
            samples.push(after);
        }
        crate::color_storage::set_samples(&mut state.layers[n], &d.color, extent, samples)?;
        return finish(d,state,key,vec![key],vec!["Floating point RGB/alpha conversion; one Undo restores the original precision and content.".into()],false);
    }
    for (i, pixel) in state.layers[n].pixels.iter_mut().enumerate() {
        if pixel[3] == 0 {
            *pixel = [0; 4];
            continue;
        }
        let point = [
            extent.offset[0] as f64 + (i % extent.width as usize) as f64 + 0.5,
            extent.offset[1] as f64 + (i / extent.width as usize) as f64 + 0.5,
        ];
        let a = if selection {
            d.scene.selection_coverage(point) as u32
        } else {
            255
        };
        if a == 0 {
            continue;
        }
        let before = *pixel;
        if let Some(c) = color {
            for n in 0..3 {
                pixel[n] = ((before[n] as u32 * (255 - a) + c[n] as u32 * a + 127) / 255) as u8;
            }
        } else {
            let target = (before[3] as u32 * (255 - gray(before) as u32) + 127) / 255;
            let total = before[3] as u64 * (255 - a) as u64 + target as u64 * a as u64;
            let alpha = (total + 127) / 255;
            for n in 0..3 {
                pixel[n] = if alpha == 0 {
                    0
                } else {
                    ((before[n] as u64 * before[3] as u64 * (255 - a) as u64 + total / 2) / total)
                        .min(255) as u8
                };
            }
            pixel[3] = alpha as u8;
        }
    }
    finish(d,state,key,vec![key],vec![if color.is_some(){"Current RGB replaces selected content color; original alpha is preserved."}else{"White becomes transparent, black keeps its alpha; gray reduces alpha proportionally. Selection strengths mix premultiplied results."}.into()],false)
}
fn vector_color(
    d: &Document,
    key: Key,
    id: u64,
    paths: &[u64],
    color: [u8; 4],
) -> Result<Prepared, String> {
    let flags = p::inherited(d, Target::Vector(id));
    if flags.hidden || flags.locked || flags.pixels_locked || paths.is_empty() || paths.len() > 128
    {
        return Err("Select unlocked visible vector strokes first".into());
    }
    let mut state = d.snapshot();
    let layer = state
        .scene
        .lines
        .layers
        .iter_mut()
        .find(|l| l.id == id)
        .ok_or("Missing vector")?;
    let ids = paths
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if ids.len() != paths.len()
        || ids
            .iter()
            .any(|id| !layer.paths.iter().any(|p| p.id == *id))
    {
        return Err("Vector selection changed".into());
    }
    for path in &mut layer.paths {
        if ids.contains(&path.id) {
            path.color[..3].copy_from_slice(&color[..3]);
            if let Some(fill) = path.fill.as_mut() {
                fill[..3].copy_from_slice(&color[..3]);
                if fill[3] == 0 {
                    *fill = [0; 4];
                }
            }
            if path.color[3] == 0 {
                path.color = [0; 4];
            }
        }
    }
    finish(d,state,key,vec![key],vec!["Only selected vector ink RGB changes; stroke/fill alpha, geometry and pressure metadata are preserved.".into()],false)
}
