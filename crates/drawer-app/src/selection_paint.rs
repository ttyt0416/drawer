use super::*;
use drawer_core::pointer_input::Phase;
fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64) {
    *time += 0.1;
    super::input_replay::frame(ctx, app, *time, vec![]);
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::selection_paint::verify(root)?;
    for ink_alpha in [0, 128, 255] {
        for coverage in [0, 128, 255] {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            let mut time = 0.0;
            app.create_canvas(CanvasSpec {
                width: 24,
                height: 24,
                dpi: 300.0,
                background: Background::Transparent,
            })
            .map_err(|e| e.to_string())?;
            app.documents[0].layers[0].pixels.fill([20, 80, 160, 128]);
            app.set_tool(BrushTip::PixelPen);
            app.workspaces.store.current.input.profiles[6]
                .pixel
                .overwrite = true;
            app.workspaces.store.current.input.profiles[6]
                .pixel
                .pressure = false;
            app.workspaces.store.current.input.profiles[6].minimum_opacity = 1.0;
            app.panels.settings.brush_size = 8.0;
            app.panels.settings.brush_hardness[6] = 1.0;
            app.panels.settings.brush_opacity[6] = 1.0;
            app.choose_color([220, 40, 20, ink_alpha]);
            app.documents[0].scene.selection_mask = Some(drawer_core::selection::Mask {
                runs: if coverage == 0 {
                    vec![]
                } else {
                    (0..24)
                        .map(|y| drawer_core::selection::Run {
                            x: 0,
                            y,
                            length: 24,
                            alpha: coverage,
                        })
                        .collect()
                },
            });
            drawer_core::files::save(
                &mut app.documents[0],
                &root.join(format!("selected-mouse-{ink_alpha}-{coverage}.drawer")),
            )
            .map_err(|e| e.to_string())?;
            let before = app.documents[0].snapshot();
            step(&ctx, &mut app, &mut time);
            step(&ctx, &mut app, &mut time);
            for phase in [Phase::Down, Phase::Up] {
                let event = super::precision::pointer(&ctx, &app, [12.5, 12.5], phase);
                app.drawing.virtual_events(vec![event], false);
                step(&ctx, &mut app, &mut time);
            }
            let want = match (ink_alpha, coverage) {
                (_, 0) => [20, 80, 160, 128],
                (0, 128) => [20, 80, 160, 64],
                (128, 128) => [120, 60, 90, 128],
                (255, 128) => [154, 53, 67, 192],
                _ => [220, 40, 20, ink_alpha],
            };
            let actual = app.documents[0].layers[0].pixel(24, 24, 12, 12);
            if actual != want || app.drawing.pending() || !app.workspaces.input_idle() {
                return Err(format!(
                    "Actual masked pixel overwrite {coverage}: {actual:?}/{want:?}, status {:?}",
                    app.status
                ));
            }
            if coverage == 0 {
                if app.documents[0].snapshot() != before
                    || app.documents[0].dirty
                    || app.documents[0].history_counts() != (0, 0)
                {
                    return Err("Empty selection mouse stroke changed document".into());
                }
            } else {
                let after = app.documents[0].snapshot();
                if app.documents[0].history_counts() != (1, 0)
                    || !app.documents[0].undo()
                    || app.documents[0].snapshot() != before
                    || !app.documents[0].redo()
                    || app.documents[0].snapshot() != after
                {
                    return Err("Selected mouse stroke Undo/Redo mismatch".into());
                }
                if coverage == 128 {
                    let path = root.join("selected-mouse-final.drawer");
                    drawer_core::files::save(&mut app.documents[0], &path)
                        .map_err(|e| e.to_string())?;
                    if drawer_core::files::open(&path, 65536)
                        .map_err(|e| e.to_string())?
                        .snapshot()
                        != after
                    {
                        return Err("Mouse metadata/pixels/mask reopen mismatch".into());
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
    fn production_mouse_zero_half_full_selection_and_native() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
