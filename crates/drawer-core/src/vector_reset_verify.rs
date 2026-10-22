//! Pressure reset preserves device input and every unrelated document field.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    vector_edit::{self, Selection},
    vector_selection::Control,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = crate::vector_nodes_verify::fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let id = d.scene.lines.layers[0].paths[0].id;
    crate::vector_nodes::prepare(
        &d,
        layer,
        vector_edit::Hit {
            parameter: None,
            path: id,
            control: Some(1),
            point: [0.; 2],
            distance: 0.,
        },
        crate::vector_nodes::Operation::Smooth,
    )?
    .restore(&mut d);
    for (n, p) in d.scene.lines.layers[0].paths.iter_mut().enumerate() {
        p.pressure.controls = vec![0.2, 1.8, 0.4, 0.7, 1.2];
        p.pressure.control_positions = vec![0., 0.13, 0.42, 0.77, 1.];
        p.pressure.device = vec![Some(0.), None, Some(0.7)];
        p.pressure.device_positions = vec![0., 0.31, 1.];
        p.pressure.device_enabled = n == 0;
    }
    let other = d.scene.lines.add_layer("Untouched layer")?;
    d.scene.lines.add_path(
        other,
        vec![[1., 1.], [5., 1.], [5., 5.]],
        true,
        2.,
        [20, 30, 40, 80],
    )?;
    d.scene.lines.layers[1].paths[0].pressure.controls = vec![0., 2., 0.5];
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let first = d.scene.lines.layers[0].paths[0].id;
    let second = d.scene.lines.layers[0].paths[1].id;
    for mode in 0..4 {
        let s = Selection {
            controls: if mode == 0 || mode == 2 {
                vec![Control {
                    path: first,
                    point: 1,
                }]
            } else {
                vec![]
            },
            paths: if mode == 1 {
                vec![first]
            } else if mode == 2 {
                vec![second]
            } else {
                vec![]
            },
        };
        let before = d.snapshot();
        let count = d.history_counts();
        let after = vector_edit::reset_pressure(&d, layer, &s, mode == 3)?;
        check(
            d.snapshot() == before && d.history_counts() == count,
            "Reset preview changed source/history",
        )?;
        check(
            after.scene.lines.selection == before.scene.lines.selection,
            "Reset changed persisted selection",
        )?;
        for (n, (old, new)) in before.scene.lines.layers[0]
            .paths
            .iter()
            .zip(&after.scene.lines.layers[0].paths)
            .enumerate()
        {
            let mut without_pressure = new.clone();
            without_pressure.pressure = old.pressure.clone();
            check(
                without_pressure == *old
                    && old.pressure.device == new.pressure.device
                    && old.pressure.device_positions == new.pressure.device_positions
                    && old.pressure.device_enabled == new.pressure.device_enabled,
                "Reset changed geometry/style/device samples",
            )?;
            let whole = mode == 3 || s.paths.contains(&old.id);
            if whole {
                for i in 0..=100 {
                    check(
                        new.pressure.control_at(i as f64 / 100.) == 1.,
                        "Whole reset left an interior pressure knot",
                    )?;
                }
            } else if n == 0 && !s.controls.is_empty() {
                let params = old.control_parameters()?;
                let selected = params[1];
                check(
                    (new.pressure.control_at(selected) - 1.).abs() < 1e-12,
                    "Wrong logical pressure anchor",
                )?;
                for (i, t) in params.iter().enumerate() {
                    if i != 1 {
                        check(
                            (new.pressure.control_at(*t) - old.pressure.control_at(*t)).abs()
                                < 1e-12,
                            "Reset changed unselected logical pressure",
                        )?;
                    }
                }
                for (i, t) in old.pressure.control_positions.iter().enumerate() {
                    if (*t - selected).abs() > 1e-12 {
                        check(
                            (new.pressure.control_at(*t) - old.pressure.controls[i]).abs() < 1e-12,
                            "Reset discarded an off-node pressure knot",
                        )?;
                    }
                }
            } else {
                check(old == new, "Reset changed an unselected stroke")?;
            }
        }
        let mut expected = before.clone();
        expected.scene.lines.layers[0].paths = after.scene.lines.layers[0].paths.clone();
        check(
            expected == after,
            "Reset changed fields outside the target paths",
        )?;
        d.commit_state(after.clone())?;
        check(
            d.history_counts().0 == count.0 + 1,
            "Reset must be one Undo",
        )?;
        let file = root.join("pressure-reset.drawer");
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == after,
            "Pressure reset native roundtrip",
        )?;
        check(
            d.undo()
                && d.snapshot() == before
                && d.redo()
                && d.snapshot() == after
                && d.undo()
                && d.snapshot() == before,
            "Pressure reset Undo/Redo",
        )?;
    }
    let all = vector_edit::reset_pressure(&d, layer, &Selection::default(), true)?;
    d.commit_state(all)?;
    let before = d.snapshot();
    let count = d.history_counts();
    check(
        !d.commit_state(vector_edit::reset_pressure(
            &d,
            layer,
            &Selection::default(),
            true,
        )?)? && d.snapshot() == before
            && d.history_counts() == count,
        "Default reset created a spurious edit",
    )?;
    check(
        vector_edit::reset_pressure(&d, layer, &Selection::default(), false).is_err(),
        "Empty selection reset accepted",
    )?;
    let bad = Selection {
        controls: vec![Control {
            path: first,
            point: 4000,
        }],
        paths: vec![],
    };
    check(
        vector_edit::reset_pressure(&d, layer, &bad, false).is_err(),
        "Stale point accepted",
    )?;
    for hidden in [false, true] {
        d.scene.lines.layers[0].locked = !hidden;
        d.scene.lines.layers[0].hidden = hidden;
        check(
            vector_edit::reset_pressure(&d, layer, &Selection::default(), true).is_err(),
            "Hidden/locked reset accepted",
        )?;
    }
    d.scene.lines.layers[0].hidden = false;
    let closed_layer = d.scene.lines.layers[1].id;
    let path = d.scene.lines.layers[1].paths[0].id;
    let selection = Selection {
        controls: vec![Control { path, point: 0 }],
        paths: vec![],
    };
    let state = vector_edit::reset_pressure(&d, closed_layer, &selection, false)?;
    check(
        state.scene.lines.layers[1].paths[0].pressure.control_at(0.) == 1.
            && state.scene.lines.layers[1].paths[0]
                .pressure
                .control_at(0.5)
                == 2.,
        "Closed path point reset crossed scope",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn vector_pressure_reset_scopes_knots_device_native_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
