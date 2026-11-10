use super::*;
use std::path::Path;
fn check(v: bool, s: &str) -> Result<(), String> {
    if v { Ok(()) } else { Err(s.into()) }
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::history_audit::run()?;
    let dir = root.join("history-audit-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut a = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        a.simple_view = true;
        a.create_canvas(CanvasSpec {
            width: 32,
            height: 32,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let file = dir.join("saved.drawer");
        a.save_to(0, &file)?;
        let saved = std::fs::read(&file).map_err(|e| e.to_string())?;
        a.panels.settings.brush_size = 2.;
        a.panels.settings.brush_opacity = [1.; 7];
        a.panels.settings.brush_hardness = [1.; 7];
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
        let drag =
            |a: &mut Drawer, t: &mut f64, from: [f64; 2], to: [f64; 2]| -> Result<(), String> {
                settle(a, t);
                let rect = a
                    .canvas_view
                    .viewport_rect()
                    .ok_or("History canvas viewport missing")?;
                let d = &a.documents[a.active.ok_or("History active document missing")?];
                let center = [rect.center().x as f64, rect.center().y as f64];
                let screen = |p| {
                    let p = d.view.document_to_screen(
                        p,
                        [d.width as f64, d.height as f64],
                        center,
                        ctx.pixels_per_point() as f64,
                    );
                    egui::pos2(p[0] as f32, p[1] as f32)
                };
                let from = screen(from);
                let to = screen(to);
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(from),
                        egui::Event::PointerButton {
                            pos: from,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                    ],
                );
                frame(a, t, vec![egui::Event::PointerMoved(to)]);
                frame(
                    a,
                    t,
                    vec![egui::Event::PointerButton {
                        pos: to,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    }],
                );
                settle(a, t);
                Ok(())
            };
        let key = |a: &mut Drawer, t: &mut f64, key: egui::Key| {
            for pressed in [true, false] {
                frame(
                    a,
                    t,
                    vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::CTRL,
                    }],
                );
            }
        };
        settle(&mut a, &mut time);
        let mut states = vec![a.documents[0].snapshot()];
        a.execute_command(&ctx, "Pen");
        drag(&mut a, &mut time, [4., 4.], [14., 10.])?;
        check(
            a.documents[0].history_counts() == (1, 0),
            "Mouse stroke was not a single history command",
        )?;
        states.push(a.documents[0].snapshot());
        a.execute_command(&ctx, "new-raster-layer");
        check(
            a.documents[0].layers.len() == 2 && a.documents[0].history_counts() == (2, 0),
            "Layer command history grouping",
        )?;
        states.push(a.documents[0].snapshot());
        let id = a.documents[0].id;
        a.panels
            .targets
            .insert(id, drawer_core::thumbnail::Target::Raster(0));
        a.execute_command(&ctx, "select-rectangle");
        drag(&mut a, &mut time, [0., 0.], [20., 20.])?;
        check(
            a.documents[0].history_counts() == (3, 0),
            "Selection drag history grouping",
        )?;
        states.push(a.documents[0].snapshot());
        a.execute_command(&ctx, "scale-rotate");
        let e = a
            .affine_transform
            .draft
            .as_mut()
            .ok_or("Transform command unavailable")?;
        e.pixels = true;
        e.filter = drawer_core::interpolation::Filter::Nearest;
        e.settings.translation = [2., 1.];
        a.preview_affine_transform(&ctx)?;
        a.wait_transform_preview(&ctx, &mut time)?;
        a.apply_affine_transform()?;
        check(
            a.documents[0].history_counts() == (4, 0),
            "Transform Apply history grouping",
        )?;
        states.push(a.documents[0].snapshot());
        for (n, state) in states[..4].iter().rev().enumerate() {
            key(&mut a, &mut time, egui::Key::Z);
            check(
                a.documents[0].snapshot() == *state,
                &format!("Ctrl+Z mixed-command boundary {n}: {:?}", a.status),
            )?;
        }
        check(
            !a.documents[0].dirty && a.documents[0].history_counts() == (0, 4),
            "Undo baseline saved revision",
        )?;
        for (n, state) in states[1..].iter().enumerate() {
            key(&mut a, &mut time, egui::Key::Y);
            check(
                a.documents[0].snapshot() == *state,
                &format!("Ctrl+Y mixed-command boundary {n}: {:?}", a.status),
            )?;
        }
        check(
            a.documents[0].history_counts() == (4, 0) && a.documents[0].dirty,
            "Redo endpoint",
        )?;
        let first = a.documents[0].snapshot();
        a.create_canvas(CanvasSpec {
            width: 32,
            height: 32,
            dpi: 150.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let second = a.documents[1].snapshot();
        key(&mut a, &mut time, egui::Key::Z);
        key(&mut a, &mut time, egui::Key::Y);
        check(
            a.documents[0].snapshot() == first
                && a.documents[1].snapshot() == second
                && a.documents[1].history_counts() == (0, 0),
            "Active document history leaked to another tab",
        )?;
        a.select_tab(0);
        settle(&mut a, &mut time);
        key(&mut a, &mut time, egui::Key::Z);
        let checkpoint = (
            a.documents[0].snapshot(),
            a.documents[0].history_counts(),
            a.documents[0].revision,
        );
        let mut invalid = a.documents[0].snapshot();
        invalid.width = 0;
        check(
            a.documents[0].commit_state(invalid).is_err()
                && a.documents[0].snapshot() == checkpoint.0
                && a.documents[0].history_counts() == checkpoint.1
                && a.documents[0].revision == checkpoint.2,
            "Failed command destroyed redo/source state",
        )?;
        a.execute_command(&ctx, "Pen");
        drag(&mut a, &mut time, [17., 15.], [18., 17.])?;
        check(
            a.documents[0].history_counts() == (4, 0),
            "New real stroke failed to discard redo branch",
        )?;
        let fork = a.documents[0].snapshot();
        key(&mut a, &mut time, egui::Key::Y);
        check(
            a.documents[0].snapshot() == fork && a.documents[1].snapshot() == second,
            "Redo after branch edited another state/document",
        )?;
        a.request_close(0);
        a.accept_close();
        settle(&mut a, &mut time);
        key(&mut a, &mut time, egui::Key::Z);
        check(
            a.documents.len() == 1
                && a.documents[0].snapshot() == second
                && a.documents[0].history_counts() == (0, 0),
            "Closing a document transferred its history",
        )?;
        check(
            std::fs::read(&file).map_err(|e| e.to_string())? == saved,
            "Undo/Redo overwrote the saved file",
        )?;
        drop(a);
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn mouse_layer_selection_transform_shortcut_history_tabs_failures_branch_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
