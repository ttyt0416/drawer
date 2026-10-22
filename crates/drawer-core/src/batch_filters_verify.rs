use crate::{
    batch_filters::{self as core, Task},
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::{self as p, Target},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = crate::warp_filters_verify::fixture()?;
    for name in ["Second target", "Not selected"] {
        let state = crate::layers::prepare(
            &d,
            crate::layers::Command::Create {
                kind: crate::layers::Kind::Raster,
                name: name.into(),
                parent: None,
            },
        )?
        .state;
        d.commit_state(state)?;
        let n = d.layers.len() - 1;
        d.layers[n] = d.layers[0].clone();
        d.layers[n].name = name.into();
        d.layers[n].pixels[0] = [20, 30, 200, 230];
    }
    Ok(d)
}
pub fn tasks() -> [Task; 4] {
    [
        Task::default(),
        Task::Blur(crate::spatial_blur::Settings {
            length: 2.,
            ..Default::default()
        }),
        Task::Gaussian {
            radius: 1,
            selection: false,
        },
        Task::Surface {
            operation: crate::surface_effects::Operation::defaults()[0],
            selection: false,
        },
    ]
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let targets = [p::raster(&d, 0), p::raster(&d, 1)];
    for task in tasks() {
        let counts = d.history_counts();
        let state = core::prepare(&d, &targets, task, DEFAULT_DOCUMENT_BUDGET, |_, _| Ok(()))?;
        check(
            state != original && state.layers[2] == original.layers[2],
            "Batch affects selected layers only",
        )?;
        if !matches!(task, Task::Surface { .. }) {
            check(
                state.layers[0] != original.layers[0] && state.layers[1] != original.layers[1],
                "Every selected raster receives the filter",
            )?;
        } else {
            check(
                state.scene.surface_effects.rasters.len() == 2 && state.layers == original.layers,
                "Batch surface effects preserve source rasters",
            )?;
        }
        check(
            state
                .scene
                .last_batch_filter
                .as_ref()
                .is_some_and(|r| r.task == task && r.targets == vec![1, 2]),
            "Batch recipe records complete target list and parameters",
        )?;
        d.commit_state(state)?;
        check(
            d.history_counts().0 == counts.0 + 1,
            "Batch creates one Undo",
        )?;
        let path = root.join("batch-filter.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "Batch native roundtrip",
        )?;
        let mut old = std::fs::read(&path).map_err(|e| e.to_string())?;
        old[8..12].copy_from_slice(&75u32.to_le_bytes());
        let path = root.join("batch-old.drawer");
        std::fs::write(&path, old).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).is_err(),
            "Old header must reject batch records",
        )?;
        check(
            d.undo() && d.snapshot() == original,
            "Batch Undo restores all layers and metadata",
        )?;
    }
    d.layers[1].locked = true;
    let locked = d.snapshot();
    let statuses = core::inspect(&d, &[targets[0], targets[1], Target::Vector(1)]);
    check(
        statuses.len() == 3
            && statuses[0].reason.is_none()
            && statuses[1..].iter().all(|s| s.reason.is_some()),
        "Preflight lists locked and unsupported targets",
    )?;
    let mut ran = false;
    check(
        core::prepare(&d, &targets, tasks()[0], DEFAULT_DOCUMENT_BUDGET, |_, _| {
            ran = true;
            Ok(())
        })
        .is_err()
            && !ran
            && d.snapshot() == locked,
        "Preflight must reject before any processing",
    )?;
    d.layers[1].locked = false;
    let mut cancelled = false;
    check(
        core::prepare(
            &d,
            &targets,
            tasks()[2],
            DEFAULT_DOCUMENT_BUDGET,
            |done, _| {
                if done > 100 {
                    cancelled = true;
                    Err("cancel".into())
                } else {
                    Ok(())
                }
            },
        )
        .is_err()
            && cancelled
            && d.snapshot() == original,
        "Cancellation during second layer leaves first layer untouched",
    )?;
    let mut failing = fixture()?;
    failing.layers[1].placement = Some(crate::document::PlacedRaster {
        width: 32768,
        height: 1,
        offset: [0; 2],
    });
    failing.layers[1].pixels = vec![[200, 30, 50, 255]; 32768];
    let before = failing.snapshot();
    let mut processed_first = false;
    check(
        core::prepare(
            &failing,
            &targets,
            tasks()[2],
            DEFAULT_DOCUMENT_BUDGET,
            |done, _| {
                processed_first |= done >= 100;
                Ok(())
            },
        )
        .is_err()
            && processed_first
            && failing.snapshot() == before,
        "Later target extent failure must not partially apply",
    )?;
    let counts = d.history_counts();
    let dirty = d.dirty;
    let zero = Task::Gaussian {
        radius: 0,
        selection: false,
    };
    check(
        core::prepare(&d, &targets, zero, DEFAULT_DOCUMENT_BUDGET, |_, _| Ok(()))? == original
            && d.history_counts() == counts
            && d.dirty == dirty,
        "All-noop batch preserves dirty and Redo",
    )?;
    d.scene.selection = Some(crate::scene::Rect {
        min: [2.; 2],
        max: [5.; 2],
    });
    let mut scoped = tasks()[2];
    *scoped.selection() = true;
    let after = core::prepare(&d, &targets, scoped, DEFAULT_DOCUMENT_BUDGET, |_, _| Ok(()))?;
    for n in 0..2 {
        for y in 0..7 {
            for x in 0..7 {
                if !(2..5).contains(&x) || !(2..5).contains(&y) {
                    check(
                        after.layers[n].pixel(7, 7, x as i64, y as i64)
                            == d.layers[n].pixel(7, 7, x as i64, y as i64),
                        "Selection outside each target preserved",
                    )?;
                }
            }
        }
    }
    check(
        core::prepare(
            &d,
            &[targets[0], targets[0]],
            tasks()[0],
            DEFAULT_DOCUMENT_BUDGET,
            |_, _| Ok(()),
        )
        .is_err(),
        "Duplicate target rejected",
    )?;
    check(
        core::prepare(&d, &targets, tasks()[0], 1, |_, _| Ok(())).is_err(),
        "Insufficient aggregate budget rejected",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn preflight_atomic_failure_cancel_native_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
