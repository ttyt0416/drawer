use super::*;
use drawer_core::tool_input::{Controller, Settings};
#[derive(Default)]
pub(super) struct ToolInput {
    pub controller: Controller,
    pub open: bool,
    draft: Option<Settings>,
    modifiers: [bool; 3],
    blocked: [bool; 3],
    #[cfg(test)]
    apply_button: Option<egui::Rect>,
    #[cfg(test)]
    cancel_button: Option<egui::Rect>,
}
impl Drawer {
    fn runtime_tool(&mut self, tool: BrushTip, document: Option<u64>) {
        self.sync_tool_properties();
        if let Some(id) = document {
            if let Some(n) = self.documents.iter().position(|d| d.id == id) {
                self.documents[n].tool = tool;
                if self.active == Some(n) {
                    self.selected = tool;
                }
            }
        } else {
            self.selected = tool;
            if let Some(n) = self.active {
                self.documents[n].tool = tool;
            }
        }
        self.sync_tool_properties();
    }
    pub(super) fn held_tool_inputs(&mut self, ctx: &egui::Context) {
        let document = self.active.map(|n| self.documents[n].id);
        let context_changed = self
            .tool_input
            .controller
            .document()
            .is_some_and(|d| d != document);
        let focus_lost = ctx.input(|i| {
            !i.focused
                || i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        });
        let modifiers = ctx.input(|i| [i.modifiers.ctrl, i.modifiers.shift, i.modifiers.alt]);
        let chord = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
        });
        let modal = self.tool_shortcuts_blocked(ctx);
        let lost = focus_lost || modal;
        let suspend = chord && modifiers.iter().any(|m| *m);
        for (n, down) in modifiers.iter().enumerate() {
            if !down {
                self.tool_input.blocked[n] = false;
            } else if lost || context_changed || suspend {
                self.tool_input.blocked[n] = true;
            }
        }
        if focus_lost {
            ctx.input_mut(|i| {
                i.keys_down.clear();
                i.pointer = Default::default();
                i.modifiers = Default::default();
            });
            ctx.stop_dragging();
        }
        if context_changed || lost || suspend {
            if let Some((tool, id)) = self.tool_input.controller.cancel() {
                self.runtime_tool(tool, id);
                if let Some(storage) = &self.storage
                    && let Err(e) = storage.save_brush(self.selected)
                {
                    self.status = Some(e.to_string());
                }
            }
            self.canvas_view.cancel_input();
        }
        let time = ctx.input(|i| i.time);
        let settings = self.workspaces.store.current.input;
        let releases = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: false,
                        ..
                    } => Some(key.name().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        for key in releases {
            if let Some((tool, id)) = self.tool_input.controller.release(&key, time, settings) {
                self.runtime_tool(tool, id);
                if self.tool_input.controller.persisted_tool().is_none()
                    && let Some(storage) = &self.storage
                    && let Err(e) = storage.save_brush(self.selected)
                {
                    self.status = Some(e.to_string());
                }
            }
        }
        for (n, down) in modifiers.iter().copied().enumerate() {
            let key = ["@Ctrl", "@Shift", "@Alt"][n];
            if !down && self.tool_input.modifiers[n] {
                if let Some((tool, id)) = self.tool_input.controller.release(key, time, settings) {
                    self.runtime_tool(tool, id);
                }
            } else if down
                && !self.tool_input.modifiers[n]
                && !self.tool_input.blocked[n]
                && let Some(tip) = settings.modifiers[n]
                && let Some(tool) = self.tool_input.controller.press_modifier(
                    key,
                    tip,
                    self.selected,
                    document,
                    time,
                )
            {
                self.runtime_tool(tool, document);
            }
        }
        self.tool_input.modifiers = modifiers;
        if self.tool_input.controller.persisted_tool().is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
    pub(super) fn press_tool(
        &mut self,
        ctx: &egui::Context,
        key: egui::Key,
        tip: BrushTip,
        repeat: bool,
    ) {
        if !repeat {
            self.precision.finish();
            self.drawing.mode = super::precision::Mode::Paint;
            self.drawing.connect_line = false;
        }
        let id = self.active.map(|n| self.documents[n].id);
        if let Some(tool) = self.tool_input.controller.press(
            key.name(),
            tip,
            self.selected,
            id,
            ctx.input(|i| i.time),
            repeat,
        ) {
            self.runtime_tool(tool, id);
        }
    }
    pub(super) fn input_settings(&mut self, ctx: &egui::Context) {
        if !self.tool_input.open {
            return;
        }
        let mut draft = self
            .tool_input
            .draft
            .unwrap_or(self.workspaces.store.current.input);
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let mut layer = None;
        egui::Window::new("Tool input behavior").open(&mut open).collapsible(false).vscroll(true).show(ctx,|ui|{
            layer=Some(ui.layer_id());
            ui.horizontal(|ui|{let a=ui.add_enabled(draft.valid()&&!super::edit::invalid_layer(ctx,ui.layer_id()),egui::Button::new("Apply input behavior"));let c=crate::help::button(ui,"Cancel input behavior","modifier-tools");apply=a.clicked();cancel=c.clicked();#[cfg(test)]{self.tool_input.apply_button=Some(a.rect);self.tool_input.cancel_button=Some(c.rect);}});
            super::edit::slider(ui,&mut draft.hold_millis,0..=5000,"Temporary tool threshold (ms)","modifier-tools");
            super::edit::slider(ui,&mut draft.drag_pixels,0.0..=64.0,"Drag threshold (screen pixels)","modifier-tools");
            for (n,tip) in BrushTip::ALL.iter().enumerate(){super::edit::slider(ui,&mut draft.smoothing[n],0.0..=0.95,format!("{} stabilization",tip.label()),"modifier-tools");}
            ui.collapsing("Stroke start / end geometry — all drawing devices",|ui| {
                ui.label("Choose document length, stroke percentage, or a held fade. Geometry remains independent of position and pressure.");
                for(n,tip)in BrushTip::ALL.into_iter().enumerate(){ui.push_id(("stroke-ends",n),|ui| {
                    ui.label(tip.label());super::tip_sequence::ends_ui(ui,&mut draft.ends[n]);
                });}
            });
            ui.collapsing("Color variation and dual tip masks",|ui|{for(n,tip)in BrushTip::ALL.into_iter().enumerate(){ui.push_id(("brush-variation",n),|ui|{ui.collapsing(tip.label(),|ui|super::brush_variation::settings_ui(ui,&mut draft.variations[n]));});}});
            ui.collapsing("Scatter, ribbon and repeating decorations",|ui|{for(n,tip)in BrushTip::ALL.into_iter().enumerate(){ui.push_id(("brush-pattern",n),|ui|{ui.collapsing(tip.label(),|ui|super::brush_pattern::settings_ui(ui,&mut draft.patterns[n]));});}});
            ui.collapsing("Tip direction, size dynamics and spacing",|ui|{for(n,tip)in BrushTip::ALL.into_iter().enumerate(){ui.push_id(("brush-dynamics",n),|ui|{ui.collapsing(tip.label(),|ui|super::brush_dynamics::settings_ui(ui,&mut draft.dynamics[n]));});}});
            ui.collapsing("Multiple local tips and order",|ui|{for(n,tip)in BrushTip::ALL.into_iter().enumerate(){ui.push_id(("tip-sequence",n),|ui|{ui.collapsing(tip.label(),|ui|super::tip_sequence::settings_ui(ui,&mut draft.sequences[n]));});}});
            ui.collapsing("Independent position and device pressure filters",|ui|{for(n,tip)in BrushTip::ALL.into_iter().enumerate(){ui.push_id(("stroke-filter",n),|ui|{ui.collapsing(tip.label(),|ui|super::stroke_filter::settings_ui(ui,&mut draft.filters[n]));});}});
            ui.collapsing("Tip images and minimum outputs",|ui|{for(n,tip)in BrushTip::ALL.into_iter().enumerate(){ui.push_id(("tip-profile",n),|ui|{ui.collapsing(tip.label(),|ui|super::tip_profile::settings_ui(ui,&mut draft.profiles[n],tip));});}});
            ui.collapsing("Brush texture — all drawing devices",|ui| {for(n,tip)in BrushTip::ALL.into_iter().enumerate(){ui.push_id(("brush-texture",n),|ui|{ui.collapsing(tip.label(),|ui|super::texture_panel::pattern_ui(ui,&mut draft.textures[n]));});}});
            ui.collapsing("Brush pigment and water — all drawing devices",|ui| {
                for(n,tip)in BrushTip::ALL.into_iter().enumerate(){ui.push_id(("pigment",n),|ui|{ui.collapsing(tip.label(),|ui|super::brush_material::settings_ui(ui,&mut draft.materials[n],tip));});}
            });
            ui.collapsing("Mouse virtual pressure",|ui| {
                for (n,tip) in BrushTip::ALL.iter().enumerate() {ui.collapsing(tip.label(),|ui|super::mouse_pressure::settings_ui(ui,&mut draft.mouse_pressure[n]));}
            });
            ui.separator();
            ui.label("Hold a modifier to use its tool; release always restores the previous tool.");
            for (n, label) in ["Ctrl", "Shift", "Alt"].into_iter().enumerate() {
                egui::ComboBox::from_id_salt(("modifier-tool",n)).selected_text(format!("{label}: {}",draft.modifiers[n].map_or("Unassigned",|t|t.label()))).show_ui(ui,|ui|{
                    ui.selectable_value(&mut draft.modifiers[n], None,"Unassigned");
                    for tip in BrushTip::ALL { ui.selectable_value(&mut draft.modifiers[n],Some(tip),tip.label()); }
                });
                if draft.modifiers[n].is_some() {
                    let shared = self.workspaces.store.current.bindings.iter().filter(|b| [b.ctrl,b.shift,b.alt][n]).map(|b|b.label()).collect::<Vec<_>>();
                    if !shared.is_empty(){ui.colored_label(egui::Color32::YELLOW,format!("Shared shortcuts: {}. A key chord suspends the modifier tool until release.",shared.join(", ")));}
                }
            }
            ui.label("Input fields, menus and dialogs take priority. After suspension, release the modifier before using it again.");
            ui.label("Short tool keys keep the choice. Hold longer, then release to return. The drag threshold uses physical screen pixels.");
            ui.horizontal_wrapped(|ui|{apply|=ui.add_enabled(draft.valid()&&!super::edit::invalid_layer(ctx,ui.layer_id()),egui::Button::new("Apply input behavior")).clicked();cancel|=crate::help::button(ui,"Cancel input behavior","modifier-tools").clicked();if crate::help::button(ui,"Reset draft","modifier-tools").clicked(){draft=Default::default();super::edit::discard_layer(ctx,ui.layer_id());}
                if ui.add_enabled(self.storage.is_some(),egui::Button::new("Load before-apply backup")).clicked(){match drawer_core::workspaces::Snapshot::read(&self.storage.as_ref().unwrap().root().join("settings/workspace-before-apply.json")){Ok(s)=>{draft=s.input;super::edit::discard_layer(ctx,ui.layer_id());},Err(e)=>self.status=Some(e.to_string())}}
            });
        });
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if let Some(layer) = layer {
                super::edit::discard_layer(ctx, layer);
            }
            self.tool_input.open = false;
            self.tool_input.draft = None;
        } else if apply && layer.is_some_and(|layer| super::edit::invalid_layer(ctx, layer)) {
            self.status = Some("Finish or cancel the invalid numeric input before applying".into());
            self.tool_input.draft = Some(draft);
        } else if apply {
            let mut s = self.capture_workspace();
            s.input = draft;
            if let Some(properties) = &mut s.properties {
                for (tip, value) in BrushTip::ALL.into_iter().zip(draft.smoothing) {
                    properties
                        .tools
                        .get_mut(&drawer_core::tool_properties::Tool::brush(tip))
                        .unwrap()
                        .get_mut(&drawer_core::tool_properties::Property::Stabilization)
                        .unwrap()
                        .value = value;
                }
            }
            self.stage_workspace(s, self.workspaces.store.active.clone());
            match self.apply_workspace(
                drawer_core::workspaces::Scope {
                    other: true,
                    ..Default::default()
                },
                None,
            ) {
                Ok(()) => {
                    if let Some(layer) = layer {
                        super::edit::discard_layer(ctx, layer);
                    }
                    self.tool_input.open = false;
                    self.tool_input.draft = None;
                }
                Err(e) => {
                    self.status = Some(e);
                    self.workspaces.staged = None;
                    self.tool_input.draft = Some(draft);
                }
            }
        } else {
            self.tool_input.draft = Some(draft);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, tiny_spec};
    fn frame(ctx: &egui::Context, app: &mut Drawer, time: f64, events: Vec<egui::Event>) {
        let mut o = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 760.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        o.textures_delta.clear();
    }
    fn key(key: egui::Key, pressed: bool, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn modifier_tools_release_during_pointer_input_suspend_for_chords_popups_and_focus() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        app.workspaces.store.current.input.modifiers = [
            Some(BrushTip::Eraser),
            Some(BrushTip::Wash),
            Some(BrushTip::Pencil),
        ];
        app.persist_workspace().unwrap();
        // Isolate cursor/input behavior from painting, which is verified separately.
        app.documents[0].scene.selection_mask = Some(drawer_core::selection::Mask::default());
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let mut time = 1.0;
        let mut run =
            |app: &mut Drawer, modifiers: egui::Modifiers, mut events: Vec<egui::Event>| {
                events.insert(0, egui::Event::ModifiersChanged(modifiers));
                time += 0.1;
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 760.0),
                        )),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| app.show(ui),
                );
                out.textures_delta.clear();
            };
        let none = egui::Modifiers::NONE;
        let ctrl = egui::Modifiers::CTRL;
        run(&mut app, none, vec![]);
        for (mods, tool) in [
            (ctrl, BrushTip::Eraser),
            (egui::Modifiers::SHIFT, BrushTip::Wash),
            (egui::Modifiers::ALT, BrushTip::Pencil),
        ] {
            run(&mut app, mods, vec![]);
            assert_eq!(app.selected, tool);
            assert_eq!(app.capture_workspace().tool, "Pen");
            let pos = app.canvas_view.viewport.unwrap().center();
            run(
                &mut app,
                mods,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: mods,
                    },
                ],
            );
            run(&mut app, none, vec![]);
            assert_eq!(app.selected, BrushTip::Pen);
            run(
                &mut app,
                none,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: none,
                }],
            );
            assert!(app.tool_input.controller.persisted_tool().is_none());
        }
        run(&mut app, ctrl, vec![]);
        assert_eq!(app.selected, BrushTip::Eraser);
        run(
            &mut app,
            ctrl,
            vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: ctrl,
            }],
        );
        assert_eq!(app.selected, BrushTip::Pen);
        run(
            &mut app,
            ctrl,
            vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: ctrl,
            }],
        );
        assert_eq!(app.selected, BrushTip::Pen);
        run(&mut app, none, vec![]);
        run(&mut app, ctrl, vec![]);
        assert_eq!(app.selected, BrushTip::Eraser);
        app.controls.quick = Some(2);
        run(&mut app, ctrl, vec![]);
        assert_eq!(app.selected, BrushTip::Pen);
        app.controls.quick = None;
        run(&mut app, ctrl, vec![]);
        assert_eq!(app.selected, BrushTip::Pen);
        run(&mut app, none, vec![]);
        run(&mut app, ctrl, vec![]);
        run(&mut app, ctrl, vec![egui::Event::WindowFocused(false)]);
        assert_eq!(app.selected, BrushTip::Pen);
        run(&mut app, none, vec![egui::Event::WindowFocused(true)]);
        assert!(app.tool_input.controller.persisted_tool().is_none());
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(
            restarted.workspaces.store.current.input.modifiers,
            app.workspaces.store.current.input.modifiers
        );
        assert_eq!(restarted.selected, BrushTip::Pen);
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
    }
    #[test]
    fn stabilization_and_geometry_settings_cancel_apply_backup_restart_without_drawing() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, 0.0, vec![]);
        let before = app.documents[0].snapshot();
        let baseline = app.capture_workspace();
        let mut draft = baseline.input;
        draft.variations[0] = drawer_core::brush_variation::Settings {
            hue: 75.0,
            saturation: 0.4,
            value: 0.3,
            scope: drawer_core::brush_variation::Scope::Dab,
            seed: 513,
            dual: drawer_core::brush_variation::Dual {
                enabled: true,
                mask: drawer_core::brush_variation::Mask::Grain,
                blend: drawer_core::brush_variation::Blend::Add,
                ..Default::default()
            },
        };
        draft.patterns[0] = drawer_core::brush_pattern::Settings {
            mode: drawer_core::brush_pattern::Mode::Scatter,
            width: 123.0,
            count: 8,
            seed: 778,
            aspect: 0.3,
            ..Default::default()
        };
        draft.dynamics[0] = drawer_core::brush_dynamics::Settings {
            direction: drawer_core::brush_dynamics::Direction::Pen,
            minimum_size: 1.0,
            maximum_size: 19.0,
            random_minimum: 0.3,
            seed: 201,
            tilt: drawer_core::brush_dynamics::Curve {
                enabled: true,
                maximum: 90.0,
                missing: 0.2,
                ..Default::default()
            },
            ..Default::default()
        };
        draft.sequences[0] = drawer_core::tip_sequence::Settings {
            enabled: true,
            count: 3,
            tips: [7, 2, 999, 0, 0, 0, 0, 0],
            seed: 825,
            order: drawer_core::tip_sequence::Order::OneRandomCycle,
        };
        draft.filters[0] = drawer_core::stroke_filter::Settings {
            position_enabled: false,
            pressure_enabled: true,
            pressure_strength: 0.85,
        };
        draft.profiles[0] = drawer_core::tip_profile::Settings {
            minimum_size: 0.4,
            minimum_opacity: 0.7,
            image: drawer_core::tip_profile::Image::Square,
        };
        draft.smoothing[0] = 0.95;
        draft.ends[0] = drawer_core::stroke_ends::Settings {
            method: drawer_core::stroke_ends::Method::Percentage,
            fade_minimum: 0.3,
            start: 9.0,
            end: 17.0,
        };
        draft.materials[0] = drawer_core::brush_material::Settings {
            mode: drawer_core::brush_material::Mode::RunningColor,
            space: drawer_core::color_mix::Space::Perceptual,
            quality: drawer_core::color_mix::Quality::High,
            supply: 0.2,
            retention: 0.8,
            keep_density: true,
            alpha_lock: true,
            ..Default::default()
        };
        app.tool_input.open = true;
        app.tool_input.draft = Some(draft);
        frame(&ctx, &mut app, 1.0, vec![]);
        frame(&ctx, &mut app, 1.1, vec![]);
        let rect = app.tool_input.cancel_button.unwrap();
        crate::tests::click(&ctx, &mut app, rect);
        assert_eq!(app.capture_workspace(), baseline);
        app.tool_input.open = true;
        app.tool_input.draft = Some(draft);
        frame(&ctx, &mut app, 2.0, vec![]);
        frame(&ctx, &mut app, 2.1, vec![]);
        let rect = app.tool_input.apply_button.unwrap();
        crate::tests::click(&ctx, &mut app, rect);
        assert!(!app.tool_input.open);
        assert_eq!(app.workspaces.store.current.input, draft);
        let backup = drawer_core::workspaces::Snapshot::read(
            &storage.root().join("settings/workspace-before-apply.json"),
        )
        .unwrap();
        assert_eq!(backup.input, baseline.input);
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.workspaces.store.current.input, draft);
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
    #[test]
    fn timed_tool_keys_release_together_focus_loss_overlap_and_restart() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, 1.0, vec![]);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        frame(&ctx, &mut app, 2.0, vec![key(egui::Key::N, true, false)]);
        assert_eq!(app.selected, BrushTip::Pencil);
        assert_eq!(app.capture_workspace().tool, "Pen");
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.selected, BrushTip::Pen);
        frame(&ctx, &mut app, 2.299, vec![key(egui::Key::N, false, false)]);
        assert_eq!(app.selected, BrushTip::Pencil);
        assert_eq!(app.capture_workspace().tool, "Pencil");
        frame(&ctx, &mut app, 3.0, vec![key(egui::Key::E, true, false)]);
        frame(&ctx, &mut app, 3.29, vec![key(egui::Key::E, true, true)]);
        assert_eq!(app.selected, BrushTip::Eraser);
        assert!(
            app.tool_input
                .controller
                .temporary(3.3, Settings::default())
        );
        let pos = app.canvas_view.viewport.unwrap().center();
        let button = |button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &ctx,
            &mut app,
            3.3,
            vec![
                button(egui::PointerButton::Primary, true),
                button(egui::PointerButton::Secondary, true),
            ],
        );
        frame(
            &ctx,
            &mut app,
            3.301,
            vec![
                key(egui::Key::E, false, false),
                button(egui::PointerButton::Primary, false),
                button(egui::PointerButton::Secondary, false),
            ],
        );
        assert_eq!(app.selected, BrushTip::Pencil);
        assert!(app.workspaces.input_idle());
        frame(&ctx, &mut app, 4.0, vec![key(egui::Key::W, true, false)]);
        assert_eq!(app.selected, BrushTip::Wash);
        frame(
            &ctx,
            &mut app,
            4.35,
            vec![
                button(egui::PointerButton::Primary, true),
                egui::Event::WindowFocused(false),
            ],
        );
        assert_eq!(app.selected, BrushTip::Pencil);
        assert!(ctx.input(|i| i.keys_down.is_empty() && !i.pointer.any_down()));
        assert!(app.canvas_view.first_point.is_none());
        frame(&ctx, &mut app, 4.5, vec![egui::Event::WindowFocused(true)]);
        frame(&ctx, &mut app, 5.0, vec![key(egui::Key::B, true, false)]);
        frame(&ctx, &mut app, 5.01, vec![key(egui::Key::W, true, false)]);
        frame(
            &ctx,
            &mut app,
            5.1,
            vec![
                key(egui::Key::W, false, false),
                key(egui::Key::B, false, false),
            ],
        );
        assert_eq!(app.selected, BrushTip::Wash);
        app.workspaces.store.current.input = Settings {
            hold_millis: 650,
            drag_pixels: 12.0,
            ..Default::default()
        };
        app.workspaces.store.current.scratchpad.visible = true;
        app.workspaces.store.current.scratchpad.stroke(
            [2.0, 3.0],
            [50.0, 70.0],
            7.0,
            BrushTip::Pen,
            [30, 60, 90, 255],
        );
        app.persist_workspace().unwrap();
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.selected, BrushTip::Wash);
        assert_eq!(
            restarted.workspaces.store.current.input,
            app.workspaces.store.current.input
        );
        assert_eq!(
            restarted.workspaces.store.current.scratchpad,
            app.workspaces.store.current.scratchpad
        );
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
    }
    #[test]
    fn configured_drag_threshold_pan_and_document_switch_do_not_leave_held_inputs() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        app.workspaces.store.current.input.drag_pixels = 12.0;
        frame(&ctx, &mut app, 1.0, vec![]);
        frame(&ctx, &mut app, 1.01, vec![]);
        let before = app.documents[0].snapshot();
        let pos = app.canvas_view.viewport.unwrap().center();
        let pan = app.documents[0].view.pan;
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Middle,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &ctx,
            &mut app,
            2.0,
            vec![egui::Event::PointerMoved(pos), press(true)],
        );
        frame(
            &ctx,
            &mut app,
            2.1,
            vec![egui::Event::PointerMoved(pos + egui::vec2(11.0, 0.0))],
        );
        assert_eq!(app.documents[0].view.pan, pan);
        frame(
            &ctx,
            &mut app,
            2.2,
            vec![egui::Event::PointerMoved(pos + egui::vec2(12.0, 0.0))],
        );
        assert!((app.documents[0].view.pan[0] - pan[0] - 12.0).abs() < 1e-9);
        frame(&ctx, &mut app, 2.3, vec![press(false)]);
        frame(&ctx, &mut app, 3.0, vec![key(egui::Key::E, true, false)]);
        let first = app.documents[0].id;
        app.create_canvas(tiny_spec()).unwrap();
        assert_eq!(app.selected, BrushTip::Pen);
        frame(&ctx, &mut app, 3.1, vec![]);
        assert_eq!(
            app.documents.iter().find(|d| d.id == first).unwrap().tool,
            BrushTip::Pen
        );
        assert!(app.tool_input.controller.persisted_tool().is_none());
        frame(&ctx, &mut app, 3.5, vec![key(egui::Key::E, false, false)]);
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
}
