use super::*;
impl Drawer {
    pub(super) fn restore_locked_tool(&mut self) {
        self.sync_tool_properties();
        let current = self.capture_workspace();
        if drawer_core::tool_locks::baseline(&current, self.selected).is_none() {
            return;
        }
        match drawer_core::tool_locks::restore_current(&current, self.selected) {
            Ok(next) => {
                self.restore_workspace(&next);
                self.workspaces.store.current = next;
            }
            Err(e) => self.status = Some(e),
        }
    }
    pub(super) fn tool_lock_controls(&mut self, ui: &mut egui::Ui) {
        let current = self.capture_workspace();
        let mut locked = drawer_core::tool_locks::baseline(&current, self.selected).is_some();
        if ui
            .checkbox(
                &mut locked,
                "Lock saved tool settings (changes stay temporary)",
            )
            .changed()
        {
            self.finish_stroke();
            self.sync_tool_properties();
            let before = self.capture_workspace();
            match drawer_core::tool_locks::set(&before, self.selected, locked) {
                Ok(next) => {
                    self.restore_workspace(&next);
                    self.workspaces.store.current = next;
                    if let Err(e) = self.persist_workspace() {
                        self.restore_workspace(&before);
                        self.workspaces.store.current = before;
                        self.status = Some(e);
                    }
                }
                Err(e) => self.status = Some(e),
            }
        }
        if locked {
            ui.label("Temporary values last for this tool. Switching tools or restarting restores the locked values. Unlock or save a separate copy to change a stored preset.");
        }
    }
}
