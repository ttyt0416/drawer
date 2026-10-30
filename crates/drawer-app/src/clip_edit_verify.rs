use super::*;
use drawer_core::{
    animation::Fps,
    document::{Background, CanvasSpec},
    timeline_editor::{self, Edit as CelEdit},
};
pub(crate) fn run(root: &std::path::Path) -> Result<(), String> {
    let dir = root.join("clip-ui-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 4,
            height: 3,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let state = timeline_editor::create(&app.documents[0], 12, Fps::default())?;
        app.documents[0].commit_state(state)?;
        let p = timeline_editor::prepare(
            &app.documents[0],
            CelEdit::NewTrack("Cels".into()),
            DEFAULT_DOCUMENT_BUDGET,
        )?;
        let track = p.track.ok_or("Track missing")?;
        let source = p.source.ok_or("Cel missing")?;
        app.documents[0].commit_state(p.state)?;
        let mut child = app.documents[0].scene.cels.get(source).unwrap().open()?;
        child.edit_pixel(0, 0, 0, [255, 0, 0, 255]);
        let s =
            drawer_core::cels::replace(&app.documents[0], source, &child, DEFAULT_DOCUMENT_BUDGET)?;
        app.documents[0].commit_state(s)?;
        app.documents[0].current_frame = 6;
        let p = timeline_editor::prepare(
            &app.documents[0],
            CelEdit::NewCel {
                track,
                name: "Blue".into(),
            },
            DEFAULT_DOCUMENT_BUDGET,
        )?;
        let source = p.source.unwrap();
        app.documents[0].commit_state(p.state)?;
        let mut child = app.documents[0].scene.cels.get(source).unwrap().open()?;
        child.edit_pixel(0, 0, 0, [0, 0, 255, 255]);
        let s =
            drawer_core::cels::replace(&app.documents[0], source, &child, DEFAULT_DOCUMENT_BUDGET)?;
        app.documents[0].commit_state(s)?;
        app.documents[0].current_frame = 3;
        app.execute_command(&ctx, "timeline-clips");
        check(app.clip_editor.open, "Clip command did not open")?;
        let mut time = 0.;
        let frame =
            |app: &mut Drawer, time: &mut f64, modifiers, focused, mut events: Vec<egui::Event>| {
                *time += 0.05;
                events.insert(0, egui::Event::ModifiersChanged(modifiers));
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000., 700.),
                        )),
                        time: Some(*time),
                        focused,
                        events,
                        ..Default::default()
                    },
                    |ui| app.show(ui),
                );
                out.textures_delta.clear();
            };
        let plain =
            |app: &mut Drawer, t: &mut f64| frame(app, t, egui::Modifiers::NONE, true, vec![]);
        let click = |app: &mut Drawer, t: &mut f64, r: egui::Rect| {
            for pressed in [true, false] {
                frame(
                    app,
                    t,
                    egui::Modifiers::NONE,
                    true,
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
            plain(app, t);
        };
        let drain = |app: &mut Drawer, t: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.clip_editor.job.is_some() {
                plain(app, t);
                if std::time::Instant::now() > end {
                    return Err("Clip worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            plain(app, t);
            Ok(())
        };
        for _ in 0..3 {
            plain(&mut app, &mut time);
        }
        let original = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        let view = app.documents[0].view;
        let key = |pressed| egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::ALT,
        };
        frame(
            &mut app,
            &mut time,
            egui::Modifiers::ALT,
            true,
            vec![key(true)],
        );
        check(
            app.documents[0].current_frame == 6
                && app.canvas_view.timeline_read_only
                && app.clip_flip_active(),
            "Held cel flip did not move/read-lock preview",
        )?;
        app.documents[0].view.pan = [41., 27.];
        frame(
            &mut app,
            &mut time,
            egui::Modifiers::ALT,
            true,
            vec![key(false)],
        );
        check(
            app.documents[0].current_frame == 3
                && app.documents[0].view == view
                && !app.clip_flip_active()
                && app.documents[0].snapshot() == original
                && app.documents[0].history_counts() == counts
                && app.documents[0].dirty == dirty,
            "Cel key release did not restore frame/view without edits",
        )?;
        frame(
            &mut app,
            &mut time,
            egui::Modifiers::ALT,
            true,
            vec![key(true)],
        );
        frame(&mut app, &mut time, egui::Modifiers::NONE, false, vec![]);
        check(
            app.documents[0].current_frame == 3 && !app.clip_flip_active(),
            "Focus loss did not restore cel flip",
        )?;
        plain(&mut app, &mut time);
        frame(
            &mut app,
            &mut time,
            egui::Modifiers::ALT,
            true,
            vec![key(true)],
        );
        app.clip_editor.open = false;
        plain(&mut app, &mut time);
        check(
            app.documents[0].current_frame == 3 && !app.clip_flip_active(),
            "Panel close did not restore cel flip",
        )?;
        app.execute_command(&ctx, "timeline-clips");
        app.clip_editor.at = 7;
        for _ in 0..3 {
            plain(&mut app, &mut time);
        }
        let r = *app
            .clip_editor
            .controls
            .get("split")
            .ok_or("Split button missing")?;
        click(&mut app, &mut time, r);
        drain(&mut app, &mut time)?;
        check(
            app.clip_editor.staged.is_some(),
            &format!(
                "Pointer clip split did not prepare: {}",
                app.clip_editor.notice
            ),
        )?;
        let r = *app
            .clip_editor
            .controls
            .get("apply")
            .ok_or("Apply missing")?;
        click(&mut app, &mut time, r);
        check(
            app.documents[0].snapshot() == original,
            "Unreviewed clip split applied",
        )?;
        let r = app.clip_editor.controls["review"];
        click(&mut app, &mut time, r);
        let r = app.clip_editor.controls["apply"];
        click(&mut app, &mut time, r);
        check(
            app.documents[0].scene.timing.clips.len() == 2,
            &format!(
                "Reviewed pointer clip split failed: {}",
                app.clip_editor.notice
            ),
        )?;
        let id = app.documents[0].scene.timing.clips[0].id;
        let before = app.documents[0].snapshot();
        app.stage_clip(&ctx, Edit::Delete(vec![id]), None)?;
        app.clip_editor
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        drain(&mut app, &mut time)?;
        check(
            app.clip_editor.staged.is_none() && app.documents[0].snapshot() == before,
            "Cancelled clip preparation changed drawing",
        )?;
        app.stage_clip(&ctx, Edit::Delete(vec![id]), None)?;
        app.documents[0].edit_pixel(0, 3, 2, [4, 5, 6, 255]);
        let edited = app.documents[0].snapshot();
        drain(&mut app, &mut time)?;
        check(
            app.clip_editor.staged.is_none() && app.documents[0].snapshot() == edited,
            "Old clip result overwrote new drawing",
        )?;
        app.clip_editor.preferences.frames = [0, 6];
        let frame_before = app.documents[0].current_frame;
        let counts = app.documents[0].history_counts();
        for _ in 0..2 {
            plain(&mut app, &mut time);
        }
        check(
            app.clip_editor.images.iter().all(Option::is_some)
                && app.clip_editor.image_keys[0].as_ref().unwrap().3 == 0
                && app.clip_editor.image_keys[1].as_ref().unwrap().3 == 6
                && app.documents[0].current_frame == frame_before
                && app.documents[0].snapshot() == edited
                && app.documents[0].history_counts() == counts,
            "Independent split panes changed playhead or document",
        )?;
        app.clip_editor.preferences.frames = [2, 8];
        app.clip_editor.preferences.default_linked = false;
        let r = app.clip_editor.controls["options"];
        click(&mut app, &mut time, r);
        let expected = app.clip_editor.preferences;
        let restarted = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        check(
            restarted.clip_editor.preferences == expected,
            "Local split layout/defaults did not survive restart",
        )?;
        drop(restarted);
        // Close a preview while a result is staged; reopen rebuilds released images.
        app.stage_clip(&ctx, Edit::Delete(vec![id]), None)?;
        drain(&mut app, &mut time)?;
        app.clip_editor.open = false;
        app.restore_clip_flip();
        app.clip_editor.job = None;
        app.clip_editor.staged = None;
        app.clip_editor.images = [None, None];
        app.clip_editor.image_keys = [None, None];
        app.execute_command(&ctx, "timeline-clips");
        for _ in 0..2 {
            plain(&mut app, &mut time);
        }
        check(
            app.clip_editor.images.iter().all(Option::is_some)
                && app.documents[0].snapshot() == edited,
            "Reopened split view failed or cancelled preview changed drawing",
        )?;
        app.stage_clip(&ctx, Edit::Delete(vec![id]), None)?;
        frame(
            &mut app,
            &mut time,
            egui::Modifiers::ALT,
            true,
            vec![key(true)],
        );
        app.request_close(app.active_tab.ok_or("Close tab missing")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        plain(&mut app, &mut time);
        check(
            app.documents.is_empty()
                && !app.clip_flip_active()
                && app.clip_editor.job.is_none()
                && app.clip_editor.staged.is_none()
                && app.clip_editor.images.iter().all(Option::is_none),
            "Document close left a held flip, staged result or frame cache",
        )?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
