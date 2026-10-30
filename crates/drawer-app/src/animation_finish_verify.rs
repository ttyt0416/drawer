use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::animation_finish_verify::verify(root)?;
    let dir = root.join("finish-ui-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        let (d, _) = drawer_core::animation_finish_verify::fixture()?;
        app.add_document(d);
        app.execute_command(&ctx, "animation-finish");
        check(app.animation_finish.open, "Finishing command did not open")?;
        let mut time = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, app, *t, events);
        };
        let click = |app: &mut Drawer, t: &mut f64, r: egui::Rect| {
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
            frame(app, t, vec![]);
        };
        let drain = |app: &mut Drawer, t: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.animation_finish.job.is_some() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Finishing worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            for _ in 0..2 {
                frame(app, t, vec![]);
            }
            Ok(())
        };
        let apply = |app: &mut Drawer, t: &mut f64| -> Result<(), String> {
            check(
                app.animation_finish.staged.is_some(),
                &format!(
                    "Finishing preparation failed: {}",
                    app.animation_finish.notice
                ),
            )?;
            let r = app.animation_finish.controls["review"];
            click(app, t, r);
            let r = app.animation_finish.controls["apply"];
            click(app, t, r);
            check(
                app.animation_finish.staged.is_none(),
                &format!("Pointer apply failed: {}", app.animation_finish.notice),
            )
        };
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let original = app.documents[0].snapshot();
        app.animation_finish.safe = true;
        app.animation_finish.percent = [80, 80];
        app.animation_finish.area = Area::Overflow;
        app.animation_finish.first = 4;
        app.animation_finish.last = 8;
        app.animation_finish.update_frame = true;
        app.animation_finish.bounds = [-1., 0., 3., 3.];
        // Match real interaction: let the optional frame fields enter layout before clicking.
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app.animation_finish.controls["output"];
        click(&mut app, &mut time, r);
        drain(&mut app, &mut time)?;
        check(
            app.animation_finish.staged.is_some(),
            &format!(
                "Pointer output settings failed: {}",
                app.animation_finish.notice
            ),
        )?;
        let r = app.animation_finish.controls["apply"];
        click(&mut app, &mut time, r);
        check(
            app.documents[0].snapshot() == original,
            "Unreviewed finishing changes applied",
        )?;
        apply(&mut app, &mut time)?;
        check(
            app.documents[0].scene.animation_output.settings.safe == Some([80, 80])
                && app.documents[0]
                    .scene
                    .timing
                    .playback
                    .is_some_and(|p| p.first == 3 && p.end == 8),
            "Reviewed range / safe settings differ",
        )?;
        let saved = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        app.animation_finish.preferences.view.overflow = true;
        app.animation_finish.preferences.view.safe = false;
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        check(
            app.documents[0].snapshot() == saved
                && app.documents[0].history_counts() == history
                && app.documents[0].dirty == dirty,
            "Guide toggles changed document/Undo/dirty",
        )?;
        let r = app.animation_finish.controls["save"];
        click(&mut app, &mut time, r);
        let expected = app.animation_finish.preferences.clone();
        let restarted = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        check(
            restarted.animation_finish.preferences == expected,
            "Local guide/naming defaults did not survive restart",
        )?;
        drop(restarted);
        let id = app.documents[0].scene.timing.audio[0].id;
        app.stage_animation_finish(
            &ctx,
            Edit::Audio {
                id,
                start: 4,
                gain: 50,
                mute: false,
            },
        )?;
        drain(&mut app, &mut time)?;
        apply(&mut app, &mut time)?;
        check(
            app.documents[0].animation.as_ref().unwrap().audio[0].start == 4
                && app.documents[0].scene.timing.audio[0].sound.gain == 50,
            "Reviewed audio start/volume not applied",
        )?;
        app.stage_animation_finish(
            &ctx,
            Edit::Names {
                cels: true,
                shots: true,
                cel: core::Rule::default(),
                shot: core::Rule {
                    pattern: "Shot_{n}".into(),
                    ..Default::default()
                },
            },
        )?;
        drain(&mut app, &mut time)?;
        apply(&mut app, &mut time)?;
        check(
            app.documents[0].scene.cels.definitions[0].name == "Cel_001"
                && app.documents[0].scene.shots.entries[0].shot.name == "Shot_001",
            "Reviewed cell/shot names not applied",
        )?;
        let before = app.documents[0].snapshot();
        app.stage_animation_finish(&ctx, Edit::ClearFrames)?;
        app.animation_finish
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        drain(&mut app, &mut time)?;
        check(
            app.animation_finish.staged.is_none() && app.documents[0].snapshot() == before,
            "Cancelled finishing operation changed drawing",
        )?;
        app.stage_animation_finish(&ctx, Edit::ClearFrames)?;
        app.documents[0].edit_pixel(0, 3, 2, [1, 2, 3, 255]);
        let changed = app.documents[0].snapshot();
        drain(&mut app, &mut time)?;
        check(
            app.animation_finish.staged.is_none() && app.documents[0].snapshot() == changed,
            "Stale finishing result overwrote edits",
        )?;
        app.stage_animation_finish(&ctx, Edit::ClearFrames)?;
        app.request_close(app.active_tab.ok_or("Closing document missing")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        frame(&mut app, &mut time, vec![]);
        check(
            app.documents.is_empty()
                && app.animation_finish.job.is_none()
                && app.animation_finish.staged.is_none(),
            "Document close retained finishing work",
        )?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
