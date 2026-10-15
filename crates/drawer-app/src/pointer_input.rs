use super::*;
use drawer_core::pointer_input::{Event, Kind, Phase, Settings, Source};

/// egui-winit emits Touch followed immediately by its promoted GUI pointer events.
/// Preserve the original identity; an untyped touch is never guessed to be a pen.
pub(super) fn gui_events(ctx: &egui::Context) -> Vec<Event> {
    let raw = ctx.input(|i| i.events.clone());
    let mut result = vec![];
    let mut n = 0;
    while n < raw.len() {
        let source = Source::MOUSE;
        match raw[n] {
            egui::Event::Touch {
                device_id,
                id,
                phase,
                pos,
                force,
            } => {
                let phase = match phase {
                    egui::TouchPhase::Start => Phase::Down,
                    egui::TouchPhase::Move => Phase::Move,
                    egui::TouchPhase::End => Phase::Up,
                    egui::TouchPhase::Cancel => Phase::Cancel,
                };
                result.push(Event {
                    source: Source {
                        kind: Kind::Unknown,
                        device: device_id.0,
                        contact: id.0,
                    },
                    phase,
                    position: [pos.x as f64, pos.y as f64],
                    pressure: force.map(|p| p as f64),
                });
                if matches!(phase, Phase::Down | Phase::Move)
                    && matches!(raw.get(n+1),Some(egui::Event::PointerMoved(p)) if *p==pos)
                {
                    n += 1;
                }
                if matches!(phase, Phase::Down | Phase::Up)
                    && matches!(raw.get(n+1),Some(egui::Event::PointerButton {pos:p,button:egui::PointerButton::Primary,pressed,..}) if *p==pos && *pressed==(phase==Phase::Down))
                {
                    n += 1;
                }
                if matches!(phase, Phase::Up | Phase::Cancel)
                    && matches!(raw.get(n + 1), Some(egui::Event::PointerGone))
                {
                    n += 1;
                }
            }
            egui::Event::PointerMoved(p) => result.push(Event {
                source,
                phase: Phase::Move,
                position: [p.x as f64, p.y as f64],
                pressure: None,
            }),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                ..
            } => result.push(Event {
                source,
                phase: if pressed { Phase::Down } else { Phase::Up },
                position: [pos.x as f64, pos.y as f64],
                pressure: None,
            }),
            egui::Event::PointerGone => result.push(Event {
                source,
                phase: Phase::Cancel,
                position: [0.0; 2],
                pressure: None,
            }),
            _ => {}
        }
        n += 1;
    }
    result
}
#[cfg(windows)]
struct ToolState {
    device: u64,
    pose: Option<octotablet::axis::Pose>,
    phase: Option<Phase>,
    contact: u64,
    down: bool,
}
#[derive(Default)]
pub(super) struct Native {
    // Drop the manager (and its window reference) before uninitializing COM.
    #[cfg(windows)]
    manager: Option<octotablet::Manager>,
    #[cfg(windows)]
    tools: std::collections::HashMap<octotablet::tool::ID, ToolState>,
    #[cfg(windows)]
    next: u64,
    pub report: String,
    pub open: bool,
    draft: Option<Settings>,
    #[cfg(test)]
    buttons: [Option<egui::Rect>; 2],
    #[cfg(windows)]
    com: Option<winsafe::guard::CoUninitializeGuard>,
}
impl Native {
    pub fn attach(&mut self, cc: &eframe::CreationContext<'_>) {
        #[cfg(windows)]
        {
            let Some(window) = cc.winit_window() else {
                return;
            };
            match winsafe::CoInitializeEx(
                winsafe::co::COINIT::APARTMENTTHREADED | winsafe::co::COINIT::DISABLE_OLE1DDE,
            ) {
                Ok(guard) => self.com = Some(guard),
                Err(e) => {
                    self.report =
                        format!("Pen initialization unavailable: {e}; mouse remains available");
                    return;
                }
            }
            // This version's builder panics on a native provider failure. Keep mouse
            // input usable rather than letting that optional provider abort the app.
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                octotablet::Builder::new()
                    .emulate_tool_from_mouse(false)
                    .build_shared(window)
            }));
            match built {
                Ok(Ok(manager)) => {
                    self.manager = Some(manager);
                    self.report = "Windows Ink connected; physical pen validation pending".into();
                }
                Ok(Err(e)) => {
                    self.report = format!("Pen provider unavailable: {e}; mouse remains available")
                }
                Err(_) => {
                    self.report =
                        "Pen provider declined initialization; mouse remains available".into()
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = cc;
            self.report =
                "Native pen backend unavailable on this build; mouse remains available".into();
        }
    }
    pub fn pump(&mut self, ctx: &egui::Context, output: &mut Vec<Event>) {
        #[cfg(windows)]
        {
            use octotablet::{
                events::{Event as Raw, ToolEvent},
                tool::Type,
            };
            let Some(manager) = &mut self.manager else {
                return;
            };
            let Ok(events) = manager.pump();
            for raw in events {
                let Raw::Tool { tool, event } = raw else {
                    continue;
                };
                let kind = match tool.tool_type {
                    Some(
                        Type::Pen | Type::Pencil | Type::Brush | Type::Eraser | Type::Airbrush,
                    ) => Kind::Pen,
                    Some(Type::Finger) => Kind::Touch,
                    _ => Kind::Unknown,
                };
                let id = tool.id();
                if matches!(event, ToolEvent::Removed) {
                    if let Some(s) = self.tools.remove(&id)
                        && s.down
                    {
                        output.push(Event {
                            source: Source {
                                kind,
                                device: s.device,
                                contact: s.contact,
                            },
                            phase: Phase::Cancel,
                            position: [0.0; 2],
                            pressure: None,
                        });
                    }
                    continue;
                }
                if !self.tools.contains_key(&id) && self.tools.len() >= 256 {
                    continue;
                }
                let s = self.tools.entry(id).or_insert_with(|| {
                    self.next += 1;
                    ToolState {
                        device: self.next,
                        pose: None,
                        phase: None,
                        contact: 0,
                        down: false,
                    }
                });
                match event {
                    ToolEvent::Down => {
                        self.next += 1;
                        s.contact = self.next;
                        s.phase = Some(Phase::Down);
                        s.down = true;
                    }
                    ToolEvent::Up => {
                        s.phase = Some(Phase::Up);
                        s.down = false;
                    }
                    ToolEvent::Pose(pose) => s.pose = Some(pose),
                    ToolEvent::Out => {
                        if s.down {
                            s.phase = Some(Phase::Cancel);
                            s.down = false;
                        }
                    }
                    ToolEvent::Frame(_) => {
                        let phase = s.phase.take().or(s.down.then_some(Phase::Move));
                        if let (Some(phase), Some(pose)) = (phase, s.pose) {
                            // octotablet reports native logical pixels; egui can apply an additional zoom.
                            let zoom = ctx.zoom_factor() as f64;
                            let event = Event {
                                source: Source {
                                    kind,
                                    device: s.device,
                                    contact: s.contact,
                                },
                                phase,
                                position: pose.position.map(|p| p as f64 / zoom),
                                pressure: pose.pressure.get().map(|p| p as f64),
                            };
                            if event.valid() && output.len() < 65536 {
                                output.push(event);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if ctx.input(|i| i.focused) {
                ctx.request_repaint_after(std::time::Duration::from_millis(8));
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (ctx, output);
        }
    }
}
impl Drawer {
    pub(super) fn pointer_settings(&mut self, ctx: &egui::Context) {
        if !self.native_input.open {
            return;
        }
        let mut draft = self
            .native_input
            .draft
            .unwrap_or(self.workspaces.store.current.pointer);
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Mouse and pen input").open(&mut open).show(ctx,|ui| {
            ui.label(&self.native_input.report);
            ui.label("The first drawing contact owns a stroke until release or cancellation.");
            if let Some(source)=self.drawing.last_source { ui.label(format!("Last drawing: {:?}, session device {}, contact {}",source.kind,source.device,source.contact)); }
            for (label,s) in [("Mouse",&mut draft.mouse),("Pen",&mut draft.pen)] {
                ui.separator();ui.label(label);
                crate::help::checkbox(ui,&mut s.enabled,"Enable drawing","pointer-input");
                let mut override_filter=s.stabilization.is_some();
                if crate::help::checkbox(ui,&mut override_filter,"Override tool stabilization","pointer-input").changed() {
                    s.stabilization=override_filter.then_some(0.0);
                }
                if let Some(value)=&mut s.stabilization { super::edit::slider(ui,value,0.0..=0.95,"Stabilization","pointer-input"); }
            }
            ui.label("Touch and untyped contacts do not draw. Mouse ID 0 is the GUI aggregate; pen IDs are valid for this session only.");
            let a=crate::help::button(ui,"Apply device input","pointer-input");
            let c=crate::help::button(ui,"Cancel device input","pointer-input");
            apply=a.clicked();cancel=c.clicked();
            #[cfg(test)] {self.native_input.buttons=[Some(a.rect),Some(c.rect)];}
            if crate::help::button(ui,"Reset draft","pointer-input").clicked(){draft=Default::default();}
        });
        if !open || cancel {
            self.native_input.open = false;
            self.native_input.draft = None;
        } else if apply {
            let mut s = self.capture_workspace();
            s.pointer = draft;
            self.stage_workspace(s, self.workspaces.store.active.clone());
            match self.apply_workspace(
                drawer_core::workspaces::Scope {
                    other: true,
                    ..Default::default()
                },
                None,
            ) {
                Ok(()) => {
                    self.native_input.open = false;
                    self.native_input.draft = None;
                }
                Err(e) => {
                    self.status = Some(e);
                    self.native_input.draft = Some(draft);
                    self.workspaces.staged = None;
                }
            }
        } else {
            self.native_input.draft = Some(draft);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_settings_cancel_apply_backup_restart_leave_document_unchanged() {
        use crate::tests::{TestDirectory, click, frame, tiny_spec};
        let dir = TestDirectory::new();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None).with_storage(UserStorage::at(dir.0.clone()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        let initial = app.workspaces.store.current.pointer;
        let mut next = initial;
        next.mouse.enabled = false;
        next.pen.stabilization = Some(0.7);
        app.native_input.open = true;
        app.native_input.draft = Some(next);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let pos = app.native_input.buttons[1].unwrap();
        click(&ctx, &mut app, pos);
        assert_eq!(app.workspaces.store.current.pointer, initial);
        app.native_input.open = true;
        app.native_input.draft = Some(next);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let pos = app.native_input.buttons[0].unwrap();
        click(&ctx, &mut app, pos);
        assert!(!app.native_input.open);
        assert_eq!(app.workspaces.store.current.pointer, next);
        assert_eq!(
            drawer_core::workspaces::Snapshot::read(
                &dir.0.join("settings/workspace-before-apply.json")
            )
            .unwrap()
            .pointer,
            initial
        );
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(dir.0.clone()).unwrap());
        assert_eq!(restarted.workspaces.store.current.pointer, next);
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
    #[test]
    fn untyped_gui_touch_retains_ids_and_promoted_sequence_is_not_a_mouse() {
        let ctx = egui::Context::default();
        let p = egui::pos2(10.0, 20.0);
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::Touch {
                        device_id: egui::TouchDeviceId(97),
                        id: egui::TouchId(44),
                        phase: egui::TouchPhase::Start,
                        pos: p,
                        force: Some(0.5),
                    },
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| {
                let e = gui_events(ui.ctx());
                assert_eq!(e.len(), 2);
                assert_eq!(
                    e[0].source,
                    Source {
                        kind: Kind::Unknown,
                        device: 97,
                        contact: 44
                    }
                );
                assert_eq!(e[0].pressure, Some(0.5));
                assert_eq!(e[1].source, Source::MOUSE);
                assert_eq!(e[1].phase, Phase::Up);
            },
        );
        out.textures_delta.clear();
    }
}
