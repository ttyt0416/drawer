use super::*;
use std::{collections::HashSet, path::Path};

pub(super) struct DocumentTab {
    pub id: u64,
    pub document_id: u64,
    pub view: drawer_core::view::View,
}

impl Drawer {
    pub(super) fn remember_file(&mut self, path: &Path) {
        if let Err(error) = self.recent.record(path, self.storage.as_ref()) {
            self.status = Some(format!(
                "Drawing is ready, but the recent list could not be saved: {error}"
            ));
        }
    }
    fn recent_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Recent files", |ui| {
            let paths = self.recent.paths().to_vec();
            if paths.is_empty() {
                ui.label("No recent files");
            }
            for path in paths {
                let missing = !path.is_file();
                if ui
                    .button(format!(
                        "{}{}",
                        path.display(),
                        if missing { " (missing)" } else { "" }
                    ))
                    .clicked()
                {
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("drawer"))
                    {
                        if let Err(error) = self.open_path(&path) {
                            self.status =
                                Some(format!("Cannot reopen {}: {error}", path.display()));
                        }
                    } else {
                        self.begin_import(ui.ctx(), &path);
                    }
                    ui.close();
                }
            }
            ui.separator();
            let (undo, redo) = self.recent.counts();
            let result = if ui
                .add_enabled(undo > 0, egui::Button::new("Undo list change"))
                .clicked()
            {
                self.recent.undo(self.storage.as_ref())
            } else if ui
                .add_enabled(redo > 0, egui::Button::new("Redo list change"))
                .clicked()
            {
                self.recent.redo(self.storage.as_ref())
            } else if crate::help::button(ui, "Clear list", "files").clicked() {
                self.recent.reset(self.storage.as_ref())
            } else if ui
                .add_enabled(
                    self.storage.is_some(),
                    egui::Button::new("Restore previous list"),
                )
                .clicked()
            {
                self.recent.restore(self.storage.as_ref().unwrap())
            } else {
                Ok(())
            };
            if let Err(error) = result {
                self.status = Some(error.to_string());
            }
        });
    }
    pub(super) fn add_document(&mut self, mut document: Document) {
        document.tool = self
            .tool_input
            .controller
            .persisted_tool()
            .unwrap_or(self.selected);
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
    pub(super) fn sync_active_view(&mut self) {
        if let (Some(tab), Some(document)) = (self.active_tab, self.active) {
            self.tabs[tab].view = self.documents[document].view;
        }
    }
    pub(super) fn select_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self
            .drawing
            .active_view()
            .is_some_and(|view| view != self.tabs[index].id)
        {
            self.finish_stroke();
        }
        self.sync_tool_properties();
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
        self.sync_tool_properties();
    }
    pub(super) fn duplicate_view(&mut self) {
        if self.tabs.len() >= 256 {
            self.status = Some("The workspace supports up to 256 views.".into());
            return;
        }
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
        self.precision.finish();
        self.drawing.mode = super::precision::Mode::Paint;
        self.drawing.connect_line = false;
        self.sync_tool_properties();
        self.tool_input.controller.cancel();
        self.selected = tip;
        if let Some(index) = self.active {
            self.documents[index].tool = tip;
        }
        self.sync_tool_properties();
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
            self.remember_file(&path);
            return Ok(());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if self.tabs.len() >= 256 {
            return Err("The workspace supports up to 256 views.".into());
        }
        let mut document =
            drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET.saturating_sub(used))
                .map_err(|e| e.to_string())?;
        let missing = document.scene.paper.missing();
        if !missing.is_empty() {
            self.status = Some(missing.join("; "));
        }
        if document
            .source_path
            .as_ref()
            .is_some_and(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("psd")))
        {
            self.status=Some("PSD opened as its merged preview. Layer effects and editable layers were not imported.".into());
        }
        document.view = self.default_view;
        self.documents.try_reserve(1).map_err(|e| e.to_string())?;
        self.tabs.try_reserve(1).map_err(|e| e.to_string())?;
        self.add_document(document);
        self.remember_file(&path);
        Ok(())
    }
    pub(super) fn save_to(&mut self, index: usize, path: &Path) -> Result<(), String> {
        self.finish_stroke();
        let _awake = self.synchronous_power();
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
        if self.protected_sources.contains(&target) {
            return Err("Choose another path to keep the imported source unchanged.".into());
        }
        drawer_core::files::save(&mut self.documents[index], &target).map_err(|e| e.to_string())?;
        self.remember_file(&target);
        Ok(())
    }
    // Native save dialogs confirm replacement; their cancellation is represented by None.
    pub(super) fn save_as_from(
        &mut self,
        index: usize,
        choose: impl FnOnce() -> Option<PathBuf>,
    ) -> Result<bool, String> {
        let Some(path) = choose() else {
            return Ok(false);
        };
        self.save_to(index, &path)?;
        Ok(true)
    }
    pub(super) fn save_as(&mut self, index: usize) -> Result<bool, String> {
        let filename = Path::new(&self.documents[index].title)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
            + ".drawer";
        self.save_as_from(index, || {
            rfd::FileDialog::new()
                .set_title("Save Drawer document as")
                .add_filter("Drawer document", &["drawer"])
                .set_file_name(filename)
                .save_file()
        })
    }
    pub(super) fn save_document(&mut self, index: usize) -> Result<bool, String> {
        if let Some(path) = self.documents[index].path.clone() {
            self.save_to(index, &path)?;
            Ok(true)
        } else {
            self.save_as(index)
        }
    }
    pub(super) fn export_png_to(&mut self, index: usize, path: &Path) -> Result<(), String> {
        self.finish_stroke();
        let _awake = self.synchronous_power();
        let target = self.export_target(index, path)?;
        let available = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        drawer_core::png_export::export(&self.documents[index], &target, available)
            .map_err(|e| e.to_string())
    }
    pub(super) fn export_target(&self, index: usize, path: &Path) -> Result<PathBuf, String> {
        let target = if path.exists() {
            path.canonicalize()
        } else {
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .canonicalize()
                .map(|p| p.join(path.file_name().unwrap_or_default()))
        }
        .map_err(|e| e.to_string())?;
        if self.protected_sources.contains(&target) {
            return Err("Choose another filename to keep the imported source unchanged.".into());
        }
        if self.documents.iter().enumerate().any(|(other, doc)| {
            other != index
                && (doc.path.as_ref() == Some(&target) || doc.source_path.as_ref() == Some(&target))
        }) {
            return Err("That source is open in another drawing. Choose another filename.".into());
        }
        Ok(target)
    }
    pub(super) fn export_from(
        &mut self,
        index: usize,
        choose: impl FnOnce() -> Option<PathBuf>,
    ) -> Result<bool, String> {
        let Some(path) = choose() else {
            return Ok(false);
        };
        self.export_png_to(index, &path)?;
        Ok(true)
    }
    pub(super) fn request_close(&mut self, tab: usize) {
        self.finish_stroke();
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
        self.finish_stroke();
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
        ui.horizontal_wrapped(|ui| {
            if crate::help::button(ui, "Open…", "files").clicked()
                && let Some(paths) = rfd::FileDialog::new()
                    .set_title("Open local drawings")
                    .add_filter("Drawer document", &["drawer"])
                    .add_filter("PNG", &["png"])
                    .add_filter("Raw RGBA8 PSD merged preview", &["psd"])
                    .pick_files()
            {
                for path in paths {
                    if let Err(error) = self.open_path(&path) {
                        self.status = Some(format!("{}: {error}", path.display()));
                    }
                }
            }
            if crate::help::button(ui, "Image as new document…", "files").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(
                        "PNG / JPEG / TIFF / BMP",
                        &["png", "jpg", "jpeg", "tif", "tiff", "bmp"],
                    )
                    .pick_file()
            {
                self.begin_import(ui.ctx(), &path);
            }
            if crate::help::button(ui, "New from clipboard", "files").clicked()
                && let Err(error) = self.document_from_clipboard(|| {
                    reference_pane::clipboard_image_with_budget(DEFAULT_DOCUMENT_BUDGET)
                })
            {
                self.status = Some(error);
            }
            self.recent_menu(ui);
            if crate::help::button(ui, "Scanner / camera…", "acquisition").clicked() {
                self.finish_stroke();
                self.acquisition.open = true;
            }
            self.view_menu(ui);
            self.local_file_menu(ui);
            if let Some(active) = self.active {
                if ui
                    .button(format!("Save  {}", self.workspaces.command_label("save")))
                    .clicked()
                    && let Err(error) = self.save_document(active)
                {
                    self.status = Some(error);
                }
                if ui
                    .button(format!(
                        "Save as…  {}",
                        self.workspaces.command_label("save-as")
                    ))
                    .clicked()
                    && let Err(error) = self.save_as(active)
                {
                    self.status = Some(error);
                }
                let (undo, redo) = self.documents[active].history_counts();
                if ui
                    .add_enabled(
                        undo > 0,
                        egui::Button::new(format!(
                            "Undo  {}",
                            self.workspaces.command_label("undo")
                        )),
                    )
                    .clicked()
                {
                    self.documents[active].undo();
                }
                if ui
                    .add_enabled(
                        redo > 0,
                        egui::Button::new(format!(
                            "Redo  {}",
                            self.workspaces.command_label("redo")
                        )),
                    )
                    .clicked()
                {
                    self.documents[active].redo();
                }
                if crate::help::button(ui, "Duplicate view", "files").clicked() {
                    self.duplicate_view();
                }
            }
        });
        self.document_tabs(ui);
        if self.active.is_some() {
            ui.horizontal_wrapped(|ui| {
                use drawer_core::sizing::Change;
                ui.label("Image:");
                for (label, change) in [
                    ("Flip left/right", Change::FlipHorizontal),
                    ("Flip up/down", Change::FlipVertical),
                    ("Rotate 90° right", Change::Rotate90 { clockwise: true }),
                    ("Rotate 90° left", Change::Rotate90 { clockwise: false }),
                ] {
                    let button = crate::help::button(ui, label, "files");
                    #[cfg(test)]
                    {
                        self.transform_buttons.insert(label, button.rect);
                    }
                    if button.clicked() {
                        self.begin_transform(ui.ctx(), change);
                    }
                }
                if crate::help::button(ui, "Layer / folder paper…", "textures").clicked() {
                    self.textures.open = true;
                }
                if crate::help::button(ui, "Export rendered PSD…", "textures").clicked() {
                    self.request_psd();
                }
                if crate::help::button(ui, "Export PNG…", "files").clicked()
                    && let Err(error) = self.export_from(self.active.unwrap(), || {
                        rfd::FileDialog::new()
                            .add_filter("PNG", &["png"])
                            .set_file_name("drawing.png")
                            .save_file()
                    })
                {
                    self.status = Some(error);
                }
            });
        }
    }
    pub(super) fn document_tabs(&mut self, ui: &mut egui::Ui) {
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
                        if document.dirty || self.drawing.changed(document.id) {
                            " *"
                        } else {
                            ""
                        }
                    );
                    let title =
                        crate::help::selectable(ui, self.active_tab == Some(index), label, "files");
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
                let save = crate::help::button(
                    ui,
                    format!("Save  {}", self.workspaces.command_label("save")),
                    "files",
                );
                if save.clicked() {
                    action = Some(0);
                }
                let discard = crate::help::button(ui, "Discard", "files");
                if discard.clicked() {
                    action = Some(1);
                }
                let cancel = crate::help::button(ui, "Cancel", "files");
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
