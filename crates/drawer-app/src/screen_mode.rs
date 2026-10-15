use super::*;
pub(super) fn preview_switch(ui: &mut egui::Ui, simple: bool) -> egui::Response {
    crate::help::button(
        ui,
        if simple {
            "Show all panels"
        } else {
            "Simple screen"
        },
        "screen-mode",
    )
}

impl Drawer {
    pub(super) fn toggle_screen_mode(&mut self) {
        self.simple_view = !self.simple_view;
        self.canvas_view.cancel_input();
    }
    pub(super) fn screen_mode_inputs(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || !ctx.input(|i| i.focused) {
            return;
        }
        let bindings = self
            .workspaces
            .store
            .current
            .bindings
            .iter()
            .filter(|b| b.command == "screen-mode")
            .cloned()
            .collect::<Vec<_>>();
        if ctx
            .input_mut(|i| super::workspaces::shortcut(i, &bindings))
            .is_some()
        {
            self.toggle_screen_mode();
        }
    }
    pub(super) fn screen_mode_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(if self.simple_view {
                "Simple screen"
            } else {
                "Full tool screen"
            });
            let button = crate::help::button(
                ui,
                format!(
                    "{}  {}",
                    if self.simple_view {
                        "Show all panels"
                    } else {
                        "Simple screen"
                    },
                    self.workspaces.command_label("screen-mode")
                ),
                "screen-mode",
            );
            #[cfg(test)]
            {
                self.screen_button = Some(button.rect);
            }
            if button.clicked() {
                self.toggle_screen_mode();
                self.canvas_view.simple = self.simple_view;
            }
        });
    }
    pub(super) fn full_commands(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading(APP_NAME);
            ui.label("Local drawing workspace");
            if crate::help::button(ui, "Help", "screen-mode").clicked() {
                self.open_help("welcome");
            }
            if crate::help::button(ui, "About", "screen-mode").clicked() {
                self.open_help("version");
            }
            let new_response = crate::help::button(ui, "New canvas", "screen-mode");
            #[cfg(test)]
            {
                self.new_button = Some(new_response.rect);
            }
            if new_response.clicked() {
                self.new_canvas = NewCanvas::default();
                self.show_new_canvas = true;
            }
            if crate::help::button(ui, "References", "screen-mode").clicked() {
                self.references.session.workspace.visible = true;
            }
            if crate::help::button(ui, "Canvas aids", "screen-mode").clicked() {
                self.aids.visible = true;
            }
            if crate::help::button(ui, "Display view", "screen-mode").clicked() {
                self.display.settings_open = true;
            }
            if crate::help::button(ui, "Image quality", "screen-mode").clicked() {
                self.show_interpolation = true;
            }
            if crate::help::button(ui, "Interface size", "screen-mode").clicked() {
                self.interface.open = true;
            }
            if crate::help::button(ui, "Information", "screen-mode").clicked() {
                self.workspaces.store.current.information.visible = true;
            }
            if crate::help::button(ui, "Cursor styles", "screen-mode").clicked() {
                self.cursors.open = true;
            }
            if crate::help::button(ui, "Panels, cursor and power", "screen-mode").clicked() {
                self.ui_behavior.open = true;
            }
            if crate::help::button(ui, "Tool input behavior", "screen-mode").clicked() {
                self.tool_input.open = true;
            }
            if crate::help::button(ui, "Input and commands", "screen-mode").clicked() {
                self.controls.open = true;
            }
            if crate::help::button(ui, "Workspaces", "screen-mode").clicked() {
                self.workspaces.open = true;
            }
            if crate::help::button(ui, "Test painting", "screen-mode").clicked() {
                self.workspaces.store.current.scratchpad.visible = true;
            }
            if crate::help::button(ui, "Panels", "screen-mode").clicked() {
                self.panels.open = true;
            }
            if crate::help::button(ui, "Navigator settings", "screen-mode").clicked() {
                self.show_navigator_settings = true;
            }
        });
        self.document_controls(ui);
        self.sizing_controls(ui);
        self.paste_controls(ui);
        self.movie_controls(ui);
        self.access_controls(ui);
        ui.separator();
        if let Some(status) = &self.status {
            ui.colored_label(egui::Color32::LIGHT_RED, status);
        }
    }
    pub(super) fn simple_canvas(&mut self, ui: &mut egui::Ui) {
        if let Some(n) = self.active {
            let overlay = self.sizing_overlay();
            let mut hidden = ui.new_child(egui::UiBuilder::new().invisible());
            let changed = self.canvas_view.show(
                ui,
                n,
                &mut self.documents[n],
                overlay,
                &mut hidden,
                self.aids.preferences,
            );
            if changed
                && let Some(storage) = &self.storage
                && let Err(e) = storage.save_view(self.documents[n].view)
            {
                self.status = Some(e.to_string());
            }
        } else {
            ui.heading("Create a canvas to begin");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::tiny_spec;
    fn frame(ctx: &egui::Context, app: &mut Drawer, events: Vec<egui::Event>) {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        out.textures_delta.clear();
    }
    fn key(pressed: bool, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::F12,
            physical_key: None,
            pressed,
            repeat,
            modifiers: Default::default(),
        }
    }
    fn click(ctx: &egui::Context, app: &mut Drawer, pos: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }
    #[test]
    fn screen_switch_keeps_documents_tools_selection_tabs_and_pending_preview() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        app.set_tool(BrushTip::Pencil);
        app.create_canvas(tiny_spec()).unwrap();
        app.set_tool(BrushTip::Wash);
        app.documents[0].scene.selection_mask = Some(drawer_core::selection::Mask::default());
        app.duplicate_view();
        frame(&ctx, &mut app, vec![]);
        let docs = app
            .documents
            .iter()
            .map(|d| (d.snapshot(), d.history_counts(), d.tool, d.view, d.dirty))
            .collect::<Vec<_>>();
        let tabs = app
            .tabs
            .iter()
            .map(|t| (t.id, t.document_id, t.view))
            .collect::<Vec<_>>();
        let workspace = app.capture_workspace();
        let full = app.canvas_view.viewport.unwrap();
        frame(&ctx, &mut app, vec![key(true, false)]);
        assert!(app.simple_view);
        assert!(app.canvas_view.viewport.unwrap().width() > full.width());
        assert!(app.canvas_view.navigator_rect.is_none());
        frame(&ctx, &mut app, vec![key(true, true)]);
        assert!(app.simple_view);
        frame(&ctx, &mut app, vec![key(false, false)]);
        for (key, tool) in [
            (egui::Key::N, BrushTip::Pencil),
            (egui::Key::W, BrushTip::Wash),
        ] {
            for pressed in [true, false] {
                frame(
                    &ctx,
                    &mut app,
                    vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                );
            }
            assert_eq!(app.selected, tool);
        }
        let menu = app.commands_button.unwrap().center();
        click(&ctx, &mut app, menu);
        assert!(egui::Popup::is_any_open(&ctx));
        assert!(app.new_button.is_some());
        egui::Popup::close_all(&ctx);
        let button = app.screen_button.unwrap().center();
        click(&ctx, &mut app, button);
        assert!(!app.simple_view);
        app.begin_sizing(sizing_dialog::Mode::Dpi);
        app.sizing_dialog.as_mut().unwrap().dpi = "450".into();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        // The preview provides the same switch inside its modal layer.
        let button = app.screen_button.unwrap().center();
        click(&ctx, &mut app, button);
        assert!(app.simple_view);
        assert_eq!(app.sizing_dialog.as_ref().unwrap().dpi, "450");
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.screen_button.unwrap().center();
        click(&ctx, &mut app, button);
        assert!(!app.simple_view);
        assert_eq!(
            app.documents
                .iter()
                .map(|d| (d.snapshot(), d.history_counts(), d.tool, d.view, d.dirty))
                .collect::<Vec<_>>(),
            docs
        );
        assert_eq!(
            app.tabs
                .iter()
                .map(|t| (t.id, t.document_id, t.view))
                .collect::<Vec<_>>(),
            tabs
        );
        assert_eq!(app.capture_workspace(), workspace);
        assert_eq!(app.selected, BrushTip::Wash);
        assert_eq!(app.screen_modes_seen, 3);
    }
}
