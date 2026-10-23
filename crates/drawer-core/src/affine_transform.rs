//! One document-space pivot transform, prepared from the original source.
pub use crate::material_transform::Prepared;
use crate::{
    document::{Document, PlacedRaster},
    interpolation::Filter,
    layer_properties as p,
    layer_search::Key,
    scene::{Affine, Point, Rect},
    vector_scope::Transform,
};
fn corners(r: Rect) -> [Point; 4] {
    [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]]
}
fn bounds(points: impl IntoIterator<Item = Point>) -> Rect {
    let mut r = Rect {
        min: [f64::INFINITY; 2],
        max: [f64::NEG_INFINITY; 2],
    };
    for point in points {
        for (k, v) in point.iter().enumerate() {
            r.min[k] = r.min[k].min(*v);
            r.max[k] = r.max[k].max(*v);
        }
    }
    r
}
pub fn selection_bounds(d: &Document) -> Result<Rect, String> {
    if d.scene.selection_empty() {
        return Err("Create a nonempty pixel selection first".into());
    }
    let mut points = vec![];
    if let Some(mask) = &d.scene.selection_mask {
        for r in &mask.runs {
            points.extend(corners(Rect {
                min: [r.x as f64, r.y as f64],
                max: [r.x as f64 + r.length as f64, r.y as f64 + 1.],
            }));
        }
    } else if let Some(r) = d.scene.selection {
        points.extend(corners(r));
    }
    Ok(bounds(
        points.into_iter().map(|p| d.scene.selection_point(p)),
    ))
}
pub fn prepare(
    d: &Document,
    key: Key,
    pixels: bool,
    t: Transform,
    bake: bool,
    filter: Filter,
) -> Result<Prepared, String> {
    prepare_with_progress(d, key, pixels, t, bake, filter, &mut || Ok(()))
}
pub fn prepare_with_progress(
    d: &Document,
    key: Key,
    pixels: bool,
    t: Transform,
    bake: bool,
    filter: Filter,
    tick: &mut impl FnMut() -> Result<(), String>,
) -> Result<Prepared, String> {
    let mut out = prepare_inner(d, key, pixels, t, bake, filter, tick)?;
    if out.state != d.snapshot() {
        out.state.scene.last_transform = Some(crate::transform_handles::Record::Affine {
            degrees: t.degrees,
            matrix: t.matrix()?,
        });
    }
    if out.state.memory_bytes().saturating_add(d.memory_bytes())
        > crate::document::DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Transform receipt/output exceeds Undo memory budget".into());
    }
    Ok(out)
}
fn prepare_inner(
    d: &Document,
    key: Key,
    pixels: bool,
    t: Transform,
    bake: bool,
    filter: Filter,
    tick: &mut impl FnMut() -> Result<(), String>,
) -> Result<Prepared, String> {
    tick()?;
    t.matrix()?;
    let (s, c) = crate::texture::rotation(t.degrees);
    let mut matrix = Affine([
        c * t.scale[0],
        s * t.scale[0],
        -s * t.scale[1],
        c * t.scale[1],
        0.,
        0.,
    ]);
    let pivot = matrix.map(t.pivot);
    matrix.0[4] = t.pivot[0] - pivot[0] + t.translation[0];
    matrix.0[5] = t.pivot[1] - pivot[1] + t.translation[1];
    if t.duplicate {
        return Err("Use Duplicate vector selection or copy the layer before transforming".into());
    }
    if pixels {
        let Key::Layer(p::Target::Raster(id)) = key else {
            return Err("Selected pixels require a raster layer".into());
        };
        return selected(
            d,
            crate::layers::index(d, id).ok_or("Raster target changed")?,
            matrix,
            filter,
            tick,
        );
    }
    let keys = crate::layer_groups::expanded(d, &[key], crate::layer_groups::Operation::Transform)?;
    crate::layer_groups::preflight(d, &keys, crate::layer_groups::Operation::Transform)?;
    if keys.len() == 1
        && (matches!(key, Key::Layer(p::Target::Vector(_)))
            || d.scene.file_objects.get(key).is_some()
            || matches!(key, Key::Object(_))
                && crate::layer_catalog::source_index(d, key).is_some_and(|n| {
                    matches!(
                        d.scene.objects.get(n),
                        Some(crate::scene::PageObject::Material(_))
                    )
                }))
    {
        let mut settings = crate::material_transform::settings(d, key)?;
        settings.mode = crate::material_transform::Mode::Free;
        settings.free = matrix;
        settings.pivot = [0.; 2];
        settings.movement = [0.; 2];
        settings.scale = [1.; 2];
        settings.angle = 0.;
        settings.preserve_width = false;
        return crate::material_transform::prepare(d, key, &settings);
    }
    let attempted = crate::group_edits::transform_with_progress(d, &[key], matrix, tick);
    if let Ok(state) = attempted {
        return Ok(Prepared {state,selected:key,notes:vec!["Layers use their stored pixels; linked transforms, masks, texture and rulers follow. Unsupported linked members reject the whole operation. Undo restores the original.".into()]});
    }
    let reason = attempted.err().unwrap();
    tick()?;
    let unsupported = matches!(
        key,
        Key::Object(_) | Key::World(_) | Key::Track(_) | Key::Cell { .. }
    );
    if keys.len() == 1 && unsupported && bake && !matrix.is_identity() {
        let converted = crate::raster_conversion::prepare(
            d,
            key,
            crate::raster_conversion::Action::Rasterize(crate::layer_search::Expression::Color),
        )?;
        let working = crate::layer_merge::preview(&converted.state, d.current_frame)?;
        let state = crate::group_edits::transform_with_progress(
            &working,
            &[converted.selected],
            matrix,
            tick,
        )?;
        crate::layer_groups::finish(d, &state)?;
        let mut notes = converted.notes;
        notes.push("This source becomes a paintable raster on Apply. Undo restores the editable original and every property.".into());
        return Ok(Prepared {
            state,
            selected: converted.selected,
            notes,
        });
    }
    Err(reason)
}

