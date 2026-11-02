use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::pose_sequence_verify::verify(root)?;
    let dir = root.join("pose-sequence-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |ok: bool, s: &str| if ok { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        let (d, asset) = drawer_core::pose_sequence_verify::fixture()?;
        app.add_document(d);
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
                .pose_sequences
                .controls
                .get(key)
                .ok_or_else(|| format!("Missing pose control {key}"))?;
            check(r.center().y < 690., &format!("Pose control clipped: {key}"))?;
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
            while app.pose_sequences.job.is_some() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Pose worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "pose-sequences");
        settle(&mut app, &mut time);
        app.pose_sequences.start = 2;
        app.pose_sequences.repeats = 2;
        settle(&mut app, &mut time);
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "bind")?;
        drain(&mut app, &mut time)?;
        check(
            app.pose_sequences.staged.is_some() && app.documents[0].snapshot() == before,
            &format!(
                "Pose link preview missing: {}; open{} rig{:?} asset{:?} rect{:?} clip{:?} stamp{:?} doc{:?}",
                app.pose_sequences.notice,
                app.pose_sequences.open,
                app.pose_sequences.rig,
                app.pose_sequences.asset,
                app.pose_sequences.controls.get("bind"),
                app.pose_sequences.controls.get("bind-clip"),
                app.pose_sequences.stamp,
                (
                    app.documents[0].id,
                    app.documents[0].revision,
                    app.documents[0].current_frame
                )
            ),
        )?;
        check(
            app.begin_lighting().is_err() && app.begin_world3d().is_err(),
            "Other editor replaced pose preview",
        )?;
        click(&mut app, &mut time, "play")?;
        check(
            app.pose_sequences.staged.is_some() && app.pose_sequences.play.is_none(),
            "Playback discarded prepared edit",
        )?;
        click(&mut app, &mut time, "apply")?;
        check(
            core::active(&app.documents[0].scene, 1).is_some(),
            "Explicit mouse link not committed",
        )?;
        let linked = app.documents[0].snapshot();
        click(&mut app, &mut time, "pose-1")?;
        app.pose_sequences.draft.as_mut().unwrap().angles[0][2] = 60.;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "pose-edit")?;
        drain(&mut app, &mut time)?;
        check(
            app.pose_sequences.staged.is_some() && app.documents[0].snapshot() == linked,
            &format!("Selected pose preview: {}", app.pose_sequences.notice),
        )?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.rigs[&1] == before.scene.world3d.rigs[&1]
                && app.documents[0].scene.world3d.sequences.assets[&asset]
                    .clip
                    .poses[1]
                    .angles[0][2]
                    == 60.
                && app.documents[0]
                    .undo_label()
                    .is_some_and(|s| s.contains("selected sequence pose")),
            "Mouse pose scope/history differs",
        )?;
        let poses = app.documents[0].scene.world3d.sequences.clone();
        click(&mut app, &mut time, "scope-base")?;
        app.pose_sequences.draft.as_mut().unwrap().angles[0][2] = 10.;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "pose-edit")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.sequences == poses
                && app.documents[0].scene.world3d.rigs[&1].angles[0][2] == 10.
                && app.documents[0]
                    .undo_label()
                    .is_some_and(|s| s.contains("base scene")),
            "Mouse base scope changed saved poses",
        )?;
        click(&mut app, &mut time, "undo")?;
        check(
            app.documents[0].scene.world3d.rigs[&1].angles[0][2] == 0.
                && app.documents[0].scene.world3d.sequences == poses,
            "Base scope Undo changed pose sequence",
        )?;
        click(&mut app, &mut time, "scope-pose")?;
        let frozen = app.documents[0].snapshot();
        let playhead = app.documents[0].current_frame;
        let undo = app.documents[0].undo_label().map(str::to_owned);
        click(&mut app, &mut time, "play")?;
        for _ in 0..30 {
            frame(&mut app, &mut time, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        check(
            app.pose_sequences.texture.is_some(),
            &format!("Loop preview absent: {}", app.pose_sequences.notice),
        )?;
        click(&mut app, &mut time, "stop")?;
        check(
            app.documents[0].snapshot() == frozen
                && app.documents[0].current_frame == playhead
                && app.documents[0].undo_label() == undo.as_deref()
                && !app.sequence_pending(),
            "Preview loop changed document/history/playhead",
        )?;
        let file = dir.join("poses.drawerposes");
        app.pose_sequences.output = file.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "export")?;
        let bytes = std::fs::read(&file)
            .map_err(|e| format!("Sequence export: {e}; {}", app.pose_sequences.notice))?;
        app.pose_sequences.path = file.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "load")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.sequences.assets.len() == 2
                && std::fs::read(&file).map_err(|e| e.to_string())? == bytes,
            "Mouse snapshot registration absent / source changed",
        )?;
        let asset = app.pose_sequences.asset.unwrap();
        let frozen = app.documents[0].snapshot();
        click(&mut app, &mut time, "rename")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "cancel")?;
        check(
            app.documents[0].snapshot() == frozen && !app.sequence_pending(),
            "Cancel committed pose edit",
        )?;
        app.stage_sequence(
            &ctx,
            Some(Edit::Rename {
                asset,
                name: "Stale".into(),
            }),
            None,
        )?;
        app.documents[0].current_frame = 1;
        drain(&mut app, &mut time)?;
        check(
            !app.sequence_pending() && app.documents[0].snapshot() == frozen,
            "Frame change retained prepared pose edit",
        )?;
        app.stage_sequence(
            &ctx,
            Some(Edit::Rename {
                asset,
                name: "Closing".into(),
            }),
            None,
        )?;
        app.request_close(app.active_tab.ok_or("Missing pose tab")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.pose_sequences.open && !app.sequence_pending(),
            "Closed document retained pose job",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
