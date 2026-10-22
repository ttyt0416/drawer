//! Click-path history is transient; document history receives only the final path.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Vertex {
    pub point: Point,
    pub pressure: Option<f64>,
}
#[derive(Clone, Default)]
pub(super) struct Journal {
    pub current: Vec<Vertex>,
    undo: Vec<Vec<Vertex>>,
    redo: Vec<Vec<Vertex>>,
    gesture: Option<(Vec<Vertex>, usize)>,
}
impl Journal {
    pub fn begin(&mut self, vertex: Vertex, radius: f64) -> Result<(), String> {
        let index = self
            .current
            .iter()
            .enumerate()
            .rev()
            .find(|(_, v)| {
                (v.point[0] - vertex.point[0]).hypot(v.point[1] - vertex.point[1]) <= radius
            })
            .map(|(i, _)| i);
        if index.is_none() && self.current.len() >= 128 {
            return Err("Path control limit128".into());
        }
        let old = self.current.clone();
        let index = if let Some(i) = index {
            if vertex.pressure.is_some() {
                self.current[i].pressure = vertex.pressure;
            }
            i
        } else {
            self.current.push(vertex);
            self.current.len() - 1
        };
        self.gesture = Some((old, index));
        Ok(())
    }
    pub fn move_to(&mut self, point: Point) {
        if let Some((_, i)) = &self.gesture {
            self.current[*i].point = point;
        }
    }
    pub fn finish(&mut self) {
        if let Some((old, _)) = self.gesture.take()
            && old != self.current
        {
            self.undo.push(old);
            if self.undo.len() > 128 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }
    pub fn cancel_contact(&mut self) {
        if let Some((old, _)) = self.gesture.take() {
            self.current = old;
        }
    }
    pub fn undo(&mut self) -> bool {
        if self.gesture.is_some() {
            self.cancel_contact();
            return true;
        }
        if let Some(old) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.current, old));
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self) -> bool {
        if self.gesture.is_some() {
            return false;
        }
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.current, next));
            true
        } else {
            false
        }
    }
}
impl Drawing {
    pub(crate) fn has_path_draft(&self) -> bool {
        self.selection.as_ref().is_some_and(|s| s.journal.is_some())
    }
    pub(crate) fn path_count(&self) -> usize {
        self.selection
            .as_ref()
            .filter(|s| s.journal.is_some())
            .map_or(0, |s| s.points.len())
    }
    pub(crate) fn discard_path(&mut self) {
        if self.has_path_draft() {
            self.selection = None;
            self.ownership.clear();
            self.events.clear();
        }
    }
    pub(crate) fn path_history(&mut self, redo: bool) -> bool {
        let Some(s) = self.selection.as_mut().filter(|s| s.journal.is_some()) else {
            return false;
        };
        let j = s.journal.as_mut().unwrap();
        let changed = if redo { j.redo() } else { j.undo() };
        s.points = j.current.iter().map(|v| v.point).collect();
        s.hover = None;
        self.ownership.clear();
        changed
    }
    pub(crate) fn commit_path(&mut self, documents: &mut [Document]) -> Result<(), String> {
        if !self.has_path_draft() {
            return Ok(());
        }
        let Some(mut s) = self.selection.take() else {
            return Ok(());
        };
        s.journal.as_mut().unwrap().finish();
        s.points = s
            .journal
            .as_ref()
            .unwrap()
            .current
            .iter()
            .map(|v| v.point)
            .collect();
        let result = if s.points.len() < 2 {
            Err("Add at least two path controls before confirming".into())
        } else if let Some(d) = documents.iter_mut().find(|d| d.id == s.document) {
            if d.current_frame != s.frame || d.scene.transform != s.space {
                Err("Page/frame changed; discard the unfinished path".into())
            } else {
                self.commit_shape(&s, d).map(|_| ())
            }
        } else {
            Err("The path page was closed".into())
        };
        if result.is_err() {
            self.selection = Some(s);
        }
        self.ownership.clear();
        result
    }
    pub(super) fn path_keys(&mut self, ctx: &egui::Context, documents: &mut [Document]) -> bool {
        if !self.has_path_draft() {
            return false;
        }
        let action = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if !modifiers.alt => {
                    if (modifiers.ctrl || modifiers.command) && *key == egui::Key::Z {
                        Some((if modifiers.shift { 1 } else { 0 }, *key, *modifiers))
                    } else if (modifiers.ctrl || modifiers.command) && *key == egui::Key::Y {
                        Some((1, *key, *modifiers))
                    } else if !modifiers.ctrl
                        && !modifiers.command
                        && [*key].iter().any(|k| {
                            matches!(
                                k,
                                egui::Key::Backspace | egui::Key::Enter | egui::Key::Escape
                            )
                        })
                    {
                        Some((
                            match key {
                                egui::Key::Backspace => 0,
                                egui::Key::Enter => 2,
                                _ => 3,
                            },
                            *key,
                            *modifiers,
                        ))
                    } else {
                        None
                    }
                }
                _ => None,
            })
        });
        if let Some((action, key, mods)) = action {
            ctx.input_mut(|i| {
                i.consume_key(mods, key);
            });
            match action {
                0 => {
                    self.path_history(false);
                }
                1 => {
                    self.path_history(true);
                }
                2 => {
                    if let Err(e) = self.commit_path(documents) {
                        self.error = Some(e);
                    }
                }
                _ => self.discard_path(),
            }
            return true;
        }
        let right = ctx.input(|i| {
            i.pointer
                .button_clicked(egui::PointerButton::Secondary)
                .then(|| i.pointer.interact_pos())
                .flatten()
        });
        if let Some(pos) = right
            && self.selection.as_ref().is_some_and(|s| {
                s.canvas.contains(pos) && ctx.layer_id_at(pos) == Some(s.canvas_layer)
            })
        {
            self.path_history(false);
            return true;
        }
        false
    }
}
#[derive(Clone)]
pub(crate) enum Action {
    Command(String),
    Tool(BrushTip),
    Mode(crate::precision::Mode),
    Tab(u64),
    Save(u64, PathBuf),
    SaveDialog(u64, bool),
    Close(u64),
    Quit,
    Open(PathBuf),
    New(CanvasSpec, drawer_core::color_storage::Config),
    Configure(drawer_core::shapes::Settings, bool),
    UserTool(u64),
    Layer(u64, drawer_core::thumbnail::Target),
}
impl Drawer {
    pub(crate) fn guard_path_layer(&mut self) {
        if let Some((mode, settings, vector)) = self
            .drawing
            .selection
            .as_ref()
            .filter(|s| s.journal.is_some())
            .and_then(|s| {
                s.shape.map(|shape| {
                    (
                        s.mode,
                        shape.settings,
                        matches!(shape.target, drawer_core::shapes::Target::Vector(_)),
                    )
                })
            })
        {
            if self.drawing.mode != mode && self.defer_path(Action::Mode(self.drawing.mode)) {
                self.drawing.mode = mode;
            }
            if (self.workspaces.store.current.shapes != settings
                || self.workspaces.store.current.shapes_vector != vector)
                && self.defer_path(Action::Configure(
                    self.workspaces.store.current.shapes,
                    self.workspaces.store.current.shapes_vector,
                ))
            {
                self.workspaces.store.current.shapes = settings;
                self.workspaces.store.current.shapes_vector = vector;
            }
        }
        let Some(s) = self
            .drawing
            .selection
            .as_ref()
            .filter(|s| s.journal.is_some())
        else {
            return;
        };
        let Some(shape) = s.shape else {
            return;
        };
        let id = s.document;
        let old = shape.source_target;
        if let Some(current) = self.panels.targets.get(&id).copied()
            && current != old
            && self.defer_path(Action::Layer(id, current))
        {
            self.panels.targets.insert(id, old);
        }
    }
    pub(crate) fn defer_path(&mut self, action: Action) -> bool {
        if self.path_transition.is_some() {
            return true;
        }
        if !self.drawing.has_path_draft() {
            return false;
        }
        if self.drawing.path_count() < 2 {
            self.drawing.discard_path();
            return false;
        }
        self.path_transition = Some(Box::new(action));
        true
    }
    pub(crate) fn resolve_path(&mut self, ctx: &egui::Context, choice: u8) -> Result<(), String> {
        if choice == 2 {
            self.path_transition = None;
            return Ok(());
        }
        if choice == 0 {
            self.drawing.commit_path(&mut self.documents)?;
            if let Some((id, target, _)) = self.drawing.picked_layer.take() {
                self.panels.targets.insert(id, target);
            }
        } else {
            self.drawing.discard_path();
        }
        self.status = None;
        let Some(action) = self.path_transition.take() else {
            return Ok(());
        };
        match *action {
            Action::Command(c) => self.execute_command(ctx, &c),
            Action::Tool(t) => self.set_tool(t),
            Action::Mode(mode) => {
                self.finish_stroke();
                self.restore_locked_tool();
                self.drawing.mode = mode;
                self.drawing.connect_line = false;
            }
            Action::Tab(id) => {
                if let Some(n) = self.tabs.iter().position(|t| t.id == id) {
                    self.select_tab(n);
                }
            }
            Action::Save(id, path) => {
                if let Some(n) = self.documents.iter().position(|d| d.id == id) {
                    self.save_to(n, &path)?;
                }
            }
            Action::SaveDialog(id, as_new) => {
                if let Some(n) = self.documents.iter().position(|d| d.id == id) {
                    if as_new {
                        self.save_as(n)?;
                    } else {
                        self.save_document(n)?;
                    }
                }
            }
            Action::Close(id) => {
                if let Some(n) = self.tabs.iter().position(|t| t.id == id) {
                    self.request_close(n);
                }
            }
            Action::Quit => self.begin_quit(),
            Action::Open(p) => self.open_path(&p)?,
            Action::New(spec, color) => self
                .create_canvas_color(spec, color)
                .map_err(|e| e.to_string())?,
            Action::Configure(settings, vector) => {
                self.workspaces.store.current.shapes = settings;
                self.workspaces.store.current.shapes_vector = vector;
                self.drawing.mode = if settings.input == drawer_core::shapes::Input::ByClick {
                    crate::precision::Mode::ShapeClick
                } else {
                    crate::precision::Mode::ShapeDrag
                };
            }
            Action::UserTool(id) => self.select_user_tool(id)?,
            Action::Layer(id, target) => {
                self.panels.targets.insert(id, target);
            }
        }
        Ok(())
    }
    pub(crate) fn path_transition_window(&mut self, ctx: &egui::Context) {
        if self.path_transition.is_none() {
            return;
        }
        let mut choice = None;
        let mut buttons = DecisionButtons::default();
        egui::Modal::new(egui::Id::new("unfinished-click-path")).show(ctx,|ui|{
   ui.heading("Unfinished path");ui.label(format!("{} controls are still being edited. Confirm them as one drawing command, discard the draft, or keep drawing before continuing.",self.drawing.path_count()));
   ui.horizontal(|ui|{for (index,label) in ["Confirm path and continue","Discard path and continue","Keep drawing"].into_iter().enumerate(){let r=ui.button(label);buttons.0[index]=Some(r.rect);if r.clicked(){choice=Some(index as u8);}}});
  });
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("path-decision-buttons"), buttons));
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            choice = Some(2);
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
        }
        if let Some(choice) = choice
            && let Err(e) = self.resolve_path(ctx, choice)
        {
            self.status = Some(e);
        }
    }
}

fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn journal_verify() -> Result<(), String> {
    let mut j = Journal::default();
    for n in 0..128 {
        j.begin(
            Vertex {
                point: [n as f64, 0.],
                pressure: Some(0.5),
            },
            -1.,
        )?;
        j.finish();
    }
    check(
        j.begin(
            Vertex {
                point: [999., 0.],
                pressure: None,
            },
            -1.,
        )
        .is_err(),
        "Path journal limit",
    )?;
    for _ in 0..128 {
        check(j.undo(), "Path journal lost an Undo")?;
    }
    check(
        j.current.is_empty() && !j.undo(),
        "Draft Undo crossed its empty state",
    )?;
    for _ in 0..128 {
        check(j.redo(), "Path journal lost a redo")?;
    }
    j.undo();
    j.begin(
        Vertex {
            point: [999., 1.],
            pressure: None,
        },
        -1.,
    )?;
    j.finish();
    check(!j.redo(), "Draft redo survived a new branch")?;
    let old = j.current.clone();
    j.begin(
        Vertex {
            point: [0., 0.],
            pressure: None,
        },
        0.1,
    )?;
    j.move_to([1., 2.]);
    j.finish();
    check(
        j.current[0].point == [1., 2.]
            && j.undo()
            && j.current == old
            && j.redo()
            && j.current[0].point == [1., 2.],
        "Edited node history",
    )?;
    let old = j.current.clone();
    j.begin(
        Vertex {
            point: [1., 2.],
            pressure: None,
        },
        0.1,
    )?;
    j.move_to([5., 6.]);
    j.cancel_contact();
    check(
        j.current == old,
        "Cancelled node contact lost committed controls",
    )?;
    Ok(())
}
fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    crate::input_replay::frame(ctx, app, *time, events);
}
fn click(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, point: Point, pen: Option<f64>) {
    for phase in [
        drawer_core::pointer_input::Phase::Down,
        drawer_core::pointer_input::Phase::Up,
    ] {
        let mut event = crate::precision::pointer(ctx, app, point, phase);
        if let Some(pressure) = pen {
            event.source = drawer_core::pointer_input::Source {
                kind: drawer_core::pointer_input::Kind::Pen,
                device: 149,
                contact: 1,
            };
            event.pressure = Some(pressure);
        }
        app.drawing.virtual_events(vec![event], false);
        step(ctx, app, time, vec![]);
    }
}
fn key(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, key: egui::Key, command: bool) {
    let modifiers = egui::Modifiers {
        ctrl: command,
        command,
        ..Default::default()
    };
    for pressed in [true, false] {
        step(
            ctx,
            app,
            time,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                },
            ],
        );
    }
    step(
        ctx,
        app,
        time,
        vec![egui::Event::ModifiersChanged(Default::default())],
    );
}
fn fixture(root: &std::path::Path) -> Result<(egui::Context, Box<Drawer>, f64), String> {
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 48,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let mut state = app.documents[0].snapshot();
    state.layers[0].pixels[0] = [30, 50, 90, 180];
    app.documents[0].commit_state(state)?;
    drawer_core::files::save(&mut app.documents[0], &root.join("path-baseline.drawer"))
        .map_err(|e| e.to_string())?;
    let mut time = 0.;
    step(&ctx, &mut app, &mut time, vec![]);
    step(&ctx, &mut app, &mut time, vec![]);
    app.execute_command(&ctx, "vector-line-tool");
    step(&ctx, &mut app, &mut time, vec![]);
    Ok((ctx, app, time))
}
pub(crate) fn smoke(root: &std::path::Path) -> Result<(), String> {
    journal_verify()?;
    for pen in [false, true] {
        let (ctx, mut app, mut time) = fixture(root)?;
        app.workspaces.store.current.pressure.tools[6].size = true;
        step(&ctx, &mut app, &mut time, vec![]);
        let before = app.documents[0].snapshot();
        let dirty = app.documents[0].dirty;
        let count = app.documents[0].history_counts();
        click(&ctx, &mut app, &mut time, [8., 8.], pen.then_some(0.25));
        key(&ctx, &mut app, &mut time, egui::Key::Enter, false);
        check(
            app.drawing.path_count() == 1
                && app.documents[0].snapshot() == before
                && app.documents[0].history_counts() == count
                && app.documents[0].dirty == dirty,
            "One-control path confirmed or changed document",
        )?;
        key(&ctx, &mut app, &mut time, egui::Key::Z, true);
        check(
            app.drawing.path_count() == 0
                && app.documents[0].snapshot() == before
                && app.documents[0].history_counts() == count,
            "Ctrl+Z reached drawing history",
        )?;
        key(&ctx, &mut app, &mut time, egui::Key::Y, true);
        check(app.drawing.path_count() == 1, "Draft Ctrl+Y failed")?;
        click(&ctx, &mut app, &mut time, [24., 8.], pen.then_some(0.75));
        click(&ctx, &mut app, &mut time, [40., 20.], pen.then_some(0.5));
        key(&ctx, &mut app, &mut time, egui::Key::Backspace, false);
        check(app.drawing.path_count() == 2, "Draft Backspace failed")?;
        // A right click on the canvas is another local point Undo.
        let pos = crate::precision::pointer(
            &ctx,
            &app,
            [24., 8.],
            drawer_core::pointer_input::Phase::Move,
        )
        .position;
        let pos = egui::pos2(pos[0] as f32, pos[1] as f32);
        for pressed in [true, false] {
            step(
                &ctx,
                &mut app,
                &mut time,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        check(
            app.drawing.path_count() == 1,
            "Right-click did not undo a draft control",
        )?;
        key(&ctx, &mut app, &mut time, egui::Key::Y, true);
        click(&ctx, &mut app, &mut time, [48., 24.], pen.then_some(1.));
        key(&ctx, &mut app, &mut time, egui::Key::Y, true);
        check(
            app.drawing.path_count() == 3,
            "New point did not discard draft redo",
        )?;
        // Text composition owns its keys, including Escape/Undo/Backspace.
        let points = app.drawing.path_count();
        let mut text = "가".to_owned();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 700.),
                )),
                time: Some(time + 0.1),
                ..Default::default()
            },
            |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut text).id(egui::Id::new("path-composition")),
                );
                r.request_focus();
                app.process_drawing(ui.ctx());
            },
        );
        out.textures_delta.clear();
        time += 0.1;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 700.),
                )),
                time: Some(time + 0.1),
                events: vec![
                    egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: "한".into(),
                        active_range_chars: None,
                    }),
                    egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(egui::Id::new("path-composition")));
                app.process_drawing(ui.ctx());
            },
        );
        out.textures_delta.clear();
        time += 0.1;
        check(
            app.drawing.path_count() == points && app.documents[0].snapshot() == before,
            "Text/IME keys edited the draft or document",
        )?;
        ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("path-composition")));
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![
                egui::Event::Ime(egui::ImeEvent::Commit("한".into())),
                egui::Event::Key {
                    key: egui::Key::Backspace,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: Default::default(),
                },
            ],
        );
        key(&ctx, &mut app, &mut time, egui::Key::Enter, false);
        check(
            !app.drawing.has_path_draft() && app.documents[0].history_counts().0 == count.0 + 1,
            "Confirm did not make one drawing Undo",
        )?;
        let p = &app.documents[0].scene.lines.layers[0].paths[0];
        check(
            p.definition
                .as_ref()
                .is_some_and(|d| d.controls == [[8., 8.], [24., 8.], [48., 24.]])
                && (!pen || p.pressure.device == [Some(0.25), Some(0.75), Some(1.)]),
            "Confirm lost control geometry/device pressure",
        )?;
        let after = app.documents[0].snapshot();
        let file = root.join("confirmed-path.drawer");
        app.save_to(0, &file)?;
        check(
            drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == after
                && app.documents[0].undo()
                && app.documents[0].snapshot() == before,
            "Confirmed path native save or one Undo",
        )?;
    }
    transitions(root)?;
    curves_and_contacts(root)?;
    Ok(())
}
fn transitions(root: &std::path::Path) -> Result<(), String> {
    for operation in 0..5 {
        for choice in 0..3 {
            let (ctx, mut app, mut time) = fixture(root)?;
            if operation == 2 {
                app.create_canvas(CanvasSpec {
                    width: 16,
                    height: 16,
                    dpi: 300.,
                    background: Background::Transparent,
                })
                .map_err(|e| e.to_string())?;
                app.select_tab(0);
                app.execute_command(&ctx, "vector-line-tool");
                step(&ctx, &mut app, &mut time, vec![]);
            }
            let before = app.documents[0].snapshot();
            let count = app.documents[0].history_counts();
            let dirty = app.documents[0].dirty;
            click(&ctx, &mut app, &mut time, [8., 8.], None);
            click(&ctx, &mut app, &mut time, [32., 16.], None);
            let output = root.join("unfinished-save.drawer");
            std::fs::write(&output, b"unchanged until decision").map_err(|e| e.to_string())?;
            match operation {
                0 => app.execute_command(&ctx, "Pencil"),
                1 => {
                    check(
                        app.save_to(0, &output).is_err(),
                        "Unconfirmed save succeeded",
                    )?;
                }
                2 => app.select_tab(1),
                3 => app.request_close(0),
                _ => app.begin_quit(),
            };
            check(
                app.path_transition.is_some()
                    && app.drawing.path_count() == 2
                    && app.documents[0].snapshot() == before
                    && app.documents[0].history_counts() == count
                    && app.documents[0].dirty == dirty,
                "Transition changed draft/page before decision",
            )?;
            step(&ctx, &mut app, &mut time, vec![]);
            check(app.drawing.path_count() == 2, "Modal frame discarded path")?;
            step(&ctx, &mut app, &mut time, vec![]);
            step(&ctx, &mut app, &mut time, vec![]);
            let rect = ctx
                .data(|d| d.get_temp::<DecisionButtons>(egui::Id::new("path-decision-buttons")))
                .and_then(|b| b.0[choice as usize])
                .ok_or("Path decision button missing")?;
            for pressed in [true, false] {
                step(
                    &ctx,
                    &mut app,
                    &mut time,
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }

            if choice == 2 {
                check(
                    app.path_transition.is_none()
                        && app.drawing.path_count() == 2
                        && app.documents[0].snapshot() == before
                        && !app.quit_ready
                        && app.active == Some(0),
                    "Keep drawing performed deferred action",
                )?;
                key(&ctx, &mut app, &mut time, egui::Key::Escape, false);
                check(
                    !app.drawing.has_path_draft()
                        && app.documents[0].snapshot() == before
                        && app.documents[0].history_counts() == count,
                    "Escape changed document/history",
                )?;
            } else if choice == 0 {
                check(
                    !app.drawing.has_path_draft()
                        && app.documents[0].scene.lines.layers.len() == 1
                        && app.documents[0].history_counts().0 == count.0 + 1,
                    &format!(
                        "Transition confirm op={operation} choice={choice}: draft={}, pending={}, layers={}, counts={:?}/{count:?}, status={:?}",
                        app.drawing.path_count(),
                        app.path_transition.is_some(),
                        app.documents[0].scene.lines.layers.len(),
                        app.documents[0].history_counts(),
                        app.status
                    ),
                )?;
            } else {
                check(
                    !app.drawing.has_path_draft(),
                    "Transition discard retained draft",
                )?;
                if !app.documents.is_empty() {
                    let n = app.documents.iter().position(|d| d.width == 64);
                    if let Some(n) = n {
                        check(
                            app.documents[n].snapshot() == before
                                && app.documents[n].history_counts() == count,
                            "Discard changed original artwork/history",
                        )?;
                    }
                }
            }
            if operation == 1 {
                if choice == 2 {
                    check(
                        std::fs::read(&output).map_err(|e| e.to_string())?
                            == b"unchanged until decision",
                        "Keep drawing overwrote destination",
                    )?;
                } else {
                    let read = drawer_core::files::open(&output, DEFAULT_DOCUMENT_BUDGET)
                        .map_err(|e| e.to_string())?;
                    check(
                        read.scene.lines.layers.len() == if choice == 0 { 1 } else { 0 },
                        "Saved document included an unconfirmed path",
                    )?;
                }
            }
            if operation == 2 && choice != 2 {
                check(app.active == Some(1), "Resolved tab switch did not replay")?;
            }
            if operation == 3 && choice == 0 {
                check(
                    app.pending_close.is_some(),
                    "Confirmed dirty path bypassed normal close warning",
                )?;
            }
            if operation == 4 && choice == 2 {
                check(
                    !app.quit_requested && !app.quit_ready,
                    "Keep drawing allowed application exit",
                )?;
            }
        }
    }
    Ok(())
}
fn curves_and_contacts(root: &std::path::Path) -> Result<(), String> {
    for curve in [
        drawer_core::shapes::Curve::Spline,
        drawer_core::shapes::Curve::QuadraticBezier,
        drawer_core::shapes::Curve::CubicBezier,
    ] {
        let (ctx, mut app, mut time) = fixture(root)?;
        app.execute_command(&ctx, "vector-curve-tool");
        app.workspaces.store.current.shapes.curve = curve;
        app.workspaces.store.current.shapes.width = 5.;
        app.workspaces.store.current.shapes.line = [110, 30, 170, 180];
        step(&ctx, &mut app, &mut time, vec![]);
        let before = app.documents[0].snapshot();
        let count = app.documents[0].history_counts();
        for point in [[8., 8.], [20., 30.], [36., 30.], [48., 8.]] {
            click(&ctx, &mut app, &mut time, point, None);
        }
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![egui::Event::ModifiersChanged(egui::Modifiers {
                alt: true,
                ..Default::default()
            })],
        );
        for (point, phase) in [
            ([20., 30.], drawer_core::pointer_input::Phase::Down),
            ([24., 26.], drawer_core::pointer_input::Phase::Move),
            ([24., 26.], drawer_core::pointer_input::Phase::Up),
        ] {
            let event = crate::precision::pointer(&ctx, &app, point, phase);
            app.drawing.virtual_events(vec![event], false);
            step(&ctx, &mut app, &mut time, vec![]);
        }
        check(
            app.drawing.path_count() == 4
                && app.drawing.selection.as_ref().unwrap().points[1] == [24., 26.],
            "Alt-drag edited control not recorded",
        )?;
        key(&ctx, &mut app, &mut time, egui::Key::Z, true);
        check(
            app.drawing.selection.as_ref().unwrap().points[1] == [20., 30.],
            "Edited control Undo removed a point",
        )?;
        key(&ctx, &mut app, &mut time, egui::Key::Y, true);
        check(
            app.drawing.selection.as_ref().unwrap().points[1] == [24., 26.],
            "Edited control redo failed",
        )?;
        let down = crate::precision::pointer(
            &ctx,
            &app,
            [52., 16.],
            drawer_core::pointer_input::Phase::Down,
        );
        app.drawing.virtual_events(vec![down], false);
        step(&ctx, &mut app, &mut time, vec![]);
        let mut cancel = down;
        cancel.phase = drawer_core::pointer_input::Phase::Cancel;
        app.drawing.virtual_events(vec![cancel], false);
        step(&ctx, &mut app, &mut time, vec![]);
        check(
            app.drawing.path_count() == 4,
            "Contact cancellation discarded prior path controls",
        )?;
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![egui::Event::WindowFocused(false)],
        );
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![egui::Event::WindowFocused(true)],
        );
        check(
            app.drawing.path_count() == 4,
            "Focus loss discarded a clicked path",
        )?;
        app.execute_command(&ctx, "zoom-in");
        step(&ctx, &mut app, &mut time, vec![]);
        check(
            app.drawing.path_count() == 4 && app.documents[0].snapshot() == before,
            "View-only zoom changed path/source",
        )?;
        key(&ctx, &mut app, &mut time, egui::Key::Enter, false);
        let p = &app.documents[0].scene.lines.layers[0].paths[0];
        check(
            p.definition.as_ref().is_some_and(|d| {
                d.controls == [[8., 8.], [24., 26.], [36., 30.], [48., 8.]]
                    && d.settings.curve == curve
            }) && p.width == 5.
                && p.color == [110, 30, 170, 180]
                && app.documents[0].history_counts().0 == count.0 + 1,
            "Curve confirm lost edited controls/style/history",
        )?;
        check(
            app.documents[0].undo() && app.documents[0].snapshot() == before,
            "Curve one Undo failed",
        )?;
    }
    for choice in 0..3 {
        let (ctx, mut app, mut time) = fixture(root)?;
        let layer = app.documents[0].scene.lines.add_layer("Destination")?;
        let before = app.documents[0].snapshot();
        click(&ctx, &mut app, &mut time, [8., 8.], None);
        click(&ctx, &mut app, &mut time, [32., 16.], None);
        app.panels.targets.insert(
            app.documents[0].id,
            drawer_core::thumbnail::Target::Vector(layer),
        );
        step(&ctx, &mut app, &mut time, vec![]);
        check(
            app.path_transition.is_some()
                && app.drawing.path_count() == 2
                && app.documents[0].snapshot() == before,
            "Layer change discarded draft before decision",
        )?;
        app.resolve_path(&ctx, choice)?;
        step(&ctx, &mut app, &mut time, vec![]);
        check(
            if choice == 2 {
                app.drawing.path_count() == 2
            } else {
                !app.drawing.has_path_draft()
            },
            "Layer decision scope",
        )?;
        if choice != 2 {
            check(
                app.panels.targets.get(&app.documents[0].id)
                    == Some(&drawer_core::thumbnail::Target::Vector(layer)),
                "Auto-created layer overrode deferred destination",
            )?;
        }
    }
    let (ctx, mut app, mut time) = fixture(root)?;
    click(&ctx, &mut app, &mut time, [8., 8.], None);
    click(&ctx, &mut app, &mut time, [32., 16.], None);
    app.press_tool(&ctx, egui::Key::B, BrushTip::Pencil, false);
    check(
        app.path_transition.is_some() && app.drawing.mode == crate::precision::Mode::ShapeClick,
        "Keyboard tool switched before path choice",
    )?;
    app.resolve_path(&ctx, 2)?;
    let (ctx, mut app, mut time) = fixture(root)?;
    click(&ctx, &mut app, &mut time, [8., 8.], None);
    click(&ctx, &mut app, &mut time, [32., 16.], None);
    let mut scene = app.documents[0].scene.clone();
    scene.lines.settings.darkness = 40;
    app.documents[0].edit_scene(scene)?;
    step(&ctx, &mut app, &mut time, vec![]);
    check(
        app.drawing.path_count() == 2,
        "Unrelated metadata silently discarded draft",
    )?;
    key(&ctx, &mut app, &mut time, egui::Key::Enter, false);
    check(
        app.documents[0].scene.lines.settings.darkness == 40
            && app.documents[0].scene.lines.layers.len() == 1,
        "Draft confirmation overwrote intervening metadata",
    )?;
    app.execute_command(&ctx, "vector-line-tool");
    click(&ctx, &mut app, &mut time, [8., 8.], None);
    click(&ctx, &mut app, &mut time, [32., 16.], None);
    app.documents[0].scene.transform = drawer_core::scene::Affine([2., 0., 0., 2., 0., 0.]);
    step(&ctx, &mut app, &mut time, vec![]);
    check(
        app.drawing.path_count() == 2 && app.drawing.commit_path(&mut app.documents).is_err(),
        "Changed page geometry dropped or silently applied draft",
    )?;
    key(&ctx, &mut app, &mut time, egui::Key::Escape, false);
    check(
        !app.drawing.has_path_draft(),
        "Retained stale draft could not be discarded",
    )?;
    Ok(())
}

#[derive(Clone, Default)]
struct DecisionButtons([Option<egui::Rect>; 3]);

#[cfg(test)]
mod tests {
    #[test]
    fn transient_path_journal_add_edit_branch_and_cancel() {
        super::journal_verify().unwrap();
    }
    #[test]
    fn click_path_keyboard_ime_pressure_transition_decisions_native_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
