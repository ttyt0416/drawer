use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    magnetic_lasso::{self as core, Input, Reference, Scope, Segment, Settings, Trace},
    selection::{Mask, Run},
    selection_tools,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn make() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = make()?;
    for y in 2..10 {
        for x in 2..10 {
            d.layers[0].pixels[y * 16 + x] = [100, 50, 20, 255];
        }
    }
    let before = d.snapshot();
    let settings = Settings {
        enabled: true,
        distance: 2.0,
        ..Default::default()
    };
    let mut trace = Trace::new(&d, 0, settings)?;
    let route = trace.route(Some([2.2, 2.2]), [9.8, 9.8], Segment::Freehand)?;
    check(
        route.first() == Some(&[2.0, 2.0])
            && route.last() == Some(&[10.0, 10.0])
            && route.len() > 2
            && route
                .iter()
                .all(|p| p[0] == 2.0 || p[0] == 10.0 || p[1] == 2.0 || p[1] == 10.0),
        "Raster lasso did not follow a corner boundary",
    )?;
    check(
        trace.route(None, [6.5, 6.5], Segment::Freehand)? == vec![[6.5, 6.5]],
        "Raster beyond magnetic radius snapped",
    )?;
    for scope in [Scope::All, Scope::FreehandOnly, Scope::PolylineOnly] {
        for kind in [Segment::Freehand, Segment::Polyline] {
            let mut t = Trace::new(&d, 0, Settings { scope, ..settings })?;
            let result = t.route(None, [2.2, 2.1], kind)?;
            let applies = scope == Scope::All
                || scope == Scope::FreehandOnly && kind == Segment::Freehand
                || scope == Scope::PolylineOnly && kind == Segment::Polyline;
            check(
                result == vec![if applies { [2.0, 2.0] } else { [2.2, 2.1] }],
                "Magnetic input scope",
            )?;
        }
    }
    let mut combined = d.snapshot();
    combined.scene.selection_mask = Some(Mask {
        runs: vec![Run {
            x: 0,
            y: 5,
            length: 16,
            alpha: 128,
        }],
    });
    d.commit_state(combined)?;
    for (op, inside, outside) in [
        (crate::selection_edit::Combine::Replace, 255, 0),
        (crate::selection_edit::Combine::Add, 255, 128),
        (crate::selection_edit::Combine::Subtract, 0, 128),
        (crate::selection_edit::Combine::Intersect, 128, 0),
    ] {
        let mut tracer = Trace::new(&d, 0, settings)?;
        let mut polygon = vec![];
        let mut previous = None;
        for p in [[2.2, 2.2], [9.8, 2.2], [9.8, 9.8], [2.2, 9.8], [2.2, 2.2]] {
            let part = tracer.route(previous, p, Segment::Freehand)?;
            previous = part.last().copied();
            for q in part {
                if polygon.last() != Some(&q) {
                    polygon.push(q);
                }
            }
        }
        let scene = crate::selection_geometry::prepare(
            &d,
            crate::selection_geometry::Kind::Polygon,
            &polygon,
            op,
        )?;
        check(
            scene.selection_coverage([5.5, 5.5]) == inside
                && scene.selection_coverage([0.5, 5.5]) == outside,
            "Magnetic trace four overlapping selection operations",
        )?;
    }
    check(
        d.undo() && d.snapshot() == before,
        "Magnetic combine fixture Undo",
    )?;
    check(d.snapshot() == before, "Magnetic discovery mutated source")?;
    let layer = d.scene.lines.add_layer("Guide path")?;
    let path = d.scene.lines.add_path(
        layer,
        vec![[2.0, 2.0], [10.0, 2.0], [10.0, 10.0]],
        false,
        8.0,
        [0, 0, 0, 255],
    )?;
    d.scene.lines.layers[0].locked = true;
    let mut t = Trace::new(
        &d,
        0,
        Settings {
            reference: Reference::Vector(path),
            ..settings
        },
    )?;
    let route = t.route(Some([2.2, 2.3]), [9.8, 9.8], Segment::Freehand)?;
    check(
        route.len() == 3 && route[1] == [10.0, 2.0] && route[0][1] == 2.0 && route[2][0] == 10.0,
        "Vector trace cut a corner / used stroke outline",
    )?;
    d.scene.transform = crate::scene::Affine([2.0, 0.0, 0.0, 2.0, 1.0, 1.0]);
    let mut t = Trace::new(
        &d,
        0,
        Settings {
            reference: Reference::Vector(path),
            ..settings
        },
    )?;
    check(
        t.route(Some([5.1, 5.2]), [20.8, 20.8], Segment::Polyline)?
            .contains(&[21.0, 5.0]),
        "Vector tracing ignored page transform",
    )?;
    d.scene.lines.layers[0].hidden = true;
    check(
        Trace::new(
            &d,
            0,
            Settings {
                reference: Reference::Vector(path),
                ..settings
            },
        )
        .is_err(),
        "Hidden magnetic reference accepted",
    )?;
    let mut refs = make()?;
    refs.layers.push(refs.layers[0].clone());
    for y in 2..10 {
        for x in 2..10 {
            refs.layers[1].pixels[y * 16 + x] = [0, 0, 0, 32];
        }
    }
    for (reference, contrast, snaps) in [
        (Reference::Current, 32, false),
        (Reference::Composite, 32, true),
        (Reference::Composite, 33, false),
    ] {
        let mut t = Trace::new(
            &refs,
            0,
            Settings {
                reference,
                contrast,
                ..settings
            },
        )?;
        check(
            t.route(None, [2.2, 2.1], Segment::Freehand)?
                == vec![if snaps { [2.0, 2.0] } else { [2.2, 2.1] }],
            "Raster reference/contrast threshold",
        )?;
    }
    refs.layers[0].pixels.fill([255, 0, 0, 0]);
    let mut t = Trace::new(&refs, 0, settings)?;
    check(
        t.route(None, [2.2, 2.1], Segment::Freehand)? == vec![[2.2, 2.1]],
        "Hidden transparent RGB created a magnetic edge",
    )?;
    let mut e = make()?;
    e.layers[0].pixels.fill([80, 90, 100, 200]);
    let before = e.snapshot();
    check(
        !core::can_delete_outside(&e, 0) && core::delete_outside(&e, 0).is_err(),
        "Delete outside enabled without selection",
    )?;
    e.scene.selection_mask = Some(Mask::default());
    check(
        !core::can_delete_outside(&e, 0) && core::delete_outside(&e, 0).is_err(),
        "Empty mask cleared page",
    )?;
    for alpha in [64, 128, 255] {
        e.scene.selection_mask = Some(Mask {
            runs: vec![Run {
                x: 2,
                y: 3,
                length: 4,
                alpha,
            }],
        });
        let before = e.snapshot();
        let state = core::delete_outside(&e, 0)?;
        check(
            state.layers[0].pixels[3 * 16 + 3]
                == [80, 90, 100, ((200u32 * alpha as u32 + 127) / 255) as u8]
                && state.layers[0].pixels[0] == [0; 4]
                && state.scene == before.scene,
            "Outside erase inverse mask/partial alpha",
        )?;
        e.commit_state(state)?;
        check(
            e.undo() && e.snapshot() == before,
            "Outside deletion single Undo",
        )?;
    }
    e.scene.selection_mask = Some(Mask {
        runs: (0..16)
            .map(|y| Run {
                x: 0,
                y,
                length: 16,
                alpha: 255,
            })
            .collect(),
    });
    let history = e.history_counts();
    check(
        !e.commit_state(core::delete_outside(&e, 0)?)? && e.history_counts() == history,
        "Full selection outside delete changed page/history",
    )?;
    e.layers[0].pixels[0] = [250, 100, 30, 0];
    let before_hidden = e.snapshot();
    check(
        core::delete_outside(&e, 0)? == before_hidden,
        "Full selection changed hidden RGB",
    )?;
    e.layers[0].locked = true;
    check(
        !core::can_delete_outside(&e, 0) && core::delete_outside(&e, 0).is_err(),
        "Outside erase bypassed layer lock",
    )?;
    check(
        before.layers[0].pixels[0] == [80, 90, 100, 200],
        "Invalid erase fixture",
    )?;
    let mut a = make()?;
    selection_tools::rectangle(&mut a, [1.0, 1.0], [4.0, 4.0])?;
    selection_tools::clear(&mut a)?;
    selection_tools::clear(&mut a)?;
    selection_tools::rectangle(&mut a, [8.0, 8.0], [12.0, 12.0])?;
    selection_tools::reselect(&mut a)?;
    check(
        a.scene.selection_coverage([2.5, 2.5]) == 255
            && a.scene.selection_coverage([9.5, 9.5]) == 0,
        "Reselect lost last deselected mask after a newer selection",
    )?;
    let b = make()?;
    check(
        b.scene.selections.last_cleared.is_none(),
        "Reselect leaked between documents",
    )?;
    selection_tools::clear(&mut a)?;
    let file = root.join("last-deselected.drawer");
    crate::files::save(&mut a, &file).map_err(|e| e.to_string())?;
    let mut a = crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    selection_tools::reselect(&mut a)?;
    check(
        a.scene.selection_coverage([2.5, 2.5]) == 255,
        "Last deselected native reopen",
    )?;
    let w = crate::workspaces::Snapshot {
        magnetic_lasso: Settings {
            enabled: true,
            input: Input::Multiple,
            scope: Scope::PolylineOnly,
            reference: Reference::Composite,
            distance: 3.0,
            contrast: 64,
        },
        ..Default::default()
    };
    let file = root.join("magnetic-settings.json");
    w.export(&file).map_err(|e| e.to_string())?;
    check(
        crate::workspaces::Snapshot::read(&file)
            .map_err(|e| e.to_string())?
            .magnetic_lasso
            == w.magnetic_lasso,
        "Magnetic settings reopen",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn boundary_vector_following_scopes_inverse_erase_reselect_storage() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
