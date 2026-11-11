use super::*;
use std::path::Path;
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    drawer_core::tiled_raster::verify::run(root)?;
    let dir = root.join("tiled-raster-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.create_canvas_color(
            CanvasSpec {
                width: 32768,
                height: 32768,
                dpi: 300.,
                background: Background::Transparent,
            },
            drawer_core::color_storage::Config {
                depth: drawer_core::color_precision::Depth::Eight,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        app.panels.settings.brush_size = 3.;
        app.panels.settings.brush_opacity = [1.; 7];
        app.panels.settings.brush_hardness = [1.; 7];
        let mut time = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, app, *t, events);
        };
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let rect = app
            .canvas_view
            .viewport_rect()
            .ok_or("Sparse canvas viewport missing")?;
        let center = [rect.center().x as f64, rect.center().y as f64];
        let doc = &app.documents[0];
        let map = |p| {
            let p = doc.view.document_to_screen(
                p,
                [doc.width as f64, doc.height as f64],
                center,
                ctx.pixels_per_point() as f64,
            );
            egui::pos2(p[0] as f32, p[1] as f32)
        };
        let from = map([16384.5, 16384.5]);
        let to = map([16392.5, 16384.5]);
        for (pos, down) in [(from, true), (to, true), (to, false)] {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if pos == from || !down {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: down,
                    modifiers: Default::default(),
                });
            }
            frame(&mut app, &mut time, events);
        }
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        check(
            app.documents[0].layers[0]
                .sample(32768, 32768, 16384, 16384)
                .rgba8()[3]
                > 0
                && app.documents[0].resident_bytes() < 2 * 1024 * 1024,
            "Mouse did not paint sparse canvas within tile/cache budget",
        )?;
        let picture = app.documents[0].snapshot();
        app.execute_command(&ctx, "undo");
        check(
            app.documents[0].layers[0]
                .sample(32768, 32768, 16384, 16384)
                .rgba8()
                == [0; 4],
            "Sparse mouse Undo",
        )?;
        app.execute_command(&ctx, "redo");
        check(app.documents[0].snapshot() == picture, "Sparse mouse Redo")?;
        let file = dir.join("large.drawer");
        app.save_to(0, &file)?;
        let reopened =
            drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == app.documents[0].snapshot()
                && reopened.allocation_bytes() < 2 * 1024 * 1024,
            "Sparse mouse native save/reopen",
        )?;
        Ok(())
    })();
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn sparse32k_mouse_drawing_undo_redo_and_native_reopen() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
