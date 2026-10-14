use super::*;
use std::{collections::HashSet, path::Path};

pub(super) struct DocumentTab {
    pub id: u64,
    pub document_id: u64,
    pub view: drawer_core::view::View,
}

impl Drawer {
    pub(super) fn add_document(&mut self, mut document: Document) {
        document.tool = self.selected;
        let id = document.id;
        let view = document.view;
        self.documents.push(document);
        self.next_tab_id += 1;
        self.tabs.push(DocumentTab {
            id: self.next_tab_id,
            document_id: id,
            view,
        });
        self.select_tab(self.tabs.len() - 1);
    }
    fn sync_active_view(&mut self) {
        if let (Some(tab), Some(document)) = (self.active_tab, self.active) {
            self.tabs[tab].view = self.documents[document].view;
        }
    }
    pub(super) fn select_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.sync_active_view();
        self.active_tab = Some(index);
        self.active = self
            .documents
            .iter()
            .position(|doc| doc.id == self.tabs[index].document_id);
        if let Some(document) = self.active {
            self.documents[document].view = self.tabs[index].view;
            self.selected = self.documents[document].tool;
        }
    }
    pub(super) fn duplicate_view(&mut self) {
        let Some(document) = self.active else {
            return;
        };
        self.sync_active_view();
        self.next_tab_id += 1;
        self.tabs.push(DocumentTab {
            id: self.next_tab_id,
            document_id: self.documents[document].id,
            view: self.documents[document].view,
        });
        self.select_tab(self.tabs.len() - 1);
    }
    pub(super) fn set_tool(&mut self, tip: BrushTip) {
        self.selected = tip;
        if let Some(index) = self.active {
            self.documents[index].tool = tip;
        }
        if let Some(storage) = &self.storage
            && let Err(error) = storage.save_brush(tip)
        {
            self.status = Some(error.to_string());
        }
    }
    pub(super) fn open_path(&mut self, path: &Path) -> Result<(), String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if let Some(id) = self
            .documents
            .iter()
            .find(|doc| doc.path.as_ref() == Some(&path) || doc.source_path.as_ref() == Some(&path))
            .map(|doc| doc.id)
        {
            if let Some(tab) = self.tabs.iter().position(|tab| tab.document_id == id) {
                self.select_tab(tab);
            }
            return Ok(());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        let mut document =
            drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET.saturating_sub(used))
                .map_err(|e| e.to_string())?;
        document.view = self.default_view;
        self.documents.try_reserve(1).map_err(|e| e.to_string())?;
        self.tabs.try_reserve(1).map_err(|e| e.to_string())?;
        self.add_document(document);
        Ok(())
    }
    pub(super) fn save_to(&mut self, index: usize, path: &Path) -> Result<(), String> {
        let target = if path.exists() {
            path.canonicalize()
        } else {
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .canonicalize()
                .map(|parent| parent.join(path.file_name().unwrap_or_default()))
        }
        .map_err(|e| e.to_string())?;
        if self
            .documents
            .iter()
            .enumerate()
            .any(|(other, doc)| other != index && doc.path.as_ref() == Some(&target))
        {
            return Err("That file is open in another document. Choose another name.".into());
        }
        drawer_core::files::save(&mut self.documents[index], &target).map_err(|e| e.to_string())
    }
    fn save_document(&mut self, index: usize) -> Result<bool, String> {
        let path = match self.documents[index].path.clone() {
            Some(path) => path,
            None => {
                let filename = Path::new(&self.documents[index].title)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
                    + ".drawer";
                let Some(path) = rfd::FileDialog::new()
                    .set_title("Save Drawer document")
                    .add_filter("Drawer document", &["drawer"])
                    .set_file_name(filename)
                    .save_file()
                else {
                    return Ok(false);
                };
                path
            }
        };
        self.save_to(index, &path)?;
        Ok(true)
    }
    pub(super) fn request_close(&mut self, tab: usize) {
        if self.pending_close.is_some() || tab >= self.tabs.len() {
            return;
        }
        let id = self.tabs[tab].document_id;
        let count = self.tabs.iter().filter(|t| t.document_id == id).count();
        let document = self.documents.iter().find(|doc| doc.id == id).unwrap();
        if count == 1 && document.dirty {
            self.pending_close = Some(self.tabs[tab].id);
        } else {
            self.remove_tab(tab);
        }
    }
    fn remove_tab(&mut self, index: usize) {
        self.sync_active_view();
        let selected = self.active_tab.map(|tab| self.tabs[tab].id);
        let removed = self.tabs.remove(index);
        if !self
            .tabs
            .iter()
            .any(|tab| tab.document_id == removed.document_id)
        {
            self.documents.retain(|doc| doc.id != removed.document_id);
        }
        self.active_tab = None;
        self.active = None;
        if !self.tabs.is_empty() {
            let target = selected
                .and_then(|id| self.tabs.iter().position(|tab| tab.id == id))
                .unwrap_or(index.min(self.tabs.len() - 1));
            self.select_tab(target);
        }
    }
    pub(super) fn begin_quit(&mut self) {
        self.quit_requested = true;
        self.advance_quit();
    }
    fn advance_quit(&mut self) {
        if let Some(id) = self
            .documents
            .iter()
            .find(|doc| doc.dirty && !self.quit_approved.contains(&doc.id))
            .map(|doc| doc.id)
        {
            self.pending_close = self
                .tabs
                .iter()
                .rev()
                .find(|tab| tab.document_id == id)
                .map(|tab| tab.id);
        } else {
            self.pending_close = None;
            self.quit_ready = true;
        }
    }
    pub(super) fn accept_close(&mut self) {
        let Some(id) = self.pending_close.take() else {
            return;
        };
        let Some(tab) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        if self.quit_requested {
            self.quit_approved.insert(self.tabs[tab].document_id);
            self.advance_quit();
        } else {
            self.remove_tab(tab);
        }
    }
    pub(super) fn cancel_close(&mut self) {
        self.pending_close = None;
        self.quit_requested = false;
        self.quit_ready = false;
        self.quit_approved.clear();
    }
    pub(super) fn document_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Open…").clicked()
                && let Some(paths) = rfd::FileDialog::new()
                    .set_title("Open local drawings")
                    .add_filter("Drawer / PNG", &["drawer", "png"])
                    .pick_files()
            {
                for path in paths {
                    if let Err(error) = self.open_path(&path) {
                        self.status = Some(format!("{}: {error}", path.display()));
                    }
                }
            }
            if let Some(active) = self.active {
                if ui.button("Save").clicked()
                    && let Err(error) = self.save_document(active)
                {
                    self.status = Some(error);
                }
                let (undo, redo) = self.documents[active].history_counts();
                if ui
                    .add_enabled(undo > 0, egui::Button::new("Undo"))
                    .clicked()
                {
                    self.documents[active].undo();
                }
                if ui
                    .add_enabled(redo > 0, egui::Button::new("Redo"))
                    .clicked()
                {
                    self.documents[active].redo();
                }
                if ui.button("Duplicate view").clicked() {
                    self.duplicate_view();
                }
            }
        });
        let mut activate = None;
        let mut close = None;
        ui.horizontal_wrapped(|ui| {
            for (index, tab) in self.tabs.iter().enumerate() {
                let document = self
                    .documents
                    .iter()
                    .find(|doc| doc.id == tab.document_id)
                    .unwrap();
                ui.push_id(tab.id, |ui| {
                    let label = format!(
                        "{}{}",
                        document.title,
                        if document.dirty { " *" } else { "" }
                    );
                    let title = ui.selectable_label(self.active_tab == Some(index), label);
                    if let Some(path) = document.path.as_ref().or(document.source_path.as_ref()) {
                        title.clone().on_hover_text(path.display().to_string());
                    }
                    if title.clicked() {
                        activate = Some(index);
                    }
                    let button = ui.small_button("×");
                    #[cfg(test)]
                    {
                        self.close_buttons.insert(tab.id, button.rect);
                    }
                    if button.clicked() {
                        close = Some(index);
                    }
                });
            }
        });
        if let Some(index) = activate {
            self.select_tab(index);
        }
        if let Some(index) = close {
            self.request_close(index);
        }
    }
    pub(super) fn close_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.pending_close else {
            return;
        };
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == id) else {
            self.pending_close = None;
            return;
        };
        let Some(document) = self
            .documents
            .iter()
            .position(|doc| doc.id == tab.document_id)
        else {
            self.pending_close = None;
            return;
        };
        let title = self.documents[document].title.clone();
        let mut action = None;
        egui::Modal::new(egui::Id::new("close-document")).show(ctx, |ui| {
            ui.heading(format!("Save changes to {title}?"));
            ui.label("Your drawing will stay open until you choose.");
            ui.horizontal(|ui| {
                let save = ui.button("Save");
                if save.clicked() {
                    action = Some(0);
                }
                let discard = ui.button("Discard");
                if discard.clicked() {
                    action = Some(1);
                }
                let cancel = ui.button("Cancel");
                if cancel.clicked() {
                    action = Some(2);
                }
                #[cfg(test)]
                {
                    self.discard_close_button = Some(discard.rect);
                    self.cancel_close_button = Some(cancel.rect);
                    self.save_close_button = Some(save.rect);
                }
            });
            if let Some(status) = &self.status {
                ui.colored_label(egui::Color32::LIGHT_RED, status);
            }
        });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            action = Some(2);
        }
        match action {
            Some(0) => match self.save_document(document) {
                Ok(true) => self.accept_close(),
                Ok(false) => {}
                Err(error) => self.status = Some(error),
            },
            Some(1) => self.accept_close(),
            Some(2) => self.cancel_close(),
            _ => {}
        }
        ctx.request_repaint();
    }
}

pub(super) fn empty_approvals() -> HashSet<u64> {
    HashSet::new()
}
