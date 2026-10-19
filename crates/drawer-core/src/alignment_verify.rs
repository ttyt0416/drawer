use crate::{
    alignment::{self as a, Align, Item, Operation, Reference},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    scene::{Affine, PageObject, Rect},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        min: [x, y],
        max: [x + w, y + h],
    }
}
fn make() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 64,
            height: 48,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].placement = Some(PlacedRaster {
        width: 4,
        height: 4,
        offset: [-4, 3],
    });
    d.layers[0].pixels = vec![[20, 40, 200, 128]; 16];
    for (x, y, w, h) in [
        (-8.0, -4.0, 4.0, 4.0),
        (12.0, 6.0, 8.0, 6.0),
        (32.0, 12.0, 12.0, 8.0),
    ] {
        d.scene.objects.push(PageObject::Tone {
            bounds: rect(x, y, w, h),
            period_px: 4.0,
            coverage: 0.5,
        });
    }
    Ok(d)
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-8
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = make()?;
    let items = [Item::Object(0), Item::Object(1), Item::Object(2)];
    let original = d.snapshot();
    for n in 0..3 {
        for op in [Operation::GapsHorizontal, Operation::CentersVertical] {
            check(
                a::prepare(&d, &items[..n], Reference::Canvas, op)? == original,
                "Distribution below three objects changed page",
            )?;
        }
    }
    check(
        a::prepare(&d, &[], Reference::Canvas, Operation::Align(Align::Left))? == original,
        "Empty alignment changed page",
    )?;
    for alignment in Align::ALL {
        let state = a::prepare(
            &d,
            &items[..1],
            Reference::Canvas,
            Operation::Align(alignment),
        )?;
        d.commit_state(state)?;
        let r = a::bounds(&d, items[0])?;
        check(
            match alignment {
                Align::Left => close(r.min[0], 0.0),
                Align::Center => close((r.min[0] + r.max[0]) / 2.0, 32.0),
                Align::Right => close(r.max[0], 64.0),
                Align::Top => close(r.min[1], 0.0),
                Align::Middle => close((r.min[1] + r.max[1]) / 2.0, 24.0),
                Align::Bottom => close(r.max[1], 48.0),
            },
            "Single-object canvas alignment",
        )?;
        check(
            d.history_counts().0 == 1 && d.undo() && d.snapshot() == original,
            "Single alignment Undo",
        )?;
    }
    d.commit_state(a::prepare(
        &d,
        &items[..2],
        Reference::Selection,
        Operation::Align(Align::Left),
    )?)?;
    check(
        close(a::bounds(&d, items[0])?.min[0], -8.0)
            && close(a::bounds(&d, items[1])?.min[0], -8.0),
        "Negative selection-bound alignment",
    )?;
    check(
        d.undo() && d.snapshot() == original,
        "Multiple alignment one Undo",
    )?;
    d.commit_state(a::prepare(
        &d,
        &items,
        Reference::Key(items[0]),
        Operation::Align(Align::Right),
    )?)?;
    check(
        d.scene.objects[0] == original.scene.objects[0]
            && close(a::bounds(&d, items[1])?.max[0], -4.0)
            && close(a::bounds(&d, items[2])?.max[0], -4.0),
        "Key object moved or target edge wrong",
    )?;
    d.undo();
    for op in [
        Operation::GapsHorizontal,
        Operation::GapsVertical,
        Operation::CentersHorizontal,
        Operation::CentersVertical,
    ] {
        let before = d.snapshot();
        d.commit_state(a::prepare(&d, &items, Reference::Canvas, op)?)?;
        let r: Vec<_> = items
            .iter()
            .map(|i| a::bounds(&d, *i))
            .collect::<Result<_, _>>()?;
        let axis = usize::from(matches!(
            op,
            Operation::GapsVertical | Operation::CentersVertical
        ));
        if matches!(op, Operation::GapsHorizontal | Operation::GapsVertical) {
            check(
                close(
                    r[1].min[axis] - r[0].max[axis],
                    r[2].min[axis] - r[1].max[axis],
                ),
                "Equal gaps independent geometry",
            )?;
        } else {
            let c: Vec<_> = r
                .iter()
                .map(|r| (r.min[axis] + r.max[axis]) / 2.0)
                .collect();
            check(close(c[1] - c[0], c[2] - c[1]), "Equal center spacing")?;
        }
        check(
            close(r[0].min[axis], 0.0)
                && close(r[2].max[axis], if axis == 0 { 64.0 } else { 48.0 }),
            "Distribution reference outer edges",
        )?;
        check(d.history_counts().0 == 1, "Distribution grouped history")?;
        let file = root.join(format!("alignment-{op:?}.drawer"));
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "Distribution native reopen",
        )?;
        check(
            d.undo() && d.snapshot() == before && d.redo(),
            "Distribution Undo/Redo",
        )?;
        d.undo();
    }
    d.layers[0].locked = true;
    let before = d.snapshot();
    check(
        a::prepare(
            &d,
            &[Item::Raster(0), items[0]],
            Reference::Canvas,
            Operation::Align(Align::Left),
        )
        .is_err()
            && d.snapshot() == before,
        "Locked raster caused partial alignment",
    )?;
    d.commit_state(a::prepare(
        &d,
        &[Item::Raster(0), items[0]],
        Reference::Key(Item::Raster(0)),
        Operation::Align(Align::Left),
    )?)?;
    check(
        d.layers[0] == before.layers[0] && close(a::bounds(&d, items[0])?.min[0], -4.0),
        "Locked key was not read-only",
    )?;
    d.undo();
    d.layers[0].locked = false;
    d.layers[0].placement = Some(PlacedRaster {
        width: 3,
        height: 4,
        offset: [-4, 3],
    });
    d.layers[0].pixels.truncate(12);
    let before = d.snapshot();
    d.commit_state(a::prepare(
        &d,
        &[Item::Raster(0)],
        Reference::Canvas,
        Operation::Align(Align::Center),
    )?)?;
    check(
        d.layers[0].pixels == before.layers[0].pixels
            && close(a::bounds(&d, Item::Raster(0))?.min[0], 31.0),
        "Raster alignment pixel rounding/content",
    )?;
    d.undo();
    d.scene.selection_mask = Some(crate::selection::Mask {
        runs: vec![crate::selection::Run {
            x: 1,
            y: 4,
            length: 4,
            alpha: 128,
        }],
    });
    d.scene.selection_translation = [2.0, 3.0];
    let before = d.snapshot();
    d.commit_state(a::prepare(
        &d,
        &items[..1],
        Reference::Selection,
        Operation::Align(Align::Middle),
    )?)?;
    check(
        close(a::bounds(&d, items[0])?.min[1], 5.5)
            && d.scene.selection_mask == before.scene.selection_mask
            && d.scene.selection_translation == before.scene.selection_translation,
        "Translated selection reference/selection preservation",
    )?;
    d.undo();
    d.scene.selection_mask = None;
    d.scene.selection_translation = [0.0; 2];
    let before = d.snapshot();
    check(
        a::prepare(
            &d,
            &[items[0], items[0]],
            Reference::Canvas,
            Operation::Align(Align::Left),
        )
        .is_err()
            && d.snapshot() == before,
        "Duplicate objects accepted",
    )?;
    for object in &mut d.scene.objects {
        if let PageObject::Tone { bounds, .. } = object {
            bounds.max[0] = bounds.min[0] + 30.0;
        }
    }
    let before = d.snapshot();
    for op in [Operation::GapsHorizontal, Operation::CentersHorizontal] {
        check(
            a::prepare(&d, &items, Reference::Canvas, op).is_err() && d.snapshot() == before,
            "Insufficient distribution space changed page",
        )?;
    }
    let mut v = make()?;
    let state = crate::shapes::prepare(
        &v,
        crate::shapes::Target::Vector(None),
        &[[8.0, 8.0], [24.0, 24.0]],
        crate::shapes::Settings {
            paint: crate::shapes::Paint::CreateFill,
            ..Default::default()
        },
    )?;
    v.commit_state(state)?;
    let path = v.scene.lines.layers[0].paths[0].id;
    v.scene.lines.layers[0].locked = true;
    let before = v.snapshot();
    check(
        a::prepare(
            &v,
            &[Item::Vector(path), Item::Object(0)],
            Reference::Canvas,
            Operation::Align(Align::Left),
        )
        .is_err()
            && v.snapshot() == before,
        "Locked vector alignment was partial",
    )?;
    v.scene.lines.layers[0].locked = false;
    v.scene.transform = Affine([0.0, 2.0, -2.0, 0.0, 50.0, 0.0]);
    let old = v.scene.lines.layers[0].paths[0].definition.clone().unwrap();
    v.commit_state(a::prepare(
        &v,
        &[Item::Vector(path)],
        Reference::Canvas,
        Operation::Align(Align::Left),
    )?)?;
    check(
        close(a::bounds(&v, Item::Vector(path))?.min[0], 0.0),
        "Vector transformed page alignment",
    )?;
    let moved = &v.scene.lines.layers[0].paths[0];
    let def = moved.definition.as_ref().unwrap();
    check(
        def.controls == old.controls
            && def.settings == old.settings
            && def.creation != old.creation,
        "Vector construction definition lost after alignment",
    )?;
    let rebuilt = crate::shapes::geometry(def.settings, &def.controls)?;
    check(
        rebuilt.points.iter().zip(&moved.points).all(|(a, b)| {
            let p = def.creation.inverse().map(*a);
            close(p[0], b[0]) && close(p[1], b[1])
        }),
        "Moved vector recipe no longer reconstructs path",
    )?;
    let mut g = make()?;
    g.scene.selection = Some(rect(2.0, 2.0, 4.0, 4.0));
    g.commit_state(crate::gradient::prepare(
        &g,
        0,
        crate::gradient::Settings {
            first: [200, 0, 0, 255],
            last: [0, 0, 200, 255],
            start: [2.5, 2.5],
            end: [6.5, 2.5],
            ..Default::default()
        },
        true,
        Default::default(),
    )?)?;
    g.scene.selection = None;
    let id = g.scene.gradients.layers[0].id;
    let before = g.snapshot();
    let color = crate::render::Renderer::new(&g).pixel([2.5, 2.5]);
    g.commit_state(a::prepare(
        &g,
        &[Item::Gradient(id)],
        Reference::Canvas,
        Operation::Align(Align::Left),
    )?)?;
    check(
        close(a::bounds(&g, Item::Gradient(id))?.min[0], 0.0)
            && crate::render::Renderer::new(&g).pixel([0.5, 2.5]) == color,
        "Editable gradient alignment did not move color/mask together",
    )?;
    check(
        g.undo() && g.snapshot() == before,
        "Gradient alignment Undo",
    )?;
    let samples = [
        PageObject::Text {
            baseline: [-10.0, 12.0],
            text: "X".into(),
            size_px: 10.0,
            color: [0, 0, 0, 255],
        },
        PageObject::Line {
            start: [-4.0, -4.0],
            end: [4.0, 8.0],
            width_px: 2.0,
            color: [0, 0, 0, 255],
        },
        PageObject::Shadow {
            bounds: rect(-4.0, -3.0, 8.0, 6.0),
            offset: [2.0, 3.0],
            radius_px: 2.0,
            color: [0, 0, 0, 128],
        },
    ];
    for object in samples {
        let mut d = make()?;
        d.scene.objects = vec![object];
        d.commit_state(a::prepare(
            &d,
            &[Item::Object(0)],
            Reference::Canvas,
            Operation::Align(Align::Top),
        )?)?;
        check(
            close(a::bounds(&d, Item::Object(0))?.min[1], 0.0),
            "Page object kind alignment",
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn references_counts_locks_negative_distribution_transforms_undo_reopen() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
