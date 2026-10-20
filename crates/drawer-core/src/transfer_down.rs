//! Transfer selected raster content while retaining both layer identities and source storage.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    layer_properties::{self as p, Target},
    paper::Node,
    sizing::DocumentState,
};
pub struct Prepared {
    pub state: DocumentState,
    pub lower: Option<Target>,
    pub changed: usize,
    pub maximum_delta: u8,
}
pub fn lower(d: &Document, upper: u64) -> Result<usize, String> {
    let n = crate::layers::index(d, upper).ok_or("Select a live raster layer")?;
    let node = Node::Raster(n);
    let siblings = d
        .scene
        .paper
        .folders
        .iter()
        .find(|f| f.children.contains(&node))
        .map_or_else(
            || d.scene.paper.root_nodes(d.layers.len()),
            |f| f.children.clone(),
        );
    let at = siblings
        .iter()
        .position(|v| *v == node)
        .ok_or("Missing layer order")?;
    siblings[..at]
        .iter()
        .rev()
        .find_map(|v| {
            if let Node::Raster(n) = v {
                crate::layer_effects::adjustment(d, p::raster(d, *n))
                    .is_none()
                    .then_some(*n)
            } else {
                None
            }
        })
        .ok_or_else(|| "No editable raster sibling below this layer".into())
}
fn writable(d: &Document, n: usize) -> Result<(), String> {
    p::paintable(d, n)?;
    let t = p::raster(d, n);
    let props = p::properties(d, t);
    if props.opacity != 100
        || !props.blend.is_normal()
        || d.scene.clipping.alpha.contains(&t)
        || d.scene
            .layer_effects
            .get(t)
            .is_some_and(|e| e.adjustment.is_some() || e.mask.as_ref().is_some_and(|m| m.enabled))
        || d.scene
            .appearance
            .get(t)
            .is_some_and(crate::appearance::Style::active)
        || d.scene.paper.layers.get(&n).is_some_and(|v| {
            v.pattern.effective() || v.blend != crate::brush_composite::Mode::Normal
        })
        || d.scene
            .catalog
            .rasters
            .get(&d.scene.layer_ids.id(n))
            .is_some_and(|i| i.expression != crate::layer_search::Expression::Color)
    {
        return Err("Transfer needs Normal/color raster layers at 100% opacity without alpha protection or active effects; rasterize the displayed content first".into());
    }
    if p::targets(d).into_iter().any(|other| {
        crate::clipping::active(d, other)
            && (other == t || d.scene.clipping.binding(other) == Some(t))
    }) {
        return Err("Release the clipping run before transferring its content".into());
    }
    Ok(())
}
/// Recover the lower RGBA after a fraction of the upper alpha has been removed.
/// This keeps the combined Normal source-over color and alpha at soft selection edges.
fn transfer(u: [u8; 4], b: [u8; 4], coverage: u8) -> ([u8; 4], [u8; 4]) {
    let mut upper = u;
    upper[3] = ((u[3] as u32 * (255 - coverage as u32) + 127) / 255) as u8;
    if upper[3] == 0 {
        upper = [0; 4];
    }
    let a = u[3] as f64 / 255.;
    let r = upper[3] as f64 / 255.;
    let ba = b[3] as f64 / 255.;
    let output = a + ba * (1. - a);
    let lower = (output - r) / (1. - r);
    if lower <= 0. {
        return (upper, [0; 4]);
    }
    let mut below = [0; 4];
    below[3] = (lower * 255.).round().clamp(0., 255.) as u8;
    for k in 0..3 {
        let premul = u[k] as f64 * a + b[k] as f64 * ba * (1. - a);
        below[k] = ((premul - u[k] as f64 * r) / ((1. - r) * lower))
            .round()
            .clamp(0., 255.) as u8;
    }
    (upper, below)
}
pub fn prepare(d: &Document, upper: u64) -> Result<Prepared, String> {
    let source = crate::layers::index(d, upper).ok_or("Select a live raster layer")?;
    if !d.scene.valid()
        || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
        || d.scene.clipping.solo.is_some()
    {
        return Err("Invalid page, snapshot limit, or folder solo active".into());
    }
    let restricted = d.scene.selection.is_some() || d.scene.selection_mask.is_some();
    if restricted && d.scene.selection_empty() {
        return Ok(Prepared {
            state: d.snapshot(),
            lower: None,
            changed: 0,
            maximum_delta: 0,
        });
    }
    let below = lower(d, upper)?;
    writable(d, source)?;
    writable(d, below)?;
    crate::layer_roles::common(d, [source, below])?;
    crate::render_order::allow_partial_merge(d, &[p::raster(d, source), p::raster(d, below)])?;
    let a = d.layers[source].extent(d.width, d.height);
    let b = d.layers[below].extent(d.width, d.height);
    let min = [a.offset[0].min(b.offset[0]), a.offset[1].min(b.offset[1])];
    let max = [
        (a.offset[0] + a.width as i64).max(b.offset[0] + b.width as i64),
        (a.offset[1] + a.height as i64).max(b.offset[1] + b.height as i64),
    ];
    let extent = PlacedRaster {
        offset: min,
        width: (max[0] - min[0])
            .try_into()
            .map_err(|_| "Transfer width limit")?,
        height: (max[1] - min[1])
            .try_into()
            .map_err(|_| "Transfer height limit")?,
    };
    let count = extent.width as usize * extent.height as usize;
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
    if !extent.valid()
        || count > 4 * 1024 * 1024
        || count.saturating_mul(complexity).saturating_mul(2) > 64 * 1024 * 1024
        || d.resident_bytes()
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(count.saturating_mul(8))
            .saturating_add(128 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Transfer extent, render work or memory limit".into());
    }
    let mut state = d.snapshot();
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(count).map_err(|e| e.to_string())?;
    for y in min[1]..max[1] {
        for x in min[0]..max[0] {
            pixels.push(d.layers[below].pixel(d.width, d.height, x, y));
        }
    }
    let mut changed = 0;
    for y in 0..a.height {
        for x in 0..a.width {
            let point = [a.offset[0] + x as i64, a.offset[1] + y as i64];
            let c = d
                .scene
                .selection_coverage([point[0] as f64 + 0.5, point[1] as f64 + 0.5]);
            if c == 0 {
                continue;
            }
            let n = y as usize * a.width as usize + x as usize;
            let u = d.layers[source].pixels[n];
            if u[3] == 0 && c < 255 {
                continue;
            }
            let at =
                (point[1] - min[1]) as usize * extent.width as usize + (point[0] - min[0]) as usize;
            if u[3] == 0 {
                if u != [0; 4] {
                    state.layers[source].pixels[n] = [0; 4];
                    changed += 1;
                }
                continue;
            }
            let (up, down) = transfer(u, pixels[at], c);
            if up != u || down != pixels[at] {
                changed += 1;
                state.layers[source].pixels[n] = up;
                pixels[at] = down;
            }
        }
    }
    if changed == 0 {
        return Ok(Prepared {
            state: d.snapshot(),
            lower: Some(p::raster(d, below)),
            changed: 0,
            maximum_delta: 0,
        });
    }
    state.layers[below].pixels = pixels;
    state.layers[below].placement = Some(extent);
    p::validate_edit(d, &state)?;
    let after = crate::layer_merge::preview(&state, d.current_frame)?;
    let before = crate::render::Renderer::new(d);
    let rendered = crate::render::Renderer::new(&after);
    if let Some(e) = before.error().or_else(|| rendered.error()) {
        return Err(e.to_string());
    }
    let roots = d.scene.paper.root_nodes(d.layers.len());
    let after_roots = after.scene.paper.root_nodes(after.layers.len());
    let mut maximum_delta = 0;
    for y in min[1]..max[1] {
        for x in min[0]..max[0] {
            let point = [x, y];
            let a = d
                .scene
                .paper
                .composite_nodes_with(d, &roots, point, &mut |n, _| before.raster_pixel(n, point));
            let b =
                after
                    .scene
                    .paper
                    .composite_nodes_with(&after, &after_roots, point, &mut |n, _| {
                        rendered.raster_pixel(n, point)
                    });
            for k in 0..4 {
                maximum_delta = maximum_delta.max(a[k].abs_diff(b[k]));
            }
        }
    }
    if maximum_delta > 1 {
        return Err("Intervening layers or backdrop-dependent effects prevent appearance-preserving transfer; use a directly adjacent plain raster below".into());
    }
    if !state.scene.valid()
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Transfer result exceeds document/Undo limits".into());
    }
    Ok(Prepared {
        state,
        lower: Some(p::raster(d, below)),
        changed,
        maximum_delta,
    })
}
