use super::*;
use drawer_core::precision::{Nudge, Target};
use std::collections::HashSet;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Paint,
    Move,
    Eyedropper,
    SelectRectangle,
    SelectLasso,
}
#[derive(Default)]
pub(super) struct Precision {
    pub target: Target,
    nudge: Option<Nudge>,
    held: HashSet<egui::Key>,
    blocked: HashSet<egui::Key>,
    context: Option<(u64, usize, Target)>,
}
impl Precision {
    pub fn finish(&mut self) {
        self.nudge = None;
        self.blocked.extend(self.held.iter().copied());
    }
}
fn arrow(key: egui::Key) -> Option<[i64; 2]> {
    Some(match key {
        egui::Key::ArrowLeft => [-1, 0],
        egui::Key::ArrowRight => [1, 0],
        egui::Key::ArrowUp => [0, -1],
        egui::Key::ArrowDown => [0, 1],
        _ => return None,
    })
}
impl Drawer {
    pub(super) fn precision_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (label, mode) in [
                ("Paint", Mode::Paint),
                ("Move", Mode::Move),
                ("Eyedropper", Mode::Eyedropper),
                ("Rectangle select", Mode::SelectRectangle),
                ("Lasso select", Mode::SelectLasso),
            ] {
                if ui
                    .selectable_label(self.drawing.mode == mode, label)
                    .clicked()
                {
                    self.finish_stroke();
                    self.restore_locked_tool();
                    self.drawing.mode = mode;
                    self.drawing.connect_line = false;
                }
            }
        });
        if self.drawing.mode == Mode::Eyedropper {
            self.eyedropper_controls(ui);
        }
        ui.horizontal_wrapped(|ui| {
            for (command, label) in [
                ("move-selection-boundary", "Move boundary"),
                ("move-selected-pixels", "Move selected pixels"),
                ("copy-selection", "Copy selected pixels"),
                ("paste-selection", "Paste to new layer…"),
            ] {
                if ui.button(label).clicked() {
                    self.execute_command(ui.ctx(), command);
                }
            }
        });
        ui.collapsing("Pick a visible layer on canvas",|ui|{
            let settings=&mut self.workspaces.store.current.layer_pick;
            egui::ComboBox::from_id_salt("layer-pick-modifier").selected_text(format!("{:?} + click",settings.modifier)).show_ui(ui,|ui|{for m in [drawer_core::layer_pick::Modifier::Alt,drawer_core::layer_pick::Modifier::Ctrl,drawer_core::layer_pick::Modifier::Shift]{ui.selectable_value(&mut settings.modifier,m,format!("{m:?}"));}});
            ui.checkbox(&mut settings.include_locked,"Include locked layers (keeps locks)");ui.checkbox(&mut settings.folders,"Pick folders separately");
            if ui.button("Next overlapping candidate").clicked(){self.execute_command(ui.ctx(),"pick-next-layer");}
            if let Some(n)=self.active {let d=&self.documents[n];for target in drawer_core::layer_pick::visible(d,self.workspaces.store.current.layer_pick){if ui.selectable_label(self.panels.targets.get(&d.id)==Some(&target),drawer_core::layer_pick::label(d,target)).clicked(){self.drawing.finish(&mut self.documents);self.panels.targets.insert(self.documents[n].id,target);break;}}}
            ui.label("Use the modifier with Move/selection tools. Empty space keeps the current layer. Raster drag uses one Undo; locked/non-raster targets remain selected without bypassing editing restrictions.");
        });
        if ui.button("Page selections…").clicked() {
            self.execute_command(ui.ctx(), "selection-tools");
        }
        if ui.button("Line references and vector layers…").clicked() {
            self.finish_stroke();
            self.lines.open = true;
        }
        let old = self.precision.target;
        egui::ComboBox::from_id_salt("keyboard-move-target")
            .selected_text(self.precision.target.label())
            .show_ui(ui, |ui| {
                for target in Target::ALL {
                    ui.selectable_value(&mut self.precision.target, target, target.label());
                }
            });
        if old != self.precision.target {
            self.finish_stroke();
        }
        ui.label("Arrows: 1 document pixel · Shift: 10 · Ctrl-drag: fine values");
        if self.drawing.mode == Mode::Paint {
            self.tool_lock_controls(ui);
            self.clone_controls(ui);
            let n = BrushTip::ALL
                .iter()
                .position(|t| *t == self.selected)
                .unwrap();
            ui.add_enabled_ui(
                self.user_tool_supports(drawer_core::tool_properties::Property::Opacity),
                |ui| {
                    super::edit::slider(
                        ui,
                        &mut self.panels.settings.brush_opacity[n],
                        0.0..=1.0,
                        "Brush opacity",
                        "tool-properties",
                    );
                },
            );
            if ui
                .selectable_label(
                    self.drawing.connect_line,
                    "Connect last stroke to next click (L)",
                )
                .clicked()
            {
                self.execute_command(ui.ctx(), "connect-line");
            }
        } else {
            ui.label("This tool has no brush size or opacity");
        }
    }
    pub(super) fn precision_inputs(&mut self, ctx: &egui::Context) {
        let focus_transition = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::WindowFocused(_)))
        });
        if ctx.input(|i| {
            !i.focused
                || i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        }) {
            self.precision.nudge = None;
            self.precision.held.clear();
            self.precision.blocked.clear();
        }
        let context = self.active.and_then(|n| {
            let doc = &self.documents[n];
            match self
                .panels
                .targets
                .get(&doc.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
            {
                drawer_core::thumbnail::Target::Raster(layer) => {
                    Some((doc.id, layer, self.precision.target))
                }
                _ => None,
            }
        });
        let blocked = focus_transition
            || self.tool_shortcuts_blocked(ctx)
            || !self.workspaces.is_focused()
            || ctx.input(|i| !i.focused);
        if context != self.precision.context || blocked {
            self.precision.finish();
            self.precision.context = context;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !super::preferences::composing(ctx) {
            self.drawing.connect_line = false;
            self.precision.finish();
        }
        let events = ctx.input(|i| i.events.clone());
        let mut consumed = HashSet::new();
        for event in events {
            let egui::Event::Key {
                key,
                pressed,
                modifiers,
                repeat,
                ..
            } = event
            else {
                continue;
            };
            let Some(delta) = arrow(key) else {
                if pressed {
                    self.precision.finish();
                }
                continue;
            };
            if !pressed {
                self.precision.held.remove(&key);
                self.precision.blocked.remove(&key);
                if self.precision.held.is_empty() {
                    self.precision.nudge = None;
                }
                continue;
            }
            if repeat && !self.precision.held.contains(&key) {
                self.precision.blocked.insert(key);
                continue;
            }
            if !repeat && !self.precision.held.contains(&key) {
                self.precision.blocked.remove(&key);
            }
            self.precision.held.insert(key);
            if blocked || modifiers.ctrl || modifiers.alt {
                self.precision.blocked.insert(key);
                continue;
            }
            if self.precision.blocked.contains(&key) {
                continue;
            }
            consumed.insert(key);
            let Some((id, layer, target)) = context else {
                self.status = Some("Select a raster layer to move".into());
                continue;
            };
            let n = self.documents.iter().position(|d| d.id == id).unwrap();
            if self
                .precision
                .nudge
                .as_ref()
                .is_some_and(|s| !s.matches(&self.documents[n], layer, target))
            {
                self.precision.finish();
                if repeat {
                    continue;
                }
                self.precision.blocked.remove(&key);
            }
            if self.precision.nudge.is_none() {
                self.drawing.finish(&mut self.documents);
                match Nudge::begin(&self.documents[n], layer, target) {
                    Ok(s) => self.precision.nudge = Some(s),
                    Err(e) => {
                        self.status = Some(e);
                        self.precision.blocked.insert(key);
                        continue;
                    }
                }
            }
            let step = if modifiers.shift { 10 } else { 1 };
            if let Err(e) = self
                .precision
                .nudge
                .as_mut()
                .unwrap()
                .step(&mut self.documents[n], delta.map(|v| v * step))
            {
                self.status = Some(e);
                self.precision.finish();
            }
        }
        ctx.input_mut(|i| {
            i.events.retain(
                |e| !matches!(e,egui::Event::Key{key,pressed:true,..} if consumed.contains(key)),
            )
        });
    }
}
fn key(key: egui::Key, pressed: bool, repeat: bool, shift: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers: if shift {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        },
    }
}
pub(super) fn pointer(
    ctx: &egui::Context,
    app: &Drawer,
    point: [f64; 2],
    phase: drawer_core::pointer_input::Phase,
) -> drawer_core::pointer_input::Event {
    let doc = &app.documents[app.active.unwrap()];
    let rect = app.canvas_view.viewport_rect().unwrap();
    let p = doc.view.document_to_screen(
        point,
        [doc.width as f64, doc.height as f64],
        [rect.center().x as f64, rect.center().y as f64],
        ctx.pixels_per_point() as f64,
    );
    drawer_core::pointer_input::Event {
        source: drawer_core::pointer_input::Source::MOUSE,
        phase,
        position: p,
        pressure: None,
        attributes: Default::default(),
        origin: Default::default(),
    }
}
pub(super) fn smoke() -> Result<(), String> {
    use super::input_replay::frame;
    use drawer_core::pointer_input::Phase;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 48,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.panels.settings.brush_size = 1.0;
    app.workspaces.store.current.input.smoothing = [0.0; 7];
    frame(&ctx, &mut app, 0.0, vec![]);
    frame(&ctx, &mut app, 0.1, vec![]);
    let initial = app.documents[0].snapshot();
    frame(
        &ctx,
        &mut app,
        0.2,
        vec![key(egui::Key::ArrowRight, true, false, false)],
    );
    frame(
        &ctx,
        &mut app,
        0.3,
        vec![key(egui::Key::ArrowRight, true, true, false)],
    );
    frame(
        &ctx,
        &mut app,
        0.4,
        vec![key(egui::Key::ArrowRight, false, false, false)],
    );
    if app.documents[0].history_counts() != (1, 0)
        || app.documents[0].layers[0].extent(64, 48).offset != [2, 0]
    {
        return Err("Continuous keyboard movement did not group one document-space command".into());
    }
    app.documents[0].undo();
    if app.documents[0].snapshot() != initial {
        return Err("Nudge undo did not restore the original".into());
    }
    let stroke = vec![
        pointer(&ctx, &app, [12.5, 12.5], Phase::Down),
        pointer(&ctx, &app, [24.5, 12.5], Phase::Up),
    ];
    app.drawing.virtual_events(stroke, false);
    frame(&ctx, &mut app, 0.5, vec![]);
    let first = app.documents[0].snapshot();
    if app.documents[0].history_counts() != (1, 0)
        || app.documents[0].composite_pixel(20, 12)[3] == 0
    {
        return Err("Starting stroke was not committed".into());
    }
    app.execute_command(&ctx, "connect-line");
    let stroke = vec![
        pointer(&ctx, &app, [40.5, 12.5], Phase::Down),
        pointer(&ctx, &app, [40.5, 12.5], Phase::Up),
    ];
    app.drawing.virtual_events(stroke, false);
    frame(&ctx, &mut app, 0.6, vec![]);
    if app.documents[0].history_counts() != (2, 0)
        || app.documents[0].composite_pixel(32, 12)[3] == 0
    {
        return Err("Connected line was not one straight committed command".into());
    }
    let second = app.documents[0].snapshot();
    app.documents[0].undo();
    if app.documents[0].snapshot() != first {
        return Err("Line undo changed the previous stroke".into());
    }
    app.documents[0].redo();
    if app.documents[0].snapshot() != second {
        return Err("Line redo differs".into());
    }
    app.set_tool(BrushTip::Pencil);
    app.execute_command(&ctx, "connect-line");
    let stroke = vec![
        pointer(&ctx, &app, [40.5, 20.5], Phase::Down),
        pointer(&ctx, &app, [40.5, 20.5], Phase::Up),
    ];
    app.drawing.virtual_events(stroke, false);
    frame(&ctx, &mut app, 0.7, vec![]);
    if app.documents[0].snapshot() != second
        || app
            .status
            .as_ref()
            .is_none_or(|e| !e.contains("starting stroke"))
    {
        return Err(format!(
            "Missing per-tool line endpoint was not rejected: unchanged={}, status={:?}",
            app.documents[0].snapshot() == second,
            app.status
        ));
    }
    app.panels.settings.brush_opacity[1] = 0.0;
    let stroke = vec![
        pointer(&ctx, &app, [20.5, 24.5], Phase::Down),
        pointer(&ctx, &app, [40.5, 24.5], Phase::Up),
    ];
    app.drawing.virtual_events(stroke, false);
    frame(&ctx, &mut app, 0.8, vec![]);
    if app.documents[0].snapshot() != second || app.documents[0].history_counts() != (2, 0) {
        return Err("Zero opacity changed pixels/history".into());
    }
    let color = app.documents[0].composite_pixel(20, 12);
    app.execute_command(&ctx, "Eyedropper");
    app.references.session.workspace.color = [255, 0, 0, 255];
    let stroke = vec![
        pointer(&ctx, &app, [20.5, 12.5], Phase::Down),
        pointer(&ctx, &app, [20.5, 12.5], Phase::Up),
    ];
    app.drawing.virtual_events(stroke, false);
    frame(&ctx, &mut app, 0.9, vec![]);
    frame(&ctx, &mut app, 1.0, vec![]);
    if app.references.session.workspace.color != color || app.documents[0].snapshot() != second {
        return Err("Eyedropper did not sample without painting".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::super::{
        input_replay::frame,
        tests::{TestDirectory, tiny_spec},
    };
    use super::*;
    #[test]
    fn keyboard_line_opacity_and_eyedropper_use_production_pixel_history_path() {
        smoke().unwrap();
    }
    #[test]
    fn held_movement_release_target_change_focus_and_modal_input_never_stick() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, 0.0, vec![]);
        frame(&ctx, &mut app, 0.1, vec![]);
        frame(
            &ctx,
            &mut app,
            0.2,
            vec![key(egui::Key::ArrowRight, true, false, false)],
        );
        app.precision.target = Target::Boundary;
        frame(
            &ctx,
            &mut app,
            0.3,
            vec![key(egui::Key::ArrowRight, true, true, false)],
        );
        assert_eq!(app.documents[0].layers[0].extent(1, 1).offset, [1, 0]);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        frame(
            &ctx,
            &mut app,
            0.4,
            vec![key(egui::Key::ArrowRight, false, false, false)],
        );
        app.precision.target = Target::Layer;
        frame(
            &ctx,
            &mut app,
            0.5,
            vec![key(egui::Key::ArrowDown, true, false, true)],
        );
        frame(
            &ctx,
            &mut app,
            0.6,
            vec![key(egui::Key::ArrowDown, false, false, true)],
        );
        assert_eq!(app.documents[0].layers[0].extent(1, 1).offset, [1, 10]);
        assert_eq!(app.documents[0].history_counts(), (2, 0));
        frame(&ctx, &mut app, 0.7, vec![egui::Event::WindowFocused(false)]);
        frame(
            &ctx,
            &mut app,
            0.8,
            vec![
                egui::Event::WindowFocused(true),
                key(egui::Key::ArrowRight, true, true, false),
            ],
        );
        assert_eq!(app.documents[0].layers[0].extent(1, 1).offset, [1, 10]);
        frame(
            &ctx,
            &mut app,
            0.9,
            vec![key(egui::Key::ArrowRight, false, false, false)],
        );
        app.controls.open = true;
        frame(
            &ctx,
            &mut app,
            1.0,
            vec![key(egui::Key::ArrowRight, true, false, false)],
        );
        frame(
            &ctx,
            &mut app,
            1.05,
            vec![key(egui::Key::Escape, true, false, false)],
        );
        assert!(!app.controls.open);
        frame(
            &ctx,
            &mut app,
            1.1,
            vec![key(egui::Key::ArrowRight, true, true, false)],
        );
        assert_eq!(app.documents[0].history_counts(), (2, 0));
        frame(
            &ctx,
            &mut app,
            1.2,
            vec![key(egui::Key::ArrowRight, false, false, false)],
        );
        app.documents[0].layers[0].locked = true;
        let before = app.documents[0].snapshot();
        frame(
            &ctx,
            &mut app,
            1.3,
            vec![key(egui::Key::ArrowLeft, true, false, false)],
        );
        assert!(app.documents[0].snapshot() == before);
        assert!(app.status.as_ref().unwrap().contains("locked"));
    }
    #[test]
    fn tool_values_limits_shortcuts_unsupported_modes_and_restart_are_separate_from_document() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, 0.0, vec![]);
        frame(&ctx, &mut app, 0.1, vec![]);
        let before = app.documents[0].snapshot();
        frame(
            &ctx,
            &mut app,
            0.2,
            vec![key(egui::Key::Num5, true, false, false)],
        );
        frame(
            &ctx,
            &mut app,
            0.3,
            vec![key(egui::Key::Num5, false, false, false)],
        );
        assert_eq!(app.panels.settings.brush_opacity[..4], [0.5, 1.0, 1.0, 0.6]);
        app.panels.settings.brush_size = 0.1;
        app.execute_command(&ctx, "brush-shrink");
        assert_eq!(app.panels.settings.brush_size, 0.1);
        app.execute_command(&ctx, "brush-grow");
        assert_eq!(app.panels.settings.brush_size, 0.2);
        app.panels.settings.brush_size = 4096.0;
        app.execute_command(&ctx, "brush-grow");
        assert_eq!(app.panels.settings.brush_size, 4096.0);
        app.execute_command(&ctx, "opacity-0");
        app.execute_command(&ctx, "opacity-down");
        assert_eq!(app.panels.settings.brush_opacity[0], 0.0);
        app.execute_command(&ctx, "opacity-100");
        app.execute_command(&ctx, "opacity-up");
        assert_eq!(app.panels.settings.brush_opacity[0], 1.0);
        for mode in ["Move", "Eyedropper"] {
            app.execute_command(&ctx, mode);
            let size = app.panels.settings.brush_size;
            app.execute_command(&ctx, "brush-shrink");
            app.execute_command(&ctx, "opacity-0");
            assert_eq!(app.panels.settings.brush_size, size);
            assert_eq!(app.panels.settings.brush_opacity[0], 1.0);
            assert!(app.status.as_ref().unwrap().contains("does not support"));
        }
        frame(
            &ctx,
            &mut app,
            0.4,
            vec![key(egui::Key::B, true, false, false)],
        );
        frame(
            &ctx,
            &mut app,
            0.5,
            vec![key(egui::Key::B, false, false, false)],
        );
        assert_eq!(app.drawing.mode, Mode::Paint);
        app.execute_command(&ctx, "opacity-30");
        app.set_tool(BrushTip::Pencil);
        app.execute_command(&ctx, "opacity-70");
        app.persist_workspace().unwrap();
        assert_eq!(app.panels.settings.brush_opacity[..4], [0.3, 0.7, 1.0, 0.6]);
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(
            restarted.panels.settings.brush_opacity,
            app.panels.settings.brush_opacity
        );
    }
    #[test]
    fn text_focus_and_ime_leave_arrow_and_opacity_input_to_the_field() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        let mut text = "123".to_string();
        for composing in [false, true] {
            let mut events = vec![
                key(egui::Key::ArrowRight, true, false, false),
                key(egui::Key::Num1, true, false, false),
            ];
            if composing {
                events.insert(
                    0,
                    egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: "한".into(),
                        active_range_chars: None,
                    }),
                );
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.text_edit_singleline(&mut text).request_focus();
                    app.begin_input_preferences(ui.ctx());
                    app.basic_shortcuts(ui.ctx());
                    app.precision_inputs(ui.ctx());
                },
            );
            output.textures_delta.clear();
        }
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        assert_eq!(
            app.panels.settings.brush_opacity,
            drawer_core::panels::Settings::default().brush_opacity
        );
    }
}
