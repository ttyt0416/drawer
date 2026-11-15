use crate::*;
#[derive(Default)]
pub(super) struct Navigator {
    pub open: bool,
    pub query: String,
    step: i32,
    activate: bool,
    selected: usize,
    focus: bool,
    previous: Option<egui::Id>,
    pub rect: Option<egui::Rect>,
    pub close_id: Option<egui::Id>,
    pub matches: Vec<String>,
}
pub(super) fn query_id() -> egui::Id {
    egui::Id::new("drawer-command-query")
}
impl Navigator {
    fn restore(&self, ctx: &egui::Context) {
        ctx.memory_mut(|m| {
            if let Some(id) = self.previous {
                m.request_focus(id);
            } else if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }
}
impl Drawer {
    pub(super) fn accessibility_keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F10)) {
            if self.keyboard.open {
                self.keyboard.open = false;
                self.keyboard.restore(ctx);
            } else {
                self.keyboard.previous = ctx.memory(|m| m.focused());
                self.keyboard.open = true;
                self.keyboard.focus = true;
                self.keyboard.query.clear();
                self.keyboard.selected = 0;
            }
        }
        if self.keyboard.open && ctx.memory(|m| m.focused()) == Some(query_id()) {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
                self.keyboard.step = 1;
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                self.keyboard.step = -1;
            }
            self.keyboard.activate =
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.keyboard.open {
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                self.keyboard.open = false;
                self.keyboard.restore(ctx);
            } else if self.interface.open {
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                self.interface.open = false;
                self.keyboard.restore(ctx);
            } else if self.graphics.open {
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                self.graphics.open = false;
                self.keyboard.restore(ctx);
            } else if self.panels.open {
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                self.panels.open = false;
                self.keyboard.restore(ctx);
            }
        }
    }
    pub(super) fn accessibility_window(&mut self, ctx: &egui::Context) {
        if !self.keyboard.open {
            return;
        }
        let mut nav = std::mem::take(&mut self.keyboard);
        let enter = std::mem::take(&mut nav.activate);
        let direction = std::mem::take(&mut nav.step);
        let down = direction > 0;
        let up = direction < 0;
        let mut commands = self.workspaces.store.current.actions.commands();
        commands
            .extend(["@scale", "@panels", "@information", "@help", "@graphics"].map(String::from));
        let labels = self.workspaces.store.current.actions.clone();
        let label = |id: &str| match id {
            "@graphics" => "Display recovery".to_string(),
            "@scale" => crate::i18n::text("Interface size").to_string(),
            "@panels" => crate::i18n::text("Panel settings").to_string(),
            "@information" => crate::i18n::text("Information").to_string(),
            "@help" => crate::i18n::text("Help").to_string(),
            _ => labels.label(id),
        };
        commands.sort_by_cached_key(|id| label(id).to_lowercase());
        let mut open = true;
        let mut execute = None;
        let mut contrast = self.interface.settings.high_contrast;
        let available = ctx.content_rect().shrink(8.);
        let response = egui::Window::new(crate::i18n::text("Commands and panels"))
            .id(egui::Id::new("drawer-keyboard-navigation"))
            .open(&mut open)
            .default_pos(egui::pos2(8., 40.))
            .default_width(620f32.min(available.width()))
            .max_width(available.width())
            .max_height(available.height())
            .constrain_to(available)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("F10: commands · Up/Down: select · Enter: run · Esc: return · F1: help");
                let q = ui.add(
                    egui::TextEdit::singleline(&mut nav.query)
                        .id(query_id())
                        .hint_text(crate::i18n::text("Find a command")),
                );
                crate::help::tag(ctx, &q, "accessibility");
                if nav.focus || down || up {
                    q.request_focus();
                    nav.focus = false;
                }
                if q.changed() {
                    nav.selected = 0;
                }
                let needle = nav.query.to_lowercase();
                nav.matches = commands
                    .iter()
                    .filter(|id| {
                        id.to_lowercase().contains(&needle)
                            || label(id).to_lowercase().contains(&needle)
                    })
                    .cloned()
                    .collect();
                if nav.matches.is_empty() {
                    nav.selected = 0;
                    ui.label("No matching command");
                } else {
                    nav.selected = nav.selected.min(nav.matches.len() - 1);
                    if down {
                        nav.selected = (nav.selected + 1) % nav.matches.len();
                    }
                    if up {
                        nav.selected = (nav.selected + nav.matches.len() - 1) % nav.matches.len();
                    }
                }
                let blocked = self.pending_close.is_some()
                    || self.sizing_dialog.is_some()
                    || self.import_dialog.is_some()
                    || self.paste_dialog.is_some()
                    || self.movie_dialog.is_some()
                    || self.revert_pending.is_some()
                    || self.workspaces.staged.is_some();
                egui::ScrollArea::vertical()
                    .id_salt("keyboard-command-list")
                    .max_height((available.height() * 0.45).clamp(40., 380.))
                    .show(ui, |ui| {
                        for (n, id) in nav.matches.iter().enumerate() {
                            let reason = if blocked {
                                Some("Resolve the open dialog first".into())
                            } else if id.starts_with('@') {
                                None
                            } else {
                                self.command_reason(id)
                            };
                            let text = format!(
                                "{} {}{}",
                                if nav.selected == n { "[>]" } else { "[ ]" },
                                label(id),
                                reason
                                    .as_ref()
                                    .map_or(String::new(), |r| format!(" — unavailable: {r}"))
                            );
                            let row = ui
                                .selectable_label(nav.selected == n, text)
                                .on_hover_text(format!("{} · F1: keyboard help", id));
                            if row.clicked() {
                                nav.selected = n;
                                if let Some(reason) = reason.as_ref() {
                                    self.status = Some(reason.clone());
                                } else {
                                    execute = Some(id.clone());
                                }
                            }
                            if nav.selected == n {
                                if down || up || q.changed() || row.has_focus() {
                                    row.scroll_to_me(Some(egui::Align::Center));
                                }
                                if enter {
                                    if let Some(reason) = reason.as_ref() {
                                        self.status = Some(reason.clone());
                                    } else {
                                        execute = Some(id.clone());
                                    }
                                }
                                if ui
                                    .add_enabled(
                                        reason.is_none(),
                                        egui::Button::new(crate::i18n::text(
                                            "Run selected command",
                                        )),
                                    )
                                    .clicked()
                                {
                                    execute = Some(id.clone());
                                }
                            }
                        }
                    });
                crate::help::checkbox(
                    ui,
                    &mut contrast,
                    crate::i18n::text("High contrast"),
                    "accessibility",
                );
                let close = crate::help::button(ui, crate::i18n::text("Close"), "accessibility");
                nav.close_id = Some(close.id);
                if close.clicked() {
                    nav.open = false;
                }
            });
        if direction != 0 {
            ctx.memory_mut(|m| {
                m.move_focus(egui::FocusDirection::None);
                m.request_focus(query_id());
                m.set_focus_lock_filter(
                    query_id(),
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });
        }
        nav.rect = response.map(|r| r.response.rect);
        nav.open &= open;
        let closed = !nav.open;
        self.keyboard = nav;
        if contrast != self.interface.settings.high_contrast
            && let Err(e) = self.interface_contrast(ctx, contrast)
        {
            self.status = Some(e);
        }
        if let Some(id) = execute {
            self.keyboard.open = false;
            match id.as_str() {
                "@graphics" => self.graphics.open = true,
                "@scale" => self.interface.open = true,
                "@panels" => self.panels.open = true,
                "@information" => self.workspaces.store.current.information.visible = true,
                "@help" => self.open_help("accessibility"),
                _ => self.execute_command(ctx, &id),
            }
        } else if closed {
            self.keyboard.restore(ctx);
        }
    }
}
