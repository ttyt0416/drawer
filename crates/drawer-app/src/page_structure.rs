use super::*;
use drawer_core::{
    comics::Kind,
    page_batch::{self, Size},
    page_work::Edit,
    scene::PageObject,
    sizing::DocumentState,
    story_text::Work,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    section: u8,
    mode: u8,
    range: String,
    position: usize,
    files: Vec<PathBuf>,
    directory: String,
    fit: bool,
    width: u32,
    height: u32,
    dpi: f64,
    physical: bool,
    split_auto: bool,
    split_x: u32,
    blank_count: u32,
    remove_originals: bool,
    confirm: bool,
    skip: bool,
    prepared: Option<Prepared>,
    job: Option<Job>,
    notice: String,
    image: Option<egui::TextureHandle>,
    frames: Vec<u64>,
    frame_context: Option<u64>,
    divisions: u32,
    vertical: bool,
    frame_prepared: Option<FramePrepared>,
    frame_image: Option<egui::TextureHandle>,
    bindings: Vec<Binding>,
    pub apply: Option<egui::Rect>,
    pub frame_apply: Option<egui::Rect>,
}
#[derive(Clone)]
struct Stamp {
    document: u64,
    revision: u64,
    frame: u32,
}
impl Stamp {
    fn of(d: &Document) -> Self {
        Self {
            document: d.id,
            revision: d.revision,
            frame: d.current_frame,
        }
    }
    fn matches(&self, d: &Document) -> bool {
        self.document == d.id && self.revision == d.revision && self.frame == d.current_frame
    }
}
struct FramePrepared {
    stamp: Stamp,
    state: DocumentState,
}
struct Binding {
    parent: Stamp,
    object: u64,
    child: u64,
}
struct Prepared {
    work: Work,
    stamps: Vec<Stamp>,
    batch: page_batch::Prepared,
    remove: Vec<u64>,
    replace: Option<u64>,
    index: usize,
}
struct Job {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<JobValue, String>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
enum JobValue {
    Prepared(Box<Prepared>),
    Resources(Box<Prepared>, drawer_core::page_resources::Prepared),
}
fn image(ctx: &egui::Context, d: &Document, name: &str) -> Result<egui::TextureHandle, String> {
    let r = drawer_core::render::Renderer::new(d);
    if let Some(e) = r.error() {
        return Err(e.into());
    }
    let scale = (d.width.max(d.height) as f64 / 220.).max(1.);
    let w = (d.width as f64 / scale).round().max(1.) as usize;
    let h = (d.height as f64 / scale).round().max(1.) as usize;
    let mut bytes = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            bytes.extend(r.pixel([
                (x as f64 + 0.5) * d.width as f64 / w as f64,
                (y as f64 + 0.5) * d.height as f64 / h as f64,
            ]));
        }
    }
    Ok(ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied([w, h], &bytes),
        egui::TextureOptions::LINEAR,
    ))
}
fn pdf(
    bytes: &[u8],
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<drawer_core::book_io::Page>, String> {
    #[cfg(windows)]
    {
        drawer_platform::book_pdf::render(bytes, "", 300., budget, cancel)
    }
    #[cfg(not(windows))]
    {
        let _ = (bytes, budget, cancel);
        Err("PDF import requires Windows".into())
    }
}
impl Drawer {
    pub(super) fn begin_page_structure(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Apply or cancel pending edits first".into());
        }
        let p = &mut self.page_structure;
        p.open = true;
        if p.width == 0 {
            p.width = 800;
            p.height = 1200;
            p.dpi = 300.;
            p.position = 1;
            p.blank_count = 1;
            p.divisions = 2;
            p.vertical = true;
            p.physical = true;
            p.split_auto = true;
            p.range = "1-2".into();
        }
        Ok(())
    }
    fn structure_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum())
    }
    fn structure_source(&self, path: &Path) -> Option<&Document> {
        self.page_work_document_id(path)
            .and_then(|id| self.documents.iter().find(|d| d.id == id))
            .or_else(|| {
                self.documents.iter().find(|d| {
                    d.path.as_ref().is_some_and(|p| {
                        p == path || p.canonicalize().ok().as_deref() == Some(path)
                    })
                })
            })
    }
    fn prepare_structure(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let p = &self.page_structure;
        if p.job.is_some() {
            return Err("Wait for the current preparation".into());
        }
        let work = self
            .active_page_work()
            .ok_or("Open a local project in Page manager first")?;
        let selected = if ![1, 2, 3].contains(&p.mode) {
            vec![]
        } else {
            drawer_core::book_io::selection(&p.range, work.pages.len())?
        };
        let mut snapshots = vec![];
        let mut stamps = vec![];
        let mut held = 0usize;
        for n in &selected {
            if let Some(d) = self.structure_source(&work.pages[*n]) {
                held = held.saturating_add(d.memory_bytes());
                if held > self.structure_budget() / 2 {
                    return Err("Page snapshot memory limit; choose fewer pages".into());
                }
                let mut copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
                copy.title = d.title.clone();
                snapshots.push((*n, copy));
                stamps.push(Stamp::of(d));
            }
        }
        let mode = p.mode;
        let fit = if p.fit {
            Size::Fit {
                width: p.width,
                height: p.height,
                dpi: p.dpi,
            }
        } else {
            Size::Original
        };
        let files = p.files.clone();
        let physical = p.physical;
        let split = if p.split_auto { None } else { Some(p.split_x) };
        let dimensions = (p.width, p.height, p.dpi, p.blank_count);
        let index = p.position.saturating_sub(1);
        let remove = p.remove_originals;
        let budget = self.structure_budget().saturating_sub(held);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        self.page_structure.prepared = None;
        self.page_structure.image = None;
        self.page_structure.confirm = false;
        self.page_structure.skip = false;
        self.page_structure.notice =
            "Preparing independent pages; originals stay unchanged…".into();
        let worker = std::thread::spawn(move || {
            let result = (|| {
                let mut source_stamps = vec![];
                let mut loaded = vec![];
                let mut used = 0usize;
                if ![0, 4, 5].contains(&mode) {
                    for n in &selected {
                        drawer_core::book_io::check_cancel(&flag)?;
                        let d = if let Some(i) = snapshots.iter().position(|(i, _)| i == n) {
                            snapshots.remove(i).1
                        } else {
                            let path = &work.pages[*n];
                            let mut input = page_batch::prepare(
                                std::slice::from_ref(path),
                                Size::Original,
                                budget.saturating_sub(used),
                                &flag,
                                &mut pdf,
                            )?;
                            if let Some((_, error)) = input.rejected.first() {
                                return Err(error.clone());
                            }
                            source_stamps.append(&mut input.sources);
                            input.pages.pop().ok_or("Source page was not prepared")?
                        };
                        used = used.saturating_add(d.memory_bytes());
                        if used > budget / 2 {
                            return Err("Selected source pages exceed memory limit".into());
                        }
                        loaded.push((work.page_ids[*n], d));
                    }
                }
                let mut batch = if [0, 4].contains(&mode) {
                    page_batch::prepare(&files, fit, budget, &flag, &mut pdf)?
                } else {
                    let pages = match mode {
                        1 => {
                            let mut order =
                                loaded.iter().map(|(id, d)| (*id, d)).collect::<Vec<_>>();
                            if work.right_binding {
                                order.reverse();
                            }
                            vec![drawer_core::page_parts::merge(
                                &order,
                                physical,
                                budget.saturating_sub(used),
                            )?]
                        }
                        2 => {
                            if loaded.len() != 1 {
                                return Err("Choose one project page to split".into());
                            }
                            let d = &loaded[0].1;
                            let mut pages = drawer_core::page_parts::split(
                                d,
                                split.unwrap_or(d.width / 2),
                                budget.saturating_sub(used),
                            )?
                            .into_iter()
                            .collect::<Vec<_>>();
                            if work.right_binding {
                                pages.reverse();
                            }
                            pages
                        }
                        3 => loaded.into_iter().map(|(_, d)| d).collect(),
                        5 => {
                            let (w, h, dpi, count) = dimensions;
                            if !(1..=32).contains(&count) {
                                return Err("Choose 1..32 blank pages".into());
                            }
                            let mut pages = vec![];
                            let mut used = 0usize;
                            for _ in 0..count {
                                drawer_core::book_io::check_cancel(&flag)?;
                                let d = Document::create(
                                    CanvasSpec {
                                        width: w,
                                        height: h,
                                        dpi,
                                        background: Background::Transparent,
                                    },
                                    budget.saturating_sub(used),
                                )
                                .map_err(|e| e.to_string())?;
                                used = used.saturating_add(d.memory_bytes());
                                pages.push(d);
                            }
                            pages
                        }
                        _ => return Err("Unknown page operation".into()),
                    };
                    page_batch::Prepared {
                        pages,
                        sources: source_stamps,
                        rejected: vec![],
                    }
                };
                drawer_core::book_io::check_cancel(&flag)?;
                if batch
                    .pages
                    .iter()
                    .map(Document::memory_bytes)
                    .sum::<usize>()
                    > budget
                {
                    return Err("Prepared result pages exceed memory limit".into());
                }
                let replace = if mode == 4 {
                    let id = *work
                        .page_ids
                        .get(index)
                        .ok_or("Replacement page number is outside the project")?;
                    if batch.pages.len() != 1 {
                        return Err("Choose one valid replacement page".into());
                    }
                    Some(id)
                } else {
                    None
                };
                let remove = if remove && [1, 2, 3].contains(&mode) {
                    selected.iter().map(|n| work.page_ids[*n]).collect()
                } else {
                    vec![]
                };
                let index = if remove.is_empty() {
                    index
                } else {
                    index.saturating_sub(selected.iter().filter(|n| **n < index).count())
                };
                let total = if replace.is_some() {
                    work.pages.len()
                } else {
                    work.pages
                        .len()
                        .saturating_sub(remove.len())
                        .saturating_add(batch.pages.len())
                };
                if total > 512 {
                    return Err("Project page limit512".into());
                }
                for (n, d) in batch.pages.iter_mut().enumerate() {
                    let p = &mut d.scene.comics.page;
                    p.right_binding = work.right_binding;
                    p.first_right = work.first_right;
                    p.total = total as u32;
                    p.number = (index + n + 1).min(total).max(1) as u32;
                }
                batch.fresh()?;
                Ok(Prepared {
                    work,
                    stamps,
                    batch,
                    remove,
                    replace,
                    index,
                })
            })();
            let _ = tx.send(result.map(|ready| JobValue::Prepared(Box::new(ready))));
            ctx.request_repaint();
        });
        self.page_structure.job = Some(Job {
            cancel,
            receiver,
            worker: Some(worker),
        });
        Ok(())
    }
    fn check_structure(&self, ready: &Prepared) -> Result<(), String> {
        if self.active_page_work().as_ref() != Some(&ready.work)
            || ready.stamps.iter().any(|s| {
                self.documents
                    .iter()
                    .find(|d| d.id == s.document)
                    .is_none_or(|d| !s.matches(d))
            })
        {
            return Err("Project order or a source drawing changed; prepare again".into());
        }
        let remaining = ready.work.pages.len().saturating_sub(ready.remove.len());
        if ready.replace.is_none()
            && (ready.index > remaining || remaining.saturating_add(ready.batch.pages.len()) > 512)
        {
            return Err("Project insertion position or512-page limit; review the batch".into());
        }
        ready.batch.fresh()
    }
    fn start_structure_publication(
        &mut self,
        ctx: &egui::Context,
        directory: &Path,
    ) -> Result<(), String> {
        let p = &self.page_structure;
        let ready = p
            .prepared
            .as_ref()
            .ok_or("Prepare and review pages first")?;
        self.check_structure(ready)?;
        if ready.batch.pages.is_empty() || (!ready.batch.rejected.is_empty() && !p.skip) {
            return Err(
                "Confirm exclusion of the listed rejected files, or correct them and prepare again"
                    .into(),
            );
        }
        if (!ready.remove.is_empty() || ready.replace.is_some()) && !p.confirm {
            return Err("Confirm the reviewed original-page membership change".into());
        }
        let ready = Box::new(self.page_structure.prepared.take().unwrap());
        let directory = directory.to_owned();
        let ctx = ctx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result =
                drawer_core::page_resources::prepare(&ready.batch.pages, &directory, &flag)
                    .map(|resources| JobValue::Resources(ready, resources));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.page_structure.job = Some(Job {
            cancel,
            receiver,
            worker: Some(worker),
        });
        self.page_structure.notice =
            "Writing new page resources; cancellation removes only this operation's files…".into();
        Ok(())
    }
    fn commit_structure(
        &mut self,
        ready: Prepared,
        resources: drawer_core::page_resources::Prepared,
    ) -> Result<(), String> {
        self.check_structure(&ready)?;
        let edit = if let Some(id) = ready.replace {
            Edit::Replace {
                id,
                path: resources.paths[0].clone(),
            }
        } else {
            Edit::Batch {
                paths: resources.paths.clone(),
                remove: ready.remove,
                index: ready.index,
            }
        };
        let count = resources.paths.len();
        self.edit_project_structure(edit)?;
        resources.keep();
        self.page_structure.prepared = None;
        self.page_structure.image = None;
        self.page_structure.notice = format!(
            "Added {count} owned pages to the local project. One project Undo restores the previous list; source files and open originals remain. Save the project snapshot in Page manager."
        );
        Ok(())
    }
    fn prepare_frame_structure(&mut self, kind: u8, ctx: &egui::Context) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a drawing page")?];
        let p = &self.page_structure;
        let state = match kind {
            1 => drawer_core::frame_ops::merge(d, &p.frames)?,
            2 => {
                if p.frames.len() != 1 {
                    return Err("Choose one frame to divide".into());
                }
                drawer_core::frame_ops::divide(d, p.frames[0], p.divisions, p.vertical)?
            }
            3 | 4 => drawer_core::frame_ops::hide_overlaps(d, &p.frames, kind == 3)?,
            _ => return Err("Unknown frame operation".into()),
        };
        let preview = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        let texture = image(ctx, &preview, "frame-structure-preview")?;
        self.page_structure.frame_context = Some(d.id);
        self.page_structure.frame_prepared = Some(FramePrepared {
            stamp: Stamp::of(d),
            state,
        });
        self.page_structure.frame_image = Some(texture);
        Ok(())
    }
    fn apply_frame_structure(&mut self) -> Result<(), String> {
        let prepared = self
            .page_structure
            .frame_prepared
            .as_ref()
            .ok_or("Prepare frame preview first")?;
        let d = &mut self.documents[self.active.ok_or("Open the source drawing")?];
        if !prepared.stamp.matches(d) {
            return Err("Drawing changed; prepare frame preview again".into());
        }
        d.commit_state(prepared.state.clone())?;
        self.page_structure.frames.retain(|id|d.scene.catalog.objects.index(*id,d.scene.objects.len()).is_some_and(|n|matches!(&d.scene.objects[n],PageObject::Comic(o)if matches!(o.kind,Kind::Frame(_)))));
        self.page_structure.frame_prepared = None;
        self.page_structure.frame_image = None;
        self.page_structure.notice = "Frame edit applied with one drawing Undo".into();
        Ok(())
    }
    fn edit_contained_page(&mut self, id: u64) -> Result<(), String> {
        if self.tabs.len() >= 256 || self.page_structure.bindings.len() >= 4 {
            return Err("Close some views or finish the current contained-page edit".into());
        }
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a combined drawing")?];
        let source = d
            .scene
            .page_parts
            .get(id)
            .ok_or("Choose a contained page")?;
        let child = source.open()?;
        let binding = Binding {
            parent: Stamp::of(d),
            object: id,
            child: child.id,
        };
        self.add_document(child);
        self.page_structure.bindings.push(binding);
        Ok(())
    }
    fn apply_contained_page(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed() || self.text_editor.draft.is_some() {
            return Err("Apply or cancel the pending child edit first".into());
        }
        let b = self
            .page_structure
            .bindings
            .last()
            .ok_or("Open a contained page first")?;
        let parent = self
            .documents
            .iter()
            .position(|d| d.id == b.parent.document)
            .ok_or("Parent drawing was closed")?;
        let child = self
            .documents
            .iter()
            .find(|d| d.id == b.child)
            .ok_or("Child drawing was closed")?;
        if !b.parent.matches(&self.documents[parent]) {
            return Err(
                "Parent drawing changed; reopen its contained source to review again".into(),
            );
        }
        let state = drawer_core::page_parts::replace(
            &self.documents[parent],
            b.object,
            child,
            DEFAULT_DOCUMENT_BUDGET,
        )?;
        let id = b.parent.document;
        self.documents[parent].commit_state(state)?;
        self.page_structure.bindings.pop();
        if let Some(tab) = self.tabs.iter().position(|t| t.document_id == id) {
            self.select_tab(tab);
        }
        self.page_structure.notice="Contained page applied to parent with one drawing Undo. The separate edited view remains open.".into();
        Ok(())
    }
    pub(super) fn page_structure_window(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.page_structure.job {
            let ready = match j.receiver.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Page preparation stopped".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = ready {
                let cancelled = j.cancel.load(Ordering::Relaxed);
                self.page_structure.job = None;
                match result {
                    Ok(JobValue::Prepared(p)) if !cancelled => {
                        self.page_structure.notice = format!(
                            "{} pages prepared; {} rejected inputs. Review before adding.",
                            p.batch.pages.len(),
                            p.batch.rejected.len()
                        );
                        self.page_structure.prepared = Some(*p);
                    }
                    Ok(JobValue::Resources(ready, resources)) if !cancelled => {
                        if let Err(e) = self.commit_structure(*ready, resources) {
                            self.page_structure.notice = e;
                        }
                    }
                    Ok(_) => {
                        self.page_structure.notice =
                            "Page operation cancelled; project unchanged".into()
                    }
                    Err(e) => self.page_structure.notice = e,
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        if !self.page_structure.open {
            return;
        }
        let work = self.active_page_work();
        let facing = self.project_structure_facing_pair();
        let active = self.active.map(|n| &self.documents[n]);
        let mut p = std::mem::take(&mut *self.page_structure);
        let mut open = true;
        let mut action = 0;
        let mut edit_source = None;
        let mut move_file = None;
        if p.frame_context != active.map(|d| d.id) {
            p.frame_context = active.map(|d| d.id);
            p.frames.clear();
            p.frame_prepared = None;
            p.frame_image = None;
        }
        let old = (
            p.mode,
            p.range.clone(),
            p.position,
            p.files.clone(),
            p.fit,
            [p.width, p.height, p.split_x, p.blank_count],
            p.dpi,
            [p.physical, p.split_auto, p.remove_originals],
        );
        let old_frames = (p.frames.clone(), p.divisions, p.vertical);
        let working = p.job.is_some();
        egui::Window::new("Page and frame structure").id(egui::Id::new("page-structure")).open(&mut open).default_width(670.).show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{ui.selectable_value(&mut p.section,0,"Project pages");ui.selectable_value(&mut p.section,1,"Frame folders");ui.selectable_value(&mut p.section,2,"Contained pages");});
            egui::ScrollArea::vertical().id_salt("page-structure-scroll").max_height((ctx.content_rect().height()-180.).max(200.)).show(ui,|ui|{
                ui.add_enabled_ui(!working,|ui|{
                    if p.section==0 {ui.vertical(|ui|{
                        if let Some(w)=&work{ui.label(format!("{} · {} pages · {} binding",w.title,w.pages.len(),if w.right_binding{"right"}else{"left"}));}else{ui.label("Open/create a local project in Page manager first.");}
                        if ui.button("Open Page manager").clicked(){action=20;}
                        egui::ComboBox::from_label("Page operation").selected_text(["Import local files","Combine selected pages","Split one page","Duplicate selected pages","Replace a page","Add blank pages"][p.mode as usize]).show_ui(ui,|ui|{for(i,label)in ["Import local files","Combine selected pages","Split one page","Duplicate selected pages","Replace a page","Add blank pages"].iter().enumerate(){ui.selectable_value(&mut p.mode,i as u8,*label);}});
                        if [0,4].contains(&p.mode){if ui.button("Choose local input files…").clicked(){action=1;}
                            if ui.button("Natural filename order").clicked(){page_batch::natural_sort(&mut p.files);}
                            for(i,file)in p.files.iter().enumerate(){ui.horizontal(|ui|{ui.label(format!("{} · {}",i+1,file.display()));if ui.add_enabled(i>0,egui::Button::new("Up")).clicked(){move_file=Some((i,0));}
                            if ui.add_enabled(i+1<p.files.len(),egui::Button::new("Down")).clicked(){move_file=Some((i,1));}
                            if ui.button("Remove").clicked(){move_file=Some((i,2));}});}
                            ui.checkbox(&mut p.fit,"Fit imported pages to the specified size and DPI");
                        }else if p.mode!=5{ui.horizontal(|ui|{ui.label("Project page range (1,3-5)");ui.text_edit_singleline(&mut p.range);});}
                        if p.fit || p.mode==5{ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut p.width).range(1..=32768).prefix("Width "));ui.add(egui::DragValue::new(&mut p.height).range(1..=32768).prefix("Height "));ui.add(egui::DragValue::new(&mut p.dpi).range(1. ..=4800.).prefix("DPI "));});}
                        if p.mode==5{ui.add(egui::DragValue::new(&mut p.blank_count).range(1..=32).prefix("Blank pages "));}
                        if p.mode==1{ui.checkbox(&mut p.physical,"Match physical DPI when placing pages side by side");
                        if ui.button("Use current facing pair").clicked(){if let Some((a,b))=facing{p.range=format!("{},{}",a+1,b+1);}else{p.notice="The selected project page has no facing partner".into();}}
                        ui.label("Pages follow the project's left-to-right or right-to-left reading direction; source pages keep their own cameras and timelines.");}
                        if p.mode==2{ui.checkbox(&mut p.split_auto,"Split at half the page width");ui.add_enabled(!p.split_auto,egui::DragValue::new(&mut p.split_x).range(1..=32767).prefix("Split X "));}
                        ui.add(egui::DragValue::new(&mut p.position).range(1..=513).prefix(if p.mode==4{"Replace project page "}else{"Insert at project position "}));
                        if [1,2,3].contains(&p.mode){ui.checkbox(&mut p.remove_originals,"Also remove selected originals from the project list (keep source files and open drawings)");}
                        if ui.add_enabled(work.is_some(),egui::Button::new("Prepare project pages")).clicked(){action=2;}
                        if let Some(ready)=&p.prepared{
                            for(path,error)in &ready.batch.rejected{ui.colored_label(egui::Color32::YELLOW,format!("{} · {error}",path.display()));}
                            if !ready.batch.rejected.is_empty(){ui.checkbox(&mut p.skip,"Exclude the listed rejected inputs");}
                            if !ready.remove.is_empty() || ready.replace.is_some(){ui.checkbox(&mut p.confirm,"Confirm removal/replacement of the reviewed original project entries; keep their source files");}
                            if p.image.is_none() && let Some(d)=ready.batch.pages.first(){match image(ctx,d,"project-structure-preview"){Ok(t)=>p.image=Some(t),Err(e)=>p.notice=e,}}
                            if let Some(t)=&p.image{ui.image((t.id(),t.size_vec2()));}
                            for(i,d)in ready.batch.pages.iter().enumerate().take(24){ui.label(format!("Result {} · {}×{} · {:.3}DPI · {}",i+1,d.width,d.height,d.dpi,d.title));}
                            let enabled=!ready.batch.pages.is_empty() && (ready.batch.rejected.is_empty() || p.skip) && ((ready.remove.is_empty() && ready.replace.is_none()) || p.confirm);
                            ui.horizontal(|ui|{ui.label("Local resource folder");ui.text_edit_singleline(&mut p.directory);if ui.button("Browse folder…").clicked(){action=7;}});
                            let r=ui.add_enabled(enabled,egui::Button::new("Add reviewed pages to project"));p.apply=Some(r.rect);if r.clicked(){action=3;}
                            if ui.button("Discard prepared pages").clicked(){action=4;}
                        }
                        ui.horizontal(|ui|{if ui.button("Undo project page change").clicked(){action=5;}
                        if ui.button("Redo project page change").clicked(){action=6;}});
                    });
                    }
                    if p.section==1 {ui.vertical(|ui|{
                        if let Some(d)=active{for(i,o)in d.scene.objects.iter().enumerate(){if let PageObject::Comic(o)=o && matches!(o.kind,Kind::Frame(_)){let id=d.scene.catalog.objects.id(i);let mut selected=p.frames.contains(&id);if ui.checkbox(&mut selected,format!("{} · #{id}",o.name)).changed(){if selected{p.frames.push(id);}else{p.frames.retain(|n|*n!=id);}}}}}
                        ui.label("Combining keeps all selected contours and source references; the first frame supplies gap and border style.");
                        if ui.button("Preview combined frames").clicked(){action=11;}
                        ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut p.divisions).range(2..=32).prefix("Equal parts "));ui.checkbox(&mut p.vertical,"Vertical dividing lines");if ui.button("Preview equal division").clicked(){action=12;}});
                        ui.horizontal_wrapped(|ui|{if ui.button("Preview hidden coincident borders").clicked(){action=13;}
                        if ui.button("Preview restored borders").clicked(){action=14;}});
                        if let Some(t)=&p.frame_image{ui.image((t.id(),t.size_vec2()));}
                        if p.frame_prepared.is_some(){let r=ui.button("Apply frame structure");p.frame_apply=Some(r.rect);if r.clicked(){action=15;}
                        if ui.button("Discard frame preview").clicked(){p.frame_prepared=None;p.frame_image=None;}}
                    });
                    }
                    if p.section==2 {ui.vertical(|ui|{
                        if let Some(d)=active{for source in &d.scene.page_parts.entries{if ui.button(format!("Open {} for independent editing",source.name)).clicked(){edit_source=Some(source.object);}}}
                        if let Some(binding)=p.bindings.last(){ui.label(format!("Contained edit view #{} → parent view #{}",binding.child,binding.parent.document));if ui.button("Apply edited contained page to parent").clicked(){action=16;}
                        if ui.button("Return to parent without applying").clicked(){action=17;}}
                    });
                    }
                });
            });
            if working{ui.spinner();if ui.button("Cancel page operation").clicked(){p.job.as_ref().unwrap().cancel.store(true,Ordering::Relaxed);}}
            if !p.notice.is_empty(){ui.label(&p.notice);}
            if ui.button("Close").clicked(){action=99;}
        });
        if let Some((n, kind)) = move_file {
            match kind {
                0 => p.files.swap(n, n - 1),
                1 => p.files.swap(n, n + 1),
                _ => {
                    p.files.remove(n);
                }
            }
        }
        if old
            != (
                p.mode,
                p.range.clone(),
                p.position,
                p.files.clone(),
                p.fit,
                [p.width, p.height, p.split_x, p.blank_count],
                p.dpi,
                [p.physical, p.split_auto, p.remove_originals],
            )
        {
            p.prepared = None;
            p.image = None;
            p.confirm = false;
            p.skip = false;
        }
        if old_frames != (p.frames.clone(), p.divisions, p.vertical) {
            p.frame_prepared = None;
            p.frame_image = None;
        }
        if action == 99 {
            open = false;
        }
        p.open = open;
        if !open {
            p.prepared = None;
            p.image = None;
            p.frame_prepared = None;
            p.frame_image = None;
            if let Some(j) = &p.job {
                j.cancel.store(true, Ordering::Relaxed);
            }
        }
        *self.page_structure = p;
        let result = if let Some(id) = edit_source {
            self.edit_contained_page(id)
        } else {
            match action {
                1 => {
                    if let Some(files) = rfd::FileDialog::new()
                        .add_filter(
                            "Local page files",
                            &[
                                "drawer", "png", "jpg", "jpeg", "bmp", "tif", "tiff", "pdf", "epub",
                            ],
                        )
                        .pick_files()
                    {
                        self.page_structure.files = files;
                        self.page_structure.prepared = None;
                        self.page_structure.image = None;
                    }
                    Ok(())
                }
                2 => self.prepare_structure(ctx),
                3 => {
                    let directory = PathBuf::from(&self.page_structure.directory);
                    self.start_structure_publication(ctx, &directory)
                }
                7 => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Choose a local folder for new project pages")
                        .pick_folder()
                    {
                        self.page_structure.directory = path.to_string_lossy().into_owned();
                    }
                    Ok(())
                }
                4 => {
                    self.page_structure.prepared = None;
                    self.page_structure.image = None;
                    Ok(())
                }
                5 | 6 => self.project_structure_history(action == 6),
                11..=14 => self.prepare_frame_structure(action - 10, ctx),
                15 => self.apply_frame_structure(),
                16 => self.apply_contained_page(),
                17 => {
                    if let Some(b) = self.page_structure.bindings.pop()
                        && let Some(tab) = self
                            .tabs
                            .iter()
                            .position(|t| t.document_id == b.parent.document)
                    {
                        self.select_tab(tab);
                    }
                    Ok(())
                }
                20 => self.begin_page_work(),
                _ => Ok(()),
            }
        };
        if let Err(e) = result {
            self.page_structure.notice = e;
        }
    }
}

