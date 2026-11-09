use super::*;
pub(super) fn run(root: &Path) -> Result<(), String> {
    core::verify::run(root)?;
    let dir = root.join("output-batch-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let ctx = egui::Context::default();
    let mut a = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
    a.create_canvas(CanvasSpec {
        width: 3,
        height: 2,
        dpi: 299.75,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let before = a.documents[0].snapshot();
    let identity = (
        a.documents[0].title.clone(),
        a.documents[0].path.clone(),
        a.documents[0].dirty,
        a.documents[0].history_counts(),
    );
    a.begin_output_batch()?;
    a.output_batch.folder = dir.display().to_string();
    let mut time = 0.;
    let frame = |a: &mut Drawer, t: &mut f64, events| {
        *t += 0.05;
        input_replay::frame(&ctx, a, *t, events);
    };
    let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
        for _ in 0..3 {
            frame(a, t, vec![]);
        }
        let r = *a
            .output_batch
            .controls
            .get(key)
            .ok_or("Missing output control")?;
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
        for _ in 0..2000 {
            frame(a, t, vec![]);
            if a.output_batch.job.is_none() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Err("Output worker timeout".into())
    };
    click(&mut a, &mut time, "prepare")?;
    wait(&mut a, &mut time)?;
    let path = a
        .output_batch
        .plan
        .as_ref()
        .ok_or_else(|| a.output_batch.message.clone())?
        .files[0]
        .path
        .clone();
    click(&mut a, &mut time, "close")?;
    if path.exists() || a.documents[0].snapshot() != before {
        return Err("Output preview cancel wrote/edited document".into());
    }
    a.begin_output_batch()?;
    a.output_batch.folder = dir.display().to_string();
    click(&mut a, &mut time, "prepare")?;
    wait(&mut a, &mut time)?;
    a.output_batch.reviewed = true;
    click(&mut a, &mut time, "write")?;
    wait(&mut a, &mut time)?;
    if !path.exists()
        || a.output_batch
            .report
            .as_ref()
            .is_none_or(|r| r.files.len() != 1 || r.files[0].result.is_err())
        || a.documents[0].snapshot() != before
        || identity
            != (
                a.documents[0].title.clone(),
                a.documents[0].path.clone(),
                a.documents[0].dirty,
                a.documents[0].history_counts(),
            )
    {
        return Err(format!(
            "Output pointer apply/source preservation: {}",
            a.output_batch.message
        ));
    }
    drop(a);
    drawer_core::asset_catalog::cleanup_verification(root, &dir)
}