fn selected(
    d: &Document,
    layer: usize,
    matrix: Affine,
    filter: Filter,
    tick: &mut impl FnMut() -> Result<(), String>,
) -> Result<Prepared, String> {
    tick()?;
    p::paintable(d, layer)?;
    p::movable(d, p::raster(d, layer))?;
    if d.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Pixel transform snapshot limit".into());
    }
    let selection = selection_bounds(d)?;
    let source = &d.layers[layer];
    let old = source.extent(d.width, d.height);
    let rect = Rect {
        min: old.offset.map(|v| v as f64),
        max: [
            old.offset[0] as f64 + old.width as f64,
            old.offset[1] as f64 + old.height as f64,
        ],
    };
    let selected_bounds = selection.intersection(rect);
    if (0..2).any(|k| selected_bounds.max[k] <= selected_bounds.min[k]) {
        return Err("Selection does not intersect the raster extent".into());
    }
    if !matches!(filter, Filter::Nearest | Filter::Bilinear) {
        return Err("Selected-pixel transforms support nearest or bilinear interpolation".into());
    }
    let key = Key::Layer(p::raster(d, layer));
    if matrix.is_identity() {
        return Ok(Prepared {
            state: d.snapshot(),
            selected: key,
            notes: vec![],
        });
    }
    let mut sample_bounds = selected_bounds;
    if filter == Filter::Bilinear {
        for k in 0..2 {
            sample_bounds.min[k] -= 0.5;
            sample_bounds.max[k] += 0.5;
        }
    }
    let mapped = bounds(corners(sample_bounds).map(|p| matrix.map(p)));
    let total = bounds(corners(rect).into_iter().chain(corners(mapped)));
    let lo = total.min.map(f64::floor);
    let hi = total.max.map(f64::ceil);
    let width = hi[0] - lo[0];
    let height = hi[1] - lo[1];
    if lo
        .iter()
        .chain(&hi)
        .any(|v| !v.is_finite() || v.abs() > 1e6)
        || !(1. ..=32768.).contains(&width)
        || !(1. ..=32768.).contains(&height)
        || width * height > 4. * 1024. * 1024.
    {
        return Err("Transformed pixel extent limit4M pixels".into());
    }
    let extent = PlacedRaster {
        offset: lo.map(|v| v as i64),
        width: width as u32,
        height: height as u32,
    };
    let inverse = matrix.inverse();
    let at = |x: i64, y: i64| {
        source
            .sample(d.width, d.height, x, y)
            .coverage(d.scene.selection_coverage([x as f64 + 0.5, y as f64 + 0.5]) as f64 / 255.)
            .unwrap()
    };
    let mut low = Vec::with_capacity(extent.width as usize * extent.height as usize);
    let mut high = (d.color.depth == crate::color_precision::Depth::Sixteen)
        .then(|| Vec::with_capacity(low.capacity()));
    for y in 0..extent.height {
        for x in 0..extent.width {
            if x.is_multiple_of(128) {
                tick()?;
            }
            let pos = [extent.offset[0] + x as i64, extent.offset[1] + y as i64];
            let point = [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5];
            let input = inverse.map(point).map(|v| v - 0.5);
            let footprint = corners(Rect {
                min: pos.map(|v| v as f64),
                max: [pos[0] as f64 + 1., pos[1] as f64 + 1.],
            })
            .map(|p| inverse.map(p).map(|v| v - 0.5));
            let moved = crate::interpolation::sample_float(input, footprint, filter, at);
            let coverage = d.scene.selection_coverage(point) as f64 / 255.;
            if coverage == 0. && moved.alpha() == 0. {
                let index = source.pixel_index(d.width, d.height, pos[0], pos[1]);
                low.push(index.map_or([0; 4], |i| source.pixels[i]));
                if let Some(high) = &mut high {
                    high.push(index.map_or([0; 4], |i| source.sample_at(i).rgba16()));
                }
            } else {
                let result = source
                    .sample(d.width, d.height, pos[0], pos[1])
                    .coverage(1. - coverage)?
                    .over(moved, crate::layer_blend::Mode::Normal, 1.)?;
                let stored = result.store(d.color.depth, d.color.dither, pos);
                if let Some(high) = &mut high {
                    high.push(stored);
                    low.push(crate::color_precision::Sample::from16(stored).rgba8());
                } else {
                    low.push(stored.map(|v| v as u8));
                }
            }
        }
    }
    let moved_selection = bounds(corners(selection).map(|p| matrix.map(p)));
    let grid = crate::selection_edit::Grid::with_bounds(d, &corners(moved_selection))?;
    let values = grid.sample_with_progress(
        d,
        false,
        |p| d.scene.selection_coverage(inverse.map(p)),
        tick,
    )?;
    let mut state = d.snapshot();
    state.layers[layer].pixels = low;
    state.layers[layer].precise = high;
    state.layers[layer].placement = Some(extent);
    state.scene = grid.scene(d, &values)?;
    crate::layer_groups::finish(d, &state)?;
    Ok(Prepared{state,selected:key,notes:vec!["Cut and transform only the selected coverage in the current raster. Premultiplied interpolation preserves transparent edges and native precision. The layer mask stays fixed; the area selection follows the transformed pixels.".into()]})
}
