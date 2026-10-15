use super::*;
use drawer_core::brush_cursor::{Appearance, Mark, Mode, Settings, Shape, Style, Trace};
#[derive(Default)]
pub(super) struct Cursors {
    pub open: bool,
    draft: Option<Settings>,
    tip: usize,
}
#[derive(Clone, Copy)]
pub(super) struct Projection {
    pub view: drawer_core::view::View,
    pub size: [f64; 2],
    pub center: [f64; 2],
    pub ppp: f64,
}
pub(super) fn paint(
    painter: &egui::Painter,
    trace: &Trace,
    appearance: Appearance,
    shape: Shape,
    projection: Projection,
    icon: egui::TextureId,
) -> Vec<egui::Pos2> {
    let Projection {
        view,
        size,
        center,
        ppp,
    } = projection;
    let mut corrected_outline = Vec::new();
    if appearance.style == Style::System {
        return corrected_outline;
    }
    for (kind, point) in trace.marks(appearance) {
        let projected = view.document_to_screen(point, size, center, ppp);
        let pos = egui::pos2(projected[0] as f32, projected[1] as f32);
        let color = match kind {
            Mark::Raw => egui::Color32::WHITE,
            Mark::Corrected => egui::Color32::from_rgb(60, 220, 245),
            Mark::Prediction => egui::Color32::from_rgb(255, 160, 50),
        };
        if kind == Mark::Prediction {
            let points = [
                pos + egui::vec2(0.0, -5.0),
                pos + egui::vec2(5.0, 0.0),
                pos + egui::vec2(0.0, 5.0),
                pos + egui::vec2(-5.0, 0.0),
                pos + egui::vec2(0.0, -5.0),
            ];
            for n in [0, 2] {
                painter.line_segment([points[n], points[n + 1]], egui::Stroke::new(1.5, color));
            }
            continue;
        }
        if appearance.style == Style::Outline
            && !(appearance.mode == Mode::Both && kind == Mark::Raw)
        {
            let outline = shape
                .displayed_outline(view, point, size, center, ppp, appearance.minimum_pixels)
                .into_iter()
                .map(|p| egui::pos2(p[0] as f32, p[1] as f32))
                .collect::<Vec<_>>();
            painter.add(egui::Shape::closed_line(
                outline.clone(),
                egui::Stroke::new(3.0, egui::Color32::BLACK),
            ));
            painter.add(egui::Shape::closed_line(
                outline.clone(),
                egui::Stroke::new(1.0, color),
            ));
            corrected_outline = outline;
        }
        if matches!(appearance.style, Style::Outline | Style::Icon) {
            let axes = shape.axes(view);
            let at = pos + egui::vec2(18.0, 18.0);
            let mut mesh = egui::Mesh::with_texture(icon);
            for (p, uv) in [
                ([-8.0, -8.0], [0.0, 0.0]),
                ([8.0, -8.0], [1.0, 0.0]),
                ([8.0, 8.0], [1.0, 1.0]),
                ([-8.0, 8.0], [0.0, 1.0]),
            ] {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: at
                        + egui::vec2(
                            (axes[0][0] * p[0] + axes[1][0] * p[1]) as f32,
                            (axes[0][1] * p[0] + axes[1][1] * p[1]) as f32,
                        ),
                    uv: egui::pos2(uv[0], uv[1]),
                    color,
                });
            }
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            painter.add(egui::Shape::mesh(mesh));
        }
        for delta in [egui::vec2(7.0, 0.0), egui::vec2(0.0, 7.0)] {
            painter.line_segment(
                [pos - delta, pos + delta],
                egui::Stroke::new(3.0, egui::Color32::BLACK),
            );
            painter.line_segment([pos - delta, pos + delta], egui::Stroke::new(1.0, color));
        }
    }
    corrected_outline
}
impl Drawer {
    pub(super) fn cursor_settings(&mut self, ctx: &egui::Context) {
        if !self.cursors.open {
            return;
        }
        let mut s = self
            .cursors
            .draft
            .unwrap_or(self.workspaces.store.current.cursors);
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Cursor styles").open(&mut open).show(ctx,|ui|{
            egui::ComboBox::from_id_salt("cursor-tip").selected_text(BrushTip::ALL[self.cursors.tip].name()).show_ui(ui,|ui|{for(n,t)in BrushTip::ALL.iter().enumerate(){ui.selectable_value(&mut self.cursors.tip,n,t.name());}});
            let a=&mut s.tools[self.cursors.tip];ui.horizontal_wrapped(|ui|{for style in Style::ALL{ui.selectable_value(&mut a.style,style,format!("{style:?}"));}});
            ui.add_enabled_ui(a.style!=Style::System,|ui|{ui.horizontal(|ui|{for mode in Mode::ALL{ui.selectable_value(&mut a.mode,mode,format!("{mode:?}"));}});ui.add(egui::Slider::new(&mut a.minimum_pixels,0.0..=32.0).text("Minimum outline (screen pixels)"));ui.checkbox(&mut a.prediction,"Show prediction preview (orange; never committed)");});
            if a.style==Style::System{a.mode=Mode::Raw;a.prediction=false;}
            ui.label("Raw: current input, white. Corrected: committed filter position, cyan. Both keeps the raw cross and corrected outline separate. Stabilization is set in Tool input behavior; these display choices never change the stroke.");
            ui.horizontal(|ui|{apply=ui.button("Apply cursor styles").clicked();cancel=ui.button("Cancel cursor styles").clicked();if ui.button("Reset draft").clicked(){s=Default::default();}});
            if ui.add_enabled(self.storage.is_some(),egui::Button::new("Load before-apply backup")).clicked(){match drawer_core::workspaces::Snapshot::read(&self.storage.as_ref().unwrap().root().join("settings/workspace-before-apply.json")){Ok(p)=>s=p.cursors,Err(e)=>self.status=Some(e.to_string())}}
        });
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cursors.open = false;
            self.cursors.draft = None;
        } else if apply {
            let mut current = self.capture_workspace();
            current.cursors = s;
            self.stage_workspace(current, self.workspaces.store.active.clone());
            match self.apply_workspace(
                drawer_core::workspaces::Scope {
                    other: true,
                    ..Default::default()
                },
                None,
            ) {
                Ok(()) => {
                    self.cursors.open = false;
                    self.cursors.draft = None;
                }
                Err(e) => {
                    self.status = Some(e);
                    self.workspaces.staged = None;
                    self.cursors.draft = Some(s);
                }
            }
        } else {
            self.cursors.draft = Some(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, frame, tiny_spec};
    #[test]
    fn actual_popup_high_speed_corner_release_outside_has_identical_pixels_for_three_cursor_modes()
    {
        let mut results = Vec::new();
        for mode in Mode::ALL {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.create_canvas(tiny_spec()).unwrap();
            let before = app.documents[0].snapshot();
            app.workspaces.store.current.scratchpad.visible = true;
            app.workspaces.store.current.input.smoothing[0] = 0.6;
            app.workspaces.store.current.cursors.tools[0].mode = mode;
            app.workspaces.store.current.cursors.tools[0].prediction = true;
            frame(&ctx, &mut app, vec![]);
            frame(&ctx, &mut app, vec![]);
            let rect = app.workspaces.pad_rect.unwrap();
            let at = |p: [f32; 2]| rect.min + egui::vec2(p[0], p[1]) * 3.0;
            let start = at([8.0, 8.0]);
            frame(
                &ctx,
                &mut app,
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
            );
            frame(
                &ctx,
                &mut app,
                vec![
                    egui::Event::PointerMoved(at([70.0, 8.0])),
                    egui::Event::PointerMoved(at([70.0, 80.0])),
                    egui::Event::PointerMoved(at([15.0, 80.0])),
                ],
            );
            let end = at([-5.0, 20.0]);
            frame(
                &ctx,
                &mut app,
                vec![
                    egui::Event::PointerMoved(end),
                    egui::Event::PointerButton {
                        pos: end,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ],
            );
            assert!(!app.workspaces.painting());
            assert!(app.workspaces.input_idle());
            assert!(
                app.workspaces
                    .store
                    .current
                    .scratchpad
                    .pixels
                    .iter()
                    .any(|p| p[3] > 0)
            );
            results.push(app.workspaces.store.current.scratchpad.pixels.clone());
            assert_eq!(app.documents[0].snapshot(), before);
            assert_eq!(app.documents[0].history_counts(), (0, 0));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[1], results[2]);
    }
    #[test]
    fn live_cursor_filtered_prediction_minimum_styles_mirror_no_input_and_restart() {
        let ctx = egui::Context::default();
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(CanvasSpec {
            width: 128,
            height: 128,
            ..tiny_spec()
        })
        .unwrap();
        app.documents[0].view.angle = 37.0;
        app.documents[0].view.flipped = true;
        app.workspaces.store.current.input.smoothing[0] = 0.6;
        app.workspaces.store.current.cursors.tools[0] = Appearance {
            mode: Mode::Both,
            prediction: true,
            minimum_pixels: 4.0,
            ..Default::default()
        };
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let before = app.documents[0].snapshot();
        let rect = app.canvas_view.viewport.unwrap();
        let start = rect.center();
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        let moved = start + egui::vec2(60.0, 30.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(moved)]);
        assert_ne!(
            app.canvas_view.cursor_trace.raw,
            app.canvas_view.cursor_trace.corrected
        );
        assert!(app.canvas_view.cursor_trace.prediction.is_some());
        assert_eq!(
            app.canvas_view
                .cursor_trace
                .marks(app.workspaces.store.current.cursors.tools[0])
                .len(),
            3
        );
        let filtered = app.canvas_view.cursor_trace.corrected;
        app.workspaces.store.current.behavior.brush_cursor = false;
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.canvas_view.cursor_trace.corrected, filtered);
        assert!(app.canvas_view.brush_outline.is_empty());
        app.workspaces.store.current.behavior.brush_cursor = true;
        frame(&ctx, &mut app, vec![egui::Event::PointerGone]);
        assert!(app.canvas_view.cursor_trace.raw.is_none());
        assert!(app.canvas_view.brush_outline.is_empty());
        frame(&ctx, &mut app, vec![egui::Event::WindowFocused(false)]);
        assert!(app.canvas_view.cursor_trace.corrected.is_none());
        frame(&ctx, &mut app, vec![egui::Event::WindowFocused(true)]);
        for style in Style::ALL {
            app.workspaces.store.current.cursors.tools[0].style = style;
            if style == Style::System {
                app.workspaces.store.current.cursors.tools[0].mode = Mode::Raw;
                app.workspaces.store.current.cursors.tools[0].prediction = false;
            }
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(start)]);
            if style == Style::System {
                assert!(app.canvas_view.cursor_rect.is_none());
            }
        }
        let saved = app.capture_workspace();
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.workspaces.store.current.cursors, saved.cursors);
        assert_eq!(
            restarted.workspaces.store.current.input.smoothing,
            saved.input.smoothing
        );
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
}
