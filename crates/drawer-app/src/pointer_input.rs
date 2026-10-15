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
                    attributes: Default::default(),
                    origin: Default::default(),
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
                attributes: Default::default(),
                origin: Default::default(),
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
                attributes: Default::default(),
                origin: Default::default(),
            }),
            egui::Event::PointerGone => result.push(Event {
                source,
                phase: Phase::Cancel,
                position: [0.0; 2],
                pressure: None,
                attributes: Default::default(),
                origin: Default::default(),
            }),
            _ => {}
        }
        n += 1;
    }
    result
}
#[derive(Default)]
pub(super) struct Native {
    pub report: String,
    pub open: bool,
    draft: Option<Settings>,
    pub applied: Option<drawer_core::pointer_input::Backend>,
    pub retry: bool,
    #[cfg(test)]
    buttons: [Option<egui::Rect>; 2],
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
        egui::Window::new("Mouse and pen input").open(&mut open).vscroll(true).show(ctx,|ui| {
            let a=crate::help::button(ui,"Apply device input","pointer-input");
            let c=crate::help::button(ui,"Cancel device input","pointer-input");
            apply=a.clicked();cancel=c.clicked();
            #[cfg(test)] {self.native_input.buttons=[Some(a.rect),Some(c.rect)];}
            ui.label(&self.native_input.report);
            egui::ComboBox::from_label("Pen input backend").selected_text(format!("{:?}",draft.backend)).show_ui(ui,|ui| {
                for backend in drawer_core::pointer_input::Backend::ALL {ui.selectable_value(&mut draft.backend,backend,format!("{backend:?}"));}
            });
            if crate::help::button(ui,"Retry current backend","pointer-input").clicked(){self.native_input.retry=true;}
            ui.label("One pen backend is active. Wintab requires a locally installed driver. Lift the pen after switching or regaining focus.");
            ui.collapsing("Pen click, coordinates and buttons",|ui| {
                use drawer_core::pen_controls::{Action,Mode};
                let mut threshold=draft.controls.click_pressure.is_some();
                if crate::help::checkbox(ui,&mut threshold,"Use click pressure threshold","pointer-input").changed(){draft.controls.click_pressure=threshold.then_some(0.1);}
                if let Some(value)=&mut draft.controls.click_pressure {super::edit::slider(ui,value,0.0..=1.0,"Click pressure","pointer-input");}
                egui::ComboBox::from_label("Pen coordinates").selected_text(format!("{:?}",draft.controls.mode)).show_ui(ui,|ui| {
                    ui.selectable_value(&mut draft.controls.mode,Mode::Absolute,"Absolute");ui.selectable_value(&mut draft.controls.mode,Mode::Relative,"Relative");
                });
                super::edit::slider(ui,&mut draft.controls.speed,0.05..=10.0,"Relative speed","pointer-input");
                ui.label("Relative mode uses a drawing cursor, starting at the canvas center and continuing from its last location. It does not move the system cursor. Missing pressure uses 1. Threshold crossings begin/end strokes.");
                ui.label(format!("Last numbered button bits: {:?}",self.drawing.last_attributes.buttons));
                ui.label("Button numbers are reported logical IDs and may differ between backends. Pointer exposes its barrel as 1. Wintab excludes the tip when identified. Ignore wins over eraser; mappings are frozen when drawing starts.");
                egui::ScrollArea::vertical().id_salt("pen-button-map").max_height(140.0).show(ui,|ui|{
                    for (n,action) in draft.controls.buttons.iter_mut().enumerate() {ui.push_id(n,|ui| {
                        egui::ComboBox::from_label(format!("Pen button {}",n+1)).selected_text(format!("{action:?}")).show_ui(ui,|ui| {for value in Action::ALL {ui.selectable_value(action,value,format!("{value:?}"));}});
                    });}
                });
            });
            ui.label("The first drawing contact owns a stroke until release or cancellation.");
            ui.collapsing("Touch and palm policy",|ui| {
                crate::help::checkbox(ui,&mut draft.touch.gestures,"Two-finger pan, zoom and rotation","pointer-input");
                crate::help::checkbox(ui,&mut draft.touch.finger_drawing,"Enable single-finger drawing","pointer-input");
                crate::help::checkbox(ui,&mut draft.touch.reject_palms,"Reject OS-marked palms","pointer-input");
                ui.label("Touch is ignored during pen contact. Switching from finger drawing to a gesture cancels its pending ink. After a gesture, view/settings change or pen contact, lift all fingers before drawing again. Original UI touch clicks remain available.");
            });
            ui.collapsing("Tablet area, target and touch offset",|ui| {
                use drawer_core::tablet_mapping::Target;
                crate::help::checkbox(ui,&mut draft.mapping.enabled,"Map reported tablet area","pointer-input");
                for n in 0..2 {ui.push_id(n,|ui| {
                    super::edit::slider(ui,&mut draft.mapping.source.min[n],0.0..=1.0,if n==0 {"Area left"}else{"Area top"},"pointer-input");
                    super::edit::slider(ui,&mut draft.mapping.source.max[n],0.0..=1.0,if n==0 {"Area right"}else{"Area bottom"},"pointer-input");
                });}
                egui::ComboBox::from_label("Mapping target").selected_text(format!("{:?}",draft.mapping.target)).show_ui(ui,|ui| {
                    ui.selectable_value(&mut draft.mapping.target,Target::Canvas,"Active canvas");
                    for monitor in &self.display.monitors {if monitor.valid() {ui.selectable_value(&mut draft.mapping.target,Target::Monitor{position:monitor.position,size:monitor.size},&monitor.key);}}
                });
                super::edit::slider(ui,&mut draft.mapping.touch_offset[0],-4096.0..=4096.0,"Touch X offset (physical px)","pointer-input");
                super::edit::slider(ui,&mut draft.mapping.touch_offset[1],-4096.0..=4096.0,"Touch Y offset (physical px)","pointer-input");
                ui.label("Area fractions run from 0 to 1. Native device area is required; missing data cancels drawing. Monitor geometry is saved and must be selected again after removal/layout changes. Relative mode and speed apply after mapping. Touch offset does not enable finger drawing.");
                ui.label(format!("Last reported tablet position: {:?}",self.drawing.last_attributes.tablet));
                if let Some(report)=&self.drawing.mapping_report {ui.label(report);}
                ui.label("Mouse simulation only: move over this tablet preview. Open drawings are unchanged. Physical tablet validation remains pending.");
                let (rect,response)=ui.allocate_exact_size(egui::vec2(220.0,90.0),egui::Sense::hover());
                ui.painter().rect_stroke(rect,0.0,egui::Stroke::new(1.0,egui::Color32::GRAY),egui::StrokeKind::Inside);
                let source=egui::Rect::from_min_max(rect.min+rect.size()*egui::vec2(draft.mapping.source.min[0] as f32,draft.mapping.source.min[1] as f32),rect.min+rect.size()*egui::vec2(draft.mapping.source.max[0] as f32,draft.mapping.source.max[1] as f32));
                if draft.mapping.valid(){ui.painter().rect_stroke(source,0.0,egui::Stroke::new(1.0,egui::Color32::LIGHT_BLUE),egui::StrokeKind::Inside);}
                if let Some(pos)=response.hover_pos(){
                    let uv=[((pos.x-rect.min.x)/rect.width()) as f64,((pos.y-rect.min.y)/rect.height()) as f64];
                    let e=Event{source:Source{kind:Kind::Pen,device:0,contact:0},phase:Phase::Move,position:[0.0;2],pressure:None,attributes:drawer_core::pointer_input::Attributes{tablet:Some(uv),..Default::default()},origin:Default::default()};
                    let mut settings=draft.mapping;settings.enabled=true;
                    ui.label(match self.drawing.mapping_context.map(settings,e){Ok(e)=>format!("Virtual tablet {uv:?} → window {:?}",e.position),Err(e)=>e.message().into()});
                }
            });
            crate::help::checkbox(ui,&mut draft.features.tilt_angle,"Use supported pen tilt for tip angle","pointer-input");
            crate::help::checkbox(ui,&mut draft.features.eraser,"Start inverted pen strokes as eraser","pointer-input");
            crate::help::checkbox(ui,&mut draft.features.button_eraser,"Start as eraser while any reported pen button is down","pointer-input");
            ui.label("Eraser actions keep the current brush shape and pressure. The pen backend may include the nib among its buttons; button mapping is off by default.");
            if let Some(source)=self.drawing.last_source { ui.label(format!("Last drawing: {:?}, session device {}, contact {}",source.kind,source.device,source.contact)); }
            ui.label(format!("Reported tilt {:?}; pen eraser {:?}; pen button {:?}",self.drawing.last_attributes.tilt,self.drawing.last_attributes.eraser,self.drawing.last_attributes.button));
            for (label,s) in [("Mouse",&mut draft.mouse),("Pen",&mut draft.pen)] {
                ui.separator();ui.label(label);
                crate::help::checkbox(ui,&mut s.enabled,"Enable drawing","pointer-input");
                let mut override_filter=s.stabilization.is_some();
                if crate::help::checkbox(ui,&mut override_filter,"Override tool stabilization","pointer-input").changed() {
                    s.stabilization=override_filter.then_some(0.0);
                }
                if let Some(value)=&mut s.stabilization { super::edit::slider(ui,value,0.0..=0.95,"Stabilization","pointer-input"); }
            }
            ui.label("Untyped contacts never draw. Finger drawing uses its explicit touch option. Mouse ID 0 is the GUI aggregate; pen IDs are valid for this session only.");
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

#[cfg(windows)]
pub(super) fn smoke_backends(input: &drawer_platform::Input) -> Result<String, String> {
    use drawer_core::pointer_input::Backend;
    input.configure(Backend::Wintab);
    let wintab = input.report();
    if input.requested() != Some(Backend::Wintab) {
        return Err("Backend preference lost".into());
    }
    input.configure(Backend::Mouse);
    if input.effective() != Backend::Mouse {
        return Err("Mouse backend did not release pen provider".into());
    }
    input.configure(Backend::Pointer);
    if input.effective() != Backend::Pointer {
        return Err(input.report());
    }
    Ok(wintab)
}

pub(super) fn smoke_controls() -> Result<(), String> {
    use drawer_core::{
        input_replay::Sequence,
        pen_controls::{Action, Mode},
    };
    let run = |s: &Sequence| {
        crate::input_replay::run(
            s,
            &std::sync::atomic::AtomicBool::new(false),
            &std::sync::atomic::AtomicUsize::new(0),
        )
    };
    let mut s = Sequence {
        size: 2.0,
        ..Default::default()
    };
    s.pressure.tools[0].size = false;
    s.pressure.tools[0].opacity = false;
    s.pointer.controls.click_pressure = Some(0.5);
    let threshold = run(&s)?;
    if !threshold.record.passed
        || threshold.pixels[20 * 64 + 8][3] != 0
        || threshold.pixels[20 * 64 + 24][3] == 0
    {
        return Err("Click pressure did not gate the actual drawing".into());
    }
    s.pointer.controls.click_pressure = None;
    s.pointer.controls.mode = Mode::Relative;
    s.pointer.controls.speed = 0.5;
    let relative = run(&s)?;
    if !relative.record.passed
        || relative.pixels[24 * 64 + 32][3] == 0
        || relative.pixels[20 * 64 + 8][3] != 0
    {
        return Err("Relative pen cursor did not reach the drawing".into());
    }
    s.pointer.controls.mode = Mode::Absolute;
    s.pointer.features.eraser = false;
    s.background = [200, 80, 40, 255];
    s.pointer.controls.buttons[0] = Action::Eraser;
    for sample in &mut s.samples {
        sample.event.attributes.buttons = Some(1);
    }
    let eraser = run(&s)?;
    if !eraser.record.passed || eraser.pixels.iter().all(|p| p[3] == 255) {
        return Err("Numbered pen eraser mapping did not remove pixels".into());
    }
    s.pointer.controls.buttons[0] = Action::Ignore;
    s.expected_strokes = 0;
    let ignore = run(&s)?;
    if !ignore.record.passed || ignore.pixels.iter().any(|p| *p != s.background) {
        return Err("Ignored pen button changed the document".into());
    }
    Ok(())
}
pub(super) fn smoke_mapping() -> Result<(), String> {
    use drawer_core::{input_replay::Sequence, scene::Rect};
    let run = |s: &Sequence| {
        crate::input_replay::run(
            s,
            &std::sync::atomic::AtomicBool::new(false),
            &std::sync::atomic::AtomicUsize::new(0),
        )
    };
    let mut s = Sequence::default();
    s.pointer.mapping.enabled = true;
    s.pointer.mapping.source = Rect {
        min: [0.25; 2],
        max: [0.75; 2],
    };
    for sample in &mut s.samples {
        sample.event.attributes.tablet = Some([0.5; 2]);
        sample.event.pressure = Some(1.0);
    }
    let mapped = run(&s)?;
    if !mapped.record.passed
        || mapped.pixels[24 * 64 + 32][3] == 0
        || mapped.pixels[20 * 64 + 8][3] != 0
    {
        return Err("Tablet mapping did not reach canvas pixels".into());
    }
    s.samples[1].event.attributes.tablet = Some([0.9; 2]);
    s.expected_strokes = 0;
    let outside = run(&s)?;
    if !outside.record.passed || outside.pixels.iter().any(|p| p[3] != 0) {
        return Err("Leaving/reentering tablet area committed a partial stroke".into());
    }
    for sample in &mut s.samples {
        sample.event.attributes.tablet = None;
    }
    let missing = run(&s)?;
    if !missing.record.passed || missing.pixels.iter().any(|p| p[3] != 0) {
        return Err("Missing device area was guessed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapping_replay_reaches_pixels_and_outside_or_missing_area_cannot_start_a_stroke() {
        smoke_mapping().unwrap();
    }
    #[test]
    fn pressure_relative_coordinates_and_numbered_buttons_reach_actual_pixels() {
        smoke_controls().unwrap();
    }
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
        next.backend = drawer_core::pointer_input::Backend::Wintab;
        next.controls.click_pressure = Some(0.4);
        next.controls.mode = drawer_core::pen_controls::Mode::Relative;
        next.controls.speed = 0.5;
        next.controls.buttons[2] = drawer_core::pen_controls::Action::Eraser;
        next.mapping.source.min = [0.1, 0.2];
        next.mapping.source.max = [0.9, 0.8];
        next.mapping.enabled = true;
        next.mapping.touch_offset = [2.0, -3.0];
        next.touch.finger_drawing = true;
        next.touch.gestures = false;
        next.touch.reject_palms = false;
        next.mouse.enabled = false;
        next.pen.stabilization = Some(0.7);
        next.features.tilt_angle = true;
        next.features.eraser = false;
        next.features.button_eraser = true;
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
