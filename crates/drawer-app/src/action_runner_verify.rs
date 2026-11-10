use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    core::verify::run(root)?;
    let dir = root.join("action-runner-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut a = Drawer::new(&ctx, None);
        a.create_canvas(CanvasSpec {
            width: 3,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let before = a.documents[0].snapshot();
        let mut t = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let r = *a
                .action_runner
                .controls
                .get(key)
                .ok_or("Missing playback control")?;
            for pressed in [true, false] {
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            Ok(())
        };
        let wait = |a: &mut Drawer, t: &mut f64| -> Result<(), String> {
            for _ in 0..4000 {
                frame(a, t, vec![]);
                if a.action_runner.job.is_none() {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err("Playback worker timeout".into())
        };
        a.begin_action_runner(core::verify::fixture());
        click(&mut a, &mut t, "document")?;
        wait(&mut a, &mut t)?;
        if !matches!(&a.action_runner.run,Some(Run::Single(s)) if s.cursor==1)
            || a.documents[0].snapshot() != before
        {
            return Err(format!(
                "Stop/original protection failed: {}",
                a.action_runner.message
            ));
        }
        click(&mut a, &mut t, "skip-step")?;
        click(&mut a, &mut t, "resume")?;
        wait(&mut a, &mut t)?;
        click(&mut a, &mut t, "apply")?;
        if a.documents[0].dpi != 720.
            || a.documents[0].layers.len() != before.layers.len() + 1
            || a.documents[0].history_counts() != (1, 0)
        {
            return Err(format!(
                "Pointer resume/apply failed: {}",
                a.action_runner.message
            ));
        }
        a.documents[0].undo();
        if a.documents[0].snapshot() != before {
            return Err("Playback Undo incomplete".into());
        }
        let input = dir.join("input.drawer");
        drawer_core::files::save(&mut a.documents[0], &input).map_err(|e| e.to_string())?;
        let original = std::fs::read(&input).map_err(|e| e.to_string())?;
        a.begin_action_runner(core::verify::fixture());
        a.action_runner.inputs = vec![input.clone()];
        a.action_runner.folder = dir.display().to_string();
        a.action_runner.steps = core::Errors::Skip;
        click(&mut a, &mut t, "batch")?;
        wait(&mut a, &mut t)?;
        if !matches!(&a.action_runner.run,Some(Run::Batch(b)) if b.complete()&&b.results.len()==1&&b.results[0].output.is_some()&&b.log.exists())
            || std::fs::read(&input).map_err(|e| e.to_string())? != original
        {
            return Err(format!(
                "Pointer file batch failed: {}",
                a.action_runner.message
            ));
        }
        click(&mut a, &mut t, "close")?;
        if a.action_runner.open || a.documents[0].snapshot() != before {
            return Err(format!(
                "Playback close: open={}, changed={}",
                a.action_runner.open,
                a.documents[0].snapshot() != before
            ));
        }
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
