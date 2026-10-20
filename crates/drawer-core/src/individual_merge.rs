//! Raw Normal source-over and an independent max mask, with no appearance-preservation promise.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster, RasterLayer},
    layer_effects::Mask,
    layer_properties::{self as p, Target},
    paper::Node,
};
pub(crate) fn preflight(
    d: &Document,
    targets: &[Target],
    settings: crate::layer_merge::Settings,
) -> Result<(), String> {
    if targets.len() != 2
        || settings.lower_backdrop
        || targets.iter().any(|t| !matches!(t, Target::Raster(_)))
    {
        return Err("Individual combine requires exactly two raster siblings without a lower backdrop; use Merge selected for other sources".into());
    }
    for target in targets {
        let flags = p::inherited(d, *target);
        let properties = p::properties(d, *target);
        let Target::Raster(id) = target else {
            unreachable!()
        };
        let n = crate::layers::index(d, *id).ok_or("Missing individual-combine raster")?;
        if flags.locked || flags.pixels_locked || flags.position_locked {
            return Err("Unlock both source rasters and their parent before combining".into());
        }
        if !properties.blend.is_normal()
            || properties.opacity != 100
            || crate::layer_effects::adjustment(d, *target).is_some()
            || d.scene
                .paper
                .layers
                .get(&n)
                .is_some_and(|p| p.blend != crate::brush_composite::Mode::Normal)
            || d.scene
                .file_objects
                .get(crate::layer_search::Key::Layer(*target))
                .is_some()
        {
            return Err("Individual combine needs plain Normal rasters at 100% opacity; use the ordinary merge preview to bake blend modes, corrections or file sources".into());
        }
        if d.scene.clipping.binding(*target).is_some()
            || d.scene
                .clipping
                .rasters
                .values()
                .chain(d.scene.clipping.vectors.values())
                .chain(d.scene.clipping.folders.values())
                .any(|base| base == target)
        {
            return Err("Use ordinary merge for a clipping run, or release clipping before individual combine".into());
        }
    }
    Ok(())
}
pub(crate) fn pixels(
    d: &Document,
    nodes: &[Node],
    extent: PlacedRaster,
) -> Result<RasterLayer, String> {
    let [Node::Raster(lower), Node::Raster(upper)] = nodes else {
        return Err("Choose two raster siblings".into());
    };
    let count = extent.width as usize * extent.height as usize;
    if count > 4 * 1024 * 1024
        || d.resident_bytes()
            .saturating_add(d.memory_bytes().saturating_mul(2))
            .saturating_add(count.saturating_mul(44))
            .saturating_add(32 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Individual combine exceeds floating work memory".into());
    }
    let space = crate::color_profile::WorkingSpace::new(&d.color.profile)?;
    let mut samples = Vec::with_capacity(count);
    for y in 0..extent.height {
        for x in 0..extent.width {
            let point = [extent.offset[0] + x as i64, extent.offset[1] + y as i64];
            let a = d.layers[*lower].sample(d.width, d.height, point[0], point[1]);
            let b = d.layers[*upper].sample(d.width, d.height, point[0], point[1]);
            samples.push(space.compose(a, b, crate::layer_blend::Mode::Normal, 1.)?);
        }
    }
    let mut raster = RasterLayer {
        precise: None,
        locked: false,
        name: "Combined raw pixels and mask".into(),
        pixels: vec![],
        placement: Some(extent),
    };
    crate::color_storage::set_samples(&mut raster, &d.color, extent, samples)?;
    Ok(raster)
}
pub(crate) fn mask(d: &Document, nodes: &[Node]) -> Result<Mask, String> {
    let masks = nodes
        .iter()
        .map(|n| crate::layer_masks::get(d, crate::clipping::node_key(d, *n)))
        .collect::<Vec<_>>();
    let frame = d.scene.transform.inverse();
    let empty = |inverted| Mask {
        space: frame,
        coverage: Default::default(),
        offset: [0.; 2],
        enabled: true,
        linked: true,
        inverted,
    };
    if masks
        .iter()
        .any(|m| m.is_none_or(|m| !m.enabled || m.inverted && m.coverage.runs.is_empty()))
    {
        return Ok(empty(true));
    }
    let masks = masks.into_iter().flatten().collect::<Vec<_>>();
    let inverted = masks.iter().any(|m| m.inverted);
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for mask in &masks {
        for r in &mask.coverage.runs {
            for point in [
                [r.x as f64, r.y as f64],
                [r.x as f64 + r.length as f64, r.y as f64],
                [r.x as f64 + r.length as f64, r.y as f64 + 1.],
                [r.x as f64, r.y as f64 + 1.],
            ] {
                let p = d.scene.transform.map(
                    mask.space
                        .map([point[0] + mask.offset[0], point[1] + mask.offset[1]]),
                );
                for n in 0..2 {
                    min[n] = min[n].min(p[n]);
                    max[n] = max[n].max(p[n]);
                }
            }
        }
    }
    if !min[0].is_finite() {
        return Ok(empty(inverted));
    }
    let coverage = crate::layer_masks::raster(min, max, |point| {
        let original = d.scene.transform.inverse().map(point);
        let value = (masks
            .iter()
            .map(|m| m.factor(original))
            .fold(0f32, f32::max)
            * 255.)
            .round() as u8;
        if inverted { 255 - value } else { value }
    })?;
    Ok(Mask {
        coverage,
        ..empty(inverted)
    })
}
