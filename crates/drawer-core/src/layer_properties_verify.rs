use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::{self as core, Mark, Patch, Target},
    paper::{Folder, Node},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn make() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn set(d: &mut Document, t: Target, p: Patch) -> Result<(), String> {
    let state = core::prepare_patch(d, &[t], p)?;
    d.commit_state(state)?;
    Ok(())
}
fn pixels(d: &Document) -> Vec<[u8; 4]> {
    let r = crate::render::Renderer::new(d);
    (0..d.height)
        .flat_map(|y| (0..d.width).map(move |x| (x, y)))
        .map(|(x, y)| r.pixel([x as f64 + 0.5, y as f64 + 0.5]))
        .collect()
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = make()?;
    d.layers[0].pixels.fill([200, 40, 20, 128]);
    let source = d.snapshot();
    check(
        core::prepare_patch(&d, &[Target::Raster(1)], Default::default())? == source
            && core::prepare_patch(
                &d,
                &[Target::Raster(1)],
                Patch {
                    opacity: Some(101),
                    ..Default::default()
                },
            )
            .is_err()
            && core::prepare_patch(
                &d,
                &[Target::Raster(1), Target::Raster(1)],
                Patch {
                    hidden: Some(true),
                    ..Default::default()
                },
            )
            .is_err(),
        "Empty/invalid/duplicate property patch",
    )?;
    for (opacity, expected) in [
        (0, [0; 4]),
        (50, [200, 40, 20, 64]),
        (100, [200, 40, 20, 128]),
    ] {
        set(
            &mut d,
            Target::Raster(1),
            Patch {
                opacity: Some(opacity),
                ..Default::default()
            },
        )?;
        check(
            d.composite_pixel(0, 0) == expected && pixels(&d)[0] == expected,
            "Independent raster opacity/renderer",
        )?;
        if opacity != 100 {
            check(d.undo() && d.snapshot() == source, "Opacity Undo")?;
        }
    }
    d.layers.push(d.layers[0].clone());
    d.layers[0].pixels.fill([0, 0, 255, 255]);
    set(
        &mut d,
        Target::Raster(2),
        Patch {
            opacity: Some(50),
            ..Default::default()
        },
    )?;
    check(
        d.composite_pixel(0, 0) == [50, 10, 196, 255],
        "Opacity with opaque backdrop",
    )?;
    let before = pixels(&d);
    let first = root.join("layer-label-before.png");
    let second = root.join("layer-label-after.png");
    crate::png_export::export(&d, &first, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    set(
        &mut d,
        Target::Raster(1),
        Patch {
            color: Some(Some([255, 0, 0])),
            ..Default::default()
        },
    )?;
    set(
        &mut d,
        Target::Raster(2),
        Patch {
            color: Some(Some([0, 0, 255])),
            ..Default::default()
        },
    )?;
    crate::png_export::export(&d, &second, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        pixels(&d) == before
            && std::fs::read(first).map_err(|e| e.to_string())?
                == std::fs::read(second).map_err(|e| e.to_string())?,
        "Identification colors changed PNG output",
    )?;
    let f = d.scene.paper.group_all(2, "Mixed folder")?;
    check(
        core::mark(&d, Target::Folder(f as u16)) == Mark::Mixed,
        "Mixed parent label",
    )?;
    set(
        &mut d,
        Target::Raster(2),
        Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    let state = core::prepare_patch(
        &d,
        &[Target::Raster(1), Target::Raster(2)],
        Patch {
            opacity: Some(25),
            ..Default::default()
        },
    )?;
    d.commit_state(state)?;
    check(
        !core::properties(&d, Target::Raster(1)).hidden
            && core::properties(&d, Target::Raster(2)).hidden
            && core::properties(&d, Target::Raster(1)).color == Some([255, 0, 0])
            && core::properties(&d, Target::Raster(2)).color == Some([0, 0, 255]),
        "Batch patch overwrote unrelated values",
    )?;
    let file = root.join("layer-properties.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot(),
        "Properties native reopen",
    )?;

    for (through, expected) in [(false, [157, 70, 60, 255]), (true, [85, 50, 50, 255])] {
        let mut group = make()?;
        group.layers[0].pixels.fill([100, 100, 100, 255]);
        group.layers.push(group.layers[0].clone());
        group.layers[1].pixels.fill([214, 40, 20, 255]);
        group.scene.paper.folders.push(Folder {
            through,
            name: "Group".into(),
            children: vec![Node::Raster(1)],
            paper: Default::default(),
        });
        group.scene.paper.roots = vec![Node::Raster(0), Node::Folder(0)];
        group.scene.paper.layers.entry(1).or_default().blend =
            crate::brush_composite::Mode::BlackBurn;
        set(
            &mut group,
            Target::Folder(0),
            Patch {
                opacity: Some(50),
                ..Default::default()
            },
        )?;
        check(
            group.composite_pixel(0, 0) == expected,
            "Normal/pass-through folder opacity",
        )?;
        set(
            &mut group,
            Target::Folder(0),
            Patch {
                hidden: Some(true),
                ..Default::default()
            },
        )?;
        check(
            group.composite_pixel(0, 0) == [100, 100, 100, 255]
                && !group.scene.paper.raster_visible(1, 2),
            "Hidden folder leaked children through fallback",
        )?;
    }
    let mut vector = make()?;
    let id = vector.scene.lines.add_layer("Vector opacity")?;
    for color in [[200, 40, 20, 255], [0, 200, 0, 255]] {
        let state = crate::shapes::prepare(
            &vector,
            crate::shapes::Target::Vector(Some(id)),
            &[[0.0, 0.0], [4.0, 4.0]],
            crate::shapes::Settings {
                paint: crate::shapes::Paint::CreateFill,
                fill: color,
                ..Default::default()
            },
        )?;
        vector.commit_state(state)?;
    }
    set(
        &mut vector,
        Target::Vector(id),
        Patch {
            opacity: Some(50),
            ..Default::default()
        },
    )?;
    check(
        pixels(&vector)[2 * 8 + 2] == [0, 200, 0, 128],
        "Vector opacity was applied separately to overlapping paths",
    )?;
    set(
        &mut vector,
        Target::Vector(id),
        Patch {
            locked: Some(true),
            ..Default::default()
        },
    )?;
    let before = vector.snapshot();
    let mut scene = vector.scene.clone();
    scene.lines.layers[0].paths[0].color = [1, 2, 3, 255];
    check(
        vector.edit_scene(scene).is_err() && vector.snapshot() == before,
        "Locked vector scene edit",
    )?;
    set(
        &mut vector,
        Target::Vector(id),
        Patch {
            locked: Some(false),
            position_locked: Some(true),
            ..Default::default()
        },
    )?;
    check(
        crate::ruler_scope::translate_layer(
            &vector,
            crate::line_tools::Target::Vector(id),
            [1.0, 0.0],
        )
        .is_err(),
        "Vector position lock",
    )?;

    let mut locked = make()?;
    let style = crate::brush_presets::Preset::capture(
        &crate::workspaces::Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        },
        crate::assets::BrushTip::PixelPen,
        "Layer locks",
    )?
    .style([200, 40, 20, 255]);
    set(
        &mut locked,
        Target::Raster(1),
        Patch {
            pixels_locked: Some(true),
            ..Default::default()
        },
    )?;
    let before = locked.snapshot();
    let mut state = locked.snapshot();
    state.layers[0].pixels[0] = [1, 2, 3, 255];
    check(
        !locked.edit_pixel(0, 0, 0, [1, 2, 3, 255])
            && locked.commit_state(state).is_err()
            && crate::painting::Stroke::begin(&locked, 0, style).is_err()
            && locked.snapshot() == before,
        "Pixel lock bypassed an edit path",
    )?;
    let move_layer = crate::precision::Nudge::begin(&locked, 0, crate::precision::Target::Layer)?;
    locked.commit_state(move_layer.preview_total(&locked, [1, 0])?)?;
    set(
        &mut locked,
        Target::Raster(1),
        Patch {
            pixels_locked: Some(false),
            position_locked: Some(true),
            ..Default::default()
        },
    )?;
    check(
        locked.edit_pixel(0, 1, 0, [200, 40, 20, 255])
            && crate::precision::Nudge::begin(&locked, 0, crate::precision::Target::Layer).is_err(),
        "Position/pixel locks were not independent",
    )?;
    set(
        &mut locked,
        Target::Raster(1),
        Patch {
            position_locked: Some(false),
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    check(
        crate::painting::Stroke::begin(&locked, 0, style).is_err()
            && crate::layer_pick::candidates(&locked, [1.5, 0.5], Default::default()).is_empty(),
        "Hidden layer editing/picking",
    )?;
    set(
        &mut locked,
        Target::Raster(1),
        Patch {
            hidden: Some(false),
            locked: Some(true),
            ..Default::default()
        },
    )?;
    check(
        crate::layers::prepare(
            &locked,
            crate::layers::Command::Rename {
                target: crate::layers::Target::Raster(1),
                name: "Denied".into(),
            },
        )
        .is_err(),
        "Full lock allowed structural change",
    )?;
    set(
        &mut locked,
        Target::Raster(1),
        Patch {
            locked: Some(false),
            ..Default::default()
        },
    )?;

    let mut family = make()?;
    family.layers.push(family.layers[0].clone());
    let folder = family.scene.paper.group_all(2, "Locked family")?;
    set(
        &mut family,
        Target::Folder(folder as u16),
        Patch {
            locked: Some(true),
            ..Default::default()
        },
    )?;
    let before = family.snapshot();
    let create = crate::layers::prepare(
        &family,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "Blocked child".into(),
            parent: Some(folder),
        },
    )
    .and_then(|p| family.commit_state(p.state));
    check(
        create.is_err()
            && crate::painting::Stroke::begin(&family, 0, style).is_err()
            && family.snapshot() == before,
        "Parent folder locks were not inherited",
    )?;
    let mut baked = make()?;
    baked.layers[0].pixels.fill([200, 40, 20, 128]);
    set(
        &mut baked,
        Target::Raster(1),
        Patch {
            opacity: Some(50),
            color: Some(Some([20, 40, 80])),
            ..Default::default()
        },
    )?;
    let before = pixels(&baked);
    crate::paper::bake(&mut baked, Node::Raster(0))?;
    check(
        pixels(&baked) == before
            && core::properties(&baked, Target::Raster(1)).opacity == 100
            && core::properties(&baked, Target::Raster(1)).color == Some([20, 40, 80]),
        "Raster bake applied opacity twice / lost label",
    )?;
    let mut animation = make()?;
    let shared = std::sync::Arc::new(vec![[200, 40, 20, 128]; 4]);
    animation.animation = Some(crate::animation::Timeline {
        fps: Default::default(),
        frames: 2,
        tracks: vec![crate::animation::Track {
            name: "Shared clips".into(),
            cells: vec![crate::animation::Cell {
                name: "Shared cell".into(),
                width: 2,
                height: 2,
                offset: [0, 0],
                pixels: shared,
            }],
            exposures: vec![
                crate::animation::Exposure { frame: 0, cell: 0 },
                crate::animation::Exposure { frame: 1, cell: 0 },
            ],
            end: 2,
        }],
        audio: vec![],
    });
    set(
        &mut animation,
        Target::Track(0),
        Patch {
            opacity: Some(50),
            color: Some(Some([0, 0, 255])),
            ..Default::default()
        },
    )?;
    set(
        &mut animation,
        Target::Cell { track: 0, cell: 0 },
        Patch {
            opacity: Some(50),
            color: Some(Some([255, 0, 0])),
            ..Default::default()
        },
    )?;
    for frame in [0, 1] {
        animation.current_frame = frame;
        check(
            animation.composite_pixel(0, 0) == [200, 40, 20, 32]
                && core::canvas_mark(&animation, crate::thumbnail::Target::Track(0))
                    == Mark::Color([255, 0, 0]),
            "Shared clip color/track-cell opacity",
        )?;
    }
    set(
        &mut animation,
        Target::Cell { track: 0, cell: 0 },
        Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    check(
        animation.composite_pixel(0, 0) == [0; 4]
            && core::canvas_mark(&animation, crate::thumbnail::Target::Track(0)) == Mark::None,
        "Hidden shared cell / overlay",
    )?;
    set(
        &mut animation,
        Target::Cell { track: 0, cell: 0 },
        Patch {
            hidden: Some(false),
            pixels_locked: Some(true),
            ..Default::default()
        },
    )?;
    let before = animation.snapshot();
    let mut state = animation.snapshot();
    std::sync::Arc::make_mut(&mut state.animation.as_mut().unwrap().tracks[0].cells[0].pixels)[0] =
        [1, 2, 3, 255];
    check(
        animation.commit_state(state).is_err() && animation.snapshot() == before,
        "Animation pixel lock",
    )?;
    let file = root.join("layer-properties-animation.drawer");
    crate::files::save(&mut animation, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == animation.snapshot(),
        "Shared cell properties native reopen",
    )?;
    let workspace = crate::workspaces::Snapshot {
        layer_colors: core::Display {
            list: false,
            parents: false,
            canvas: true,
            timeline: false,
        },
        ..Default::default()
    };
    let path = root.join("layer-colors-workspace.json");
    workspace.export(&path).map_err(|e| e.to_string())?;
    check(
        crate::workspaces::Snapshot::read(&path)
            .map_err(|e| e.to_string())?
            .layer_colors
            == workspace.layer_colors,
        "Identification display settings reopen",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn opacity_locks_batch_labels_output_and_shared_cells() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
