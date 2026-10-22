use super::*;
impl Drawer {
    pub(super) fn revert_from(&mut self, index: usize, confirmed: bool) -> Result<bool, String> {
        if !confirmed {
            return Ok(false);
        }
        let current = self.documents.get(index).ok_or("No drawing selected")?;
        let path = current
            .path
            .clone()
            .ok_or("Save the document before reverting")?;
        let budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        let mut loaded = drawer_core::files::open(&path, budget).map_err(|e| e.to_string())?;
        loaded.id = current.id;
        loaded.view = current.view;
        loaded.tool = current.tool;
        loaded.revision = current.revision + 1;
        loaded.current_frame = loaded
            .animation
            .as_ref()
            .map_or(0, |a| current.current_frame.min(a.frames - 1));
        self.documents[index] = loaded;
        Ok(true)
    }
    pub(super) fn revert_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.revert_pending else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("revert-document")).show(ctx, |ui| {
            ui.heading("Reload the last saved document?");
            ui.label("Unsaved edits will be discarded. The saved file stays unchanged.");
            ui.horizontal(|ui| {
                confirm = crate::help::button(ui, "Discard edits and reload", "files").clicked();
                cancel = crate::help::button(ui, "Cancel", "files").clicked();
            });
        });
        if confirm {
            match self
                .documents
                .iter()
                .position(|d| d.id == id)
                .ok_or_else(|| "The document is no longer open".into())
                .and_then(|n| self.revert_from(n, true))
            {
                Ok(_) => self.revert_pending = None,
                Err(e) => {
                    self.status = Some(e);
                    self.revert_pending = None;
                }
            }
        }
        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.revert_pending = None;
        }
    }
    pub(super) fn file_association_from(&mut self, open: impl FnOnce() -> Result<(), String>) {
        if let Err(e) = open() {
            self.status = Some(format!(
                "Windows default-app settings could not be opened: {e}. Use Windows Settings → Apps → Default apps → .drawer."
            ));
        }
    }
    pub(super) fn local_file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Local file actions",|ui| {
            for (label,command) in [("Import SVG…","svg-import"),("Export SVG…","svg-export"),("Copy SVG text","svg-copy")] {if ui.add_enabled(self.active.is_some(),egui::Button::new(label)).clicked(){self.execute_command(ui.ctx(),command);ui.close();}}
            ui.separator();
            if ui.add_enabled(self.active.is_some_and(|n|self.documents[n].path.is_some()),egui::Button::new("Revert to saved document…")).clicked() {self.revert_pending=self.active.map(|n|self.documents[n].id);ui.close();}
            ui.separator();ui.label("To associate .drawer files, choose Drawer.exe in Windows Settings → Apps → Default apps. Windows controls whether an association is allowed.");
            if crate::help::button(ui,"Open Windows default-app settings","files").clicked() {self.file_association_from(|| {
                let mut command=std::process::Command::new("explorer.exe");command.arg("ms-settings:defaultapps");
                #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
                command.spawn().map(|_|()).map_err(|e|e.to_string())
            });ui.close();}
        });
    }
}
