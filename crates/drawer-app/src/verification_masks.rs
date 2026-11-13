use super::editor::check;
use drawer_core::{
    assets::BrushTip,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    selection::{Mask, Run},
};
use std::path::Path;
fn page() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 130,
            height: 4,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())
}
fn coverage(alpha: u8, x: i32, length: u32) -> Mask {
    Mask {
        runs: if alpha == 0 {
            vec![]
        } else {
            (0..4)
                .map(|y| Run {
                    x,
                    y,
                    length,
                    alpha,
                })
                .collect()
        },
    }
}
fn pixels(d: &Document) -> Vec<[u8; 4]> {
    (0..520)
        .map(|n| {
            d.layers[0]
                .tiled
                .as_ref()
                .map_or_else(|| d.layers[0].pixels[n], |t| t.get(n).unwrap())
        })
        .collect()
}
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    let mut cases = vec![];
    for tip in [BrushTip::Pen, BrushTip::Eraser] {
        for locked in [false, true] {
            for alpha in [0, 128, 255] {
                let mut dense = None;
                for sparse in [false, true] {
                    let mut d = page()?;
                    for (n, p) in d.layers[0].pixels.iter_mut().enumerate() {
                        *p = [20, 80, 160, if n % 130 == 64 { 0 } else { 128 }];
                    }
                    let raw = d.layers[0].pixels.clone();
                    if sparse {
                        let mut tiles = drawer_core::tiled_raster::Image::new(130, 4, [0; 4]);
                        tiles.prepare(0..520, B)?;
                        for (n, p) in raw.iter().enumerate() {
                            tiles.set_prepared(n, *p);
                        }
                        d.layers[0].tiled = Some(tiles);
                        d.layers[0].pixels.clear();
                    }
                    d.scene.selection_mask = Some(coverage(alpha, 63, 2));
                    if locked {
                        let target = drawer_core::layer_properties::raster(&d, 0);
                        d.scene.clipping.alpha.insert(target);
                    }
                    let before = d.snapshot();
                    let count = d.history_counts();
                    let mut stroke = drawer_core::painting::Stroke::begin(
                        &d,
                        0,
                        drawer_core::selection_paint::style(tip)?,
                    )?;
                    stroke.sample(&d, [60.5, 1.5])?;
                    stroke.sample(&d, [67.5, 1.5])?;
                    stroke.finish_material(&d, || false)?;
                    let changed = stroke.commit(&mut d)?;
                    let result = pixels(&d);
                    for (n, (a, b)) in result.iter().zip(&raw).enumerate() {
                        if alpha == 0 || !(63..65).contains(&(n % 130)) {
                            check(
                                a == b,
                                "Brush/eraser changed a selection-blocked original RGBA pixel across a tile edge",
                            )?;
                        }
                        if locked {
                            check(a[3] == b[3], "Alpha lock changed original alpha")?;
                            if b[3] == 0 {
                                check(a == b, "Alpha lock changed transparent hidden RGB")?;
                            }
                        }
                    }
                    if sparse {
                        check(
                            dense.as_ref() == Some(&result),
                            "Sparse/dense selection and alpha-lock results differ at x63/64",
                        )?;
                    } else {
                        dense = Some(result);
                    }
                    let after = d.snapshot();
                    if changed {
                        check(
                            d.history_counts().0 == count.0 + 1
                                && d.undo()
                                && d.snapshot() == before
                                && d.redo()
                                && d.snapshot() == after,
                            "Masked tile stroke lost semantic Undo/Redo",
                        )?;
                    } else {
                        check(
                            after == before && d.history_counts() == count,
                            "Masked no-op created history",
                        )?;
                    }
                    cases.push(serde_json::json!({"tool":format!("{tip:?}"),"alphaLock":locked,"selection":alpha,"sparse":sparse,"changed":changed}));
                }
            }
        }
    }
    let mut filtered = page()?;
    for (n, p) in filtered.layers[0].pixels.iter_mut().enumerate() {
        *p = if n % 2 == 0 { [0, 0, 0, 255] } else { [255; 4] };
    }
    filtered.scene.selection_mask = Some(coverage(255, 63, 2));
    let before = filtered.snapshot();
    let state = drawer_core::spatial_blur::prepare(
        &filtered,
        0,
        drawer_core::spatial_blur::Settings {
            length: 4.,
            selection: true,
            ..Default::default()
        },
        |_, _| Ok(()),
    )?;
    filtered.commit_state(state)?;
    let mut changed = 0;
    for n in 0..520 {
        let a = filtered.layers[0]
            .sample(130, 4, (n % 130) as i64, (n / 130) as i64)
            .rgba8();
        let b = before.layers[0].pixels[n];
        if !(63..65).contains(&(n % 130)) {
            check(
                a == b,
                "Selected filter changed an original outside-selection pixel",
            )?;
        } else {
            changed += usize::from(a != b);
        }
    }
    check(
        changed > 0 && filtered.undo() && filtered.snapshot() == before,
        "Selected filter failed to change selected pixels or restore on Undo",
    )?;
    let mut output_checks = vec![];
    for alpha in [0, 128, 255] {
        let mut d = page()?;
        d.layers[0].pixels.fill([80, 100, 160, 255]);
        let stack = drawer_core::surface_effects::Stack {
            effects: vec![drawer_core::surface_effects::Entry {
                operation: drawer_core::surface_effects::Operation::Glow {
                    intensity: 1.,
                    spread: 1,
                    blur: 0,
                    color: [255; 3],
                },
                enabled: true,
                scope: None,
            }],
            output_mask: Some(drawer_core::layer_effects::Mask {
                space: Default::default(),
                coverage: coverage(alpha, 0, 130),
                offset: [0.; 2],
                enabled: true,
                linked: false,
                inverted: false,
            }),
            ..Default::default()
        };
        let state = drawer_core::surface_effects::prepare(&d, 0, stack, |_, _| Ok(()))?;
        d.commit_state(state)?;
        let r = drawer_core::float_render::Renderer::new(&d)?;
        for x in [62, 63, 64, 65, 127, 128, 129] {
            let actual = r.pixel([f64::from(x) + 0.5, 1.5])?.rgba8()[3];
            check(
                actual == alpha,
                "Final effect output mask is not applied once at a tile boundary",
            )?;
        }
        output_checks.push(serde_json::json!({"finalOutputMask":alpha,"x":[62,63,64,65,127,128,129],"expectedAlpha":alpha}));
    }
    drawer_core::selection_paint::verify(root)?;
    drawer_core::layer_masks_verify::verify(root)?;
    drawer_core::surface_effects_verify::composition()?;
    drawer_core::surface_effects_verify::precision_and_limits(root)?;
    drawer_core::filter_options_verify::verify(root)?;
    Ok(
        serde_json::json!({"tileStrokeCases":cases,"selectedFilterOutsideOriginalUnchanged":true,"finalOutputMaskChecks":output_checks,"sevenToolIndependentSoftCoverage":true,"sourceClippingBeforeSpatialExpansion":true,"parentFinalMaskSuppressesHalo":true,"precision16AndNativeUndo":true}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn selection_alpha_lock_sparse_tile_edges_filters_source_and_final_effect_masks() {
        let r = crate::tests::TestDirectory::new();
        super::run(&r.0).unwrap();
    }
}
