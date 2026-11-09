use super::*;
pub(super) fn run(root: &Path) -> Result<(), String> {
    core::verify::run(root)?;
    let dir = root.join("template-ui-check");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let ctx = egui::Context::default();
    let mut a = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
    a.create_canvas(CanvasSpec {
        width: 4,
        height: 3,
        dpi: 299.75,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    a.documents[0].layers[0].pixels[0] = [90, 50, 20, 128];
    let before = a.documents[0].snapshot();
    let history = a.documents[0].history_counts();
    let path = dir.join("bundle");
    core::export(
        &a.documents[0],
        &path,
        "로컬 템플릿",
        Kind::Layers,
        &a.materials.library,
        &AtomicBool::new(false),
    )?;
    a.begin_template_bundle();
    a.template_bundle.path = path.display().to_string();
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
            .template_bundle
            .controls
            .get(key)
            .ok_or("Missing template control")?;
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
            if a.template_bundle.job.is_none() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Err("Template worker timed out".into())
    };
    click(&mut a, &mut time, "prepare")?;
    wait(&mut a, &mut time)?;
    if a.template_bundle.prepared.is_none() {
        return Err(a.template_bundle.message.clone());
    }
    click(&mut a, &mut time, "close")?;
    if a.documents.len() != 1
        || a.documents[0].snapshot() != before
        || a.template_bundle.prepared.is_some()
    {
        return Err("Template preview cancel changed source".into());
    }
    a.begin_template_bundle();
    click(&mut a, &mut time, "prepare")?;
    wait(&mut a, &mut time)?;
    a.template_bundle.reviewed = true;
    click(&mut a, &mut time, "apply")?;
    if a.documents.len() != 2
        || a.documents[0].snapshot() != before
        || a.documents[0].history_counts() != history
        || a.documents[1].snapshot() != before
        || a.documents[1].path.is_some()
        || a.documents[1].id == a.documents[0].id
    {
        return Err(format!(
            "Template apply failed: {}",
            a.template_bundle.message
        ));
    }
    drop(a);
    drawer_core::asset_catalog::cleanup_verification(root, &dir)
}
