use super::*;
use drawer_core::navigator_preferences::Preferences;
impl Drawer {
    pub(super) fn persist_navigator(&mut self) {
        let next = self.canvas_view.navigator;
        if self.navigator_saved != Some(next) {
            if let Some(storage) = &self.storage
                && let Err(e) = next.save(storage)
            {
                self.status = Some(format!("Navigator settings not saved: {e}"));
                return;
            }
            if let Some(previous) = self.navigator_saved {
                self.navigator_undo.push(previous);
                if self.navigator_undo.len() > 100 {
                    self.navigator_undo.remove(0);
                }
                self.navigator_redo.clear();
            }
            self.navigator_saved = Some(next);
        }
    }
    pub(super) fn navigator_history(&mut self, redo: bool) -> Result<(), String> {
        let history = if redo {
            &self.navigator_redo
        } else {
            &self.navigator_undo
        };
        let Some(next) = history.last().copied() else {
            return Ok(());
        };
        if let Some(storage) = &self.storage {
            next.save(storage).map_err(|e| e.to_string())?;
        }
        let current = self.canvas_view.navigator;
        if redo {
            self.navigator_redo.pop();
            self.navigator_undo.push(current);
        } else {
            self.navigator_undo.pop();
            self.navigator_redo.push(current);
        }
        self.canvas_view.navigator = next;
        self.canvas_view.navigator_generation += 1;
        self.navigator_saved = Some(next);
        Ok(())
    }
    pub(super) fn navigator_settings(&mut self, ctx: &egui::Context) {
        if !self.show_navigator_settings {
            return;
        }
        let mut open = true;
        let mut next = self.canvas_view.navigator;
        let mut history = None;
        egui::Window::new("Navigator settings")
            .open(&mut open)
            .show(ctx, |ui| {
                crate::help::checkbox(ui, &mut next.visible, "Show navigator", "view");
                crate::help::checkbox(ui, &mut next.floating, "Floating movable panel", "view");
                crate::help::checkbox(
                    ui,
                    &mut next.hover_outline,
                    "Show view boundary on hover or keyboard focus",
                    "view",
                );
                crate::help::checkbox(
                    ui,
                    &mut next.selection_outline,
                    "Show dashed selection outline",
                    "view",
                );
                super::edit::slider(
                    ui,
                    &mut next.min_width,
                    80.0..=640.0,
                    "Minimum width",
                    "view",
                );
                next.max_width = next.max_width.max(next.min_width);
                super::edit::slider(
                    ui,
                    &mut next.max_width,
                    next.min_width..=640.0,
                    "Maximum width",
                    "view",
                );
                next.size[0] = next.size[0].clamp(next.min_width, next.max_width);
                super::edit::slider(
                    ui,
                    &mut next.size[0],
                    next.min_width..=next.max_width,
                    "Width",
                    "view",
                );
                super::edit::slider(ui, &mut next.size[1], 80.0..=640.0, "Height", "view");
                ui.label("Drag a floating panel by its title. Drag its bottom corner to resize.");
                if crate::help::button(ui, "Reset navigator", "view").clicked() {
                    next = Default::default();
                }
                if ui
                    .add_enabled(
                        !self.navigator_undo.is_empty(),
                        egui::Button::new("Undo navigator setting"),
                    )
                    .clicked()
                {
                    history = Some(false);
                }
                if ui
                    .add_enabled(
                        !self.navigator_redo.is_empty(),
                        egui::Button::new("Redo navigator setting"),
                    )
                    .clicked()
                {
                    history = Some(true);
                }
                if ui
                    .add_enabled(
                        self.storage.is_some(),
                        egui::Button::new("Restore previous navigator settings"),
                    )
                    .clicked()
                {
                    match Preferences::read(
                        &self
                            .storage
                            .as_ref()
                            .unwrap()
                            .root()
                            .join("settings/navigator.previous.json"),
                    ) {
                        Ok(p) => next = p,
                        Err(e) => self.status = Some(e.to_string()),
                    }
                }
            });
        self.show_navigator_settings = open;
        if let Some(redo) = history {
            if let Err(e) = self.navigator_history(redo) {
                self.status = Some(e);
            }
        } else if next != self.canvas_view.navigator {
            self.canvas_view.navigator = next;
            self.canvas_view.navigator_generation += 1;
            ctx.request_repaint();
        }
    }
}
