use super::*;
use drawer_core::{
    file_browser::{Event, Ticket},
    work_library::{
        self as core, Database, Sort,
        index::{self, Index},
    },
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
enum Value {
    Index(Index),
    Recycled {
        path: PathBuf,
        database: Result<Database, String>,
    },
}
struct Job {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Value, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
pub(super) struct Panel {
    open: bool,
    db: Option<Database>,
    index: Option<Index>,
    selected: Option<u64>,
    title: String,
    labels: String,
    favorite: bool,
    ticket: Option<(u64, Ticket)>,
    job: Option<Job>,
    texture: Option<egui::TextureHandle>,
    notice: String,
    recovery_confirmed: bool,
    recycle: fn(&Path, Arc<AtomicBool>) -> Result<(), String>,
    controls: BTreeMap<&'static str, egui::Rect>,
    rows: BTreeMap<u64, egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            db: None,
            index: None,
            selected: None,
            title: String::new(),
            labels: String::new(),
            favorite: false,
            ticket: None,
            job: None,
            texture: None,
            notice: String::new(),
            recovery_confirmed: false,
            recycle: drawer_platform::file_browser::recycle,
            controls: BTreeMap::new(),
            rows: BTreeMap::new(),
        }
    }
}
impl Panel {
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub(super) fn cancel(&self) {
        if let Some(j) = &self.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
    fn select(&mut self, id: Option<u64>) {
        self.selected = id;
        self.texture = None;
        self.ticket = None;
        if let Some(e) = id.and_then(|id| self.db.as_ref()?.entry(id).ok()) {
            self.title = e.title.clone();
            self.labels = e.labels.join(", ");
            self.favorite = e.favorite;
        }
    }
    fn adopt(&mut self, db: Database) {
        if self
            .index
            .as_ref()
            .is_some_and(|i| i.source != db.signature())
        {
            self.index = None;
            self.texture = None;
        }
        self.db = Some(db);

        if self
            .selected
            .is_none_or(|id| self.db.as_ref().unwrap().entry(id).is_err())
        {
            self.select(
                self.db
                    .as_ref()
                    .unwrap()
                    .state
                    .entries
                    .first()
                    .map(|e| e.id),
            );
        }
    }
}
fn job(
    work: impl FnOnce(Arc<AtomicBool>) -> Result<Value, String> + Send + 'static,
) -> Result<Job, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("local-work-library".into())
        .spawn(move || {
            let _ = tx.send(work(flag));
        })
        .map_err(|e| e.to_string())?;
    Ok(Job { cancel, rx })
}
impl Drawer {
    fn work_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum())
    }
    pub(super) fn begin_work_library(&mut self) -> Result<(), String> {
        if self.work_library.busy() {
            self.work_library.open = true;
            return Ok(());
        }
        if self.work_budget() < 20 * 1024 * 1024 {
            return Err("Close large drawings before loading the work list".into());
        }
        let s = self.storage.as_ref().ok_or("Local storage unavailable")?;
        drawer_platform::file_browser::local_disk(s.root())?;
        let mut p = std::mem::take(&mut self.work_library);
        p.open = true;
        p.ticket = None;
        p.texture = None;
        match Database::load(s) {
            Ok(db) => {
                p.db = Some(db);
                p.index = None;
                p.notice="Registered paths remain in place. Thumbnails/status are cached; Refresh checks disk explicitly.".into();
                if self.work_budget() >= 64 * 1024 * 1024 {
                    match Index::load(s, p.db.as_ref().unwrap()) {
                        Ok(i) => p.index = i,
                        Err(e) => p.notice = e,
                    }
                }
                let id = p
                    .selected
                    .filter(|id| p.db.as_ref().unwrap().entry(*id).is_ok())
                    .or_else(|| p.db.as_ref().unwrap().state.entries.first().map(|e| e.id));
                p.select(id);
            }
            Err(e) => {
                p.db = None;
                p.index = None;
                p.notice = format!(
                    "{e}. Use confirmed library recovery below; original drawing files are unaffected."
                );
            }
        }
        self.work_library = p;
        Ok(())
    }
    fn register_work_paths(&mut self, paths: Vec<PathBuf>) -> Result<(), String> {
        if self.work_library.busy() {
            return Err("Wait for the current library job".into());
        }
        for path in &paths {
            drawer_platform::file_browser::local_disk(path)?;
        }
        let s = self.storage.as_ref().ok_or("Local storage unavailable")?;
        drawer_platform::file_browser::local_disk(s.root())?;
        let p = &mut self.work_library;
        let db =
            p.db.as_ref()
                .ok_or("Recover the library first")?
                .register(s, &paths)?;
        p.adopt(db);
        p.notice = "Registered local paths; no work file was moved or uploaded".into();
        Ok(())
    }
    fn work_action(
        &mut self,
        p: &mut Panel,
        key: &str,
        path: Option<PathBuf>,
    ) -> Result<(), String> {
        if p.busy() {
            return Err("Wait for the current library job".into());
        }
        let s = self.storage.as_ref().ok_or("Local storage unavailable")?;
        drawer_platform::file_browser::local_disk(s.root())?;
        if key == "recover" || key == "empty" {
            let db = Database::recover(s, key == "empty", p.recovery_confirmed)?;
            p.db = None;
            p.index = None;
            p.adopt(db);
            p.recovery_confirmed = false;
            p.notice="Recovered the library; the pre-recovery database was preserved separately. Drawing files were unchanged.".into();
            return Ok(());
        }
        let db = p.db.as_ref().ok_or("Recover the library first")?;
        let id = p.selected;
        let updated = match key {
            "undo" => Some(db.undo(s, false)?),
            "redo" => Some(db.undo(s, true)?),
            "save-info" => {
                let labels = p
                    .labels
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                Some(db.info(
                    s,
                    id.ok_or("Select a work")?,
                    p.title.clone(),
                    labels,
                    p.favorite,
                )?)
            }
            "remove" => Some(db.remove(s, id.ok_or("Select a work")?)?),
            "reconnect" => {
                let path = path.ok_or("Choose the replacement local file")?;
                drawer_platform::file_browser::local_disk(&path)?;
                Some(db.reconnect(s, id.ok_or("Select a work")?, &path)?)
            }

            "rebuild" => {
                let budget = self.work_budget().saturating_sub(32 * 1024 * 1024);
                let storage = s.clone();
                let db = db.clone();
                p.job = Some(job(move |flag| {
                    drawer_platform::file_browser::local_disk(storage.root())?;
                    index::rebuild(
                        &storage,
                        &db,
                        budget,
                        &flag,
                        drawer_platform::file_browser::local_disk,
                        || Ok(()),
                    )
                    .map(Value::Index)
                })?);
                p.notice="Rebuilding cached local metadata/thumbnails. The saved list and drawing files stay unchanged.".into();
                None
            }
            "recycle" => {
                let id = id.ok_or("Select a work")?;
                let e = db.entry(id)?;
                drawer_platform::file_browser::local_disk(&e.path)?;
                if super::data_maintenance::protected_storage_target(s, &e.path) {
                    return Err("Application data is managed in Local data; choose a work file outside protected storage".into());
                }
                p.ticket = Some((id, Ticket::new(&e.path, None)?));
                p.notice = "Review the exact file path. Nothing has been deleted.".into();
                None
            }
            "confirm-recycle" => {
                let (id, ticket) = p.ticket.take().ok_or("Review a file before recycling")?;
                let expected = db.entry(id)?.path.clone();
                if !drawer_core::file_browser::same(&expected, &ticket.source.path) {
                    return Err("Library path changed; review again".into());
                }
                let storage = s.clone();
                let db = db.clone();
                let recycle = p.recycle;
                p.job = Some(job(move |flag| {
                    drawer_platform::file_browser::local_disk(&ticket.source.path)?;
                    let event = ticket.run(
                        true,
                        &flag,
                        |_, _| Err("Only Recycle Bin is available".into()),
                        |path, _| recycle(path, flag.clone()),
                    )?;
                    let Event::Recycled(path) = event else {
                        return Err("Unexpected library file operation".into());
                    };
                    let database = db.remove(&storage, id);
                    Ok(Value::Recycled { path, database })
                })?);
                p.notice =
                    "OS Recycle Bin requested. Cancellation cannot undo a file already recycled."
                        .into();
                None
            }
            "discard-ticket" => {
                p.ticket = None;
                None
            }
            _ => None,
        };
        if let Some(db) = updated {
            p.adopt(db);
            p.ticket = None;
            p.notice="Saved library metadata. Library Undo/Redo is separate from drawing Undo; it never restores physically recycled files.".into();
        }
        Ok(())
    }
    pub(super) fn work_library_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.work_library);
        if let Some(j) = &p.job {
            let result = match j.rx.try_recv() {
                Ok(v) => Some(v),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Library worker stopped; reopen to inspect the last committed list".into(),
                )),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                p.job = None;
                match result {
                    Ok(Value::Index(index)) => {
                        p.index = Some(index);
                        p.texture = None;
                        p.notice="Rebuilt local cache. Missing/denied/corrupt entries stay registered; use Reconnect for moved files.".into();
                    }
                    Ok(Value::Recycled { path, database }) => {
                        let mut browser = std::mem::take(&mut self.file_browser);
                        let event = self.browser_event(&mut browser, Event::Recycled(path));
                        self.file_browser = browser;
                        p.index = None;
                        p.texture = None;
                        match database {
                            Ok(db) => {
                                p.adopt(db);
                                p.notice="Moved file to OS Recycle Bin and removed its list entry. Library Undo restores metadata only; recover the file using the OS Recycle Bin.".into();
                            }
                            Err(e) => {
                                p.notice = format!(
                                    "File recycled, but list update failed: {e}. Entry retained; refresh/reopen before editing."
                                )
                            }
                        }
                        if let Err(e) = event {
                            p.notice.push_str(&format!(" {e}"));
                        }
                    }
                    Err(e) => p.notice = e,
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
            }
        }
        if !p.open {
            self.work_library = p;
            return;
        }
        if p.texture.is_none()
            && let Some(t) = p
                .selected
                .and_then(|id| p.index.as_ref()?.row(id)?.thumbnail.as_ref())
        {
            let pixels = t.pixels.iter().flatten().copied().collect::<Vec<_>>();
            p.texture = Some(ctx.load_texture(
                "work-library-thumbnail",
                egui::ColorImage::from_rgba_unmultiplied(t.size, &pixels),
                egui::TextureOptions::LINEAR,
            ));
        }
        let mut open = true;
        let mut command = "";
        let mut picked = None;
        let mut register = false;
        let busy = p.busy();
        p.controls.clear();
        p.rows.clear();
        egui::Window::new("Local work library").id(egui::Id::new("work-library")).open(&mut open).default_pos(egui::pos2(25.,25.)).default_width(850.).max_height(650.).show(ctx,|ui|{
            let scroll=egui::ScrollArea::vertical().max_height(585.).show(ui,|ui|{
                ui.horizontal_wrapped(|ui|{for(key,label)in[("register","Register local files…"),("open","Open work"),("rebuild","Refresh / rebuild index"),("reload","Reload saved list"),("close","Close")]{let r=ui.add_enabled(!busy||key=="close",egui::Button::new(label));p.controls.insert(key,r.rect);if r.clicked(){command=key;}}});
                ui.small("Works stay in their original local files. Cached previews do not contact disks until you refresh/open/reconnect.");
                ui.add_enabled_ui(!busy,|ui|{
                    if let Some(db)=&p.db{
                        ui.horizontal_wrapped(|ui|{for(key,label,enabled)in[("undo","Undo library metadata",db.counts().0>0),("redo","Redo library metadata",db.counts().1>0),("remove","Remove list entry (keep file)",p.selected.is_some()),("recycle","Review file → Recycle Bin",p.selected.is_some())]{let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key,r.rect);if r.clicked(){command=key;}}});
                        let mut view=core::State{entries:vec![],sort:db.state.sort,descending:db.state.descending,favorites_only:db.state.favorites_only,path_labels:db.state.path_labels};
                        ui.horizontal_wrapped(|ui|{ui.label(crate::i18n::text("Sort"));for v in[Sort::Title,Sort::Path,Sort::Modified]{let r=ui.selectable_value(&mut view.sort,v,format!("{v:?}"));p.controls.insert(match v{Sort::Title=>"sort-title",Sort::Path=>"sort-path",Sort::Modified=>"sort-modified"},r.rect);}ui.checkbox(&mut view.descending,crate::i18n::text("Descending"));let r=ui.checkbox(&mut view.favorites_only,"Favorites only");p.controls.insert("favorites-only",r.rect);let r=ui.checkbox(&mut view.path_labels,"Show paths in list");p.controls.insert("path-labels",r.rect);});
                        if (view.sort,view.descending,view.favorites_only,view.path_labels)!=(db.state.sort,db.state.descending,db.state.favorites_only,db.state.path_labels){
                            match self.storage.as_ref().ok_or("Local storage unavailable".to_string()).and_then(|s|db.edit(s,"Change library view",|v,_|{v.sort=view.sort;v.descending=view.descending;v.favorites_only=view.favorites_only;v.path_labels=view.path_labels;Ok(())})){
                                Ok(db)=>{p.db=Some(db);},Err(e)=>p.notice=e
                            }
                        }
                        let db=p.db.as_ref().unwrap();
                        let mut selected=None;
                        ui.columns(2,|columns|{
                            egui::ScrollArea::vertical().id_salt("work-list").max_height(300.).show(&mut columns[0],|ui|{
                                let entries=index::ordered(db,p.index.as_ref());
                                for e in entries{
                                    let mut label=format!("{}{}",if e.favorite{"★ "}else{""},e.title);
                                    if db.state.path_labels{label.push_str(&format!(" · {}",e.path.display().to_string().chars().take(100).collect::<String>()));}
                                    let r=ui.selectable_label(p.selected==Some(e.id),label).on_hover_text(e.path.display().to_string());p.rows.insert(e.id,r.rect);if r.clicked(){selected=Some(e.id);}
                                }
                            });
                            let ui=&mut columns[1];
                            if let Some(id)=p.selected&&let Ok(e)=db.entry(id){
                                ui.label(e.path.display().to_string());ui.label(format!("Work {} · labels saved independently",e.id));
                                let r=ui.text_edit_singleline(&mut p.title);p.controls.insert("title",r.rect);
                                ui.label("Labels (comma separated)");let r=ui.text_edit_singleline(&mut p.labels);p.controls.insert("labels",r.rect);
                                let r=ui.checkbox(&mut p.favorite,"Favorite");p.controls.insert("favorite",r.rect);
                                ui.horizontal(|ui|{for(key,label)in[("save-info","Save title / labels / favorite"),("reconnect","Reconnect file…")]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){command=key;}}});
                                if let Some(row)=p.index.as_ref().and_then(|i|i.row(id)){
                                    ui.label(format!("{} bytes · {}{}",row.bytes.map_or("Unknown".into(),|n|n.to_string()),drawer_core::file_browser::modified_label(row.modified.and_then(|n|std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(n)))),if row.readonly{" · read-only"}else{""}));
                                    ui.label(&row.status);if let Some(t)=&p.texture{ui.image((t.id(),t.size_vec2()*1.5));}
                                }else{ui.label("No current cached information. Refresh to inspect the file; unavailable works remain listed.");}
                            }
                        });
                        if let Some(id)=selected{p.select(Some(id));}
                    }
                    if let Some((_,ticket))=&p.ticket{ui.separator();ui.strong("Physical file operation — separate from list removal");ui.label(ticket.source.path.display().to_string());ui.label(format!("{} bytes. OS Recycle Bin only. Library Undo cannot bring this file back; use the OS Recycle Bin.",ticket.source.bytes));ui.horizontal(|ui|{for(key,label)in[("confirm-recycle","Confirm OS Recycle Bin"),("discard-ticket","Cancel file operation")]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){command=key;}}});}
                    let recovery=ui.collapsing("Recover saved library metadata",|ui|{ui.label("Preserve the current database, then restore the previous healthy list or start empty. No drawing file is changed. Undo/Redo above restores up to32 committed library changes, including after restart.");let r=ui.checkbox(&mut p.recovery_confirmed,"I confirm library metadata recovery");p.controls.insert("recovery-confirm",r.rect);for(key,label)in[("recover","Restore previous healthy list"),("empty","Preserve database and start empty")]{let r=ui.add_enabled(p.recovery_confirmed,egui::Button::new(label));p.controls.insert(key,r.rect);if r.clicked(){command=key;}}});p.controls.insert("recovery-header",recovery.header_response.rect);
                });
                if busy{ui.spinner();if ui.button("Cancel current job").clicked(){p.cancel();}}
                ui.label(&p.notice);
            });p.controls.insert("viewport",scroll.inner_rect);
        });
        if command == "register" {
            if let Some(paths) = rfd::FileDialog::new()
                .add_filter(
                    "Local works",
                    &[
                        "drawer", "png", "jpg", "jpeg", "bmp", "tif", "tiff", "webp", "psd", "psb",
                    ],
                )
                .pick_files()
            {
                register = true;
                picked = Some(paths);
            }
        } else if command == "reconnect"
            && let Some(path) = rfd::FileDialog::new().pick_file()
        {
            picked = Some(vec![path]);
        }
        let result = match command {
            "" | "register" | "close" => Ok(()),
            "reload" => {
                self.work_library = p;
                let r = self.begin_work_library();
                p = std::mem::take(&mut self.work_library);
                r
            }
            "open" => (|| {
                let e =
                    p.db.as_ref()
                        .ok_or("Recover the library first")?
                        .entry(p.selected.ok_or("Select a work")?)?;
                drawer_platform::file_browser::local_disk(&e.path)?;
                self.open_exchange_file(ctx, &e.path)?;
                p.open = false;
                Ok(())
            })(),
            "reconnect" if picked.is_none() => Ok(()),
            key => self.work_action(
                &mut p,
                key,
                picked.as_ref().and_then(|v| v.first()).cloned(),
            ),
        };
        if let Err(e) = result {
            p.notice = e;
        }
        if !open || command == "close" {
            p.open = false;
            p.ticket = None;
            p.cancel();
            p.texture = None;
        }
        self.work_library = p;
        if register && let Err(e) = self.register_work_paths(picked.unwrap()) {
            self.work_library.notice = e;
        }
    }
    pub(super) fn quiesce_work_library(&mut self, ctx: &egui::Context) -> bool {
        let open = self.work_library.open;
        self.work_library.open = false;
        self.work_library_window(ctx);
        self.work_library.open = open;
        self.work_library.busy()
    }
}
#[path = "work_library_verify.rs"]
mod verify;
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    verify::run(root)
}
