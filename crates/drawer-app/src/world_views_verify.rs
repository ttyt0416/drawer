use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::world_views_verify::verify(root)?;
    let dir = root.join("world-views-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |ok: bool, s: &str| if ok { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::world_views_verify::fixture()?);
        let mut time = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, app, *t, events);
        };
        let settle = |app: &mut Drawer, t: &mut f64| {
            for _ in 0..3 {
                frame(app, t, vec![]);
            }
        };
        let click = |app: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            let r = *app
                .world_views
                .controls
                .get(key)
                .ok_or_else(|| format!("Missing view control {key}"))?;
            check(r.center().y < 690., "View control clipped")?;
            for pressed in [true, false] {
                frame(
                    app,
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
            settle(app, t);
            Ok(())
        };
        let drain = |app: &mut Drawer, t: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                frame(app, t, vec![]);
                if app.world_views.job.is_none()
                    && app.world_views.frames.len() == app.world_views.preferences.count
                    && app.world_views.key.as_ref().is_some_and(|k| {
                        k.preferences == app.world_views.preferences
                            && k.revision == app.documents[0].revision
                    })
                {
                    break;
                }
                if std::time::Instant::now() > end {
                    return Err(format!("View worker timeout: {}", app.world_views.notice));
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "world-views");
        drain(&mut app, &mut time)?;
        check(
            app.world_views.open
                && app.world_views.frames.len() == 2
                && app.world_views.guide_count > 0,
            "Multiple view command/guide display failed",
        )?;
        let before = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let first = app.world_views.preferences.views[0].clone();
        click(&mut app, &mut time, "pane1")?;
        let original = app.world_views.preferences.views[1].clone();
        click(&mut app, &mut time, "pan")?;
        check(
            app.world_views.preferences.views[0] == first
                && app.world_views.preferences.views[1] != original
                && app.documents[0].snapshot() == before
                && app.documents[0].history_counts() == counts,
            "Independent navigation changed scene/other view",
        )?;
        let moved = app.world_views.preferences.views[1].clone();
        click(&mut app, &mut time, "undo")?;
        check(
            app.world_views.preferences.views[1] == original,
            "View Undo failed",
        )?;
        click(&mut app, &mut time, "redo")?;
        check(
            app.world_views.preferences.views[1] == moved,
            "View Redo failed",
        )?;
        app.world_views.preferences.reference = 1;
        click(&mut app, &mut time, "link")?;
        check(
            app.world_views
                .preferences
                .views
                .iter()
                .all(|v| *v == moved)
                && app.world_views.preferences.reference == 1,
            "Explicit linking source ignored",
        )?;
        click(&mut app, &mut time, "yaw")?;
        check(
            app.world_views
                .preferences
                .views
                .iter()
                .all(|v| *v == app.world_views.preferences.views[1]),
            "Linked orbit did not synchronize",
        )?;
        click(&mut app, &mut time, "independent")?;
        let first = app.world_views.preferences.views[0].clone();
        click(&mut app, &mut time, "pan")?;
        check(
            app.world_views.preferences.views[0] == first,
            "Independent mode kept synchronization",
        )?;
        click(&mut app, &mut time, "to-scene")?;
        let copied = app.documents[0].snapshot();
        check(
            copied != before && app.documents[0].history_counts().0 == counts.0 + 1,
            "Scene camera copy not one Undo",
        )?;
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "Scene camera copy Undo failed",
        )?;
        app.documents[0].redo();
        check(
            app.documents[0].snapshot() == copied,
            "Scene camera copy Redo failed",
        )?;
        click(&mut app, &mut time, "from-scene")?;
        check(
            !app.world_views.preferences.linked,
            "One-time copy linked cameras",
        )?;
        click(&mut app, &mut time, "Top")?;
        check(
            app.world_views.preferences.views[1].lens.projection
                == drawer_core::world3d::Projection::Orthographic
                && app.documents[0].snapshot() == copied,
            "Direction view changed document",
        )?;
        app.world_views.preferences.count = 4;
        app.world_views.preferences.split = 0.2;
        drain(&mut app, &mut time)?;
        check(app.world_views.frames.len() == 4, "Four-way split failed")?;
        let previous = app
            .world_views
            .frames
            .iter()
            .map(|f| f.pixels.clone())
            .collect::<Vec<_>>();
        drawer_core::model_library::prepare(
            &app.documents[0],
            drawer_core::model_library::Edit::Duplicate {
                ids: vec![1],
                offset: [1., 1., 1.],
            },
            DEFAULT_DOCUMENT_BUDGET,
            &AtomicBool::new(false),
        )?
        .commit(&mut app.documents[0])?;
        drain(&mut app, &mut time)?;
        check(
            previous
                .iter()
                .zip(&app.world_views.frames)
                .all(|(a, b)| *a != b.pixels),
            "Committed geometry did not refresh every view",
        )?;
        click(&mut app, &mut time, "save")?;
        let stored = app.world_views.preferences.clone();
        let second = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        check(
            second.world_views.preferences == stored,
            "Local view workspace did not restart",
        )?;
        click(&mut app, &mut time, "reset")?;
        check(
            app.world_views.preferences.count == 2 && app.world_views.preferences.split == 0.5,
            "Reset split layout failed",
        )?;
        app.world_views.clear();
        app.start_world_views(&ctx)?;
        app.request_close(app.active_tab.ok_or("Missing view document tab")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.world_views.open
                && app.world_views.job.is_none()
                && app.world_views.frames.is_empty(),
            "Closed document retained camera worker/views",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
