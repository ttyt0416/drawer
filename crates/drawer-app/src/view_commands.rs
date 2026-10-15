use super::*;
use drawer_core::{
    display_view::Framing,
    view_layout::{Layout, SavedView},
};

impl Drawer {
    pub(super) fn step_view(&mut self, offset: isize) {
        if !self.tabs.is_empty() {
            self.select_tab(
                (self.active_tab.unwrap_or(0) as isize + offset)
                    .rem_euclid(self.tabs.len() as isize) as usize,
            );
        }
    }
    pub(super) fn view_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input()
            || self.pending_close.is_some()
            || self.sizing_dialog.is_some()
            || self.import_dialog.is_some()
            || self.paste_dialog.is_some()
            || self.movie_dialog.is_some()
            || self.revert_pending.is_some()
            || self.cursors.open
            || self.ui_behavior.open
            || self.tool_input.open
            || self.controls.open
            || self.controls.radial.is_some()
            || self.workspaces.staged.is_some()
            || self.show_new_canvas
        {
            return;
        }
        let action = ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::CTRL, egui::Key::PageUp) {
                Some(-1)
            } else if input.consume_key(egui::Modifiers::CTRL, egui::Key::PageDown) {
                Some(-2)
            } else {
                [
                    egui::Key::Num1,
                    egui::Key::Num2,
                    egui::Key::Num3,
                    egui::Key::Num4,
                    egui::Key::Num5,
                    egui::Key::Num6,
                    egui::Key::Num7,
                    egui::Key::Num8,
                    egui::Key::Num9,
                ]
                .iter()
                .position(|key| input.consume_key(egui::Modifiers::ALT, *key))
                .map(|n| n as isize)
            }
        });
        match action {
            Some(-1) => self.step_view(-1),
            Some(-2) => self.step_view(1),
            Some(n) => self.select_tab(n as usize),
            None => {}
        }
    }
    pub(super) fn capture_views(&mut self, ppp: f64) -> Layout {
        self.sync_active_view();
        Layout {
            version: 1,
            active: self.active_tab.map(|i| self.tabs[i].id),
            rows: self
                .tabs
                .iter()
                .map(|tab| {
                    let doc = self
                        .documents
                        .iter()
                        .find(|doc| doc.id == tab.document_id)
                        .unwrap();
                    SavedView {
                        tab: tab.id,
                        document: doc.id,
                        session: self.view_session,
                        path: doc.path.clone().or(doc.source_path.clone()),
                        title: doc.title.clone(),
                        framing: Framing::capture(
                            tab.view,
                            [doc.width as f64, doc.height as f64],
                            ppp,
                        ),
                    }
                })
                .collect(),
        }
    }
    pub(super) fn persist_views(&mut self, ppp: f64) {
        let layout = self.capture_views(ppp);
        if layout != self.last_layout {
            self.last_layout = layout.clone();
            if let Some(storage) = &self.storage {
                match layout.save(storage) {
                    Ok(()) => self.saved_layout = layout,
                    Err(e) => self.status = Some(format!("Cannot save views: {e}")),
                }
            }
        }
    }
    pub(super) fn restore_views(&mut self, layout: Layout, ppp: f64) -> Result<(), String> {
        if !layout.valid() {
            return Err("Invalid saved view layout".into());
        }
        self.sync_active_view();
        let mut restored = Vec::new();
        let mut active = None;
        let mut unavailable = Vec::new();
        for row in &layout.rows {
            let document = if let Some(path) = &row.path {
                match self.open_path(path) {
                    Ok(()) => self.active.map(|n| self.documents[n].id),
                    Err(e) => {
                        unavailable.push(format!("{} ({}): {e}", row.title, path.display()));
                        None
                    }
                }
            } else if row.session == self.view_session
                && self.documents.iter().any(|doc| doc.id == row.document)
            {
                Some(row.document)
            } else {
                unavailable.push(format!(
                    "{}: this drawing was not saved to a file",
                    row.title
                ));
                None
            };
            let Some(document) = document else {
                continue;
            };
            let tab = if let Some(n) = self
                .tabs
                .iter()
                .position(|tab| tab.document_id == document && !restored.contains(&tab.id))
            {
                n
            } else {
                let n = self
                    .tabs
                    .iter()
                    .position(|tab| tab.document_id == document)
                    .unwrap();
                self.select_tab(n);
                let count = self.tabs.len();
                self.duplicate_view();
                if count == self.tabs.len() {
                    unavailable.push("View limit reached".into());
                    break;
                }
                self.tabs.len() - 1
            };
            let doc = self
                .documents
                .iter()
                .find(|doc| doc.id == document)
                .unwrap();
            self.tabs[tab].view = row.framing.view([doc.width as f64, doc.height as f64], ppp);
            // Selecting another tab must not overwrite the just-restored active view.
            if self.active_tab == Some(tab) {
                self.documents[self.active.unwrap()].view = self.tabs[tab].view;
            }
            restored.push(self.tabs[tab].id);
            if layout.active == Some(row.tab) {
                active = Some(self.tabs[tab].id);
            }
        }
        self.sync_active_view();
        self.tabs.sort_by_key(|tab| {
            restored
                .iter()
                .position(|id| *id == tab.id)
                .unwrap_or(restored.len())
        });
        self.active_tab = None;
        self.active = None;
        if !self.tabs.is_empty() {
            self.select_tab(
                active
                    .and_then(|id| self.tabs.iter().position(|tab| tab.id == id))
                    .unwrap_or(0),
            );
        }
        if unavailable.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Available views restored. {}",
                unavailable.join("; ")
            ))
        }
    }
    pub(super) fn view_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Views", |ui| {
            if ui.button("Previous view  Ctrl+PageUp").clicked() {
                self.step_view(-1);
                ui.close();
            }
            if ui.button("Next view  Ctrl+PageDown").clicked() {
                self.step_view(1);
                ui.close();
            }
            for n in 0..self.tabs.len().min(9) {
                if ui
                    .button(format!("View {}  Alt+{}", n + 1, n + 1))
                    .clicked()
                {
                    self.select_tab(n);
                    ui.close();
                }
            }
            ui.separator();
            if ui
                .add_enabled(
                    !self.saved_layout.rows.is_empty(),
                    egui::Button::new("Restore saved views"),
                )
                .clicked()
            {
                if let Err(e) = self.restore_views(
                    self.saved_layout.clone(),
                    ui.ctx().pixels_per_point() as f64,
                ) {
                    self.status = Some(e);
                }
                ui.close();
            }
            if ui
                .add_enabled(
                    self.storage.is_some(),
                    egui::Button::new("Restore previous view layout"),
                )
                .clicked()
            {
                let result = Layout::read(
                    &self
                        .storage
                        .as_ref()
                        .unwrap()
                        .root()
                        .join("settings/views.previous.json"),
                )
                .map_err(|e| e.to_string())
                .and_then(|s| self.restore_views(s, ui.ctx().pixels_per_point() as f64));
                if let Err(e) = result {
                    self.status = Some(e);
                }
                ui.close();
            }
        });
    }
}
