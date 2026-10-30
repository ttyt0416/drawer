use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::graph_verify::verify(root)?;
    let dir = root.join("graph-ui-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        let (d, owner) = drawer_core::graph_verify::fixture()?;
        app.add_document(d);
        app.execute_command(&ctx, "animation-graph");
        check(app.graph_editor.open, "Graph command did not open")?;
        app.graph_editor.selected = 10;
        app.graph_editor.preferences = Preferences {
            time: [-2., 24.],
            value: [-2., 24.],
            snap_time: true,
            time_step: 2,
            snap_value: true,
            value_step: 2.,
            ..Default::default()
        };
        let mut time = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, app, *t, events);
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let point = |app: &Drawer, p: [f64; 2]| -> Result<egui::Pos2, String> {
            let r = app.graph_editor.plot.ok_or("Graph plot missing")?;
            let v = app.graph_editor.preferences;
            Ok(egui::pos2(
                r.left() + ((p[0] - v.time[0]) / (v.time[1] - v.time[0])) as f32 * r.width(),
                r.bottom() - ((p[1] - v.value[0]) / (v.value[1] - v.value[0])) as f32 * r.height(),
            ))
        };
        let click = |app: &mut Drawer, t: &mut f64, r: egui::Rect| {
            for down in [true, false] {
                frame(
                    app,
                    t,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        button(r.center(), down),
                    ],
                );
            }
            frame(app, t, vec![]);
        };
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        let start = *app
            .graph_editor
            .keys
            .get(&10)
            .ok_or("Graph key hit point missing")?;
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        let target = point(&app, [12., 12.])?;
        frame(&mut app, &mut time, vec![egui::Event::PointerMoved(target)]);
        check(
            app.documents[0].snapshot() == before && app.documents[0].history_counts() == history,
            "Graph drag changed document before release",
        )?;
        let target = point(&app, [13.2, 13.2])?;
        frame(&mut app, &mut time, vec![egui::Event::PointerMoved(target)]);
        frame(&mut app, &mut time, vec![button(target, false)]);
        check(
            app.documents[0]
                .scene
                .keyframes
                .get(owner)
                .unwrap()
                .keys
                .iter()
                .any(|k| k.frame == 14 && k.pose.offset[0] == 14.)
                && app.documents[0].history_counts().0 == history.0 + 1,
            &format!(
                "Pointer graph snap/release failed: {}",
                app.graph_editor.notice
            ),
        )?;
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "Graph drag Undo did not restore original key/value",
        )?;
        frame(&mut app, &mut time, vec![]);
        app.graph_editor.selected = 10;
        app.graph_editor.preferences.split_handles = true;
        app.graph_editor.preferences.snap_time = false;
        app.graph_editor.preferences.snap_value = false;
        frame(&mut app, &mut time, vec![]);
        let original = app.documents[0].snapshot();
        let start = app.graph_editor.handles[1].ok_or("Outgoing handle missing")?;
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        let target = point(&app, [12.5, 13.])?;
        frame(&mut app, &mut time, vec![egui::Event::PointerMoved(target)]);
        check(
            app.graph_editor.staged.is_some() && app.documents[0].snapshot() == original,
            &format!(
                "Handle pointer preview missing: {}",
                app.graph_editor.notice
            ),
        )?;
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        frame(
            &mut app,
            &mut time,
            vec![
                button(target, false),
                egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: Default::default(),
                },
            ],
        );
        check(
            app.documents[0].snapshot() == original && app.graph_editor.staged.is_none(),
            "Esc applied a handle drag",
        )?;
        frame(&mut app, &mut time, vec![]);
        let start = app.graph_editor.handles[1].unwrap();
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        let target = point(&app, [12.5, 13.])?;
        frame(&mut app, &mut time, vec![egui::Event::PointerMoved(target)]);
        frame(&mut app, &mut time, vec![button(target, false)]);
        let track = app.documents[0].scene.keyframes.get(owner).unwrap();
        check(
            matches!(track.keys[1].curve, Curve::Bezier { .. })
                && track.keys[0].curve == Curve::Linear,
            "Independent handle release changed adjacent tangent or failed to apply",
        )?;
        let before = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let r = app.graph_editor.controls["zoom"];
        click(&mut app, &mut time, r);
        check(
            app.documents[0].snapshot() == before
                && app.documents[0].history_counts() == counts
                && app.documents[0].dirty == dirty,
            "Graph zoom changed drawing/history/dirty",
        )?;
        let r = app.graph_editor.controls["save"];
        click(&mut app, &mut time, r);
        let expected = app.graph_editor.preferences;
        let restart = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        check(
            restart.graph_editor.preferences == expected,
            "Graph view failed to restore after restart",
        )?;
        drop(restart);
        app.stage_graph_edit(Edit::Move {
            from: 10,
            point: [12., 12.],
            options: app.graph_editor.preferences,
            replace: false,
        })?;
        let state = drawer_core::keyframes::prepare(
            &app.documents[0],
            owner,
            drawer_core::keyframes::Edit::Move { from: 20, to: 21 },
        )?;
        app.documents[0].commit_state(state)?;
        let edited = app.documents[0].snapshot();
        frame(&mut app, &mut time, vec![]);
        check(
            app.graph_editor.staged.is_none() && app.documents[0].snapshot() == edited,
            "Stale graph draft overwrote newer keys",
        )?;
        app.stage_graph_edit(Edit::Move {
            from: 10,
            point: [12., 12.],
            options: app.graph_editor.preferences,
            replace: false,
        })?;
        app.request_close(app.active_tab.ok_or("Active graph document missing")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        frame(&mut app, &mut time, vec![]);
        check(
            app.documents.is_empty()
                && app.graph_editor.staged.is_none()
                && app.graph_editor.drag.is_none(),
            "Closing document retained graph edit/drag",
        )?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
