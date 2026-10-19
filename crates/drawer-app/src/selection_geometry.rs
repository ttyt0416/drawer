use super::*;
use drawer_core::{
    pointer_input::{Kind, Phase, Source},
    selection_edit::Combine,
};
fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    super::input_replay::frame(ctx, app, *time, events);
}
fn key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Default::default(),
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::selection_geometry::verify(root)?;
    for pen in [false, true] {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        let mut time = 0.0;
        app.create_canvas(CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.0,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.documents[0].view.angle = 90.0;
        app.documents[0].view.zoom = 1.5;
        step(&ctx, &mut app, &mut time, vec![]);
        step(&ctx, &mut app, &mut time, vec![]);
        let pointer = |app: &Drawer, p, phase, contact| {
            let mut e = super::precision::pointer(&ctx, app, p, phase);
            if pen {
                e.source = Source {
                    kind: Kind::Pen,
                    device: 96,
                    contact,
                };
                e.pressure = Some(0.5);
            }
            e
        };
        for op in Combine::ALL {
            app.workspaces.store.current.selection_edit.combine = op;
            let mut scene = app.documents[0].scene.clone();
            scene.selection_mask = Some(drawer_core::selection::Mask {
                runs: vec![drawer_core::selection::Run {
                    x: 0,
                    y: 4,
                    length: 16,
                    alpha: 128,
                }],
            });
            app.documents[0].edit_scene(scene)?;
            app.execute_command(&ctx, "select-ellipse");
            step(&ctx, &mut app, &mut time, vec![]);
            let before = app.documents[0].snapshot();
            let history = app.documents[0].history_counts();
            let e = pointer(&app, [2.0, 2.0], Phase::Down, 1);
            app.drawing.virtual_events(vec![e], pen);
            step(&ctx, &mut app, &mut time, vec![]);
            let e = pointer(&app, [10.0, 10.0], Phase::Move, 1);
            app.drawing.virtual_events(vec![e], pen);
            step(&ctx, &mut app, &mut time, vec![]);
            if app.documents[0].snapshot() != before
                || !app.drawing.pending()
                || app
                    .drawing
                    .selection_preview(app.documents[0].id)
                    .is_none_or(|p| p.len() != 97)
            {
                return Err("Ellipse draft changed source or lost outline".into());
            }
            let e = pointer(&app, [10.0, 10.0], Phase::Up, 1);
            app.drawing.virtual_events(vec![e], pen);
            step(&ctx, &mut app, &mut time, vec![]);
            let expected = match op {
                Combine::Replace | Combine::Add => 255,
                Combine::Subtract => 0,
                Combine::Intersect => 128,
            };
            if app.drawing.pending()
                || app.documents[0].scene.selection_coverage([6.5, 4.5]) != expected
                || app.documents[0].history_counts().0 != history.0 + 1
            {
                return Err(format!("Ellipse production {pen}/{op:?}: {:?}", app.status));
            }
            let after = app.documents[0].snapshot();
            if after.layers != before.layers
                || !app.documents[0].undo()
                || app.documents[0].snapshot() != before
                || !app.documents[0].redo()
                || app.documents[0].snapshot() != after
            {
                return Err("Ellipse pixels/Undo/Redo".into());
            }
            app.execute_command(&ctx, "select-polygon");
            step(&ctx, &mut app, &mut time, vec![]);
            let before = app.documents[0].snapshot();
            let history = app.documents[0].history_counts();
            let vertices = [[1.5, 1.5], [12.5, 1.5], [12.5, 12.5], [1.5, 12.5]];
            for (i, p) in vertices.into_iter().chain([[14.0, 14.0]]).enumerate() {
                for phase in [Phase::Down, Phase::Up] {
                    let e = pointer(&app, p, phase, 10 + i as u64);
                    app.drawing.virtual_events(vec![e], pen);
                    step(&ctx, &mut app, &mut time, vec![]);
                }
                if !app.drawing.pending() || app.documents[0].snapshot() != before {
                    return Err(format!(
                        "Polygon click {pen}/{i} committed or lost draft: {:?}",
                        app.status
                    ));
                }
            }
            step(
                &ctx,
                &mut app,
                &mut time,
                vec![key(egui::Key::Backspace, true)],
            );
            step(
                &ctx,
                &mut app,
                &mut time,
                vec![key(egui::Key::Backspace, false)],
            );
            let expected = drawer_core::selection_geometry::prepare(
                &app.documents[0],
                drawer_core::selection_geometry::Kind::Polygon,
                &vertices,
                op,
            )?;
            let changed = expected != before.scene;
            step(&ctx, &mut app, &mut time, vec![key(egui::Key::Enter, true)]);
            step(
                &ctx,
                &mut app,
                &mut time,
                vec![key(egui::Key::Enter, false)],
            );
            if app.drawing.pending()
                || app.documents[0].scene.selection_mask != expected.selection_mask
                || app.documents[0].snapshot().layers != before.layers
                || app.documents[0].history_counts().0 != history.0 + usize::from(changed)
            {
                return Err(format!("Polygon Enter {pen}/{op:?}: {:?}", app.status));
            }
            let path = root.join(format!("shape-{pen}.drawer"));
            drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
            if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                != app.documents[0].snapshot()
            {
                return Err("Shape production native reopen".into());
            }
        }
        for cancel in 0..6 {
            app.execute_command(&ctx, "select-polygon");
            step(&ctx, &mut app, &mut time, vec![]);
            let before = app.documents[0].snapshot();
            for phase in [Phase::Down, Phase::Up] {
                let e = pointer(&app, [4.0, 4.0], phase, 90);
                app.drawing.virtual_events(vec![e], pen);
                step(&ctx, &mut app, &mut time, vec![]);
            }
            let events = match cancel {
                0 => vec![key(egui::Key::Escape, true)],
                1 => vec![egui::Event::WindowFocused(false)],
                2 => {
                    app.documents[0].view.zoom += 0.1;
                    vec![]
                }
                3 => {
                    app.drawing.mode = super::precision::Mode::Paint;
                    vec![]
                }
                4 => {
                    app.workspaces.store.current.selection_edit.combine =
                        if app.workspaces.store.current.selection_edit.combine == Combine::Add {
                            Combine::Subtract
                        } else {
                            Combine::Add
                        };
                    vec![]
                }
                _ => {
                    app.pressure.open = true;
                    vec![]
                }
            };
            step(&ctx, &mut app, &mut time, events);
            if app.drawing.pending() || app.documents[0].snapshot() != before {
                return Err(format!("Shape cancel {cancel}"));
            }
            app.pressure.open = false;
            step(
                &ctx,
                &mut app,
                &mut time,
                vec![
                    egui::Event::WindowFocused(true),
                    key(egui::Key::Escape, false),
                ],
            );
        }
        let path = root.join(format!("shape-settings-{pen}.json"));
        app.workspaces
            .store
            .current
            .export(&path)
            .map_err(|e| e.to_string())?;
        if drawer_core::workspaces::Snapshot::read(&path)
            .map_err(|e| e.to_string())?
            .selection_edit
            .combine
            != app.workspaces.store.current.selection_edit.combine
        {
            return Err("Shape operation settings reopen".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_ellipse_polygon_composition_pen_mouse_cancel_history() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
