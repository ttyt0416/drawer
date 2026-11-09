use super::*;
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub(super) fn run(root: &Path) -> Result<(), String> {
    core::verify::verify(root)?;
    let dir = root.join("asset-catalog-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let fixture = dir.join("fixture");
    std::fs::create_dir(&fixture).map_err(|e| e.to_string())?;
    let index = core::verify::fixture(&fixture)?;
    let ctx = egui::Context::default();
    let user = dir.join("data");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    let frame = |a: &mut Drawer, t: &mut f64, events| {
        *t += 0.05;
        input_replay::frame(&ctx, a, *t, events);
    };
    let settle = |a: &mut Drawer, t: &mut f64| {
        for _ in 0..3 {
            frame(a, t, vec![]);
        }
    };
    let wait = |a: &mut Drawer, t: &mut f64| -> Result<(), String> {
        for _ in 0..1000 {
            frame(a, t, vec![]);
            if a.asset_catalog.job.is_none() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Err("Catalog job timeout".into())
    };
    let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
        settle(a, t);
        let rect = *a
            .asset_catalog
            .controls
            .get(key)
            .ok_or("Missing asset catalog control")?;
        for pressed in [true, false] {
            frame(
                a,
                t,
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        settle(a, t);
        Ok(())
    };
    app.begin_asset_catalog()?;
    app.asset_catalog.root = index.root.display().to_string();
    app.asset_catalog.recursive = true;
    click(&mut app, &mut time, "scan")?;
    wait(&mut app, &mut time)?;
    check(
        app.asset_catalog
            .index
            .as_ref()
            .is_some_and(|i| i.entries.len() == 5),
        "Pointer index build failed",
    )?;
    let mut p = std::mem::take(&mut app.asset_catalog);
    Drawer::select_asset(&mut p, PathBuf::from("a.png"));
    app.preview_asset(&mut p, false)?;
    app.asset_catalog = p;
    wait(&mut app, &mut time)?;
    check(
        app.asset_catalog.draft.is_some() && app.documents[0].snapshot() == original,
        "Asset insertion preview changed drawing",
    )?;
    click(&mut app, &mut time, "cancel-preview")?;
    check(
        app.asset_catalog.draft.is_none() && app.documents[0].snapshot() == original,
        "Cancel asset preview changed drawing",
    )?;
    let mut p = std::mem::take(&mut app.asset_catalog);
    app.preview_asset(&mut p, false)?;
    app.asset_catalog = p;
    wait(&mut app, &mut time)?;
    click(&mut app, &mut time, "review")?;
    click(&mut app, &mut time, "apply")?;
    check(
        app.asset_catalog.draft.is_none()
            && app.documents[0].scene.objects.len() == 1
            && app.documents[0].history_counts().0 == 1,
        "Pointer asset Apply / one Undo failed",
    )?;
    let inserted = app.documents[0].snapshot();
    check(
        app.documents[0].undo()
            && app.documents[0].snapshot() == original
            && app.documents[0].redo()
            && app.documents[0].snapshot() == inserted,
        "Asset insert document Undo/Redo",
    )?;
    let mut p = std::mem::take(&mut app.asset_catalog);
    let current = p.index.as_ref().unwrap().clone();
    let mut metadata = current.get(Path::new("a.png"))?.metadata.clone();
    metadata.tags = vec!["ink".into()];
    metadata.pinned = true;
    app.publish_catalog(&mut p, current.edited(Path::new("a.png"), metadata)?, None)?;
    app.asset_catalog = p;
    click(&mut app, &mut time, "undo")?;
    check(
        app.documents[0].snapshot() == inserted
            && app
                .asset_catalog
                .index
                .as_ref()
                .unwrap()
                .get(Path::new("a.png"))?
                .metadata
                .tags
                .is_empty(),
        "Library Undo changed drawing or failed metadata",
    )?;
    click(&mut app, &mut time, "redo")?;
    core::verify::image(&index.root.join("a.png"), [4, 2], [20, 180, 40, 255])?;
    let mut p = std::mem::take(&mut app.asset_catalog);
    let next = core::scan(
        &index.root,
        true,
        p.index.as_deref(),
        &AtomicBool::new(false),
        &AtomicUsize::new(0),
    )?;
    app.publish_catalog(&mut p, next, None)?;
    p.size = ReplaceSize::FitExisting;
    app.preview_asset(&mut p, true)?;
    app.asset_catalog = p;
    wait(&mut app, &mut time)?;
    click(&mut app, &mut time, "review")?;
    click(&mut app, &mut time, "apply")?;
    check(
        app.asset_catalog.draft.is_none()
            && app.documents[0].scene.objects.len() == 1
            && app.documents[0].snapshot() != inserted,
        "Pointer selected image replacement failed",
    )?;
    check(
        app.documents[0].undo()
            && app.documents[0].snapshot() == inserted
            && app.documents[0].redo(),
        "Replace one Undo failed",
    )?;
    let current = app.documents[0].snapshot();
    let mut p = std::mem::take(&mut app.asset_catalog);
    app.preview_asset(&mut p, true)?;
    app.asset_catalog = p;
    wait(&mut app, &mut time)?;
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let changed = app.documents[0].snapshot();
    let mut p = std::mem::take(&mut app.asset_catalog);
    p.reviewed = true;
    check(
        app.apply_asset(&mut p).is_err() && app.documents[0].snapshot() == changed,
        "Stale asset preview was applied",
    )?;
    p.draft = None;
    app.asset_catalog = p;
    check(
        app.documents[0].undo() && app.documents[0].snapshot() == current,
        "Stale test Undo failed",
    )?;
    let expected = app.asset_catalog.index.as_ref().unwrap().as_ref().clone();
    let mut restarted =
        Drawer::new(&ctx, None).with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    restarted.begin_asset_catalog()?;
    check(
        restarted.asset_catalog.index.as_deref() == Some(&expected),
        "Catalog restart lost metadata/versions",
    )?;
    app.asset_catalog.open = false;
    app.asset_catalog_window(&ctx);
    check(
        app.asset_catalog.index.is_some(),
        "Closing catalog discarded its session state",
    )?;
    core::cleanup_verification(root, &dir)
}
