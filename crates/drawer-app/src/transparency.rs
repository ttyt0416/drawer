use super::*;
use drawer_core::{
    bucket::{Mode, Settings},
    pointer_input::Phase,
};
pub(crate) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::transparency_verify::verify(root)?;
    for pen in [false, true] {
        for wand in [false, true] {
            for mode in [Mode::Strict, Mode::CloseGaps] {
                for gap in [0usize, 1, 3] {
                    let ctx = egui::Context::default();
                    let mut app = Drawer::new(&ctx, None);
                    app.simple_view = true;
                    app.create_canvas(CanvasSpec {
                        width: 16,
                        height: 16,
                        dpi: 300.0,
                        background: Background::Transparent,
                    })
                    .map_err(|e| e.to_string())?;
                    for v in 4..12 {
                        for (x, y) in [(v, 4), (v, 11), (4, v), (11, v)] {
                            app.documents[0].layers[0].pixels[y * 16 + x] = [0, 0, 0, 255];
                        }
                    }
                    for x in 7 - gap / 2..7 - gap / 2 + gap {
                        app.documents[0].layers[0].pixels[4 * 16 + x] = [0; 4];
                    }
                    app.documents[0].view.angle = 90.0;
                    app.documents[0].view.zoom = 4.0;
                    let settings = Settings {
                        mode,
                        gap: 1,
                        antialias: false,
                        ..Default::default()
                    };
                    app.execute_command(&ctx, if wand { "wand-tool" } else { "bucket-tool" });
                    app.choose_color([200, 40, 20, 255]);
                    let slot = BrushTip::ALL
                        .iter()
                        .position(|t| *t == app.selected)
                        .unwrap();
                    app.workspaces.store.current.input.fills[slot] = settings;
                    app.workspaces.store.current.selection_edit.region = settings;
                    let mut time = 0.0;
                    let step = |app: &mut Drawer, time: &mut f64| {
                        *time += 0.1;
                        super::input_replay::frame(&ctx, app, *time, vec![]);
                    };
                    step(&mut app, &mut time);
                    step(&mut app, &mut time);
                    let before = app.documents[0].snapshot();
                    let history = app.documents[0].history_counts();
                    for phase in [Phase::Down, Phase::Up] {
                        let mut e = super::precision::pointer(&ctx, &app, [7.5, 7.5], phase);
                        if pen {
                            e.source = drawer_core::pointer_input::Source {
                                kind: drawer_core::pointer_input::Kind::Pen,
                                device: 109,
                                contact: 1,
                            };
                            e.pressure = Some(0.5);
                        }
                        app.drawing.virtual_events(vec![e], pen);
                        step(&mut app, &mut time);
                        if phase == Phase::Down
                            && (!app.drawing.pending()
                                || app.documents[0].snapshot() != before
                                || app.documents[0].history_counts() != history)
                        {
                            return Err("Transparency preview edited page".into());
                        }
                    }
                    let spill = gap > 0 && (mode == Mode::Strict || gap > 1);
                    let doc = &mut app.documents[0];
                    if doc.history_counts().0 != history.0 + 1 || app.drawing.pending() {
                        return Err(format!("Transparency commit: {:?}", app.status));
                    }
                    if wand {
                        if doc.layers != before.layers
                            || (doc.scene.selection_coverage([0.5, 0.5]) > 0) != spill
                            || doc.scene.selection_coverage([7.5, 7.5]) != 255
                        {
                            return Err("Wand strict/loose production result".into());
                        }
                    } else if (doc.layers[0].pixel(16, 16, 0, 0)[3] > 0) != spill
                        || doc.layers[0].pixel(16, 16, 7, 7) != [200, 40, 20, 255]
                    {
                        return Err("Fill strict/loose production result".into());
                    }
                    if !doc.undo() || doc.snapshot() != before || !doc.redo() {
                        return Err("Transparency single Undo/Redo".into());
                    }
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn shared_transparency_production_fill_wand_preview_commit_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
