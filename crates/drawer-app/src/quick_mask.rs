use super::*;
use drawer_core::scene::Point;
impl Drawer {
    pub(super) fn toggle_quick_mask(&mut self) {
        self.drawing.quick_mask = !self.drawing.quick_mask;
        if self.drawing.quick_mask {
            self.restore_locked_tool();
            self.drawing.quick_previous = Some(self.drawing.mode);
            self.drawing.mode = super::precision::Mode::SelectionBrush;
            self.drawing.connect_line = false;
        } else {
            self.drawing.mode = self.drawing.quick_previous.take().unwrap_or_default();
        }
    }
}
fn color(alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(255, 32, 32, (alpha as u16).div_ceil(2) as u8)
}
pub(super) fn paint(painter: &egui::Painter, doc: &Document, size: Point, center: Point, ppp: f64) {
    let quad = |a: Point, b: Point, alpha: u8, page: bool| {
        let points = [a, [b[0], a[1]], b, [a[0], b[1]]].map(|p| {
            let p = if page {
                doc.scene.selection_point(p)
            } else {
                p
            };
            let p = doc.view.document_to_screen(p, size, center, ppp);
            egui::pos2(p[0] as f32, p[1] as f32)
        });
        painter.add(egui::Shape::convex_polygon(
            points.to_vec(),
            color(alpha),
            egui::Stroke::NONE,
        ));
    };
    if let Some(mask) = &doc.scene.selection_mask {
        for r in &mask.runs {
            quad(
                [r.x as f64, r.y as f64],
                [r.x as f64 + r.length as f64, r.y as f64 + 1.0],
                r.alpha,
                true,
            );
        }
    } else if let Some(r) = doc.scene.selection {
        quad(r.min, r.max, 255, true);
    } else {
        quad([0.0; 2], size, 255, false);
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::{
        pointer_input::{Kind, Phase, Source},
        selection::{Mask, Run},
    };
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
        app.documents[0].scene.selection_mask = Some(Mask::default());
        app.documents[0].view.angle = 90.0;
        app.documents[0].view.zoom = 1.5;
        let s = &mut app.workspaces.store.current.selection_edit;
        s.diameter = 4.0;
        s.hardness = 1.0;
        s.strength = 128.0 / 255.0;
        s.erase = false;
        let step = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            super::input_replay::frame(&ctx, app, *time, events);
        };
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let oldmode = app.drawing.mode;
        app.execute_command(&ctx, "quick-mask");
        if app.documents[0].snapshot() != before || app.documents[0].history_counts() != history {
            return Err("Quick mask toggle edited document".into());
        }
        for phase in [Phase::Down, Phase::Up] {
            let mut e = super::precision::pointer(&ctx, &app, [8.0; 2], phase);
            if pen {
                e.source = Source {
                    kind: Kind::Pen,
                    device: 98,
                    contact: 1,
                };
                e.pressure = Some(1.0);
            }
            app.drawing.virtual_events(vec![e], pen);
            step(&mut app, &mut time, vec![]);
            if phase == Phase::Down && app.documents[0].snapshot() != before {
                return Err("Quick mask draft changed source".into());
            }
        }
        if app.documents[0].scene.selection_coverage([7.5; 2]) != 128
            || app.documents[0].snapshot().layers != before.layers
        {
            return Err(format!("Quick mask painting {pen}: {:?}", app.status));
        }
        app.execute_command(&ctx, "selection-erase-tool");
        for phase in [Phase::Down, Phase::Up] {
            let mut e = super::precision::pointer(&ctx, &app, [8.0; 2], phase);
            if pen {
                e.source = Source {
                    kind: Kind::Pen,
                    device: 98,
                    contact: 2,
                };
                e.pressure = Some(1.0);
            }
            app.drawing.virtual_events(vec![e], pen);
            step(&mut app, &mut time, vec![]);
        }
        if app.documents[0].scene.selection_coverage([7.5; 2]) != 64
            || app.documents[0].snapshot().layers != before.layers
        {
            return Err("Quick mask erasure".into());
        }
        app.documents[0].undo();
        let selected = app.documents[0].snapshot();
        app.selection_panel.name = "Soft mask layer".into();
        app.execute_command(&ctx, "selection-save");
        let id = app.documents[0].scene.selections.saved[0].id;
        drawer_core::selection_tools::clear(&mut app.documents[0])?;
        drawer_core::selection_tools::load_named(&mut app.documents[0], id)?;
        if app.documents[0].scene.selection_mask != selected.scene.selection_mask {
            return Err("Quick mask layer lost soft alpha".into());
        }
        let selected = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.execute_command(&ctx, "quick-mask");
        step(&mut app, &mut time, vec![]);
        if app.drawing.quick_mask
            || app.drawing.mode != oldmode
            || app.documents[0].snapshot() != selected
            || app.documents[0].history_counts() != history
        {
            return Err("Exiting quick mask changed pixels/selection/tool".into());
        }
        let path = root.join(format!("quick-mask-{pen}.drawer"));
        drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
        let d =
            drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        if d.snapshot() != selected
            || d.scene.selections.saved[0].shape.mask != selected.scene.selection_mask
        {
            return Err("Quick mask layer native reopen".into());
        }
        let mut maskdoc = d;
        maskdoc.scene.selection_mask = Some(Mask {
            runs: vec![Run {
                x: 1,
                y: 2,
                length: 2,
                alpha: 128,
            }],
        });
        maskdoc.view.angle = 90.0;
        let overlay = ctx.run_ui(Default::default(), |ui| {
            paint(ui.painter(), &maskdoc, [16.0; 2], [50.0; 2], 1.0)
        });
        let paths = overlay
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Path(p) = &s.shape {
                    Some(p)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if !paths
            .iter()
            .any(|p| p.fill == color(128) && p.fill.a() == 64 && p.points.len() == 4)
        {
            return Err("Quick mask overlay alpha/path".into());
        }
        let expected = [[1.0, 2.0], [3.0, 2.0], [3.0, 3.0], [1.0, 3.0]].map(|p| {
            let p = maskdoc.scene.selection_point(p);
            let p = maskdoc
                .view
                .document_to_screen(p, [16.0; 2], [50.0; 2], 1.0);
            egui::pos2(p[0] as f32, p[1] as f32)
        });
        if !paths
            .iter()
            .any(|p| p.fill == color(128) && p.points.as_slice() == expected)
        {
            return Err("Quick mask rotated overlay alignment".into());
        }
        let mut plain = maskdoc.fork_page()?;
        plain.scene.selection_mask = None;
        plain.scene.selection = None;
        plain.scene.selection_translation = [0.0; 2];
        let settings = drawer_core::selection_edit::Settings {
            diameter: 4.0,
            hardness: 1.0,
            strength: 128.0 / 255.0,
            erase: false,
            ..Default::default()
        };
        let mut brush = drawer_core::selection_edit::Brush::begin_mask(&plain, settings)?;
        brush.sample(&plain, [8.0; 2], 1.0)?;
        if brush.preview(&plain)? != plain.scene {
            return Err("Adding quick mask to unrestricted selection changed it".into());
        }
        let mut brush = drawer_core::selection_edit::Brush::begin_mask(
            &plain,
            drawer_core::selection_edit::Settings {
                erase: true,
                ..settings
            },
        )?;
        brush.sample(&plain, [8.0; 2], 1.0)?;
        if brush.preview(&plain)?.selection_coverage([7.5; 2]) != 127
            || brush.preview(&plain)?.selection_coverage([0.5; 2]) != 255
        {
            return Err("Erasing unrestricted quick mask".into());
        }
        let mut newapp = Drawer::new(&ctx, None);
        newapp.add_document(maskdoc);
        if newapp.drawing.quick_mask {
            return Err("Quick mask view flag persisted".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_quick_mask_soft_layers_overlay_exit_reopen() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
