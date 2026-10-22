use super::*;
use drawer_core::shapes::{self as core, Constraint, Curve, Input, Kind, Paint, Settings, Target};
#[derive(Clone, Copy)]
pub(super) struct Capture {
    pub device_pressure: bool,
    pub source_target: drawer_core::thumbnail::Target,
    pub settings: Settings,
    pub snapping: drawer_core::snapping::Settings,
    pub target: Target,
    pub budget: usize,
}
impl drawing::Drawing {
    pub(super) fn shape_changed(&self, s: Option<Capture>) -> bool {
        s.is_some_and(|s| {
            self.shape_settings != s.settings
                || self.shape_target != Some(s.target)
                || self.snap_settings != s.snapping
        })
    }
}
impl Drawer {
    pub(super) fn begin_vector_shape(&mut self, curved: bool) {
        self.finish_stroke();
        self.restore_locked_tool();
        let s = &mut self.workspaces.store.current.shapes;
        s.kind = if curved { Kind::Bezier } else { Kind::Polyline };
        s.curve = if curved {
            Curve::CubicBezier
        } else {
            Curve::StraightLine
        };
        s.input = Input::ByClick;
        s.paint = Paint::CreateLine;
        s.constraint = Constraint::Free;
        s.closed = false;
        s.corner = 0.;
        s.center = false;
        s.line = self.references.session.workspace.color;
        self.workspaces.store.current.shapes_vector = true;
        self.drawing.mode = super::precision::Mode::ShapeClick;
        self.drawing.connect_line = false;
    }
    pub(super) fn shapes_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Shape creation",|ui|{
            let old=self.workspaces.store.current.shapes;let old_vector=self.workspaces.store.current.shapes_vector;
            let s=&mut self.workspaces.store.current.shapes;
            egui::ComboBox::from_id_salt("shape-kind").selected_text(format!("{:?}",s.kind)).show_ui(ui,|ui|{for k in Kind::ALL{ui.selectable_value(&mut s.kind,k,format!("{k:?}"));}});
            if !s.kind.is_box(){egui::ComboBox::from_id_salt("shape-curve").selected_text(s.curve.label()).show_ui(ui,|ui|{for c in Curve::ALL{ui.selectable_value(&mut s.curve,c,c.label());}});}
            ui.horizontal(|ui|{ui.radio_value(&mut s.input,Input::ByClick,"By click");ui.radio_value(&mut s.input,Input::ByDrag,"By drag");});
            egui::ComboBox::from_id_salt("shape-paint").selected_text(s.paint.label()).show_ui(ui,|ui|{for p in Paint::ALL{ui.selectable_value(&mut s.paint,p,p.label());}});
            egui::ComboBox::from_id_salt("shape-constraint").selected_text(s.constraint.label()).show_ui(ui,|ui|{for c in Constraint::ALL{ui.selectable_value(&mut s.constraint,c,c.label());}});
            if s.constraint==Constraint::SpecifiedRatio{ui.add(egui::Slider::new(&mut s.ratio,0.01..=100.0).logarithmic(true).text("Width / height"));}
            if s.constraint==Constraint::SpecifyLength{ui.add(egui::DragValue::new(&mut s.length).range(0.1..=1e6).prefix("Length: "));}
            ui.add(egui::Slider::new(&mut s.width,0.1..=512.0).logarithmic(true).text("Line width in page units"));
            ui.add(egui::Slider::new(&mut s.corner,0.0..=1024.0).text("Rounded corner trim"));
            ui.checkbox(&mut s.center,"Start box at center");ui.checkbox(&mut s.closed,"Close path");ui.checkbox(&mut s.snap,"Snap control points to grid");if s.snap{ui.add(egui::DragValue::new(&mut s.snap_step).range(0.1..=1e6).prefix("Grid step: "));}
            ui.horizontal(|ui|{ui.label("Line RGBA");super::edit::rgba(ui,&mut s.line);ui.label("Fill RGBA");super::edit::rgba(ui,&mut s.fill);});
            ui.checkbox(&mut self.workspaces.store.current.shapes_vector,"Vector output (selected vector layer, or create one)");
            ui.label("Click mode: add control points, Enter to finish, Backspace/right-click/Ctrl+Z undo draft points; Ctrl+Y redo; Escape cancels. Drag mode: boxes use two corners; curves use the traced control path. Polygon/fill closes automatically. Raster output respects selection strength; editable vector output requires an unrestricted selection.");
            if old!=self.workspaces.store.current.shapes||old_vector!=self.workspaces.store.current.shapes_vector {
                let requested=self.workspaces.store.current.shapes;let vector=self.workspaces.store.current.shapes_vector;
                if self.defer_path(drawing::path_draft::Action::Configure(requested,vector)){self.workspaces.store.current.shapes=old;self.workspaces.store.current.shapes_vector=old_vector;return;}
                self.finish_stroke();if self.drawing.mode.creation_shape(){self.drawing.mode=if self.workspaces.store.current.shapes.input==Input::ByClick{super::precision::Mode::ShapeClick}else{super::precision::Mode::ShapeDrag};}}
            if ui.button("Draw shape on canvas").clicked(){self.execute_command(ui.ctx(),"shape-tool");}
            ui.horizontal(|ui|{if ui.button("Draw vector line").clicked(){self.execute_command(ui.ctx(),"vector-line-tool");}
if ui.button("Draw vector curve").clicked(){self.execute_command(ui.ctx(),"vector-curve-tool");}});
        });
    }
}
pub(super) fn vector_creation_smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::vector_creation_verify::verify(root)?;
    use drawer_core::pointer_input::Phase;
    for curved in [false, true] {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 48,
            height: 40,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let mut time = 0.;
        let step = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            super::input_replay::frame(&ctx, app, *time, events);
        };
        for _ in 0..2 {
            step(&mut app, &mut time, vec![]);
        }
        let command = if curved {
            "vector-curve-tool"
        } else {
            "vector-line-tool"
        };
        app.execute_command(&ctx, command);
        step(&mut app, &mut time, vec![]);
        let settings = app.workspaces.store.current.shapes;
        if !app.workspaces.store.current.shapes_vector
            || settings.curve
                != if curved {
                    Curve::CubicBezier
                } else {
                    Curve::StraightLine
                }
        {
            return Err("Vector creation command did not select its preset".into());
        }
        let controls = if curved {
            vec![[8.5, 8.5], [16.5, 25.5], [24.5, 25.5], [32.5, 8.5]]
        } else {
            vec![[8.5, 8.5], [32.5, 25.5]]
        };
        let before = app.documents[0].snapshot();
        let count = app.documents[0].history_counts();
        for point in &controls {
            for phase in [Phase::Down, Phase::Up] {
                let e = super::precision::pointer(&ctx, &app, *point, phase);
                app.drawing.virtual_events(vec![e], false);
                step(&mut app, &mut time, vec![]);
            }
        }
        if app.documents[0].snapshot() != before {
            return Err("Clicked vector path committed before Enter".into());
        }
        step(
            &mut app,
            &mut time,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        if app.documents[0].scene.lines.layers.len() != 1
            || app.documents[0].history_counts().0 != count.0 + 1
        {
            return Err(format!("Vector creation input failed: {:?}", app.status));
        }
        let p = &app.documents[0].scene.lines.layers[0].paths[0];
        if p.definition
            .as_ref()
            .is_none_or(|p| p.controls != controls || p.settings.curve != settings.curve)
        {
            return Err("Vector creation discarded editable controls".into());
        }
        let after = app.documents[0].snapshot();
        let file = root.join("vector-command.drawer");
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != after
        {
            return Err("Vector command native roundtrip".into());
        }
        if !app.documents[0].undo()
            || app.documents[0].snapshot() != before
            || !app.documents[0].redo()
            || app.documents[0].snapshot() != after
        {
            return Err("Vector command Undo/Redo".into());
        }
        app.execute_command(&ctx, command);
        let e = super::precision::pointer(&ctx, &app, [12.5, 10.5], Phase::Down);
        app.drawing.virtual_events(vec![e], false);
        step(&mut app, &mut time, vec![]);
        step(
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
        if app.documents[0].snapshot() != after {
            return Err("Vector creation Escape changed page".into());
        }
    }
    Ok(())
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::shapes_verify::verify(root)?;
    use drawer_core::pointer_input::{Kind as PointerKind, Phase, Source};
    for pen in [false, true] {
        for click in [false, true] {
            for vector in [false, true] {
                let ctx = egui::Context::default();
                let mut app = Drawer::new(&ctx, None);
                app.simple_view = true;
                app.create_canvas(CanvasSpec {
                    width: 48,
                    height: 40,
                    dpi: 300.0,
                    background: Background::Transparent,
                })
                .map_err(|e| e.to_string())?;
                app.workspaces.store.current.shapes = Settings {
                    input: if click { Input::ByClick } else { Input::ByDrag },
                    paint: Paint::CreateBoth,
                    corner: 2.0,
                    line: [10, 20, 30, 255],
                    fill: [200, 40, 20, 128],
                    ..Default::default()
                };
                app.workspaces.store.current.shapes_vector = vector;
                app.documents[0].view.angle = 90.0;
                app.documents[0].view.zoom = 4.0;
                let mut time = 0.0;
                let step = |app: &mut Drawer, time: &mut f64, events| {
                    *time += 0.1;
                    super::input_replay::frame(&ctx, app, *time, events);
                };
                step(&mut app, &mut time, vec![]);
                step(&mut app, &mut time, vec![]);
                app.execute_command(&ctx, "shape-tool");
                step(&mut app, &mut time, vec![]);
                let points = [[12.5, 10.5], [28.5, 26.5]];
                let before = app.documents[0].snapshot();
                let mut expected = core::prepare(
                    &app.documents[0],
                    if vector {
                        Target::Vector(None)
                    } else {
                        Target::Raster(0)
                    },
                    &points,
                    app.workspaces.store.current.shapes,
                )?;
                if pen && click && vector {
                    let path = expected
                        .scene
                        .lines
                        .layers
                        .last_mut()
                        .unwrap()
                        .paths
                        .last_mut()
                        .unwrap();
                    path.pressure.device = vec![Some(0.6); 2];
                    path.pressure.device_enabled =
                        app.workspaces.store.current.pressure.tools[6].size;
                }
                let event = |app: &Drawer, p, phase| {
                    let mut e = super::precision::pointer(&ctx, app, p, phase);
                    if pen {
                        e.source = Source {
                            kind: PointerKind::Pen,
                            device: 19,
                            contact: 4,
                        };
                        e.pressure = Some(0.6);
                    }
                    e
                };
                app.drawing
                    .virtual_events(vec![event(&app, points[0], Phase::Down)], pen);
                step(&mut app, &mut time, vec![]);
                if app.documents[0].snapshot() != before {
                    return Err("Shape down changed page".into());
                }
                let events = if click {
                    vec![
                        event(&app, points[0], Phase::Up),
                        event(&app, points[1], Phase::Down),
                        event(&app, points[1], Phase::Up),
                    ]
                } else {
                    vec![
                        event(&app, points[1], Phase::Move),
                        event(&app, points[1], Phase::Up),
                    ]
                };
                app.drawing.virtual_events(events, pen);
                step(&mut app, &mut time, vec![]);
                if click {
                    if app.documents[0].snapshot() != before {
                        return Err("Unfinished clicked shape changed page".into());
                    }
                    step(
                        &mut app,
                        &mut time,
                        vec![egui::Event::Key {
                            key: egui::Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        }],
                    );
                }
                if app.documents[0].snapshot() != expected
                    || app.documents[0].history_counts().0 != 1
                {
                    return Err(format!(
                        "Shape production pen={pen} click={click} vector={vector}: {:?}",
                        app.status
                    ));
                }
                if !app.documents[0].undo()
                    || app.documents[0].snapshot() != before
                    || !app.documents[0].redo()
                    || app.documents[0].snapshot() != expected
                {
                    return Err("Shape production Undo/Redo".into());
                }
                app.documents[0].undo();
                step(
                    &mut app,
                    &mut time,
                    vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                );
                for cancel in 0..4 {
                    if vector {
                        app.panels.targets.insert(
                            app.documents[0].id,
                            drawer_core::thumbnail::Target::Raster(0),
                        );
                    }
                    app.execute_command(&ctx, "shape-tool");
                    step(&mut app, &mut time, vec![]);
                    let before = app.documents[0].snapshot();
                    let history = app.documents[0].history_counts();
                    app.drawing
                        .virtual_events(vec![event(&app, points[0], Phase::Down)], pen);
                    step(&mut app, &mut time, vec![]);
                    let mut keys = vec![];
                    match cancel {
                        0 => keys.push(egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        }),
                        1 => app.documents[0].view.angle += 1.0,
                        2 => app.workspaces.store.current.shapes.width += 0.1,
                        _ => app
                            .drawing
                            .virtual_events(vec![event(&app, points[0], Phase::Cancel)], pen),
                    };
                    step(&mut app, &mut time, keys);
                    app.drawing
                        .virtual_events(vec![event(&app, points[1], Phase::Up)], pen);
                    step(&mut app, &mut time, vec![]);
                    if app.documents[0].snapshot() != before
                        || app.documents[0].history_counts() != history
                    {
                        return Err(format!("Shape cancel {cancel} changed page"));
                    }
                    step(
                        &mut app,
                        &mut time,
                        vec![egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: false,
                            repeat: false,
                            modifiers: Default::default(),
                        }],
                    );
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn vector_line_curve_commands_mouse_points_enter_native_undo_escape() {
        let root = crate::tests::TestDirectory::new();
        super::vector_creation_smoke(&root.0).unwrap();
    }
    #[test]
    fn mouse_pen_click_drag_raster_vector_and_cancellation() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
