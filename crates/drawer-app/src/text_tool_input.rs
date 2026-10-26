use super::*;
pub(crate) struct Request {
    pub document: u64,
    pub revision: u64,
    pub frame: u32,
    pub start: Point,
    pub end: Point,
}
pub(super) struct Capture {
    document: u64,
    revision: u64,
    frame: u32,
    view: u64,
    source: drawer_core::pointer_input::Source,
    projection: Projection,
    start: Point,
    end: Point,
}
impl Drawing {
    pub(super) fn text_tool_canvas(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &Document,
        view: u64,
        projection: Projection,
        enabled: bool,
    ) {
        use drawer_core::pointer_input::{Kind, Origin, Phase};
        if self.text_capture.as_ref().is_some_and(|c| c.view != view) {
            return;
        }
        if !enabled || !ui.input(|i| i.focused) || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.text_capture = None;
            return;
        }
        if self.text_capture.as_ref().is_some_and(|c| {
            c.document != doc.id
                || c.revision != doc.revision
                || c.frame != doc.current_frame
                || c.view != view
                || c.projection != projection
        }) {
            self.text_capture = None;
        }
        for n in 0..self.events.len() {
            if self.events[n].1 {
                continue;
            }
            let drawer_core::touch::Effect::Pointer(e) = self.events[n].0 else {
                continue;
            };
            if e.source.kind == Kind::Mouse && e.origin != Origin::Unmarked {
                continue;
            }
            let pos = egui::pos2(e.position[0] as f32, e.position[1] as f32);
            if e.phase == Phase::Down
                && self.text_capture.is_none()
                && response.rect.contains(pos)
                && ui.ctx().layer_id_at(pos) == Some(response.layer_id)
            {
                let point = projection.point(pos);
                self.text_capture = Some(Box::new(Capture {
                    document: doc.id,
                    revision: doc.revision,
                    frame: doc.current_frame,
                    view,
                    source: e.source,
                    projection,
                    start: point,
                    end: point,
                }));
                self.events[n].1 = true;
            } else if self
                .text_capture
                .as_ref()
                .is_some_and(|c| c.source == e.source)
            {
                self.events[n].1 = true;
                let c = self.text_capture.as_mut().unwrap();
                c.end = c.projection.point(pos);
                if e.phase == Phase::Up {
                    let c = self.text_capture.take().unwrap();
                    self.text_tool_request = Some(Request {
                        document: c.document,
                        revision: c.revision,
                        frame: c.frame,
                        start: c.start,
                        end: c.end,
                    });
                } else if e.phase == Phase::Cancel {
                    self.text_capture = None;
                }
            }
        }
    }
}

pub(crate) fn smoke() -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut drawing = Drawing {
        text_tool_active: true,
        ..Default::default()
    };
    let doc = drawer_core::text_layout_verify::fixture()?;
    let snapshot = doc.snapshot();
    let mut time = 0.;
    let mut run = |drawing: &mut Drawing,
                   phase: Option<drawer_core::pointer_input::Phase>,
                   position: [f64; 2],
                   escape: bool| {
        time += 0.1;
        if let Some(phase) = phase {
            drawing.events = vec![(
                drawer_core::touch::Effect::Pointer(drawer_core::pointer_input::Event {
                    source: drawer_core::pointer_input::Source::MOUSE,
                    phase,
                    position,
                    pressure: None,
                    attributes: Default::default(),
                    origin: Default::default(),
                }),
                false,
            )];
        } else {
            drawing.events.clear();
        }
        let events = if escape {
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]
        } else {
            vec![]
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500., 400.),
                )),
                time: Some(time),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(300., 200.), egui::Sense::hover());
                let response = ui.interact(
                    rect,
                    egui::Id::new("text-capture-test"),
                    egui::Sense::click_and_drag(),
                );
                let projection = Projection {
                    view: doc.view,
                    size: [doc.width as f64, doc.height as f64],
                    center: [rect.center().x as f64, rect.center().y as f64],
                    ppp: 1.,
                };
                drawing.text_tool_canvas(ui, &response, &doc, 1, projection, true);
            },
        );
        output.textures_delta.clear();
    };
    use drawer_core::pointer_input::Phase;
    run(&mut drawing, None, [0.; 2], false);
    run(&mut drawing, Some(Phase::Down), [80., 80.], false);
    run(&mut drawing, Some(Phase::Move), [100., 90.], false);
    run(&mut drawing, Some(Phase::Up), [130., 100.], false);
    let r = drawing
        .text_tool_request
        .take()
        .ok_or("Production text pointer capture did not complete")?;
    if (r.end[0] - r.start[0] - 50.).abs() > 1e-9
        || (r.end[1] - r.start[1] - 20.).abs() > 1e-9
        || doc.snapshot() != snapshot
    {
        return Err("Text capture projection or source isolation".into());
    }
    run(&mut drawing, Some(Phase::Down), [80., 80.], false);
    run(&mut drawing, None, [0.; 2], true);
    run(&mut drawing, Some(Phase::Up), [130., 100.], false);
    if drawing.text_capture.is_some() || drawing.text_tool_request.is_some() {
        return Err("Escape retained a pending text capture".into());
    }
    Ok(())
}
