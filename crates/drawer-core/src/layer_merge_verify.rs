use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    layer_effects::{Adjustment, Effect, Mask},
    layer_merge::{self as merge, Operation, Settings},
    layer_properties::{self as p, Patch, Target},
    paper::{Folder, Node},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn make(colors: &[[u8; 4]]) -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for color in colors {
        let mut l = d.layers[0].clone();
        l.pixels.fill(*color);
        d.layers.push(l);
    }
    d.layers.remove(0);
    Ok(d)
}
fn pixels(d: &Document) -> Vec<[u8; 4]> {
    let r = crate::render::Renderer::new(d);
    (0..8)
        .flat_map(|y| (0..8).map(move |x| (x, y)))
        .map(|(x, y)| r.pixel([x as f64 + 0.5, y as f64 + 0.5]))
        .collect()
}
fn mask(alpha: u8) -> Mask {
    Mask {
        space: Default::default(),
        coverage: crate::selection::Mask {
            runs: if alpha == 0 {
                vec![]
            } else {
                (0..8)
                    .map(|y| crate::selection::Run {
                        x: 0,
                        y,
                        length: 8,
                        alpha,
                    })
                    .collect()
            },
        },
        offset: [0.; 2],
        enabled: true,
        linked: true,
        inverted: false,
    }
}
fn selected() -> Operation {
    Operation::Selected(vec![Target::Raster(1), Target::Raster(2)])
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = make(&[[0, 0, 255, 255], [255, 0, 0, 128], [0, 255, 0, 0]])?;
    let before = d.snapshot();
    let expected = pixels(&d);
    let count = d.history_counts().0;
    let result = merge::prepare(&d, selected(), Default::default())?;
    check(
        result.affected.len() == 2 && result.maximum_delta == 0,
        "Merge affected set/delta",
    )?;
    let new_id = result.selected;
    d.commit_state(result.state)?;
    check(
        d.layers.len() == 2
            && pixels(&d) == expected
            && p::targets(&d).contains(&new_id)
            && d.history_counts().0 == count + 1,
        "Selected merge color/identity/history",
    )?;
    let merged = d.snapshot();
    check(
        d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == merged,
        "Merge restores full structure",
    )?;
    let file = root.join("merged-layers.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot(),
        "Merged native reopen",
    )?;
    let mut down = make(&[[0, 0, 255, 255], [255, 0, 0, 128]])?;
    check(
        merge::prepare(
            &down,
            Operation::Down(Target::Raster(1)),
            Default::default(),
        )
        .is_err(),
        "Bottom merge-down accepted",
    )?;
    let s = merge::prepare(
        &down,
        Operation::Down(Target::Raster(2)),
        Default::default(),
    )?;
    down.commit_state(s.state)?;
    check(
        down.layers.len() == 1 && down.composite_pixel(0, 0) == [128, 0, 127, 255],
        "Merge down independent RGBA",
    )?;
    let mut hidden = make(&[[0, 0, 255, 255], [255, 0, 0, 255]])?;
    let s = p::prepare_patch(
        &hidden,
        &[Target::Raster(2)],
        Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    hidden.commit_state(s)?;
    let before = hidden.snapshot();
    check(
        merge::prepare(&hidden, selected(), Default::default()).is_err()
            && hidden.snapshot() == before,
        "Hidden exclusion must not silently consume",
    )?;
    let s = merge::prepare(
        &hidden,
        selected(),
        Settings {
            include_hidden: true,
            ..Default::default()
        },
    )?;
    hidden.commit_state(s.state)?;
    check(
        hidden.composite_pixel(0, 0) == [255, 0, 0, 255]
            && hidden.undo()
            && hidden.snapshot() == before,
        "Explicit hidden inclusion/Undo",
    )?;
    let mut clip = make(&[[0, 0, 255, 128], [255, 0, 0, 128], [0; 4]])?;
    let s = crate::clipping::prepare(&clip, &[Target::Raster(2)], None, Some(true))?;
    clip.commit_state(s)?;
    let original = clip.snapshot();
    let colors = pixels(&clip);
    check(
        merge::prepare(
            &clip,
            Operation::Selected(vec![Target::Raster(2), Target::Raster(3)]),
            Default::default(),
        )
        .is_err(),
        "Incomplete clipping run merged",
    )?;
    let s = merge::prepare(&clip, selected(), Default::default())?;
    clip.commit_state(s.state)?;
    check(
        pixels(&clip) == colors
            && clip.scene.clipping.rasters.is_empty()
            && clip.undo()
            && clip.snapshot() == original,
        "Clipping merged result/structure Undo",
    )?;
    for alpha in [0, 128, 255] {
        let mut d = make(&[[128, 128, 128, 255], [0; 4]])?;
        d.scene.layer_ids.initialize(2)?;
        d.scene.layer_effects.rasters.insert(
            2,
            Effect {
                adjustment: Some(Adjustment {
                    brightness: 0.2,
                    ..Default::default()
                }),
                mask: None,
            },
        );
        d.scene.paper.folders.push(Folder {
            name: "Through adjustment".into(),
            children: vec![Node::Raster(1)],
            through: true,
            paper: Default::default(),
        });
        d.scene.paper.roots = vec![Node::Raster(0), Node::Folder(0)];
        d.scene.layer_effects.folders.insert(
            0,
            Effect {
                mask: Some(mask(alpha)),
                adjustment: None,
            },
        );
        let file = root.join(format!("merge-source-effects-{alpha}.drawer"));
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "Source mask/adjustment native reopen",
        )?;
        let expected = match alpha {
            0 => 128,
            128 => 154,
            _ => 179,
        };
        check(
            d.composite_pixel(0, 0) == [expected, expected, expected, 255],
            "Through mask/adjustment independent value",
        )?;
        let before = d.snapshot();
        let colors = pixels(&d);
        let s = merge::prepare(
            &d,
            Operation::Selected(vec![Target::Raster(1), Target::Folder(0)]),
            Default::default(),
        )?;
        d.commit_state(s.state)?;
        check(
            pixels(&d) == colors
                && !p::targets(&d).contains(&Target::Folder(0))
                && d.undo()
                && d.snapshot() == before,
            "Mask/adjustment merge and consumed folder visibility",
        )?;
        let s = merge::prepare(&d, Operation::Flatten, Default::default())?;
        d.commit_state(s.state)?;
        check(
            pixels(&d) == colors && d.scene.layer_effects.is_default(),
            "Flatten masks/adjustments exact",
        )?;
    }
    let mut backdrop = make(&[
        [60, 100, 180, 255],
        [100, 200, 30, 128],
        [180, 20, 200, 128],
    ])?;
    let s = p::prepare_patch(
        &backdrop,
        &[Target::Raster(3)],
        Patch {
            blend: Some(crate::layer_blend::Mode::Screen),
            ..Default::default()
        },
    )?;
    backdrop.commit_state(s)?;
    let op = || Operation::Selected(vec![Target::Raster(2), Target::Raster(3)]);
    let before = backdrop.snapshot();
    check(
        merge::prepare(&backdrop, op(), Default::default()).is_err()
            && backdrop.snapshot() == before,
        "Outside blend dependency must reject atomically",
    )?;
    let colors = pixels(&backdrop);
    let s = merge::prepare(
        &backdrop,
        op(),
        Settings {
            lower_backdrop: true,
            ..Default::default()
        },
    )?;
    backdrop.commit_state(s.state)?;
    check(
        backdrop.layers.len() == 1 && pixels(&backdrop) == colors,
        "Explicit lower backdrop preserves blend",
    )?;
    let mut flat = make(&[[50, 100, 150, 128], [255, 0, 0, 128]])?;
    let id = flat.scene.lines.add_layer("Editable line")?;
    flat.scene
        .lines
        .add_path(id, vec![[0., 0.5], [7., 0.5]], false, 1., [0, 0, 0, 255])?;
    flat.scene.objects.push(crate::scene::PageObject::Line {
        start: [0., 4.5],
        end: [7., 4.5],
        width_px: 1.,
        color: [0, 200, 0, 255],
    });
    flat.animation = Some(crate::animation::Timeline {
        fps: crate::animation::Fps {
            numerator: 24,
            denominator: 1,
        },
        frames: 2,
        tracks: vec![crate::animation::Track {
            name: "Animation".into(),
            cells: vec![crate::animation::Cell {
                name: "Cell".into(),
                width: 1,
                height: 1,
                offset: [3, 3],
                pixels: std::sync::Arc::new(vec![[255, 255, 0, 255]]),
            }],
            exposures: vec![crate::animation::Exposure { frame: 0, cell: 0 }],
            end: 2,
        }],
        audio: vec![],
    });
    flat.current_frame = 1;
    let before = flat.snapshot();
    let colors = pixels(&flat);
    let s = merge::prepare(&flat, Operation::Flatten, Default::default())?;
    check(
        s.notes.iter().any(|s| s.contains("animation/audio")),
        "Flatten loss disclosure",
    )?;
    flat.commit_state(s.state)?;
    check(
        pixels(&flat) == colors
            && flat.animation.is_none()
            && flat.scene.objects.is_empty()
            && flat.scene.lines.layers.is_empty()
            && flat.undo()
            && flat.snapshot() == before,
        "Flatten current frame/objects and full Undo",
    )?;
    let mut outside = make(&[[0, 0, 255, 128], [255, 0, 0, 128]])?;
    outside.layers[0].placement = Some(PlacedRaster {
        width: 8,
        height: 8,
        offset: [-2, 0],
    });
    let s = merge::prepare(&outside, selected(), Default::default())?;
    outside.commit_state(s.state)?;
    check(
        outside.layers[0].extent(8, 8).offset == [-2, 0]
            && outside.layers[0].pixel(8, 8, -2, 0) == [0, 0, 255, 128],
        "Selected merge preserves signed off-canvas pixels",
    )?;
    let mut huge = make(&[[0; 4], [0; 4]])?;
    huge.layers[0].placement = Some(PlacedRaster {
        width: 8,
        height: 8,
        offset: [-100000, 0],
    });
    huge.layers[1].placement = Some(PlacedRaster {
        width: 8,
        height: 8,
        offset: [100000, 0],
    });
    let before = huge.snapshot();
    check(
        merge::prepare(&huge, selected(), Default::default()).is_err() && huge.snapshot() == before,
        "Huge merge atomic rejection",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn merge_down_hidden_clipping_masks_adjustments_and_flatten() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
