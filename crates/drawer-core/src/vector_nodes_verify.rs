use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    vector_edit::{Hit, Patch, Selection, Tool},
    vector_nodes::{self as core, Operation},
    vector_selection::Control,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 32,
            height: 32,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    let layer = d.scene.lines.add_layer("Nodes")?;
    d.scene.lines.add_path(
        layer,
        vec![[4., 16.], [16., 4.], [28., 16.]],
        false,
        3.,
        [210, 40, 20, 180],
    )?;
    d.scene.lines.add_path(
        layer,
        vec![[4., 26.], [28., 26.]],
        false,
        4.,
        [20, 40, 210, 255],
    )?;
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.pressure.device = vec![Some(0.25), None, Some(0.75)];
    p.pressure.controls = vec![0.5, 2., 1.];
    Ok(d)
}
fn hit(path: u64, point: u16, parameter: Option<f64>) -> Hit {
    Hit {
        path,
        control: Some(point),
        parameter,
        point: [0.; 2],
        distance: 0.,
    }
}
fn bezier(p: [[f64; 2]; 4], t: f64) -> [f64; 2] {
    let u = 1. - t;
    std::array::from_fn(|k| {
        u * u * u * p[0][k]
            + 3. * u * u * t * p[1][k]
            + 3. * u * t * t * p[2][k]
            + t * t * t * p[3][k]
    })
}
fn segment(def: &crate::shapes::Definition, i: usize) -> [[f64; 2]; 4] {
    let j = (i + 1) % def.controls.len();
    let nodes = def.nodes.as_ref().unwrap();
    [
        def.controls[i],
        std::array::from_fn(|k| def.controls[i][k] + nodes.outgoing[i][k]),
        std::array::from_fn(|k| def.controls[j][k] + nodes.incoming[j][k]),
        def.controls[j],
    ]
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let id = d.scene.lines.layers[0].paths[0].id;
    let before = d.snapshot();
    let other = before.scene.lines.layers[0].paths[1].clone();
    let state = core::prepare(&d, layer, hit(id, 0, Some(0.5)), Operation::Add)?;
    let p = &state.scene.lines.layers[0].paths[0];
    let def = p.definition.as_ref().unwrap();
    check(
        def.controls == [[4., 16.], [10., 10.], [16., 4.], [28., 16.]]
            && p.points == def.controls
            && p.width == 3.
            && p.color == [210, 40, 20, 180],
        "Polyline insertion geometry/style",
    )?;
    check(
        (p.pressure.device_at(1. / 3.) - 0.625).abs() < 1e-12
            && (p.pressure.control_at(1. / 3.) - 1.25).abs() < 1e-12
            && p.pressure.device.contains(&None),
        "Node insertion retimed pressure incorrectly",
    )?;
    check(
        d.snapshot() == before && state.scene.lines.layers[0].paths[1] == other,
        "Node preview changed source/other path",
    )?;
    d.commit_state(state)?;
    let after = d.snapshot();
    let file = root.join("vector-nodes.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == after,
        "Node native save/reopen",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&65u32.to_le_bytes());
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Cubic nodes accepted under old header",
    )?;
    check(
        d.undo()
            && d.snapshot() == before
            && d.redo()
            && d.snapshot() == after
            && d.undo()
            && d.snapshot() == before,
        "Node insertion Undo/Redo",
    )?;
    d.commit_state(core::prepare(
        &d,
        layer,
        hit(id, 1, None),
        Operation::Smooth,
    )?)?;
    let p = &d.scene.lines.layers[0].paths[0];
    let def = p.definition.as_ref().unwrap();
    let data = def.nodes.as_ref().unwrap();
    let a = data.incoming[1];
    let b = data.outgoing[1];
    check(
        data.smooth[1]
            && (a[0] * b[1] - a[1] * b[0]).abs() < 1e-10
            && a[0] * b[0] + a[1] * b[1] < 0.
            && (a[0].hypot(a[1]) - b[0].hypot(b[1])).abs() < 1e-10,
        "Smooth node does not share continuous tangent",
    )?;
    check(
        def.controls == [[4., 16.], [16., 4.], [28., 16.]] && p.points.len() == 65,
        "Smooth node positions or cubic evaluation",
    )?;
    for n in 0..=100 {
        let t = n as f64 / 100.;
        check(
            (p.pressure.device_at(t) - before.scene.lines.layers[0].paths[0].pressure.device_at(t))
                .abs()
                < 1e-9
                && (p.pressure.control_at(t)
                    - before.scene.lines.layers[0].paths[0].pressure.control_at(t))
                .abs()
                    < 1e-9,
            "Smooth conversion changed pressure curve",
        )?;
    }
    let smooth = d.snapshot();
    let old_curve = segment(def, 0);
    let state = core::prepare(&d, layer, hit(id, 0, Some(0.37)), Operation::Add)?;
    let def = state.scene.lines.layers[0].paths[0]
        .definition
        .as_ref()
        .unwrap();
    for n in 0..=100 {
        let t = n as f64 / 100.;
        let expected = bezier(old_curve, t);
        let actual = if t <= 0.37 {
            bezier(segment(def, 0), t / 0.37)
        } else {
            bezier(segment(def, 1), (t - 0.37) / 0.63)
        };
        check(
            (expected[0] - actual[0]).hypot(expected[1] - actual[1]) < 1e-9,
            "Bezier subdivision changed mathematical curve",
        )?;
    }
    let state = core::prepare(&d, layer, hit(id, 1, None), Operation::Corner)?;
    let def = state.scene.lines.layers[0].paths[0]
        .definition
        .as_ref()
        .unwrap();
    let data = def.nodes.as_ref().unwrap();
    let a = data.incoming[1];
    let b = data.outgoing[1];
    check(
        !data.smooth[1] && (a[0] * b[1] - a[1] * b[0]).abs() > 1.,
        "Corner switch retained common tangent",
    )?;
    d.commit_state(state)?;
    check(d.undo() && d.snapshot() == smooth, "Corner switch Undo")?;
    let selection = Selection {
        controls: vec![Control { path: id, point: 1 }],
        paths: vec![],
    };
    let before_move = d.snapshot();
    d.scene.transform = crate::scene::Affine([2., 0., 0., 1., 3., 5.]);
    let state = crate::vector_edit::prepare(
        &d,
        layer,
        &selection,
        Patch {
            movement: Some([4., -2.]),
            ..Default::default()
        },
    )?;
    let p = &state.scene.lines.layers[0].paths[0];
    let def = p.definition.as_ref().unwrap();
    check(
        def.controls[1] == [18., 2.]
            && def.nodes.as_ref()
                == before_move.scene.lines.layers[0].paths[0]
                    .definition
                    .as_ref()
                    .unwrap()
                    .nodes
                    .as_ref(),
        "Existing point tool lost node handles or document-space movement",
    )?;
    d.scene.transform = crate::scene::Affine::default();
    d.commit_state(core::prepare(
        &d,
        layer,
        hit(id, 1, None),
        Operation::Delete,
    )?)?;
    check(
        d.scene.lines.layers[0].paths[0].control_count() == 2,
        "Node deletion minimum open count",
    )?;
    let before = d.snapshot();
    check(
        core::prepare(&d, layer, hit(id, 0, None), Operation::Delete).is_err()
            && d.snapshot() == before,
        "Deleted an open minimum control",
    )?;
    d.scene.lines.layers[0].locked = true;
    let before = d.snapshot();
    check(
        crate::vector_edit::hits(&d, layer, [4., 16.], 1., Tool::DeleteNode).is_empty()
            && core::prepare(&d, layer, hit(id, 0, Some(0.5)), Operation::Add).is_err()
            && d.snapshot() == before,
        "Locked node edit",
    )?;
    closed_and_limits()?;
    mixed_pressure()?;
    Ok(())
}
fn mixed_pressure() -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let id = d.scene.lines.layers[0].paths[0].id;
    d.scene.lines.layers[0].paths[0].points = vec![[4., 16.], [12., 4.], [20., 16.], [28., 20.]];
    let old = d.scene.lines.layers[0].paths[0].pressure.clone();
    d.commit_state(core::prepare(
        &d,
        layer,
        hit(id, 1, None),
        Operation::Smooth,
    )?)?;
    let parameters = d.scene.lines.layers[0].paths[0].control_parameters()?;
    check(
        (parameters[2] - 64. / 65.).abs() < 1e-12,
        "Mixed node geometry parameter",
    )?;
    let selection = Selection {
        controls: vec![Control { path: id, point: 2 }],
        paths: vec![],
    };
    check(
        (crate::vector_edit::values(&d, layer, &selection)?
            .pressure
            .unwrap()
            - old.control_at(2. / 3.))
        .abs()
            < 1e-9,
        "Pressure inspector read a logical index instead of physical node",
    )?;
    let before = d.scene.lines.layers[0].paths[0].pressure.clone();
    let state = crate::vector_edit::prepare(
        &d,
        layer,
        &selection,
        Patch {
            pressure: Some(0.3),
            ..Default::default()
        },
    )?;
    let p = &state.scene.lines.layers[0].paths[0];
    check(
        (p.pressure.control_at(parameters[2]) - 0.3).abs() < 1e-12
            && p.pressure.device == before.device
            && p.pressure.device_positions == before.device_positions,
        "Node pressure changed wrong point/device samples",
    )?;
    for n in [0, 1, 3] {
        check(
            (p.pressure.control_at(parameters[n]) - before.control_at(parameters[n])).abs() < 1e-9,
            "Node pressure changed unselected node multiplier",
        )?;
    }
    // Moving a corner changes adjacent sampling density. Hardware pressure and
    // editable multipliers still belong to the same logical nodes afterwards.
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let id = d.scene.lines.layers[0].paths[0].id;
    d.scene.lines.layers[0].paths[0].points = vec![[4., 16.], [12., 4.], [20., 16.], [28., 20.]];
    d.commit_state(core::prepare(
        &d,
        layer,
        hit(id, 1, None),
        Operation::Corner,
    )?)?;
    let p = &d.scene.lines.layers[0].paths[0];
    let old_params = p.control_parameters()?;
    let old = p.pressure.clone();
    let selection = Selection {
        controls: vec![Control { path: id, point: 1 }],
        paths: vec![],
    };
    let state = crate::vector_edit::prepare(
        &d,
        layer,
        &selection,
        Patch {
            movement: Some([2., 0.]),
            ..Default::default()
        },
    )?;
    let p = &state.scene.lines.layers[0].paths[0];
    let params = p.control_parameters()?;
    for n in 0..4 {
        check(
            (p.pressure.device_at(params[n]) - old.device_at(old_params[n])).abs() < 1e-9
                && (p.pressure.control_at(params[n]) - old.control_at(old_params[n])).abs() < 1e-9,
            "Point movement detached pressure from nodes",
        )?;
    }
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.definition = None;
    p.closed = true;
    p.points = vec![[4., 4.], [28., 4.], [28., 28.], [4., 28.]];
    d.commit_state(core::prepare(
        &d,
        layer,
        hit(id, 0, None),
        Operation::Smooth,
    )?)?;
    let p = &d.scene.lines.layers[0].paths[0];
    let selection = Selection {
        controls: (0..4).map(|point| Control { path: id, point }).collect(),
        paths: vec![],
    };
    let old_device = p.pressure.device.clone();
    let old_positions = p.pressure.device_positions.clone();
    let state = crate::vector_edit::prepare(
        &d,
        layer,
        &selection,
        Patch {
            pressure: Some(1.),
            ..Default::default()
        },
    )?;
    let p = &state.scene.lines.layers[0].paths[0];
    check(
        p.pressure.controls.is_empty()
            && p.pressure.control_at(1.) == 1.
            && p.pressure.device == old_device
            && p.pressure.device_positions == old_positions,
        "Closed pressure reset left a synthetic closing-end multiplier",
    )
}
fn closed_and_limits() -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let id = d.scene.lines.layers[0].paths[0].id;
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = vec![[4., 4.], [28., 4.], [28., 28.], [4., 28.]];
    p.closed = true;
    let state = core::prepare(&d, layer, hit(id, 3, Some(0.5)), Operation::Add)?;
    let p = &state.scene.lines.layers[0].paths[0];
    check(
        p.closed && p.control_count() == 5 && p.control_point(4) == Some([4., 16.]),
        "Closed last segment insertion",
    )?;
    d.commit_state(core::prepare(
        &d,
        layer,
        hit(id, 3, None),
        Operation::Delete,
    )?)?;
    check(
        d.scene.lines.layers[0].paths[0].closed
            && d.scene.lines.layers[0].paths[0].control_count() == 3,
        "Closed last control deletion",
    )?;
    let before = d.snapshot();
    check(
        core::prepare(&d, layer, hit(id, 2, None), Operation::Delete).is_err()
            && d.snapshot() == before,
        "Deleted a closed minimum control",
    )?;
    d.scene.lines.layers[0].paths[0].definition = None;
    d.scene.lines.layers[0].paths[0].closed = false;
    d.scene.lines.layers[0].paths[0].points =
        (0..4096).map(|n| [n as f64, (n % 2) as f64]).collect();
    let before = d.snapshot();
    check(
        core::prepare(&d, layer, hit(id, 2000, None), Operation::Smooth).is_err()
            && d.snapshot() == before,
        "Node geometry limit mutated source",
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn cubic_node_insert_delete_corner_smooth_continuity_pressure_native_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
