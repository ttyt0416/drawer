use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    scene::Affine,
    shapes::{self, Curve, Kind, Settings, Target},
    vector_edit::{self as core, Patch, Selection, Tool},
    vector_selection::Control,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = crate::search_verify::page()?;
    let s = Settings {
        kind: Kind::Bezier,
        curve: Curve::CubicBezier,
        width: 2.,
        line: [210, 30, 50, 255],
        ..Default::default()
    };
    shapes::prepare(
        &d,
        Target::Vector(None),
        &[[2., 2.], [4., 6.], [8., 6.], [10., 2.]],
        s,
    )?
    .restore(&mut d);
    d.scene.lines.layers[0].paths[0].pressure.device = vec![Some(0.25), None, Some(1.)];
    let id = d.scene.lines.layers[0].id;
    d.scene
        .lines
        .add_path(id, vec![[2., 6.], [10., 2.]], false, 4., [30, 50, 210, 255])?;
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let path = d.scene.lines.layers[0].paths[0].id;
    let other = d.scene.lines.layers[0].paths[1].id;
    let selected = Selection {
        controls: vec![Control { path, point: 1 }],
        paths: vec![],
    };
    let device = d.scene.lines.layers[0].paths[0].pressure.device.clone();
    let original = d.snapshot();
    for pressure in [0., 1., 2.] {
        let before = d.snapshot();
        let state = core::prepare(
            &d,
            layer,
            &selected,
            Patch {
                pressure: Some(pressure),
                ..Default::default()
            },
        )?;
        check(
            d.snapshot() == before,
            "Pressure preparation changed source",
        )?;
        d.commit_state(state)?;
        let p = &d.scene.lines.layers[0].paths[0];
        check(
            p.pressure.device == device && p.pressure.control_at(1. / 3.) == pressure,
            "Editing changed hardware samples or wrong point multiplier",
        )?;
        for n in [0, 2, 3] {
            check(
                p.pressure.control_at(n as f64 / 3.) == 1.,
                "Editing changed an unselected pressure control",
            )?;
        }
        let after = d.snapshot();
        let file = root.join("vector-edit.drawer");
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        let reopened =
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == after,
            "Vector pressure native roundtrip",
        )?;
        let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
        bytes[8..12].copy_from_slice(&63u32.to_le_bytes());
        std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).is_err(),
            "Pressure accepted under old header",
        )?;
        check(
            d.undo()
                && d.snapshot() == before
                && d.redo()
                && d.snapshot() == after
                && d.undo()
                && d.snapshot() == before,
            "Vector pressure single Undo/Redo",
        )?;
    }
    check(
        d.snapshot() == original,
        "Repeated pressure tests changed original",
    )?;
    d.scene.transform = Affine([2., 0., 0., 2., 3., 4.]);
    let before = d.snapshot();
    let controls = (0..4)
        .map(|n| d.scene.lines.layers[0].paths[0].control_point(n).unwrap())
        .collect::<Vec<_>>();
    d.commit_state(core::prepare(
        &d,
        layer,
        &selected,
        Patch {
            movement: Some([2., -1.]),
            ..Default::default()
        },
    )?)?;
    let p = &d.scene.lines.layers[0].paths[0];
    for (n, old) in controls.iter().enumerate() {
        let expected = if n == 1 {
            [old[0] + 1., old[1] - 0.5]
        } else {
            *old
        };
        check(
            p.control_point(n) == Some(expected),
            "Partial point move changed another control or used wrong document scale",
        )?;
    }
    check(
        p.pressure.device == device && d.undo() && d.snapshot() == before,
        "Point move changed device pressure or Undo",
    )?;
    let paths = Selection {
        controls: vec![],
        paths: vec![path, other],
    };
    let values = core::values(&d, layer, &paths)?;
    check(
        values.width.is_none() && values.color.is_none(),
        "Mixed vector attributes collapsed to one value",
    )?;
    let before = d.snapshot();
    d.commit_state(core::prepare(
        &d,
        layer,
        &paths,
        Patch {
            color: Some([10, 20, 30, 128]),
            ..Default::default()
        },
    )?)?;
    check(
        d.scene.lines.layers[0]
            .paths
            .iter()
            .all(|p| p.color == [10, 20, 30, 128])
            && core::values(&d, layer, &paths)?.width.is_none(),
        "Whole stroke recolor changed widths",
    )?;
    check(d.undo() && d.snapshot() == before, "Recolor Undo")?;
    check(
        core::hits(
            &d,
            layer,
            d.scene.transform.map([4., 6.]),
            0.25,
            Tool::Pressure,
        )
        .iter()
        .any(|h| h.path == path && h.control == Some(1)),
        "Pressure control picking",
    )?;
    check(
        core::hits(&d, layer, [999., 999.], 1., Tool::Color).is_empty(),
        "Empty canvas picked a vector",
    )?;
    for bad in [
        Patch {
            pressure: Some(3.),
            ..Default::default()
        },
        Patch {
            width: Some(f64::NAN),
            ..Default::default()
        },
        Patch {
            movement: Some([f64::INFINITY, 0.]),
            ..Default::default()
        },
    ] {
        let before = d.snapshot();
        check(
            core::prepare(&d, layer, &selected, bad).is_err() && d.snapshot() == before,
            "Invalid vector edit changed source",
        )?;
    }
    d.scene.lines.layers[0].locked = true;
    let before = d.snapshot();
    check(
        core::hits(&d, layer, [4., 6.], 2., Tool::Point).is_empty()
            && core::prepare(
                &d,
                layer,
                &selected,
                Patch {
                    pressure: Some(2.),
                    ..Default::default()
                },
            )
            .is_err()
            && d.snapshot() == before,
        "Locked vector edited",
    )?;
    pixels()?;
    tapered_picking()?;
    settings()?;
    Ok(())
}
fn tapered_picking() -> Result<(), String> {
    let mut d = crate::search_verify::page()?;
    let layer = d.scene.lines.add_layer("Tapered pick")?;
    d.scene
        .lines
        .add_path(layer, vec![[2., 5.], [10., 5.]], false, 4., [0, 0, 0, 255])?;
    d.scene.lines.layers[0].paths[0].pressure.controls = vec![0., 2.];
    check(
        core::hits(&d, layer, [3., 7.5], 0.1, Tool::Color).is_empty(),
        "Maximum width picked blank space beside a thin part",
    )?;
    check(
        !core::hits(&d, layer, [9., 7.], 0.1, Tool::Color).is_empty(),
        "Thick tapered segment was not picked",
    )
}
fn pixels() -> Result<(), String> {
    for float in [false, true] {
        let mut d = if float {
            Document::create_with_color(
                CanvasSpec {
                    width: 16,
                    height: 16,
                    dpi: 300.,
                    background: Background::Transparent,
                },
                DEFAULT_DOCUMENT_BUDGET,
                Default::default(),
            )
            .map_err(|e| e.to_string())?
        } else {
            crate::search_verify::page()?
        };
        let layer = d.scene.lines.add_layer("Pressure ink")?;
        let path = d.scene.lines.add_path(
            layer,
            vec![[2., 5.], [10., 5.]],
            false,
            4.,
            [20, 40, 60, 255],
        )?;
        let p = &mut d.scene.lines.layers[0].paths[0];
        p.pressure.device = vec![Some(0.5); 2];
        p.pressure.controls = vec![2.; 2];
        check(
            crate::render::Renderer::new(&d).pixel([6.5, 6.5]) == [20, 40, 60, 255]
                && crate::render::Renderer::new(&d).pixel([6.5, 7.5]) == [0; 4],
            "Device × editable pressure rendering or maximum bounds",
        )?;
        let controls = Selection {
            controls: vec![Control { path, point: 0 }, Control { path, point: 1 }],
            paths: vec![],
        };
        d.commit_state(core::prepare(
            &d,
            layer,
            &controls,
            Patch {
                pressure: Some(0.),
                ..Default::default()
            },
        )?)?;
        check(
            crate::render::Renderer::new(&d).pixel([6.5, 5.5]) == [0; 4],
            "Pressure zero left a hairline",
        )?;
        d.undo();
        let before = d.snapshot();
        d.commit_state(core::prepare(
            &d,
            layer,
            &Selection {
                controls: vec![],
                paths: vec![path],
            },
            Patch {
                width: Some(0.),
                ..Default::default()
            },
        )?)?;
        check(
            d.scene.lines.layers[0].paths[0].width == 0.
                && crate::render::Renderer::new(&d).pixel([6.5, 5.5]) == [0; 4]
                && d.undo()
                && d.snapshot() == before,
            "Width zero or Undo",
        )?;
        let p = &mut d.scene.lines.layers[0].paths[0];
        p.pressure.device = vec![Some(0.25); 2];
        p.pressure.controls.clear();
        p.pressure.device_enabled = false;
        check(
            crate::render::Renderer::new(&d).pixel([6.5, 6.5]) == [20, 40, 60, 255],
            "Disabled input size pressure changed width",
        )?;
    }
    Ok(())
}
fn settings() -> Result<(), String> {
    let before = crate::workspaces::Snapshot::default();
    let mut source = before.clone();
    source.vector_edit.sensitivity = 0.025;
    source.vector_edit.temporary = core::Modifier::Alt;
    let bytes = serde_json::to_vec(&source).map_err(|e| e.to_string())?;
    let reopened: crate::workspaces::Snapshot =
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    check(
        reopened.valid() && reopened.vector_edit == source.vector_edit,
        "Vector editing local settings roundtrip",
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn vector_point_pressure_style_mixed_native_zero_width_device_independence_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
