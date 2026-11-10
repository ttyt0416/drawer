use super::*;
pub(super) fn run(root: &Path) -> Result<(), String> {
    core::verify::run(root)?;
    let dir = root.join("printing-ui");
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
        a.documents[0].path.clone(),
        a.documents[0].dirty,
        a.documents[0].history_counts(),
    );
    a.begin_print_pages(core::verify::fixture())?;
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
            .printing
            .controls
            .get(key)
            .ok_or("Missing print control")?;
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
    click(&mut a, &mut time, "letter")?;
    if a.printing.draft.paper != Paper::Letter || a.printing.reviewed {
        return Err("Print paper pointer/review invalidation failed".into());
    }
    click(&mut a, &mut time, "save")?;
    let stored = Settings::load(a.storage.as_ref().unwrap())?;
    if stored.paper != Paper::Letter {
        return Err(a.printing.message.clone());
    }
    a.printing.reviewed = true;
    let path = dir.join("virtual.pdf");
    let mut p = std::mem::take(&mut a.printing);
    a.print_action(&mut p, Some(path.clone()))?;
    a.printing = p;
    for _ in 0..4000 {
        frame(&mut a, &mut time, vec![]);
        if a.printing.job.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    if a.printing.job.is_some()
        || !path.exists()
        || drawer_core::book_pdf::validate(&std::fs::read(&path).map_err(|e| e.to_string())?)? != 2
    {
        return Err(format!("Virtual PDF job failed: {}", a.printing.message));
    }
    click(&mut a, &mut time, "close")?;
    if a.documents[0].snapshot() != before
        || identity
            != (
                a.documents[0].path.clone(),
                a.documents[0].dirty,
                a.documents[0].history_counts(),
            )
    {
        return Err("Print changed drawing/history".into());
    }
    let n = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
    if n.print_panel().draft != stored {
        return Err("Print preferences did not restart".into());
    }
    drop(n);
    drop(a);
    drawer_core::asset_catalog::cleanup_verification(root, &dir)
}
