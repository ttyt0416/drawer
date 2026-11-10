use super::*;
use drawer_core::asset_catalog::{
    self as core, Index, Kind, Metadata, Movement, ReplaceSize, Sort,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
};
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    loaded: bool,
    index: Option<Arc<Index>>,
    token: Option<String>,
    blocked: bool,
    reset: bool,
    root: String,
    recursive: bool,
    query: String,
    kind: Option<Kind>,
    folder: String,
    children: bool,
    favorites: bool,
    sort: Sort,
    details: bool,
    selected: Option<PathBuf>,
    edit: Metadata,
    tags: String,
    history: core::History,
    job: Option<Job>,
    draft: Option<Draft>,
    size: ReplaceSize,
    reviewed: bool,
    register_name: String,
    delete_source: bool,
    delete_confirmed: bool,
    review_refs: usize,
    restore: Option<(PathBuf, Movement)>,
    restore_confirmed: bool,
    message: String,
    textures: BTreeMap<String, egui::TextureHandle>,
    controls: BTreeMap<&'static str, egui::Rect>,
    rows: BTreeMap<PathBuf, egui::Rect>,
}
struct Draft {
    prepared: Box<core::Prepared>,
    preview: egui::TextureHandle,
}
enum Value {
    Index(Index),
    Preview(Box<core::Prepared>),
}
struct Job {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    rx: mpsc::Receiver<Result<Value, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
enum FileEdit {
    Delete(Movement),
    Restore(Movement),
}
impl Drawer {
    fn catalog_file(&self) -> Result<PathBuf, String> {
        Ok(self
            .storage
            .as_ref()
            .ok_or("Local user storage is unavailable")?
            .root()
            .join("asset-index.json"))
    }
    fn reload_asset_catalog(&self, p: &mut Panel) -> Result<(), String> {
        let path = self.catalog_file()?;
        p.loaded = true;
        p.job = None;
        p.draft = None;
        p.history = Default::default();
        p.selected = None;
        p.textures.clear();
        if !path.exists() {
            p.index = None;
            p.token = None;
            p.blocked = false;
            p.message = "Choose a local folder and build its offline index".into();
            return Ok(());
        }
        match Index::read(&path) {
            Ok((index, token)) => {
                p.root = index.root.display().to_string();
                p.recursive = index.recursive;
                p.index = Some(Arc::new(index));
                p.token = Some(token);
                p.blocked = false;
                p.message="Local asset index loaded. Refresh checks source versions; existing artwork stays embedded.".into();
                Ok(())
            }
            Err(e) => {
                p.index = None;
                p.token = None;
                p.blocked = true;
                p.message = format!(
                    "Existing index is unreadable and preserved: {e}. Confirm replacement to rebuild it."
                );
                Ok(())
            }
        }
    }
    pub(super) fn begin_asset_catalog(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let mut p = std::mem::take(&mut self.asset_catalog);
        if !p.loaded {
            self.reload_asset_catalog(&mut p)?;
        }
        p.open = true;
        self.asset_catalog = p;
        Ok(())
    }
    fn select_asset(p: &mut Panel, path: PathBuf) {
        if let Some(index) = &p.index
            && let Ok(entry) = index.get(&path)
        {
            p.edit = entry.metadata.clone();
            p.tags = p.edit.tags.join(", ");
            p.selected = Some(path);
            p.delete_confirmed = false;
            p.reviewed = false;
            p.draft = None;
        }
    }
    fn publish_catalog(
        &self,
        p: &mut Panel,
        next: Index,
        file_edit: Option<FileEdit>,
    ) -> Result<(), String> {
        if p.blocked && !p.reset {
            return Err(
                "Confirm replacement of the unreadable local index before rebuilding".into(),
            );
        }
        let before = p.index.clone();
        let token = match next.write(
            &self.catalog_file()?,
            p.token.as_deref(),
            p.blocked && p.reset,
        ) {
            Ok(t) => t,
            Err(e) => {
                if let Some(change) = &file_edit {
                    let result = match change {
                        FileEdit::Delete(m) => m.apply(false),
                        FileEdit::Restore(m) => m.apply(true),
                    };
                    if let Err(rollback) = result {
                        return Err(format!(
                            "Index save failed: {e}; source rollback failed: {rollback}. Inspect the original and local Trash paths."
                        ));
                    }
                }
                return Err(e);
            }
        };
        let next = Arc::new(next);
        if let Some(before) = before {
            match file_edit {
                Some(FileEdit::Restore(m)) => p.history.record_restore(before, next.clone(), m),
                Some(FileEdit::Delete(m)) => p.history.record(before, next.clone(), Some(m)),
                None => p.history.record(before, next.clone(), None),
            }
        }
        p.root = next.root.display().to_string();
        p.recursive = next.recursive;
        p.index = Some(next);
        p.token = Some(token);
        p.blocked = false;
        p.reset = false;
        p.textures.clear();
        if let Some(selected) = p.selected.clone() {
            Self::select_asset(p, selected);
        }
        Ok(())
    }
    fn start_asset_scan(&self, p: &mut Panel) -> Result<(), String> {
        if p.job.is_some() {
            return Err("Cancel or finish the running asset job first".into());
        }
        if p.blocked && !p.reset {
            return Err("Confirm index replacement before rebuilding".into());
        }
        let path = PathBuf::from(&p.root);
        let recursive = p.recursive;
        let previous = p.index.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicUsize::new(0));
        let (c, pr) = (cancel.clone(), progress.clone());
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("index-local-assets".into())
            .spawn(move || {
                let _ = tx.send(
                    core::scan(&path, recursive, previous.as_deref(), &c, &pr).map(Value::Index),
                );
            })
            .map_err(|e| e.to_string())?;
        p.job = Some(Job {
            cancel,
            progress,
            rx,
        });
        p.message = "Indexing local files. Cancel keeps the previous index.".into();
        Ok(())
    }
    fn preview_asset(&mut self, p: &mut Panel, replace: bool) -> Result<(), String> {
        if p.job.is_some() {
            return Err("Cancel or finish the running asset job first".into());
        }
        self.finish_stroke();
        if self.drawing.has_path_draft() {
            return Err("Commit/cancel the path draft before inserting an asset".into());
        }
        let n = self.active.ok_or("Open a drawing first")?;
        let d = &self.documents[n];
        let used = self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>();
        if used.saturating_add(d.memory_bytes().saturating_mul(5)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Asset preview workspace memory limit".into());
        }
        let index = p.index.clone().ok_or("Index a local folder first")?;
        let path = p.selected.clone().ok_or("Select an image asset")?;
        let target = if replace {
            match self.panels.targets.get(&d.id) {
                Some(drawer_core::thumbnail::Target::Source(
                    drawer_core::layer_search::Key::Object(id),
                )) => Some(*id),
                _ => return Err("Select a material image object to replace".into()),
            }
        } else {
            None
        };
        let mut copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        copy.id = d.id;
        copy.revision = d.revision;
        copy.path = d.path.clone();
        copy.source_path = d.source_path.clone();
        let size = p.size;
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(used);
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let progress = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("preview-local-asset".into())
            .spawn(move || {
                let _ = tx.send(
                    core::prepare(&index, &path, &copy, target, size, budget, &c)
                        .map(|prepared| Value::Preview(Box::new(prepared))),
                );
            })
            .map_err(|e| e.to_string())?;
        p.job = Some(Job {
            cancel,
            progress,
            rx,
        });
        p.draft = None;
        p.reviewed = false;
        p.message = "Preparing an embedded image snapshot…".into();
        Ok(())
    }
    fn apply_asset(&mut self, p: &mut Panel) -> Result<(), String> {
        if !p.reviewed {
            return Err(
                "Review the embedded version, conversion and replacement size first".into(),
            );
        }
        let n = self.active.ok_or("Activate the source drawing")?;
        let draft = p.draft.as_ref().ok_or("Prepare a preview first")?;
        draft.prepared.check(&self.documents[n])?;
        let object = draft.prepared.object;
        self.documents[n].commit_state(draft.prepared.state.clone())?;
        self.panels.targets.insert(
            self.documents[n].id,
            drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Object(object)),
        );
        p.draft = None;
        p.reviewed = false;
        p.message =
            "Embedded asset applied as one document Undo. Library history is separate.".into();
        if let (Some(index), Some(path)) = (p.index.clone(), p.selected.clone()) {
            match index
                .used(&path)
                .and_then(|next| self.publish_catalog(p, next, None))
            {
                Ok(()) => {}
                Err(e) => {
                    p.message = format!("Embedded asset applied; usage metadata was not saved: {e}")
                }
            }
        }
        Ok(())
    }
    fn register_asset(&mut self, p: &mut Panel) -> Result<(), String> {
        self.finish_stroke();
        if self.drawing.has_path_draft() {
            return Err("Commit/cancel the path draft first".into());
        }
        let n = self.active.ok_or("Open a drawing first")?;
        let d = &self.documents[n];
        let index = p.index.clone().ok_or("Index a local folder first")?;
        let target = self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0));
        let key = drawer_core::layer_catalog::from_thumbnail(d, target)
            .ok_or("Choose a layer to register")?;
        let path = core::register(
            &index.root,
            d,
            key,
            &p.register_name,
            &AtomicBool::new(false),
        )?;
        let existed = index.get(&path).is_ok();
        let mut next = core::refresh_one(&index, &path, &AtomicBool::new(false))?;
        if !existed {
            let mut metadata = next.get(&path)?.metadata.clone();
            metadata.name = p.register_name.clone();
            next = next.edited(&path, metadata)?;
        }
        if let Err(e) = self.publish_catalog(p, next, None) {
            return Err(format!(
                "The local PNG copy was saved to {}; index update failed: {e}. Refresh to recover the list.",
                index.root.join(&path).display()
            ));
        }
        Self::select_asset(p, path);
        p.message="Registered an owned PNG snapshot with its document RGB profile of the current layer/selection. Original artwork/source unchanged; equal content reuses its existing copy/name.".into();
        Ok(())
    }
    fn catalog_history(&self, p: &mut Panel, redo: bool) -> Result<(), String> {
        let index = p.index.clone().ok_or("No local index")?;
        let file = self.catalog_file()?;
        let result = if redo {
            p.history.redo(&index, &file, p.token.as_deref())?
        } else {
            p.history.undo(&index, &file, p.token.as_deref())?
        };
        if let Some((index, token)) = result {
            p.root = index.root.display().to_string();
            p.recursive = index.recursive;
            p.index = Some(index);
            p.token = Some(token);
            p.textures.clear();
            p.draft = None;
            if let Some(path) = p.selected.clone() {
                Self::select_asset(p, path);
            }
            p.message = "Local list/source operation restored. Document history unchanged.".into();
        }
        Ok(())
    }
    fn open_catalog_preset(&mut self, p: &mut Panel) -> Result<(), String> {
        let index = p.index.clone().ok_or("Index a folder first")?;
        let path = p.selected.clone().ok_or("Select an asset")?;
        let entry = index.get(&path)?;
        let (absolute, _) = index.contents(&path, &AtomicBool::new(false))?;
        match entry.kind {
            Kind::Palette => self.stage_catalog_palette(&absolute)?,
            Kind::Gradient => self.stage_catalog_gradient(&absolute)?,
            Kind::Brush | Kind::BrushBundle => {
                self.stage_catalog_brush(&absolute, entry.kind == Kind::BrushBundle)?
            }
            _ => return Err("Use image insert/replace for image assets".into()),
        }
        let next = index.used(&path)?;
        self.publish_catalog(p, next, None)?;
        p.open = false;
        Ok(())
    }
    pub(super) fn asset_catalog_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.asset_catalog);
        if !p.open {
            self.asset_catalog = p;
            return;
        }
        if let Some(job) = &p.job {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
            let result = match job.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(Err("Asset worker stopped".into())),
            };
            if let Some(result) = result {
                p.job = None;
                match result {
                    Ok(Value::Index(index)) => {
                        let count = index.entries.len();
                        match self.publish_catalog(&mut p, index, None) {
                            Ok(()) => {
                                p.message =
                                    format!("Indexed {count} local assets; source files unchanged")
                            }
                            Err(e) => p.message = e,
                        }
                    }
                    Ok(Value::Preview(prepared)) => {
                        let checked =
                            self.active
                                .ok_or("Activate the source drawing")
                                .and_then(|n| {
                                    prepared
                                        .check(&self.documents[n])
                                        .map_err(|_| "Document changed during preview")
                                });
                        match checked {
                            Ok(()) => match drawer_core::layer_merge::preview(
                                &prepared.state,
                                self.documents[self.active.unwrap()].current_frame,
                            ) {
                                Ok(copy) => {
                                    let preview =
                                        super::layer_merge::image(ctx, &copy, "catalog-preview");
                                    p.draft = Some(Draft { prepared, preview });
                                    p.message="Review the embedded version, size and conversion; Apply is one document Undo".into();
                                }
                                Err(e) => p.message = e,
                            },
                            Err(e) => p.message = e.into(),
                        }
                    }
                    Err(e) => p.message = e,
                }
            }
        }
        let mut open = true;
        let mut action = None;
        let mut choose_root = false;
        let mut choose_restore = false;
        let mut select = None;
        p.controls.clear();
        p.rows.clear();
        egui::Window::new("Local asset library — offline folders").default_pos(egui::pos2(15.,20.)).default_width(920.).max_height(640.).open(&mut open).show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{for(key,label,enabled)in[("scan","Build / refresh index",p.job.is_none()),("cancel-job","Cancel indexing / preview",p.job.is_some()),("undo","Undo library",p.history.counts().0>0&&p.job.is_none()),("redo","Redo library",p.history.counts().1>0&&p.job.is_none()),("reload","Reload index",p.job.is_none()),("apply","Apply preview",p.draft.is_some()&&p.reviewed), ("cancel-preview","Cancel preview",p.draft.is_some())]{let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key,r.rect);if r.clicked(){action=Some(key);}}});
            ui.horizontal(|ui|{ui.label("Local root");ui.add_enabled(p.job.is_none(),egui::TextEdit::singleline(&mut p.root));choose_root=ui.add_enabled(p.job.is_none(),egui::Button::new("Browse folder…")).clicked();ui.add_enabled(p.job.is_none(),egui::Checkbox::new(&mut p.recursive,"Index subfolders"));});
            if p.blocked{ui.checkbox(&mut p.reset,"Confirm replacing the unreadable local index; source files stay unchanged");}
            if let Some(job)=&p.job{ui.label(format!("Visited {} entries",job.progress.load(Ordering::Relaxed)));}
            ui.horizontal_wrapped(|ui|{ui.label("Search name / path / tags");ui.text_edit_singleline(&mut p.query);egui::ComboBox::from_id_salt("catalog-kind").selected_text(p.kind.map_or("All types",Kind::label)).show_ui(ui,|ui|{ui.selectable_value(&mut p.kind,None,"All types");for kind in Kind::ALL{ui.selectable_value(&mut p.kind,Some(kind),kind.label());}});ui.checkbox(&mut p.favorites,"Favorites");ui.checkbox(&mut p.details,"Details / thumbnails");});
            ui.horizontal_wrapped(|ui|{ui.label("Logical folder");ui.text_edit_singleline(&mut p.folder);ui.checkbox(&mut p.children,"Include child folders");egui::ComboBox::from_id_salt("catalog-sort").selected_text(format!("{:?}",p.sort)).show_ui(ui,|ui|{for sort in [Sort::Name,Sort::Path,Sort::Uses]{ui.selectable_value(&mut p.sort,sort,format!("{sort:?}"));}});});
            let r=ui.checkbox(&mut p.reviewed,"I reviewed the preview's embedded version, size and conversion");p.controls.insert("review",r.rect);
            ui.label(&p.message);
            egui::ScrollArea::vertical().id_salt("catalog-body").max_height(400.).show(ui,|ui|{
                if let Some(index)=p.index.clone(){let rows=index.search(&p.query,p.kind,&p.folder,p.children,p.favorites,p.sort);ui.label(format!("{} matches / {} entries; duplicate files share a content hash",rows.len(),index.entries.len()));
                    egui::ScrollArea::vertical().id_salt("catalog-list").max_height(175.).show_rows(ui,if p.details{56.}else{72.},rows.len(),|ui,range|{for row in range{let entry=&index.entries[rows[row]];ui.horizontal(|ui|{if !entry.thumbnail.is_empty(){let texture=p.textures.entry(entry.sha256.clone()).or_insert_with(||ctx.load_texture(format!("catalog-{}",entry.sha256),egui::ColorImage::from_rgba_unmultiplied([16,16],&entry.thumbnail),egui::TextureOptions::NEAREST));ui.image((texture.id(),egui::vec2(40.,40.)));}let label=if p.details{format!("{} | {} | {} | uses {}{}{}",entry.metadata.name,entry.kind.label(),entry.path.display(),entry.metadata.uses,if entry.metadata.pinned{" | pinned"}else{""},if entry.metadata.favorite{" | favorite"}else{""})}else{format!("{} · {}{}",entry.metadata.name,entry.kind.label(),if entry.available(){""}else{" · unavailable"})};let r=ui.selectable_label(p.selected.as_ref()==Some(&entry.path),label);p.rows.insert(entry.path.clone(),r.rect);if r.clicked(){select=Some(entry.path.clone());}
if p.details&&let Some(reason)=&entry.reason{ui.colored_label(egui::Color32::YELLOW,reason);}});}});
                    if let Some(path)=&p.selected&&let Ok(entry)=index.get(path){ui.separator();ui.label(format!("Source: {}",index.root.join(path).display()));ui.small(format!("SHA256: {}",entry.sha256));if let Some(reason)=&entry.reason{ui.colored_label(egui::Color32::YELLOW,reason);}
if let Some(duplicate)=&entry.duplicate_of{ui.label(format!("Duplicate content of {}",duplicate.display()));}
                        ui.horizontal(|ui|{ui.label("Name");ui.text_edit_singleline(&mut p.edit.name);ui.label("Folder");ui.text_edit_singleline(&mut p.edit.folder);});ui.horizontal(|ui|{ui.label("Tags (comma-separated)");ui.text_edit_singleline(&mut p.tags);ui.checkbox(&mut p.edit.pinned,"Pin");ui.checkbox(&mut p.edit.favorite,"Favorite");let r=ui.button("Save metadata");p.controls.insert("metadata",r.rect);if r.clicked(){action=Some("metadata");}});
                        ui.horizontal_wrapped(|ui|{for(key,label,enabled)in[("insert","Preview insert image",entry.available()&&entry.kind==Kind::Image),("replace","Preview replace selected image",entry.available()&&entry.kind==Kind::Image),("preset","Open matching preset exchange",entry.available()&&entry.kind!=Kind::Image)]{let r=ui.add_enabled(enabled&&p.job.is_none(),egui::Button::new(label));p.controls.insert(key,r.rect);if r.clicked(){action=Some(key);}}});ui.add_enabled_ui(p.job.is_none(),|ui|{ui.horizontal(|ui|{let a=ui.selectable_value(&mut p.size,ReplaceSize::KeepPixels,"Keep new pixel size").changed();let b=ui.selectable_value(&mut p.size,ReplaceSize::FitExisting,"Fit existing pixels (nearest)").changed();if a||b{p.draft=None;p.reviewed=false;}});});
                        let references=core::reference_count(&self.documents,&index.root,entry);if p.review_refs!=references{p.delete_confirmed=false;p.review_refs=references;}ui.label(format!("{references} exact-version/source-path reference(s) in currently open drawings; embedded versions stay in documents"));if ui.checkbox(&mut p.delete_source,"Also move this source file to local Trash (only this file, not its resources/folder)").changed(){p.delete_confirmed=false;}ui.checkbox(&mut p.delete_confirmed,"Confirm the shown path, reference count and deletion scope");let r=ui.add_enabled(p.delete_confirmed&&p.job.is_none(),egui::Button::new("Remove selected asset"));p.controls.insert("delete",r.rect);if r.clicked(){action=Some("delete");}
                    }
                    for note in &index.notes{ui.small(note);}
                }
                ui.separator();ui.horizontal(|ui|{ui.label("Register current layer / selection as owned PNG");ui.text_edit_singleline(&mut p.register_name);let r=ui.add_enabled(p.index.is_some()&&p.job.is_none(),egui::Button::new("Register snapshot"));p.controls.insert("register",r.rect);if r.clicked(){action=Some("register");}});ui.small("Registration copies a PNG snapshot with the document RGB profile; original artwork is unchanged. Image objects embed RGBA8; use Image Import for precise16-bit raster layers.");
                choose_restore=ui.button("Choose local Trash record to restore…").clicked();if let Some((path,movement))=&p.restore{ui.label(format!("Restore {} → {}",movement.root.join(&movement.trash).display(),movement.root.join(&movement.original).display()));ui.small(path.display().to_string());ui.checkbox(&mut p.restore_confirmed,"Confirm restoration to this exact path; existing files are never replaced");if ui.add_enabled(p.restore_confirmed,egui::Button::new("Restore source file")).clicked(){action=Some("restore");}}
                if let Some(draft)=&p.draft{ui.separator();for note in &draft.prepared.notes{ui.label(note);}let size=draft.preview.size_vec2();let scale=(450./size.x).min(150./size.y).min(1.);ui.image((draft.preview.id(),size*scale));}
            });
        });
        if let Some(path) = select {
            Self::select_asset(&mut p, path);
        }
        if choose_root
            && let Some(root) = rfd::FileDialog::new()
                .set_title("Choose an offline asset folder")
                .pick_folder()
        {
            p.root = root.display().to_string();
        }
        if choose_restore {
            let result = (|| {
                let index = p
                    .index
                    .as_ref()
                    .ok_or("Index its original local folder first")?;
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(index.root.join(".drawer-trash"))
                    .add_filter("Drawer local Trash record", &["json"])
                    .pick_file()
                {
                    let movement = Movement::read(&index.root, &path)?;
                    p.restore = Some((path, movement));
                    p.restore_confirmed = false;
                }
                Ok::<(), String>(())
            })();
            if let Err(e) = result {
                p.message = e;
            }
        }
        let result = match action {
            Some("scan") => self.start_asset_scan(&mut p),
            Some("cancel-job") => {
                p.job = None;
                p.message = "Asset job canceled; prior catalog and artwork retained".into();
                Ok(())
            }
            Some("undo") => self.catalog_history(&mut p, false),
            Some("redo") => self.catalog_history(&mut p, true),
            Some("reload") => self.reload_asset_catalog(&mut p),
            Some("metadata") => {
                let result = (|| {
                    let index = p.index.clone().ok_or("No catalog")?;
                    let path = p.selected.clone().ok_or("Select an asset")?;
                    let mut metadata = p.edit.clone();
                    metadata.tags = p
                        .tags
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect();
                    let next = index.edited(&path, metadata)?;
                    self.publish_catalog(&mut p, next, None)
                })();
                result
                    .map(|()| p.message = "Asset metadata saved locally; drawing unchanged".into())
            }
            Some("insert") => self.preview_asset(&mut p, false),
            Some("replace") => self.preview_asset(&mut p, true),
            Some("apply") => self.apply_asset(&mut p),
            Some("cancel-preview") => {
                p.draft = None;
                p.reviewed = false;
                p.message = "Preview canceled; drawing unchanged".into();
                Ok(())
            }
            Some("register") => self.register_asset(&mut p),
            Some("preset") => self.open_catalog_preset(&mut p),
            Some("delete") => (|| {
                let index = p.index.clone().ok_or("No catalog")?;
                let path = p.selected.clone().ok_or("Select an asset")?;
                let references =
                    core::reference_count(&self.documents, &index.root, index.get(&path)?);
                let (next, movement) = core::forget(
                    &index,
                    &path,
                    p.delete_source,
                    p.delete_confirmed,
                    references,
                )?;
                self.publish_catalog(&mut p, next, movement.map(FileEdit::Delete))?;
                p.selected = None;
                p.delete_confirmed = false;
                p.message="Removed from the index. Explicit source moves can be restored with library Undo or their local Trash record.".into();
                Ok(())
            })(),
            Some("restore") => (|| {
                let index = p.index.clone().ok_or("No catalog")?;
                let (path, movement) = p.restore.clone().ok_or("Choose a Trash record")?;
                let original = core::restore_record(&index.root, &path, p.restore_confirmed)?;
                let next = match core::refresh_one(&index, &original, &AtomicBool::new(false))
                    .and_then(|next| next.edited(&original, movement.metadata.clone()))
                {
                    Ok(next) => next,
                    Err(e) => {
                        movement.apply(true)?;
                        return Err(e);
                    }
                };
                self.publish_catalog(&mut p, next, Some(FileEdit::Restore(movement)))?;
                p.restore = None;
                p.restore_confirmed = false;
                Self::select_asset(&mut p, original);
                p.message = "Source and list restored; artwork unchanged".into();
                Ok(())
            })(),
            _ => Ok(()),
        };
        if let Err(e) = result {
            p.message = e;
        }
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            p.open = false;
            p.job = None;
            p.draft = None;
        }
        self.asset_catalog = p;
    }
}
#[path = "asset_catalog_verify.rs"]
mod verify;
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    verify::run(root)
}

impl Drawer {
    pub(super) fn quiesce_asset_catalog(&mut self, ctx: &egui::Context) -> bool {
        let open = self.asset_catalog.open;
        self.asset_catalog.open = false;
        self.asset_catalog_window(ctx);
        self.asset_catalog.open = open;
        self.asset_catalog.job.is_some()
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn offline_library_scan_pointer_insert_cancel_replace_undo_and_restart() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}
