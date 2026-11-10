use super::*;
fn fixture_action() -> Action {
    use drawer_core::auto_actions::{Canvas, Operation, Step};
    Action {
        id: 1,
        name: "반복".into(),
        steps: vec![Step {
            id: 2,
            enabled: true,
            operation: Operation::Canvas(Canvas::FlipHorizontal),
        }],
    }
}
pub(super) fn run(root: &Path) -> Result<(), String> {
    core::verify::run(root)?;
    let dir = root.join("action-repeat-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut a = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        a.simple_view = true;
        a.add_document(drawer_core::png_io_verify::fixture()?);
        a.begin_action_repeat(fixture_action());
        a.action_repeat.settings = core::Settings {
            count: 2,
            wait_ms: 100,
            confirm: true,
            ..Default::default()
        };
        let original = a.documents[0].layers.clone();
        let counts = a.documents[0].history_counts();
        let mut t = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let pos = a
                .action_repeat
                .controls
                .get(key)
                .ok_or_else(|| format!("Missing repeat button {key}"))?
                .center();
            for pressed in [true, false] {
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            Ok(())
        };
        click(&mut a, &mut t, "save")?;
        if core::Settings::load(a.storage.as_ref().unwrap())? != a.action_repeat.settings {
            return Err("Repeat options pointer save/restart".into());
        }
        click(&mut a, &mut t, "document")?;
        for _ in 0..8000 {
            frame(&mut a, &mut t, vec![]);
            if a.action_repeat.job.is_none()
                && a.action_repeat
                    .document
                    .as_ref()
                    .is_some_and(|r| r.schedule.completed == 1)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let first = a.documents[0].layers.clone();
        if first == original || a.documents[0].history_counts().0 != counts.0 + 1 {
            return Err(format!("First round/UI: {}", a.action_repeat.message));
        }
        click(&mut a, &mut t, "stop")?;
        if a.documents[0].layers != first
            || !a.action_repeat.document.as_ref().unwrap().schedule.paused
        {
            return Err("Stop in wait changed completed round".into());
        }
        click(&mut a, &mut t, "continue")?;
        for _ in 0..8000 {
            frame(&mut a, &mut t, vec![]);
            if a.action_repeat.job.is_none()
                && a.action_repeat
                    .document
                    .as_ref()
                    .is_some_and(|r| r.schedule.complete())
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if a.documents[0].layers != original
            || a.documents[0].history_counts().0 != counts.0 + 2
            || !a.documents[0].undo()
            || a.documents[0].layers != first
            || !a.documents[0].undo()
            || a.documents[0].layers != original
        {
            return Err(format!(
                "Repeat continuation/individual Undo: {}",
                a.action_repeat.message
            ));
        }
        click(&mut a, &mut t, "discard")?;
        click(&mut a, &mut t, "document")?;
        a.request_close(0);
        a.accept_close();
        for _ in 0..8000 {
            frame(&mut a, &mut t, vec![]);
            if a.action_repeat.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if !a.documents.is_empty() || !a.action_repeat.document.as_ref().unwrap().schedule.paused {
            return Err("Closed document received a repeated result".into());
        }
        drop(a);
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn repeated_document_pointer_options_stop_continue_undo_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
