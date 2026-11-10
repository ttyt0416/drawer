use super::*;
use drawer_core::file_browser::{self as core, Filter, Settings, Sort, View};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
#[derive(Clone, Copy, Default)]
pub(super) enum Mode {
    #[default]
    Open,
    Image,
    Layer {
        document: u64,
        revision: u64,
        frame: u32,
    },
}
enum Value {
    Listing(core::Listing),
    Pick(PathBuf),
    Thumbnail {
        path: PathBuf,
        value: Result<core::Thumbnail, String>,
        done: usize,
        total: usize,
    },
    ThumbnailsDone,
    Event(core::Event),
    Folder(PathBuf),
}
struct Job {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Value, String>>,
    generation: u64,
    mutating: bool,
    source: Option<PathBuf>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
struct Thumb {
    image: Option<egui::TextureHandle>,
    notice: String,
}
pub(super) struct Panel {
    recycle: fn(&Path, Arc<AtomicBool>) -> Result<(), String>,
    open: bool,
    mode: Mode,
    draft: Settings,
    saved: Settings,
    blocked: bool,
    replace: bool,
    address: String,
    name: String,
    listing: Option<core::Listing>,
    selected: Option<PathBuf>,
    selections: std::collections::BTreeSet<PathBuf>,
    anchor: Option<PathBuf>,
    opens: std::collections::VecDeque<PathBuf>,
    open_errors: Vec<String>,
    ticket: Option<core::Ticket>,
    job: Option<Job>,
    generation: u64,
    pending: Option<PathBuf>,
    thumbs: BTreeMap<PathBuf, Thumb>,
    thumb_order: std::collections::VecDeque<PathBuf>,
    pause_thumbs: bool,
    message: String,
    controls: BTreeMap<&'static str, egui::Rect>,
    rows: BTreeMap<PathBuf, egui::Rect>,
    focus: Option<PathBuf>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            recycle: drawer_platform::file_browser::recycle,
            open: false,
            mode: Mode::default(),
            draft: Settings::default(),
            saved: Settings::default(),
            blocked: false,
            replace: false,
            address: String::new(),
            name: String::new(),
            listing: None,
            selected: None,
            selections: Default::default(),
            anchor: None,
            opens: Default::default(),
            open_errors: vec![],
            ticket: None,
            job: None,
            generation: 0,
            pending: None,
            thumbs: BTreeMap::new(),
            thumb_order: std::collections::VecDeque::new(),
            pause_thumbs: false,
            message: String::new(),
            controls: BTreeMap::new(),
            rows: BTreeMap::new(),
            focus: None,
        }
    }
}
fn job(
    generation: u64,
    mutating: bool,
    source: Option<PathBuf>,
    work: impl FnOnce(Arc<AtomicBool>) -> Result<Value, String> + Send + 'static,
) -> Result<Job, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("local-file-browser".into())
        .spawn(move || {
            let _ = tx.send(work(flag));
        })
        .map_err(|e| e.to_string())?;
    Ok(Job {
        cancel,
        rx,
        generation,
        mutating,
        source,
    })
}
impl Panel {
    pub(super) fn mutating(&self) -> bool {
        self.job.as_ref().is_some_and(|j| j.mutating)
    }
    fn queue(&mut self, path: PathBuf) {
        if self.mutating() {
            self.message = "Wait for the physical file operation".into();
            return;
        }
        if let Some(j) = &self.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
        self.generation = self.generation.saturating_add(1);
        self.address = path.display().to_string();
        self.pending = Some(path);
        self.listing = None;
        self.selected = None;
        self.selections.clear();
        self.anchor = None;
        self.ticket = None;
        self.thumbs.clear();
        self.thumb_order.clear();
        self.pause_thumbs = false;
    }
}
impl Drawer {
    pub(super) fn load_file_browser(&mut self, storage: &UserStorage) {
        match Settings::load(storage) {
            Ok(s) => {
                self.file_browser.draft = s.clone();
                self.file_browser.saved = s;
            }
            Err(e) => {
                self.file_browser.blocked = true;
                self.file_browser.message = e;
            }
        }
    }
    pub(super) fn begin_file_browser(&mut self, mode: Mode) -> Result<(), String> {
        let p = &mut self.file_browser;
        p.open = true;
        p.mode = mode;
        p.opens.clear();
        p.open_errors.clear();
        if p.mutating() {
            return Err("Wait for the physical file operation".into());
        }
        if matches!(mode, Mode::Image | Mode::Layer { .. }) {
            p.draft.filter = Filter::Images;
        }
        if p.listing.is_none() {
            let path = p
                .draft
                .folder
                .clone()
                .or_else(|| self.storage.as_ref().map(|s| s.root().to_owned()))
                .or_else(|| std::env::current_dir().ok())
                .ok_or("Enter a local folder")?;
            p.queue(path);
        }
        Ok(())
    }
    fn save_browser_settings(&self, p: &mut Panel) -> Result<(), String> {
        if p.blocked && !p.replace {
            return Err(
                "Unreadable browser settings are protected; choose Replace settings explicitly"
                    .into(),
            );
        }
        p.draft
            .save(self.storage.as_ref().ok_or("Local settings unavailable")?)?;
        p.saved = p.draft.clone();
        p.blocked = false;
        p.replace = false;
        Ok(())
    }
    fn pick_browser_file(
        &mut self,
        p: &mut Panel,
        ctx: &egui::Context,
        path: &Path,
    ) -> Result<(), String> {
        drawer_platform::file_browser::local_disk(path)?;
        let path = core::checked(path)?;
        if path.is_dir() {
            p.queue(path);
            return Ok(());
        }
        match p.mode {
            Mode::Open => self.open_exchange_file(ctx, &path)?,
            Mode::Image => {
                if !core::image(&path) {
                    return Err("Select a supported still image".into());
                }
                self.begin_import(ctx, &path);
            }
            Mode::Layer {
                document,
                revision,
                frame,
            } => {
                if self.active.is_none_or(|n| {
                    (
                        self.documents[n].id,
                        self.documents[n].revision,
                        self.documents[n].current_frame,
                    ) != (document, revision, frame)
                }) {
                    return Err("Original layer destination changed; reopen the picker".into());
                }
                self.begin_import_layer(ctx, &path)?;
            }
        }
        p.open = false;
        Ok(())
    }
    fn browser_event(&mut self, p: &mut Panel, event: core::Event) -> Result<(), String> {
        match event {
            core::Event::Renamed { old, new } => {
                for d in &mut self.documents {
                    if d.path.as_ref().is_some_and(|q| core::same(q, &old)) {
                        if old
                            .file_name()
                            .is_some_and(|n| d.title == n.to_string_lossy())
                        {
                            d.title = new.file_name().unwrap_or_default().to_string_lossy().into();
                        } else if old
                            .file_stem()
                            .is_some_and(|n| d.title == n.to_string_lossy())
                        {
                            d.title = new.file_stem().unwrap_or_default().to_string_lossy().into();
                        }
                    }
                    if let Some(path) = d.path.as_ref().and_then(|q| core::remap(q, &old, &new)) {
                        d.path = Some(path);
                    }
                    if let Some(path) = d
                        .source_path
                        .as_ref()
                        .and_then(|q| core::remap(q, &old, &new))
                    {
                        d.source_path = Some(path);
                    }
                }
                self.protected_sources = self
                    .protected_sources
                    .iter()
                    .map(|q| core::remap(q, &old, &new).unwrap_or_else(|| q.clone()))
                    .collect();
                for f in &mut p.draft.favorites {
                    if let Some(path) = core::remap(f, &old, &new) {
                        *f = path;
                    }
                }
                if let Some(path) = p
                    .draft
                    .folder
                    .as_ref()
                    .and_then(|q| core::remap(q, &old, &new))
                {
                    p.draft.folder = Some(path);
                }
                self.recent.remap(&old,&new,self.storage.as_ref()).map_err(|e|format!("File renamed; live paths updated, but recent settings could not be saved: {e}"))?;
                p.message = format!(
                    "Renamed to {}. File bytes and drawing Undo are unchanged.",
                    new.display()
                );
            }
            core::Event::Recycled(path) => {
                for d in &mut self.documents {
                    if d.path
                        .as_ref()
                        .is_some_and(|q| core::remap(q, &path, &path).is_some())
                    {
                        d.path = None;
                        d.dirty = true;
                    }
                }
                p.message = format!(
                    "Moved {} to the OS Recycle Bin. Open drawing content stays in memory; use Save as.",
                    path.display()
                );
            }
        }
        Ok(())
    }
    fn start_browser_thumbnails(&self, p: &mut Panel, paths: Vec<PathBuf>) -> Result<(), String> {
        let held = p.listing.as_ref().map_or(0, |l| {
            l.entries
                .iter()
                .map(|e| e.path.to_string_lossy().len() * 2 + e.name.len() * 2 + 512)
                .sum::<usize>()
        }) + p.thumbs.len() * 96 * 96 * 4
            + paths
                .iter()
                .map(|v| v.to_string_lossy().len() * 2 + 64)
                .sum::<usize>();
        let available = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(
                self.documents
                    .iter()
                    .map(Document::resident_bytes)
                    .sum::<usize>(),
            )
            .saturating_sub(held)
            .min(64 * 1024 * 1024);
        if available < 1024 * 1024 {
            return Err("Not enough workspace memory for thumbnails".into());
        }
        for path in &paths {
            p.thumbs.remove(path);
            p.thumb_order.retain(|q| q != path);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::sync_channel(2);
        let total = paths.len();
        std::thread::Builder::new()
            .name("local-thumbnail-cache".into())
            .spawn(move || {
                for (n, path) in paths.into_iter().enumerate() {
                    if flag.load(Ordering::Relaxed) {
                        break;
                    }
                    let value = drawer_platform::file_browser::local_disk(&path)
                        .and_then(|_| core::thumbnail(&path, available, &flag));
                    if tx
                        .send(Ok(Value::Thumbnail {
                            path,
                            value,
                            done: n + 1,
                            total,
                        }))
                        .is_err()
                    {
                        return;
                    }
                }
                let _ = tx.send(Ok(Value::ThumbnailsDone));
            })
            .map_err(|e| e.to_string())?;
        p.job = Some(Job {
            cancel,
            rx,
            generation: p.generation,
            mutating: false,
            source: None,
        });
        Ok(())
    }
    fn browser_command(
        &mut self,
        p: &mut Panel,
        ctx: &egui::Context,
        key: &str,
    ) -> Result<(), String> {
        match key {
            "go" => p.queue(PathBuf::from(p.address.trim())),
            "home" => p.queue(
                self.storage
                    .as_ref()
                    .map(|s| s.root().to_owned())
                    .or_else(|| std::env::current_dir().ok())
                    .ok_or("Local home unavailable")?,
            ),
            "parent" => {
                let root = p
                    .listing
                    .as_ref()
                    .ok_or("Read a folder first")?
                    .folder
                    .clone();
                p.queue(root.parent().ok_or("Already at drive root")?.into());
            }
            "refresh" => {
                let path = p
                    .listing
                    .as_ref()
                    .map(|l| l.folder.clone())
                    .unwrap_or_else(|| PathBuf::from(&p.address));
                p.queue(path);
            }
            "save" => {
                self.save_browser_settings(p)?;
                p.message = "Browser preferences saved locally; drawing history unchanged".into();
            }
            "favorite" => {
                p.draft
                    .favorite(&p.listing.as_ref().ok_or("Read a folder first")?.folder)?;
            }
            "open" => {
                if matches!(p.mode, Mode::Open) && p.selections.len() > 1 {
                    if p.selections.len() > 128 {
                        return Err("Open at most128 local files at once".into());
                    }
                    if p.selections.iter().any(|q| q.is_dir()) {
                        return Err("Open one folder or multiple files".into());
                    }
                    p.opens = p.selections.iter().cloned().collect();
                    p.open_errors.clear();
                    p.open = false;
                } else {
                    let path = p.selected.clone().ok_or("Select a file/folder")?;
                    self.pick_browser_file(p, ctx, &path)?;
                }
            }
            "rename" | "recycle" => {
                if p.selections.len() > 1 {
                    return Err("Select one physical target for rename/recycle review".into());
                }
                if p.job.is_some() {
                    return Err("Finish or cancel the current browser job first".into());
                }
                let path = p.selected.as_ref().ok_or("Select a target")?;
                drawer_platform::file_browser::local_disk(path)?;
                if self.storage.as_ref().is_some_and(|s| {
                    core::remap(s.root(), path, path).is_some()
                        || core::remap(path, s.root(), s.root()).is_some()
                }) {
                    return Err("Active application data folders are managed separately; they cannot be renamed or recycled here".into());
                }
                p.ticket = Some(core::Ticket::new(
                    path,
                    (key == "rename").then_some(p.name.as_str()),
                )?);
                p.message =
                    "Review the exact source and destination below. Nothing has changed yet."
                        .into();
            }
            "cancel-operation" => p.ticket = None,
            "confirm-operation" => {
                let ticket = p.ticket.take().ok_or("Review a target first")?;
                let source = Some(ticket.source.path.clone());
                let recycle = p.recycle;
                p.job = Some(job(p.generation, true, source, move |flag| {
                    ticket
                        .run(
                            true,
                            &flag,
                            |a, b| {
                                drawer_platform::local_files::publish_no_replace(a, b)
                                    .map_err(|e| e.to_string())
                            },
                            |path, _| recycle(path, flag.clone()),
                        )
                        .map(Value::Event)
                })?);
            }
            "new-folder" => {
                if p.job.is_some() {
                    return Err("Finish or cancel the browser job first".into());
                }
                let root = p
                    .listing
                    .as_ref()
                    .ok_or("Read a local folder first")?
                    .folder
                    .clone();
                let name = p.name.clone();
                drawer_platform::file_browser::local_disk(&root)?;
                p.job = Some(job(p.generation, true, None, move |flag| {
                    drawer_core::asset_catalog::canceled(&flag)?;
                    core::new_folder(&root, &name).map(Value::Folder)
                })?);
            }
            "thumb-selected" | "thumb-all" => {
                if p.job.is_some() {
                    return Err("Finish or cancel the current browser job first".into());
                }
                let list = p.listing.as_ref().ok_or("Read a folder first")?;
                let paths = list
                    .entries
                    .iter()
                    .filter(|e| {
                        !e.directory
                            && e.unavailable.is_none()
                            && core::supported(&e.path)
                            && (key == "thumb-all" || Some(&e.path) == p.selected.as_ref())
                    })
                    .map(|e| e.path.clone())
                    .collect::<Vec<_>>();
                if paths.is_empty() {
                    return Err("Select a supported thumbnail source".into());
                }
                p.pause_thumbs = false;
                self.start_browser_thumbnails(p, paths)?;
            }

            "cancel" => {
                p.pause_thumbs = true;
                if let Some(j) = &p.job {
                    j.cancel.store(true, Ordering::Relaxed);
                }
            }
            "close" => {
                p.open = false;
                p.ticket = None;
                p.pending = None;
                if let Some(j) = &p.job
                    && !j.mutating
                {
                    j.cancel.store(true, Ordering::Relaxed);
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn file_browser_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.file_browser);
        if let Some(j) = &p.job {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
            let value = match j.rx.try_recv() {
                Ok(v) => Some(v),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(Err("Local browser worker stopped".into())),
            };
            if let Some(value) = value {
                let j = p.job.as_ref().unwrap();
                let mutating = j.mutating;
                let generation = j.generation;
                let canceled = j.cancel.load(Ordering::Relaxed);
                let source = j.source.clone();
                if !matches!(&value, Ok(Value::Thumbnail { .. })) {
                    p.job = None;
                }
                if mutating || generation == p.generation && !canceled {
                    match value {
                        Ok(Value::Listing(list)) => {
                            p.address = list.folder.display().to_string();
                            p.draft.folder = Some(list.folder.clone());
                            p.listing = Some(list);
                            p.message = "Local folder ready. Select a file; Enter opens it.".into();
                        }
                        Ok(Value::Pick(path)) => {
                            if let Err(e) = self.pick_browser_file(&mut p, ctx, &path) {
                                p.message = e;
                            }
                        }
                        Ok(Value::Thumbnail {
                            path,
                            value,
                            done,
                            total,
                        }) => {
                            let thumb = match value {
                                Ok(v) => Thumb {
                                    image: Some(ctx.load_texture(
                                        path.display().to_string(),
                                        egui::ColorImage::from_rgba_unmultiplied(
                                            v.size,
                                            &v.pixels.iter().flatten().copied().collect::<Vec<_>>(),
                                        ),
                                        egui::TextureOptions::LINEAR,
                                    )),
                                    notice: v.notice.unwrap_or_default(),
                                },
                                Err(e) => Thumb {
                                    image: None,
                                    notice: e,
                                },
                            };
                            p.thumb_order.retain(|q| q != &path);
                            p.thumb_order.push_back(path.clone());
                            p.thumbs.insert(path, thumb);
                            while p.thumbs.len() > 128 {
                                if let Some(old) = p.thumb_order.pop_front() {
                                    p.thumbs.remove(&old);
                                } else {
                                    break;
                                }
                            }
                            p.message =
                                format!("Thumbnail cache {done}/{total}; source files unchanged");
                        }
                        Ok(Value::ThumbnailsDone) => {
                            p.message =
                                "Thumbnail refresh finished; cache only, no source files changed"
                                    .into()
                        }
                        Ok(Value::Event(event)) => {
                            let result = self.browser_event(&mut p, event);
                            let root = p.listing.as_ref().map(|l| l.folder.clone());
                            if let Some(root) = root {
                                p.queue(root);
                            }
                            if let Err(e) = result {
                                p.message = e;
                            }
                        }
                        Ok(Value::Folder(path)) => {
                            p.message = format!("Created {}", path.display());
                            if let Some(root) = path.parent() {
                                p.queue(root.into());
                            }
                        }
                        Err(e) => {
                            if let Some(source) = source
                                && !source.exists()
                            {
                                for d in &mut self.documents {
                                    if d.path
                                        .as_ref()
                                        .is_some_and(|q| core::remap(q, &source, &source).is_some())
                                    {
                                        d.path = None;
                                        d.dirty = true;
                                    }
                                }
                            }
                            p.message = e;
                        }
                    }
                    if mutating {
                        self.status = Some(p.message.clone());
                    }
                }
            }
        }
        if p.job.is_none()
            && let Some(path) = p.pending.take()
        {
            let settings = p.draft.clone();
            p.job = match job(p.generation, false, None, move |flag| {
                drawer_platform::file_browser::local_disk(&path)?;
                let path = core::checked(&path)?;
                if path.is_file() {
                    Ok(Value::Pick(path))
                } else {
                    core::scan(&path, &settings, &flag).map(Value::Listing)
                }
            }) {
                Ok(j) => Some(j),
                Err(e) => {
                    p.message = e;
                    None
                }
            };
        }
        if !p.opens.is_empty() && self.import_dialog.is_none() && p.job.is_none() {
            let path = p.opens.pop_front().unwrap();
            if let Err(e) = self.pick_browser_file(&mut p, ctx, &path) {
                p.open_errors.push(format!("{}: {e}", path.display()));
            }
            if p.opens.is_empty() {
                self.status = Some(if p.open_errors.is_empty() {
                    "Selected local files opened".into()
                } else {
                    p.open_errors.join("\n")
                });
            }
        }
        if !p.open {
            if p.draft != p.saved
                && !p.blocked
                && self.storage.is_some()
                && let Err(e) = self.save_browser_settings(&mut p)
            {
                self.status = Some(format!(
                    "Physical operation result retained; browser preferences not saved: {e}"
                ));
            }
            self.file_browser = p;
            return;
        }
        let before = p.draft.clone();
        let mut command = None;
        let mut goto = None;
        let mut open = true;
        let mut focused = None;
        let mut visible_thumbnails = vec![];
        p.controls.clear();
        p.rows.clear();
        egui::Window::new("Local files").id(egui::Id::new("local-files")).default_pos(egui::pos2(20.,20.)).default_width(790.).max_height(660.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{for(key,label)in[("open","Open selected"),("parent","Parent"),("home","Home"),("refresh","Refresh folder"),("save","Save preferences"),("close","Close")]{let r=ui.add_enabled(!p.mutating(),egui::Button::new(label));p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}});
            ui.horizontal(|ui|{let r=ui.text_edit_singleline(&mut p.address);p.controls.insert("address",r.rect);if r.lost_focus()&&ui.input(|i|i.key_pressed(egui::Key::Enter)){command=Some("go");}let r=ui.button("Go");p.controls.insert("go",r.rect);if r.clicked(){command=Some("go");}});
            ui.horizontal_wrapped(|ui|{ui.selectable_value(&mut p.draft.view,View::Details,"Details");let r=ui.selectable_value(&mut p.draft.view,View::Thumbnails,"Thumbnails");p.controls.insert("thumbnails",r.rect);ui.label("Sort");for v in[Sort::Name,Sort::Extension,Sort::Modified,Sort::ExtensionModified]{ui.selectable_value(&mut p.draft.sort,v,format!("{v:?}"));}let r=ui.checkbox(&mut p.draft.descending,"Descending");p.controls.insert("descending",r.rect);});
            ui.horizontal_wrapped(|ui|{ui.label("File type");for v in[Filter::All,Filter::Supported,Filter::Native,Filter::Images]{ui.selectable_value(&mut p.draft.filter,v,format!("{v:?}"));}ui.label("Name contains");ui.text_edit_singleline(&mut p.draft.name);let r=ui.button("Favorite this folder");p.controls.insert("favorite",r.rect);if r.clicked(){command=Some("favorite");}});
            ui.collapsing(format!("Favorite folders ({})",p.draft.favorites.len()),|ui|{egui::ScrollArea::vertical().id_salt("favorite-folders").max_height(80.).show(ui,|ui|{
ui.horizontal_wrapped(|ui|{let mut remove=None;for(n,path)in p.draft.favorites.iter().enumerate(){if ui.small_button(path.display().to_string()).clicked(){goto=Some(path.clone());}
if ui.small_button("×").on_hover_text("Remove favorite only; keep files").clicked(){remove=Some(n);}}if let Some(n)=remove{p.draft.favorites.remove(n);}});
            });});
            ui.horizontal_wrapped(|ui|{ui.label("New name / folder");if ui.text_edit_singleline(&mut p.name).changed(){p.ticket=None;}for(key,label)in[("new-folder","New folder"),("rename","Review rename"),("recycle","Review Recycle Bin"),("thumb-selected","Refresh selected thumbnail"),("thumb-all","Refresh all thumbnails")]{let r=ui.add_enabled(p.job.is_none(),egui::Button::new(label));p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}});
            if let Some(t)=&p.ticket{ui.group(|ui|{ui.label(format!("Source: {}{}",t.source.path.display(),if t.source.directory{" (folder and contents)"}else{""}));ui.label(t.destination.as_ref().map_or("Move to the OS Recycle Bin only; no permanent-delete fallback".into(),|v|format!("New path: {}. Renaming does not convert the file format.",v.display())));for(key,label)in[("confirm-operation","Confirm physical operation"),("cancel-operation","Cancel operation")]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}});}
            if p.job.is_some(){ui.horizontal(|ui|{ui.spinner();let r=ui.button("Cancel job");p.controls.insert("cancel",r.rect);if r.clicked(){command=Some("cancel");}});}
            if p.blocked{ui.checkbox(&mut p.replace,"Replace unreadable browser settings when saving");}
            ui.label(&p.message);
            if let Some(list)=&p.listing{
                if list.truncated{ui.colored_label(egui::Color32::YELLOW,"Showing the first8192 matches; narrow the name/type filter or enter a path directly.");}
                ui.small("Ctrl/Shift-click selects up to128 files for Open; image/layer pickers use one file. Folders stay first. Arrows/Home/End select, Enter opens, Backspace goes to parent, F5 refreshes, F2 reviews rename, Delete reviews Recycle Bin. Links/network drives are not followed.");
                let row_height=if p.draft.view==View::Thumbnails{60.}else{24.};
                let mut scroll=egui::ScrollArea::vertical().id_salt("local-file-rows").max_height(270.);
                if let Some(n)=p.focus.as_ref().and_then(|path|list.entries.iter().position(|e|&e.path==path)){scroll=scroll.vertical_scroll_offset(n as f32*row_height);}
                scroll.show_rows(ui,row_height,list.entries.len(),|ui,range|{for n in range{let row=&list.entries[n];ui.push_id(&row.path,|ui|{ui.horizontal(|ui|{
                    if p.draft.view==View::Thumbnails{
                        if let Some(t)=p.thumbs.get(&row.path){if let Some(image)=&t.image{ui.image((image.id(),egui::vec2(54.,54.)));}else{ui.label("No preview").on_hover_text(&t.notice);}
                            p.thumb_order.retain(|q|q!=&row.path);p.thumb_order.push_back(row.path.clone());
                        }else{ui.label(if row.directory{"Folder"}else{"Preview pending"});if !row.directory&&row.unavailable.is_none()&&core::supported(&row.path){visible_thumbnails.push(row.path.clone());}}
                    }
                    let label=format!("{}{}  · {}  · {}  · {}{}",if row.directory{"▸ "}else{""},row.name,row.extension(),if row.directory{"Folder".into()}else{format!("{} bytes",row.bytes)},core::modified_label(row.modified),if row.readonly{" · read-only"}else{""});
                    let r=ui.add_sized([ui.available_width().min(690.),row_height],egui::Button::new(&label).selected(p.selections.contains(&row.path)).wrap_mode(egui::TextWrapMode::Truncate));p.rows.insert(row.path.clone(),r.rect.intersect(ui.clip_rect()));
                    if r.clicked(){
                        let modifiers=ui.input(|i|i.modifiers);
                        if matches!(p.mode,Mode::Open)&&modifiers.shift {
                            let a=p.anchor.as_ref().and_then(|q|list.entries.iter().position(|e|&e.path==q)).unwrap_or(n);let lo=a.min(n);let hi=a.max(n);
                            if hi-lo<128{p.selections=list.entries[lo..=hi].iter().map(|e|e.path.clone()).collect();}else{p.message="Selection is limited to128 files; narrow the filter".into();}
                        }else if matches!(p.mode,Mode::Open)&&modifiers.ctrl {
                            if !p.selections.remove(&row.path){if p.selections.len()<128{p.selections.insert(row.path.clone());}else{p.message="Selection limit128".into();}}
                        }else{p.selections.clear();p.selections.insert(row.path.clone());p.anchor=Some(row.path.clone());}
                        p.selected=if p.selections.contains(&row.path){Some(row.path.clone())}else{p.selections.iter().next().cloned()};p.name=p.selected.as_ref().and_then(|q|q.file_name()).map_or(String::new(),|v|v.to_string_lossy().into());p.ticket=None;r.request_focus();
                    }
                    if p.focus.as_ref()==Some(&row.path){r.request_focus();r.scroll_to_me(Some(egui::Align::Center));p.focus=None;}
                    if r.has_focus(){focused=Some(row.path.clone());}
if r.double_clicked(){p.selected=Some(row.path.clone());p.selections.clear();p.selections.insert(row.path.clone());command=Some("open");}
                    let mut hover=format!("{}\n{}",row.path.display(),label);if let Some(reason)=&row.unavailable{hover.push_str(&format!("\nUnavailable: {reason}"));}
if let Some(t)=p.thumbs.get(&row.path)&&!t.notice.is_empty(){hover.push_str(&format!("\n{}",t.notice));}r.on_hover_text(hover);
                });});}});

            }
        });
        if let Some(path) = focused {
            if let Some(list) = &p.listing {
                let n = list
                    .entries
                    .iter()
                    .position(|e| core::same(&e.path, p.selected.as_ref().unwrap_or(&path)))
                    .unwrap_or(0);
                let last = list.entries.len().saturating_sub(1);
                let next = ctx.input(|i| {
                    if i.key_pressed(egui::Key::ArrowDown) {
                        Some((n + 1).min(last))
                    } else if i.key_pressed(egui::Key::ArrowUp) {
                        Some(n.saturating_sub(1))
                    } else if i.key_pressed(egui::Key::Home) {
                        Some(0)
                    } else if i.key_pressed(egui::Key::End) {
                        Some(last)
                    } else {
                        None
                    }
                });
                if let Some(n) = next
                    && let Some(row) = list.entries.get(n)
                {
                    p.selected = Some(row.path.clone());
                    p.selections.clear();
                    p.selections.insert(row.path.clone());
                    p.anchor = Some(row.path.clone());
                    p.focus = Some(row.path.clone());
                    p.name = row.name.clone();
                    p.ticket = None;
                }
            }
            command = ctx.input(|i| {
                if i.key_pressed(egui::Key::Enter) {
                    Some("open")
                } else if i.key_pressed(egui::Key::Backspace) {
                    Some("parent")
                } else if i.key_pressed(egui::Key::F5) {
                    Some("refresh")
                } else if i.key_pressed(egui::Key::F2) {
                    Some("rename")
                } else if i.key_pressed(egui::Key::Delete) {
                    Some("recycle")
                } else {
                    command
                }
            });
        }
        if let Some(path) = goto {
            p.queue(path);
        }
        if before.filter != p.draft.filter || before.name != p.draft.name {
            let path = p
                .listing
                .as_ref()
                .map(|l| l.folder.clone())
                .unwrap_or_else(|| PathBuf::from(&p.address));
            p.queue(path);
        } else if (before.sort != p.draft.sort || before.descending != p.draft.descending)
            && let Some(l) = &mut p.listing
        {
            core::sort(&mut l.entries, p.draft.sort, p.draft.descending);
        }
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            command = Some(if p.ticket.is_some() {
                "cancel-operation"
            } else {
                "close"
            });
        }
        if let Some(key) = command
            && let Err(e) = self.browser_command(&mut p, ctx, key)
        {
            p.message = e;
        }
        if before.view != p.draft.view {
            p.pause_thumbs = false;
        }
        if p.job.is_none()
            && p.pending.is_none()
            && !p.pause_thumbs
            && !visible_thumbnails.is_empty()
            && p.open
            && let Err(e) = self.start_browser_thumbnails(&mut p, visible_thumbnails)
        {
            p.message = e;
            p.pause_thumbs = true;
        }
        if p.draft != p.saved
            && !p.blocked
            && self.storage.is_some()
            && let Err(e) = self.save_browser_settings(&mut p)
        {
            p.message = format!("View is usable; preferences were not saved: {e}");
        }
        self.file_browser = p;
    }
}
#[path = "file_browser_verify.rs"]
mod verify;
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_picker_pointer_sort_favorite_manage_and_source_preservation() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}
