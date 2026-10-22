use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    vector_edit::Selection,
    vector_refine::{self as core, Ends, Operation, Width},
    vector_selection::Control,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    edge_cases()?;
    let mut d = crate::vector_nodes_verify::fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let id = d.scene.lines.layers[0].paths[0].id;
    let s = Selection {
        paths: vec![id],
        ..Default::default()
    };
    let before = d.snapshot();
    for (op, expected) in [
        (Operation::Width(Width::Thicken, 2., false), 5.),
        (Operation::Width(Width::Narrow, 2., false), 1.),
        (Operation::Width(Width::ScaleUp, 2., false), 6.),
        (Operation::Width(Width::ScaleDown, 2., false), 1.5),
        (Operation::Width(Width::Fix, 0., true), 1.),
    ] {
        let state = core::prepare(&d, layer, &s, op)?;
        check(
            state.scene.lines.layers[0].paths[0].width == expected,
            "Width adjustment modes",
        )?;
        check(d.snapshot() == before, "Prepare mutated source")?;
    }
    let zero = core::prepare(&d, layer, &s, Operation::Simplify(0.))?;
    check(
        zero.scene.lines.layers[0].paths[0] == d.scene.lines.layers[0].paths[0],
        "Tolerance zero changed data",
    )?;
    let p = d.scene.lines.layers[0].paths[0].clone();
    for (ends, first, last) in [
        (Ends::Both, false, false),
        (Ends::Either, false, true),
        (Ends::Free, true, true),
    ] {
        let state = core::push(&d, layer, &s, [4., 16.], [0., 3.], 100., ends)?;
        let next = &state.scene.lines.layers[0].paths[0];
        check(
            (next.points[0] != p.points[0]) == first
                && (next.points.last() != p.points.last()) == last,
            "Endpoint push policies",
        )?;
        check(
            next.pressure.device_at(0.) == p.pressure.device_at(0.)
                && next.pressure.device_at(1.) == p.pressure.device_at(1.)
                && state.scene.lines.layers[0].paths[1] == d.scene.lines.layers[0].paths[1],
            "Push pressure or unselected path changed",
        )?;
    }
    let both = Selection {
        controls: vec![
            Control { path: id, point: 0 },
            Control { path: id, point: 1 },
        ],
        paths: vec![],
    };
    let redraw = core::redraw(
        &d,
        layer,
        &both,
        &[[4., 16.], [10., 22.], [16., 4.]],
        Ends::Both,
        crate::shapes::Curve::StraightLine,
    )?;
    check(
        redraw.scene.lines.layers[0].paths[0].points
            == [[4., 16.], [10., 22.], [16., 4.], [28., 16.]],
        "Redraw changed unselected span",
    )?;
    check(
        redraw.scene.lines.layers[0].paths[0].pressure.device_at(1.) == p.pressure.device_at(1.),
        "Redraw lost pressure endpoint",
    )?;
    for curve in crate::shapes::Curve::ALL {
        let state = core::prepare(&d, layer, &s, Operation::Convert(curve))?;
        check(
            state.scene.valid() && state.scene.lines.layers[0].paths[0].color == p.color,
            "Curve conversion invalid/style changed",
        )?;
    }
    let split = Selection {
        controls: vec![Control { path: id, point: 1 }],
        paths: vec![],
    };
    let state = core::prepare(&d, layer, &split, Operation::Split)?;
    check(
        state.scene.lines.layers[0].paths.len() == 3
            && state.scene.lines.layers[0].paths[0].points == p.points[..2],
        "Split geometry",
    )?;
    d.commit_state(state)?;
    check(d.undo() && d.snapshot() == before, "Split Undo")?;
    let state = core::prepare(&d, layer, &s, Operation::Opacity(27))?;
    d.commit_state(state)?;
    let path = root.join("vector-refine.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let loaded = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(loaded.snapshot() == d.snapshot(), "Refine save/reopen")?;
    check(d.undo() && d.snapshot() == before, "Refine Undo restore")?;
    // Geometrically crossing strokes keep their own attributes, not nearest-point attributes.
    let mut other = p.clone();
    other.id = d.scene.lines.id()?;
    other.points = vec![[29., 16.], [16., 28.], [3., 17.]];
    other.definition = None;
    d.scene.lines.layers[0].paths.push(other.clone());
    let selected = Selection {
        paths: vec![id, other.id],
        ..Default::default()
    };
    check(
        core::prepare(&d, layer, &selected, Operation::Join(0.5)).is_err(),
        "Distant endpoint accepted",
    )?;
    let joined = core::prepare(&d, layer, &selected, Operation::Join(2.))?;
    check(
        joined.scene.lines.layers[0].paths.len() == 2
            && joined.scene.lines.layers[0].paths[0].points.len() == 6,
        "Join failed",
    )?;
    let joined = &joined.scene.lines.layers[0].paths[0];
    for n in 0..101 {
        let t = n as f64 / 100.;
        check(
            (joined.pressure.device_at(t * 0.4) - p.pressure.device_at(t)).abs() < 1e-9
                && (joined.pressure.control_at(0.6 + t * 0.4) - other.pressure.control_at(t)).abs()
                    < 1e-9,
            "Joined pressure retiming",
        )?;
    }
    let reverse = d.scene.lines.layers[0]
        .paths
        .iter_mut()
        .find(|p| p.id == other.id)
        .unwrap();
    reverse.points = vec![[40., 40.], [29., 16.]];
    let joined = core::prepare(&d, layer, &selected, Operation::Join(2.))?;
    let p = &joined.scene.lines.layers[0].paths[0];
    check(
        p.points[3..] == [[29., 16.], [40., 40.]],
        "Join did not reverse nearest endpoint",
    )?;
    for n in 0..101 {
        let t = n as f64 / 100.;
        check(
            (p.pressure.device_at(0.75 + t * 0.25) - other.pressure.device_at(1. - t)).abs() < 1e-9,
            "Reversed join lost pressure direction",
        )?;
    }
    d.scene
        .layer_properties
        .vectors
        .entry(layer)
        .or_default()
        .position_locked = true;
    check(
        core::push(&d, layer, &s, [4., 16.], [1., 1.], 20., Ends::Free).is_err(),
        "Position lock bypassed",
    )?;
    Ok(())
}
fn edge_cases() -> Result<(), String> {
    straight_push()?;
    let mut d = crate::vector_nodes_verify::fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let id = d.scene.lines.layers[0].paths[0].id;
    let s = Selection {
        paths: vec![id],
        ..Default::default()
    };
    let original = vec![[0., 0.], [1., 0.1], [2., 0.], [3., 0.1], [4., 0.]];
    d.scene.lines.layers[0].paths[0].points = original.clone();
    for matrix in [
        crate::scene::Affine::default(),
        crate::scene::Affine([3., 0., 1., 2., 0., 0.]),
    ] {
        d.scene.transform = matrix;
        let state = core::prepare(&d, layer, &s, Operation::Simplify(0.11))?;
        let p = &state.scene.lines.layers[0].paths[0];
        for point in &original {
            let error = p
                .segments()
                .map(|(a, b)| {
                    crate::line_tools::distance(matrix.map(*point), matrix.map(a), matrix.map(b)).0
                })
                .fold(f64::INFINITY, f64::min);
            check(
                error <= 0.11 + 1e-9,
                "Simplification exceeded document-space error",
            )?;
        }
        check(
            p.points[0] == original[0] && p.points.last() == original.last(),
            "Simplify changed endpoints",
        )?;
        if matrix.is_identity() {
            check(
                p.points.len() == 2,
                "Simplify did not remove redundant vertices",
            )?;
        }
    }
    d.scene.transform = Default::default();
    let cross = vec![[0., 0.], [10., 10.], [0., 10.], [10., 0.], [20., 0.]];
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = cross.clone();
    p.pressure.device = vec![Some(0.1), Some(0.9), None, Some(0.3), Some(0.7)];
    p.pressure.controls = vec![0.5, 1., 1.5, 2., 1.];
    let old = p.clone();
    let selected = Selection {
        controls: vec![
            Control { path: id, point: 1 },
            Control { path: id, point: 2 },
        ],
        paths: vec![],
    };
    let state = core::redraw(
        &d,
        layer,
        &selected,
        &[[10., 10.], [7., 15.], [0., 10.]],
        Ends::Free,
        crate::shapes::Curve::StraightLine,
    )?;
    let p = &state.scene.lines.layers[0].paths[0];
    check(
        p.points[0] == cross[0] && p.points[4..] == cross[3..],
        "Self-cross redraw changed unselected geometry",
    )?;
    // New indices4..5 map back to original3..4, including pressure knots at the crossing.
    for n in 0..101 {
        let f = n as f64 / 100.;
        check(
            (p.pressure.device_at((4. + f) / 5.) - old.pressure.device_at((3. + f) / 4.)).abs()
                < 1e-9
                && (p.pressure.control_at(f / 5.) - old.pressure.control_at(f / 4.)).abs() < 1e-9,
            "Self-cross pressure chose spatially nearest branch",
        )?;
    }
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = vec![[4., 16.], [16., 4.], [28., 16.]];
    p.pressure = Default::default();
    let original = p.clone();
    let span = Selection {
        controls: vec![
            Control { path: id, point: 0 },
            Control { path: id, point: 2 },
        ],
        paths: vec![],
    };
    for (ends, a, b) in [
        (Ends::Both, false, false),
        (Ends::Either, false, true),
        (Ends::Free, true, true),
    ] {
        let state = core::redraw(
            &d,
            layer,
            &span,
            &[[5., 18.], [16., 10.], [29., 18.]],
            ends,
            crate::shapes::Curve::StraightLine,
        )?;
        let p = &state.scene.lines.layers[0].paths[0];
        check(
            (p.points[0] != original.points[0]) == a && (p.points[2] != original.points[2]) == b,
            "Redraw endpoint policies",
        )?;
    }
    let all = Selection {
        paths: d.scene.lines.layers[0].paths.iter().map(|p| p.id).collect(),
        ..Default::default()
    };
    let state = core::prepare(&d, layer, &all, Operation::Width(Width::Thicken, 2., false))?;
    check(
        state.scene.lines.layers[0].paths[0].width == 5.
            && state.scene.lines.layers[0].paths[1].width == 6.
            && state.scene.lines.layers[0].paths[1].color[3] == 255,
        "Mixed width/alpha changed incorrectly",
    )?;
    d.scene.transform = crate::scene::Affine([2., 0., 0., 0.5, 0., 0.]);
    let state = core::prepare(&d, layer, &s, Operation::Width(Width::Fix, 0., true))?;
    check(
        state.scene.lines.layers[0].paths[0].width == 2.,
        "One-pixel minimum ignored smallest affine axis",
    )?;
    d.scene.transform = Default::default();
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = vec![[0., 0.], [1e-6, 0.]];
    check(
        core::prepare(&d, layer, &s, Operation::Simplify(100.))?
            .scene
            .valid(),
        "Short line rejected",
    )?;
    p_closed(&mut d, layer, id)?;
    Ok(())
}
fn straight_push() -> Result<(), String> {
    let mut d = crate::vector_nodes_verify::fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let id = d.scene.lines.layers[0].paths[0].id;
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = vec![[0., 0.], [1000., 0.]];
    p.pressure.device = vec![Some(0.2), None, Some(0.4)];
    p.pressure.controls = vec![0.5, 2.];
    let original = p.clone();
    let s = Selection {
        paths: vec![id],
        ..Default::default()
    };
    let state = core::push(&d, layer, &s, [500., 0.], [0., 20.], 40., Ends::Both)?;
    let p = &state.scene.lines.layers[0].paths[0];
    check(
        p.points[0] == [0., 0.]
            && p.points.last() == Some(&[1000., 0.])
            && p.points.contains(&[500., 20.]),
        "Two-control straight line did not bend locally",
    )?;
    for (n, point) in p.points.iter().enumerate() {
        if !(460. ..=540.).contains(&point[0]) {
            check(point[1] == 0., "Push changed outside radius")?;
        }
        let t = n as f64 / (p.points.len() - 1) as f64;
        check(
            (p.pressure.device_at(t) - original.pressure.device_at(point[0] / 1000.)).abs() < 1e-9
                && (p.pressure.control_at(t) - original.pressure.control_at(point[0] / 1000.))
                    .abs()
                    < 1e-9,
            "Push resampling changed source pressure",
        )?;
    }
    let before = d.snapshot();
    d.commit_state(state)?;
    check(d.undo() && d.snapshot() == before, "Straight push Undo")?;
    Ok(())
}
fn p_closed(d: &mut crate::document::Document, layer: u64, id: u64) -> Result<(), String> {
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = vec![[0., 0.], [10., 0.], [10., 10.], [0., 10.]];
    p.closed = true;
    p.pressure.controls = vec![0.5, 1., 2., 1.5];
    let selected = Selection {
        controls: vec![Control { path: id, point: 3 }],
        paths: vec![],
    };
    let state = core::prepare(d, layer, &selected, Operation::Split)?;
    let p = &state.scene.lines.layers[0].paths[0];
    check(
        !p.closed && p.points.len() == 5 && p.points[0] == [0., 10.] && p.points[4] == p.points[0],
        "Closed split seam",
    )?;
    let before = d.snapshot();
    d.commit_state(state)?;
    check(d.undo() && d.snapshot() == before, "Closed split Undo")?;
    let s = Selection {
        paths: vec![id],
        ..Default::default()
    };
    check(
        core::prepare(d, layer, &s, Operation::Simplify(100.))?
            .scene
            .lines
            .layers[0]
            .paths[0]
            .points
            .len()
            >= 3,
        "Closed simplification collapsed outline",
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn refine_operations_roundtrip_pressure_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
