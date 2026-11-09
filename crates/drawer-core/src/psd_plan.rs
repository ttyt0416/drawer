use super::*;
use crate::{
    layer_search::{Expression, Key},
    raster_conversion::{self, Action},
};
fn extra(d: &Document, t: Target) -> bool {
    let paper = match t {
        Target::Raster(id) => crate::layers::index(d, id)
            .and_then(|n| d.scene.paper.layers.get(&n))
            .copied()
            .unwrap_or_default(),
        Target::Folder(n) => d.scene.paper.folders[n as usize].paper,
        _ => return true,
    };
    paper.pattern != Default::default()
        || paper.blend != crate::brush_composite::Mode::Normal
        || crate::appearance::active(d, t)
        || crate::layer_effects::adjustment(d, t).is_some()
        || blend_key(props::properties(d, t).blend).is_none()
        || matches!(t,Target::Raster(id) if d.scene.surface_effects.rasters.contains_key(&id)||d.scene.file_objects.entries.iter().any(|l|l.target==Key::Layer(t)))
}
fn names(d: &Document, targets: &[Target]) -> Vec<String> {
    let rows = crate::layer_catalog::rows(d).unwrap_or_default();
    targets
        .iter()
        .map(|t| {
            rows.iter()
                .find(|r| r.key == crate::layer_catalog::key(d, *t))
                .map_or_else(|| format!("{t:?}"), |r| r.name.clone())
        })
        .collect()
}
fn unlock(d: &mut Document) {
    for l in &mut d.layers {
        l.locked = false;
    }
    for l in &mut d.scene.lines.layers {
        l.locked = false;
    }
    for p in d
        .scene
        .layer_properties
        .rasters
        .values_mut()
        .chain(d.scene.layer_properties.folders.values_mut())
        .chain(d.scene.layer_properties.vectors.values_mut())
    {
        p.locked = false;
        p.pixels_locked = false;
        p.position_locked = false;
    }
}
fn merge_dependency(
    d: &Document,
    t: Target,
    cancel: &AtomicBool,
) -> Result<crate::layer_merge::Prepared, String> {
    let mut attempts = 0usize;
    let mut node =
        crate::clipping::node(d, t).ok_or("A separate source needs the whole composite")?;
    loop {
        let parent = d
            .scene
            .paper
            .folders
            .iter()
            .position(|f| f.children.contains(&node));
        let list = parent.map_or_else(
            || d.scene.paper.root_nodes(d.layers.len()),
            |n| d.scene.paper.folders[n].children.clone(),
        );
        let index = list
            .iter()
            .position(|v| *v == node)
            .ok_or("Missing dependency node")?;
        // Increasing interval length finds the smallest consecutive sibling dependency range.
        for length in 1..=list.len() {
            for start in 0..=index {
                let end = start + length;
                if end > list.len() || end <= index {
                    continue;
                }
                canceled(cancel)?;
                attempts += 1;
                if (d.width as usize)
                    .saturating_mul(d.height as usize)
                    .saturating_mul(attempts)
                    .saturating_mul(d.layers.len().max(1))
                    > 64 * 1024 * 1024
                {
                    return Err(
                        "Dependency-range comparison work limit; use the whole composite".into(),
                    );
                }
                let selected = list[start..end]
                    .iter()
                    .map(|n| crate::clipping::node_key(d, *n))
                    .collect();
                if let Ok(p) = crate::layer_merge::prepare(
                    d,
                    crate::layer_merge::Operation::Selected(selected),
                    crate::layer_merge::Settings {
                        include_hidden: false,
                        lower_backdrop: false,
                    },
                ) {
                    return Ok(p);
                }
            }
        }
        if let Some(n) = parent {
            node = Node::Folder(n);
        } else {
            return Err(
                "No consecutive dependency range can be baked safely; use the whole composite"
                    .into(),
            );
        }
    }
}
fn partial(
    d: &Document,
    merged: &[[u8; 4]],
    budget: usize,
    cancel: &AtomicBool,
    losses: &mut Vec<String>,
    ranges: &mut Vec<Vec<String>>,
) -> Result<Image, String> {
    if d.animation.is_some() || !d.scene.transform.is_identity() {
        return Err("Timeline or page transforms require the current-frame whole composite".into());
    }
    let mut work = crate::layer_merge::preview(&d.snapshot(), d.current_frame)?;
    unlock(&mut work);
    for _ in 0..512 {
        canceled(cancel)?;
        let next = crate::render_order::normalized(&work.scene)
            .into_iter()
            .find(|e| !matches!(e.key, Key::Layer(Target::Raster(_))));
        let Some(entry) = next else {
            break;
        };
        let t = crate::layer_catalog::target(&work, entry.key)
            .ok_or("Missing source conversion target")?;
        let prepared =
            raster_conversion::prepare(&work, entry.key, Action::Rasterize(Expression::Color))?;
        let affected = prepared
            .affected
            .iter()
            .filter_map(|k| crate::layer_catalog::target(&work, *k))
            .collect::<Vec<_>>();
        let list = names(&work, &affected);
        losses.push(format!("{}: cached raster replaces editable {:?} (text/ruby/fonts, vector/gradient or 3D/material/rig data remain only in .drawer)",list.join(", "),t));
        ranges.push(list);
        prepared.state.restore(&mut work);
    }
    for _ in 0..512 {
        canceled(cancel)?;
        let target = props::targets(&work)
            .into_iter()
            .filter(|t| matches!(t, Target::Raster(_) | Target::Folder(_)))
            .find(|t| extra(&work, *t));
        let Some(t) = target else {
            break;
        };
        // A local bake preserves unrelated siblings; backdrop-dependent corrections and clipping expand the interval.
        if crate::layer_effects::adjustment(&work, t).is_none()
            && blend_key(props::properties(&work, t).blend).is_some()
            && let Ok(p) = raster_conversion::prepare(
                &work,
                crate::layer_catalog::key(&work, t),
                Action::Rasterize(Expression::Color),
            )
        {
            let affected = p
                .affected
                .iter()
                .filter_map(|k| crate::layer_catalog::target(&work, *k))
                .collect::<Vec<_>>();
            let list = names(&work, &affected);
            losses.push(format!("{}: paper/effect/expression/file cache rasterized; mask/effect parameters and source links omitted",list.join(", ")));
            ranges.push(list);
            p.state.restore(&mut work);
            continue;
        }
        let p = merge_dependency(&work, t, cancel)?;
        let list = names(&work, &p.affected);
        losses.push(format!("{}: consecutive dependency range baked with its backdrop; correction/pass-through/clipping edit data omitted",list.join(", ")));
        ranges.push(list);
        p.state.restore(&mut work);
    }
    if props::targets(&work)
        .into_iter()
        .any(|t| extra(&work, t) && matches!(t, Target::Raster(_) | Target::Folder(_)))
    {
        return Err("PSD conversion operation limit".into());
    }
    let image = capture(&work, merged.to_vec(), cancel)?;
    let imported = image.document(budget)?;
    let delta = difference(merged, &render(&imported, budget, cancel)?);
    if delta > 1 {
        return Err(format!(
            "Partial interchange differs by {delta} premultiplied RGBA8 levels; use the whole composite"
        ));
    }
    Ok(image)
}
pub fn prepare(d: &Document, budget: usize, cancel: &AtomicBool) -> Result<Plan, String> {
    canceled(cancel)?;
    if !(1..=32768).contains(&d.width)
        || !(1..=32768).contains(&d.height)
        || !d.dpi.is_finite()
        || !(1.0..=9600.0).contains(&d.dpi)
        || !d.layers.iter().all(|l| l.valid(d.width, d.height))
        || !d.color.valid_layers(&d.layers)
        || !d.scene.valid()
        || !d.scene.paper.valid_for(d.layers.len())
        || d.memory_bytes()
            .saturating_mul(6)
            .saturating_add(d.resident_bytes())
            > budget
    {
        return Err("Invalid document or PSD snapshot memory reserve".into());
    }
    let merged = render(d, budget, cancel)?;
    let flattened = Image {
        width: d.width,
        height: d.height,
        dpi: d.dpi,
        layers: vec![Layer {
            name: "Rendered whole composite".into(),
            bounds: [0, 0, d.height as i32, d.width as i32],
            pixels: merged.clone(),
            mask: None,
            blend: *b"norm",
            opacity: 255,
            hidden: false,
            clipped: false,
            alpha_lock: false,
            section: 0,
        }],
        merged,
    };
    let mut losses=vec!["PSD/PSB exchange is RGB8 sRGB. Native history, locks, guides, book/timeline metadata and application settings are omitted; keep the .drawer source.".into()];
    if d.color.depth == crate::color_precision::Depth::Sixteen {
        losses
            .push("16-bit source samples are quantized to 8-bit only in the exported file".into());
    }
    if d.color.profile != crate::color_profile::RgbProfile::Srgb
        && d.color.profile != crate::color_profile::RgbProfile::AssumedSrgb
    {
        losses.push("Document RGB profile converted to sRGB; non-sRGB blend appearance may require the whole composite".into());
    }
    if props::targets(d)
        .into_iter()
        .any(|t| crate::layer_masks::get(d, t).is_some())
    {
        losses.push("Mask geometry is sampled inside the current canvas; outside coverage and separate link/transform editing are not preserved".into());
    }
    let mut ranges = vec![];
    let mut partial_losses = vec![];
    let (partial, reason) = match partial(
        d,
        &flattened.merged,
        budget,
        cancel,
        &mut partial_losses,
        &mut ranges,
    ) {
        Ok(i) => {
            losses.extend(partial_losses);
            (Some(i), None)
        }
        Err(e) => {
            canceled(cancel)?;
            ranges.clear();
            losses.push(format!("Partial conversion unavailable: {e}. The whole composite loses all separate layer/group/mask/object editing."));
            (None, Some(e))
        }
    };
    let delta = if let Some(i) = &partial {
        difference(
            &flattened.merged,
            &render(&i.document(budget)?, budget, cancel)?,
        )
    } else {
        0
    };
    let whole_names = crate::layer_catalog::rows(d)?
        .into_iter()
        .map(|r| r.name)
        .collect();
    Ok(Plan {
        partial,
        flattened,
        losses,
        ranges,
        whole_names,
        reason,
        delta,
        source: d.id,
        revision: d.revision,
        frame: d.current_frame,
    })
}