pub(super) fn smoke(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    drawer_core::page_parts_verify::verify(root)?;
    drawer_core::page_structure_verify::verify(root)?;
    drawer_core::page_resources::verify(root)?;
    let dir = root.join("page-structure-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut paths = vec![];
        let mut source_bytes = vec![];
        for n in 0..2 {
            let mut d = drawer_core::page_parts_verify::fixture(n);
            let path = dir.join(format!("source-{n}.drawer"));
            drawer_core::files::save(&mut d, &path).map_err(|e| e.to_string())?;
            source_bytes.push(std::fs::read(&path).unwrap());
            paths.push(path.canonicalize().unwrap());
        }
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.open_path(&paths[0])?;
        app.documents[0].current_frame = 1;
        app.documents[0].edit_pixel(0, 0, 0, [220, 30, 80, 255]);
        let original = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let mut work = Work::new("Local structure".into(), paths.clone())?;
        work.right_binding = true;
        work.first_right = true;
        app.set_project_structure_work(work)?;
        app.execute_command(&ctx, "page-structure");
        app.page_structure.mode = 1;
        app.page_structure.range = "1-2".into();
        app.page_structure.position = 1;
        app.page_structure.directory = dir.to_string_lossy().into_owned();
        let mut time = 0.;
        let frame = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            input_replay::frame(&ctx, app, *time, events);
        };
        let settle = |app: &mut Drawer, time: &mut f64| {
            for _ in 0..3 {
                frame(app, time, vec![]);
            }
        };
        let wait = |app: &mut Drawer, time: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while app.page_structure.job.is_some() {
                frame(app, time, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Page structure worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(())
        };
        let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
            for pressed in [true, false] {
                frame(
                    app,
                    time,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        };
        let base = app.active_page_work().unwrap();
        app.prepare_structure(&ctx)?;
        wait(&mut app, &mut time)?;
        if app
            .page_structure
            .prepared
            .as_ref()
            .is_none_or(|p| p.batch.pages.len() != 1)
            || app.documents[0].snapshot() != original
            || app.active_page_work().unwrap() != base
        {
            return Err(format!(
                "Page structure preview changed sources or failed: {}",
                app.page_structure.notice
            ));
        }
        settle(&mut app, &mut time);
        let button = app
            .page_structure
            .apply
            .ok_or("Project page apply button missing")?;
        click(&mut app, &mut time, button);
        wait(&mut app, &mut time)?;
        let added = app.active_page_work().unwrap();
        if added.pages.len() != 3
            || !added.right_binding
            || !added.first_right
            || app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != history
        {
            return Err(format!(
                "Pointer page confirmation, binding or source Undo: {}",
                app.page_structure.notice
            ));
        }
        let combined_path = added.pages[0].clone();
        let combined = drawer_core::files::open(&combined_path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?;
        if combined.scene.page_parts.entries.len() != 2
            || combined
                .scene
                .page_parts
                .entries
                .iter()
                .find(|p| p.original_page == base.page_ids[0])
                .unwrap()
                .open()?
                .snapshot()
                != original
            || combined.scene.page_parts.entries[0].original_page != base.page_ids[1]
            || !combined.scene.comics.page.right_binding
        {
            return Err(
                "Prepared live source or contained page identity lost on native publication".into(),
            );
        }
        app.project_structure_history(false)?;
        if app.active_page_work().unwrap().pages != base.pages || !combined_path.is_file() {
            return Err("Project Undo did not restore order or deleted redo resources".into());
        }
        app.project_structure_history(true)?;
        app.page_structure.range = "2-3".into();
        app.prepare_structure(&ctx)?;
        wait(&mut app, &mut time)?;
        app.documents[0].edit_pixel(0, 0, 1, [255; 4]);
        if app.start_structure_publication(&ctx, &dir).is_ok() {
            return Err("Stale live source batch was applied".into());
        }
        app.documents[0].undo();
        app.prepare_structure(&ctx)?;
        app.page_structure
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        wait(&mut app, &mut time)?;
        if app.page_structure.prepared.is_some()
            || app.active_page_work().unwrap().pages != added.pages
        {
            return Err("Cancelled project preparation changed work".into());
        }
        let bad = dir.join("broken.png");
        std::fs::write(&bad, b"bad image").unwrap();
        app.page_structure.mode = 0;
        app.page_structure.files = vec![bad, paths[1].clone()];
        app.prepare_structure(&ctx)?;
        wait(&mut app, &mut time)?;
        if app.start_structure_publication(&ctx, &dir).is_ok() {
            return Err("Rejected batch files were skipped without review".into());
        }
        app.page_structure.skip = true;
        app.start_structure_publication(&ctx, &dir)?;
        wait(&mut app, &mut time)?;
        if app.active_page_work().unwrap().pages.len() != 4 {
            return Err(format!(
                "Reviewed partial batch failed: {}",
                app.page_structure.notice
            ));
        }
        let work_before_cancel = app.active_page_work().unwrap();
        let dirs_before = std::fs::read_dir(&dir).unwrap().count();
        app.page_structure.files = vec![paths[1].clone()];
        app.prepare_structure(&ctx)?;
        wait(&mut app, &mut time)?;
        app.start_structure_publication(&ctx, &dir)?;
        app.page_structure
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        wait(&mut app, &mut time)?;
        if app.active_page_work().unwrap() != work_before_cancel
            || std::fs::read_dir(&dir).unwrap().count() != dirs_before
        {
            return Err("Publication cancellation changed work or retained new resources".into());
        }
        app.add_document(combined);
        let parent = app.active.unwrap();
        let before = app.documents[parent].snapshot();
        let part = app.documents[parent].scene.page_parts.entries[0].object;
        app.edit_contained_page(part)?;
        let child = app.active.unwrap();
        app.documents[child].edit_pixel(0, 0, 0, [200, 0, 0, 255]);
        app.apply_contained_page()?;
        if app.documents[parent].snapshot() == before
            || app.documents[parent].history_counts().0 != 1
            || app.documents[parent].scene.page_parts.entries[1]
                .open()?
                .snapshot()
                != before.scene.page_parts.entries[1].open()?.snapshot()
        {
            return Err("Contained source apply/independence/one Undo".into());
        }
        app.documents[parent].undo();
        if app.documents[parent].snapshot() != before {
            return Err("Contained source parent Undo".into());
        }
        app.edit_contained_page(part)?;
        app.documents[parent].edit_pixel(0, 0, 0, [100; 4]);
        if app.apply_contained_page().is_ok() {
            return Err("Changed parent accepted a stale contained edit".into());
        }
        app.documents[parent].undo();
        app.page_structure.bindings.clear();
        if let Some(tab) = app
            .tabs
            .iter()
            .position(|t| t.document_id == app.documents[parent].id)
        {
            app.select_tab(tab);
        }
        let mut f = drawer_core::comics::defaults(drawer_core::scene::Rect {
            min: [0.; 2],
            max: [64., 64.],
        })[1]
            .clone();
        let Kind::Frame(frame) = &mut f.kind else {
            unreachable!()
        };
        frame.children = app.documents[parent]
            .scene
            .page_parts
            .entries
            .iter()
            .map(|p| drawer_core::layer_search::Key::Object(p.object))
            .collect();
        let state = drawer_core::comics::prepare(&app.documents[parent], None, f)?;
        app.documents[parent].commit_state(state)?;
        let id = app.documents[parent]
            .scene
            .catalog
            .objects
            .id(app.documents[parent].scene.objects.len() - 1);
        let frame_before = app.documents[parent].snapshot();
        app.page_structure.section = 1;
        app.page_structure.frames = vec![id];
        app.prepare_frame_structure(2, &ctx)?;
        settle(&mut app, &mut time);
        let button = app
            .page_structure
            .frame_apply
            .ok_or("Frame structure apply button missing")?;
        click(&mut app, &mut time, button);
        if app.page_structure.frame_prepared.is_some()
            || app.documents[parent].scene.objects.len() != frame_before.scene.objects.len() + 1
        {
            return Err(format!(
                "Frame pointer apply failed: {}",
                app.page_structure.notice
            ));
        }
        app.documents[parent].undo();
        if app.documents[parent].snapshot() != frame_before {
            return Err("Frame structure drawing Undo lost sources".into());
        }
        let snapshot = dir.join("reopened.drawer-work");
        let model = drawer_core::page_work::Model::new(app.active_page_work().unwrap())?;
        let original_id = model
            .work
            .page_ids
            .iter()
            .zip(&model.work.pages)
            .find(|(_, path)| *path == &paths[0])
            .map(|(id, _)| *id)
            .ok_or("Original page no longer in work")?;
        drawer_core::page_work::snapshot(
            &model,
            &snapshot,
            &[(original_id, &app.documents[0])],
            &mut |_, _| Ok(()),
        )?;
        let reopened = drawer_core::page_work::Model::load(&snapshot)?;
        if reopened.work.pages.len() != 4
            || !reopened.work.right_binding
            || !reopened.work.first_right
        {
            return Err("Page structure project snapshot/reopen binding/order".into());
        }
        for path in &reopened.work.pages {
            let d = drawer_core::files::open(path, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?;
            if d.scene.page_parts.entries.len() == 2
                && d.scene.page_parts.entries[0].document().is_none()
            {
                return Err("Project snapshot omitted contained native payloads".into());
            }
        }
        let parent_id = app.documents[parent].id;
        app.edit_contained_page(part)?;
        let tab = app
            .tabs
            .iter()
            .position(|t| t.document_id == parent_id)
            .unwrap();
        app.request_close(tab);
        app.accept_close();
        if app.documents.iter().any(|d| d.id == parent_id) || app.apply_contained_page().is_ok() {
            return Err("Closed parent accepted a contained update".into());
        }
        for (i, path) in paths.iter().enumerate() {
            if std::fs::read(path).unwrap() != source_bytes[i] {
                return Err("Page structure operation changed original source file".into());
            }
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_project_batch_cancel_stale_contained_edits_frames_and_owned_resource_reopen() {
        let root =
            std::env::temp_dir().join(format!("drawer-structure-app-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let result = super::smoke(&root);
        std::fs::remove_dir(&root).unwrap();
        result.unwrap();
    }
}
