use super::*;
use crate::{
    auto_actions::{Canvas, Operation, Step},
    document::DEFAULT_DOCUMENT_BUDGET as B,
};
fn action(operations: Vec<Operation>) -> Action {
    Action {
        id: 1,
        name: "반복 🎨".into(),
        steps: operations
            .into_iter()
            .enumerate()
            .map(|(n, operation)| Step {
                id: n as u64 + 2,
                enabled: true,
                operation,
            })
            .collect(),
    }
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("action-repeat-core");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let settings = Settings {
            count: 2,
            wait_ms: 10,
            confirm: true,
            ..Default::default()
        };
        settings.save(&storage)?;
        if Settings::load(&storage)? != settings {
            return Err("Repeat preference restart".into());
        }
        for bad in [
            Settings {
                count: 0,
                ..settings
            },
            Settings {
                count: 1001,
                ..settings
            },
            Settings {
                wait_ms: 60001,
                ..settings
            },
        ] {
            if bad.save(&storage).is_ok() {
                return Err("Invalid repeat range accepted".into());
            }
        }
        let mut d = crate::png_io_verify::fixture()?;
        let before = d.snapshot();
        let counts = d.history_counts();
        let mut run = Run::new(
            &d,
            action(vec![Operation::Canvas(Canvas::FlipHorizontal)]),
            settings,
            B,
        )?;
        let mut s = run.prepare(&d, 0)?;
        s.run(crate::action_runner::Errors::Stop, &AtomicBool::new(false))?;
        run.commit(s, &mut d, 100, 1.)?;
        if run.schedule.completed != 1
            || d.history_counts().0 != counts.0 + 1
            || run.prepare(&d, 1000).is_ok()
        {
            return Err("Round commit/continue confirmation".into());
        }
        let first = d.snapshot();
        run.schedule.resume();
        if run.prepare(&d, 109).is_ok() {
            return Err("Wait bypassed".into());
        }
        run.schedule.stop();
        if run.prepare(&d, 110).is_ok() || d.snapshot() != first {
            return Err("Stop during wait changed drawing".into());
        }
        run.schedule.resume();
        let mut s = run.prepare(&d, 110)?;
        s.run(crate::action_runner::Errors::Stop, &AtomicBool::new(false))?;
        run.commit(s, &mut d, 110, 1.)?;
        if !run.schedule.complete()
            || d.snapshot() != before
            || d.history_counts().0 != counts.0 + 2
            || !d.undo()
            || d.snapshot() != first
            || !d.undo()
            || d.snapshot() != before
        {
            return Err("Repeated rounds are not separate atomic Undo entries".into());
        }
        // The second iteration changes a private copy, then fails at the layer cap.
        let mut d = crate::png_io_verify::fixture()?;
        let mut state = d.snapshot();
        state.layers.resize(255, state.layers[0].clone());
        d.commit_state(state)?;
        let mut run = Run::new(
            &d,
            action(vec![
                Operation::Canvas(Canvas::FlipHorizontal),
                Operation::NewRaster { name: "New".into() },
            ]),
            Settings {
                count: 2,
                ..Default::default()
            },
            B,
        )?;
        let mut s = run.prepare(&d, 0)?;
        s.run(crate::action_runner::Errors::Stop, &AtomicBool::new(false))?;
        run.commit(s, &mut d, 0, 1.)?;
        let before = d.snapshot();
        let counts = d.history_counts();
        let mut s = run.prepare(&d, 0)?;
        if s.run(crate::action_runner::Errors::Stop, &AtomicBool::new(false))
            .is_ok()
            || d.snapshot() != before
            || d.history_counts() != counts
            || run.schedule.completed != 1
        {
            return Err("Second-round failure escaped the private transaction".into());
        }
        run.schedule.stop();
        // Document-coordinate transforms do not depend on viewport scale or input DPI.
        let mut expected = None;
        for (ppp, zoom, angle, flipped, dpi) in
            [(1., 1., 0., false, 72.), (2., 4., 77., true, 300.)]
        {
            let mut d = crate::png_io_verify::fixture()?;
            d.dpi = dpi;
            d.view.zoom = zoom;
            d.view.angle = angle;
            d.view.flipped = flipped;
            let transform = Operation::Transform {
                layer: 0,
                vector: false,
                settings: crate::vector_scope::Transform {
                    translation: [2., -1.],
                    ..Default::default()
                },
                filter: crate::interpolation::Filter::Nearest,
            };
            let a = action(vec![
                transform,
                Operation::Scroll {
                    document: [17., -9.],
                },
            ]);
            let old =
                d.view
                    .screen_to_document([0.; 2], [d.width as f64, d.height as f64], [0.; 2], ppp);
            let prepared = crate::auto_actions::prepare(&d, &a, B, &AtomicBool::new(false))?;
            prepared.commit_with_scale(&mut d, ppp)?;
            let new =
                d.view
                    .screen_to_document([0.; 2], [d.width as f64, d.height as f64], [0.; 2], ppp);
            if (new[0] - old[0] - 17.).abs() > 1e-6 || (new[1] - old[1] + 9.).abs() > 1e-6 {
                return Err("View scroll used screen instead of document coordinates".into());
            }
            let state = d.snapshot();
            let data = (state.layers.clone(), state.scene.clone());
            if expected.as_ref().is_some_and(|old| old != &data) {
                return Err("Transform geometry depends on view/DPI".into());
            }
            expected = Some(data);
            let counts = d.history_counts();
            let a = action(vec![Operation::Scroll { document: [1., 2.] }]);
            crate::auto_actions::prepare(&d, &a, B, &AtomicBool::new(false))?
                .commit_with_scale(&mut d, ppp)?;
            if d.history_counts() != counts {
                return Err("Scroll added drawing Undo".into());
            }
        }
        let input = dir.join("input.drawer");
        let mut d = crate::png_io_verify::fixture()?;
        crate::files::save(&mut d, &input).map_err(|e| e.to_string())?;
        let bytes = fs::read(&input).map_err(|e| e.to_string())?;
        let out = dir.join("outputs");
        fs::create_dir(&out).map_err(|e| e.to_string())?;
        let mut batch = crate::action_runner::Batch::new(
            vec![input.clone()],
            &out,
            "{title}-{index}",
            action(vec![Operation::Canvas(Canvas::FlipHorizontal)]),
            B,
        )?;
        batch.set_repeat(Settings {
            count: 2,
            confirm: true,
            ..Default::default()
        })?;
        batch.run(
            crate::action_runner::Errors::Stop,
            crate::action_runner::Errors::Stop,
            &AtomicBool::new(false),
        )?;
        if batch.cursor != 0
            || batch.repeat.as_ref().unwrap().completed != 1
            || batch.outputs[0].exists()
        {
            return Err("Batch published an incomplete repeat".into());
        }
        batch.run(
            crate::action_runner::Errors::Stop,
            crate::action_runner::Errors::Stop,
            &AtomicBool::new(false),
        )?;
        if batch.repeat.as_ref().unwrap().completed != 1 {
            return Err("Batch skipped continuation confirmation".into());
        }
        batch.resume_repeat();
        batch.run(
            crate::action_runner::Errors::Stop,
            crate::action_runner::Errors::Stop,
            &AtomicBool::new(false),
        )?;
        let result = crate::files::open(&batch.outputs[0], B).map_err(|e| e.to_string())?;
        if !batch.complete()
            || result.snapshot() != d.snapshot()
            || fs::read(&input).map_err(|e| e.to_string())? != bytes
        {
            return Err("Batch rounds/source/new-only output".into());
        }
        Ok(())
    })();
    crate::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn iterations_wait_confirmation_failure_view_geometry_batch_and_undo() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
