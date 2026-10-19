use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_folders::{self as f, Command},
    layer_properties::{self as p, Patch, Target},
    scene::Rect,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn make() -> Result<Document, String> {
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
    d.layers[0].pixels.fill([200, 100, 50, 255]);
    d.layers.push(d.layers[0].clone());
    Ok(d)
}
fn apply(d: &mut Document, c: Command) -> Result<Target, String> {
    let s = f::prepare(d, c)?;
    d.commit_state(s.state)?;
    Ok(s.selected)
}
fn full() -> Rect {
    Rect {
        min: [0.; 2],
        max: [8.; 2],
    }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = make()?;
    let before = d.snapshot();
    let folder = apply(
        &mut d,
        Command::Wrap {
            name: "Group".into(),
            targets: vec![Target::Raster(2), Target::Raster(1)],
        },
    )?;
    check(
        folder == Target::Folder(0)
            && d.scene.paper.folders[0].children
                == vec![crate::paper::Node::Raster(0), crate::paper::Node::Raster(1)]
            && d.composite_pixel(0, 0) == [200, 100, 50, 255],
        "Wrapping keeps sibling order and through appearance",
    )?;
    let grouped = d.snapshot();
    check(
        d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == grouped,
        "Wrap single structure Undo/Redo",
    )?;
    apply(
        &mut d,
        Command::Create {
            name: "Empty child".into(),
            parent: Some(0),
        },
    )?;
    check(
        d.scene.paper.folders[1].children.is_empty(),
        "Create nested empty folder",
    )?;
    let original = d.snapshot();
    for c in [
        Command::Move {
            folder: 0,
            parent: Some(0),
            position: 0,
        },
        Command::Move {
            folder: 0,
            parent: Some(1),
            position: 0,
        },
        Command::Move {
            folder: 1,
            parent: None,
            position: 999,
        },
        Command::Wrap {
            name: "Mixed parents".into(),
            targets: vec![Target::Raster(1), Target::Folder(0)],
        },
    ] {
        check(
            f::prepare(&d, c).is_err() && d.snapshot() == original,
            "Folder cycle/mixed parent/out-of-range atomic rejection",
        )?;
    }
    apply(
        &mut d,
        Command::Move {
            folder: 1,
            parent: None,
            position: 0,
        },
    )?;
    check(
        d.scene.paper.roots[0] == crate::paper::Node::Folder(1)
            && d.undo()
            && d.snapshot() == original,
        "Folder move/Undo",
    )?;
    apply(&mut d, Command::Unwrap(1))?;
    check(
        !p::targets(&d).contains(&Target::Folder(1)),
        "Empty folder unwrap removes it from active tree",
    )?;
    for (layer, parent, expected) in [
        (255, 255, 255),
        (128, 255, 128),
        (128, 128, 64),
        (0, 255, 0),
    ] {
        let mut d = make()?;
        d.layers[1].pixels.fill([0; 4]);
        apply(
            &mut d,
            Command::Wrap {
                name: "Outer".into(),
                targets: vec![Target::Raster(1), Target::Raster(2)],
            },
        )?;
        apply(
            &mut d,
            Command::Mode {
                folder: 0,
                through: false,
            },
        )?;
        let mask = f::rectangle_mask(&d, full(), layer)?;
        apply(
            &mut d,
            Command::Mask {
                target: Target::Raster(1),
                mask: Some(mask),
            },
        )?;
        let mask = f::rectangle_mask(&d, full(), parent)?;
        apply(
            &mut d,
            Command::Mask {
                target: Target::Folder(0),
                mask: Some(mask),
            },
        )?;
        let pixel = d.composite_pixel(0, 0);
        check(
            pixel[3] == expected && (expected == 0 || pixel[..3] == [200, 100, 50]),
            "Nested layer/folder grayscale alpha product",
        )?;
        let source = d.snapshot();
        check(
            f::prepare(&d, Command::Unwrap(0)).is_err() && d.snapshot() == source,
            "Masked folder needs explicit raster preservation",
        )?;
        let file = root.join(format!("layer-mask-{layer}-{parent}.drawer"));
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "Nested masks native reopen",
        )?;
    }
    let mut moving = make()?;
    moving.layers[1].pixels.fill([0; 4]);
    let mask = f::rectangle_mask(
        &moving,
        Rect {
            min: [0., 0.],
            max: [2., 8.],
        },
        128,
    )?;
    apply(
        &mut moving,
        Command::Mask {
            target: Target::Raster(1),
            mask: Some(mask),
        },
    )?;
    let before = moving.snapshot();
    let mut n = crate::precision::Nudge::begin(&moving, 0, crate::precision::Target::Layer)?;
    n.step(&mut moving, [1, 0])?;
    n.step(&mut moving, [1, 0])?;
    check(
        moving.scene.layer_effects.rasters[&1]
            .mask
            .as_ref()
            .unwrap()
            .offset
            == [2., 0.]
            && moving.composite_pixel(2, 0) == [200, 100, 50, 128]
            && moving.composite_pixel(4, 0) == [0; 4]
            && moving.undo()
            && moving.snapshot() == before,
        "Linked mask whole-layer movement/repeats/Undo",
    )?;
    let rotated = crate::sizing::prepare(
        &moving.snapshot(),
        crate::sizing::Change::Rotate90 { clockwise: true },
        DEFAULT_DOCUMENT_BUDGET,
        &std::sync::atomic::AtomicBool::new(false),
    )?;
    moving.commit_state(rotated)?;
    let mask = f::rectangle_mask(
        &moving,
        Rect {
            min: [0., 0.],
            max: [8., 2.],
        },
        255,
    )?;
    apply(
        &mut moving,
        Command::Mask {
            target: Target::Raster(1),
            mask: Some(mask),
        },
    )?;
    check(
        moving.composite_pixel(0, 0)[3] == 255 && moving.composite_pixel(0, 3) == [0; 4],
        "Mask creation uses transformed document coordinates",
    )?;
    let mut unwrap = make()?;
    apply(
        &mut unwrap,
        Command::Wrap {
            name: "Plain".into(),
            targets: vec![Target::Raster(1), Target::Raster(2)],
        },
    )?;
    let source = unwrap.snapshot();
    apply(&mut unwrap, Command::Unwrap(0))?;
    check(
        unwrap.scene.paper.root_nodes(2)
            == vec![crate::paper::Node::Raster(0), crate::paper::Node::Raster(1)]
            && unwrap.undo()
            && unwrap.snapshot() == source,
        "Plain unwrap preserves order/Undo",
    )?;
    let mut isolated = make()?;
    isolated.layers[0].pixels.fill([128, 128, 128, 255]);
    isolated.layers[1].pixels.fill([0; 4]);
    isolated.scene.layer_ids.initialize(2)?;
    isolated.scene.layer_effects.rasters.insert(
        2,
        crate::layer_effects::Effect {
            mask: None,
            adjustment: Some(crate::layer_effects::Adjustment {
                brightness: 0.2,
                ..Default::default()
            }),
        },
    );
    apply(
        &mut isolated,
        Command::Wrap {
            name: "Correction".into(),
            targets: vec![Target::Raster(2)],
        },
    )?;
    apply(
        &mut isolated,
        Command::Mode {
            folder: 0,
            through: false,
        },
    )?;
    let before = isolated.snapshot();
    check(
        f::prepare(&isolated, Command::Unwrap(0)).is_err() && isolated.snapshot() == before,
        "Isolated adjustment unwrap external dependency",
    )?;
    apply(
        &mut isolated,
        Command::Mode {
            folder: 0,
            through: true,
        },
    )?;
    check(
        isolated.composite_pixel(0, 0) == [179, 179, 179, 255],
        "Through adjustment reaches incoming backdrop",
    )?;
    let lock = p::prepare_patch(
        &isolated,
        &[Target::Folder(0)],
        Patch {
            locked: Some(true),
            ..Default::default()
        },
    )?;
    isolated.commit_state(lock)?;
    check(
        f::prepare(&isolated, Command::Unwrap(0)).is_err(),
        "Locked folder structure guard",
    )?;
    let mut v = make()?;
    v.layers.iter_mut().for_each(|l| l.pixels.fill([0; 4]));
    let id = v.scene.lines.add_layer("Masked vector")?;
    v.scene.lines.add_path(
        id,
        vec![[0., 0.5], [7., 0.5]],
        false,
        1.,
        [200, 100, 50, 255],
    )?;
    let mask = f::rectangle_mask(&v, full(), 0)?;
    apply(
        &mut v,
        Command::Mask {
            target: Target::Vector(id),
            mask: Some(mask),
        },
    )?;
    check(
        crate::render::Renderer::new(&v).pixel([0.5, 0.5]) == [0; 4]
            && crate::layer_pick::candidates(&v, [0.5, 0.5], Default::default()).is_empty(),
        "Masked vector output and picking",
    )?;
    check(
        f::prepare(
            &d,
            Command::Mode {
                folder: 65536,
                through: true,
            },
        )
        .is_err()
            && f::prepare(
                &d,
                Command::Move {
                    folder: 65536,
                    parent: Some(0),
                    position: 0,
                },
            )
            .is_err(),
        "Folder index must not wrap or panic",
    )?;
    let mut noop = make()?;
    let before = noop.snapshot();
    let s = f::prepare(
        &noop,
        Command::Mask {
            target: Target::Raster(1),
            mask: None,
        },
    )?;
    check(
        !noop.commit_state(s.state)? && noop.snapshot() == before,
        "No-op mask removal must not initialize metadata/Undo",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn nested_folders_masks_cycles_unwrap_modes_and_movement() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
