use super::*;
use drawer_core::{
    book_io,
    book_production::{self, Setup},
    page_batch, scan_batch,
    story_text::Work,
    storyboard,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
#[derive(Clone)]
struct Stamp {
    id: u64,
    revision: u64,
    frame: u32,
}

pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::book_production_verify::verify(root)?;
    let dir = root.join("book-workflow-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = Document::create(
            CanvasSpec {
                width: 20,
                height: 30,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())?;
        let source = dir.join("source.drawer");
        drawer_core::files::save(&mut d, &source).map_err(|e| e.to_string())?;
        let source = source.canonicalize().unwrap();
        let original = std::fs::read(&source).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.open_path(&source)?;
        app.documents[0].edit_pixel(0, 1, 1, [50, 90, 150, 255]);
        let snapshot = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let mut w = Work::new("한글 책".into(), vec![source.clone()])?;
        w.page_ids = vec![12];
        w.next_page_id = 30;
        w.right_binding = true;
        app.set_project_structure_work(w.clone())?;
        app.execute_command(&ctx, "book-production");
        app.book_workflow.setup.bleed_mm = 0.;
        app.book_workflow.setup.safe_mm = 0.;
        app.book_workflow.setup.front = Some(12);
        app.book_workflow.directory = dir.to_string_lossy().into_owned();
        let mut time = 0.;
        let frame = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            input_replay::frame(&ctx, app, *time, events);
        };
        let wait = |app: &mut Drawer, time: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while app.book_workflow.job.is_some() {
                frame(app, time, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Book workflow worker timeout".into());
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
        app.prepare_book(&ctx)?;
        wait(&mut app, &mut time)?;
        let b = app
            .book_workflow
            .book
            .as_ref()
            .ok_or_else(|| format!("Book preview unavailable: {}", app.book_workflow.notice))?;
        if b.pages[0].snapshot() != snapshot || !app.book_workflow.findings.is_empty() {
            return Err("Book preview lost unsaved source / preflight false failure".into());
        }
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        if app.book_workflow.textures.len() != 1 {
            return Err("3D preview did not bind the local page texture".into());
        }
        app.documents[0].edit_pixel(0, 2, 2, [90, 50, 100, 255]);
        if app
            .check_book(app.book_workflow.book.as_ref().unwrap())
            .is_ok()
        {
            return Err("Stale live book snapshot accepted".into());
        }
        app.documents[0].undo();
        app.prepare_book(&ctx)?;
        wait(&mut app, &mut time)?;
        let b = app.book_workflow.book.as_ref().unwrap();
        let output = dir.join("story.drawer-storyboard");
        storyboard::export(
            &output,
            &b.work,
            &b.pages,
            &app.book_workflow.setup,
            DEFAULT_DOCUMENT_BUDGET,
            &AtomicBool::new(false),
            &mut |a, b| {
                drawer_platform::local_files::publish_no_replace(a, b).map_err(|e| e.to_string())
            },
        )?;
        app.book_workflow.section = 1;
        app.book_workflow.path = output.to_string_lossy().into_owned();
        app.production_action(&ctx, 6)?;
        wait(&mut app, &mut time)?;
        if app.book_workflow.imported.is_none() {
            return Err(format!(
                "Storyboard preparation: {}",
                app.book_workflow.notice
            ));
        }
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .book_workflow
            .story_apply
            .ok_or("Storyboard confirmation button missing")?;
        click(&mut app, &mut time, r);
        wait(&mut app, &mut time)?;
        let imported = app
            .active_page_work()
            .ok_or("Storyboard project did not open")?;
        if imported.page_ids != vec![12]
            || !imported.right_binding
            || imported.pages[0] == source
            || app.documents.len() != 1
            || app.documents[0].snapshot() != snapshot
        {
            return Err(format!(
                "Storyboard pointer opening changed sources or IDs: {}",
                app.book_workflow.notice
            ));
        }
        app.set_project_structure_work(w.clone())?;
        let mut scan = scan_batch::Session::new(3, 300., DEFAULT_DOCUMENT_BUDGET)?;
        scan.accept(drawer_core::book_production_verify::scan_image(10))?;
        scan.fail("MOCK scanner: second acquisition failed".into());
        scan.skip()?;
        scan.accept(drawer_core::book_production_verify::scan_image(30))?;
        app.book_workflow.section = 2;
        app.book_workflow.scan = Some(scan);
        app.book_workflow.scan_work = Some(w.clone());
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .book_workflow
            .scan_apply
            .ok_or("Scan batch confirmation button missing")?;
        click(&mut app, &mut time, r);
        wait(&mut app, &mut time)?;
        let added = app.active_page_work().unwrap();
        if added.pages.len() != 3 {
            return Err(format!(
                "Mock scan pointer confirmation failed: {}",
                app.book_workflow.notice
            ));
        }
        for path in &added.pages[1..] {
            let page = drawer_core::files::open(path, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?;
            if page.width != 3 || page.height != 4 || page.dpi != 300. {
                return Err("Scanned native page reopen failed".into());
            }
        }
        app.project_structure_history(false)?;
        if app.active_page_work().unwrap().pages != w.pages
            || added.pages.iter().any(|p| !p.exists())
        {
            return Err("Scan Undo deleted resources or did not restore original project".into());
        }
        app.project_structure_history(true)?;
        let mut cancel = scan_batch::Session::new(3, 300., DEFAULT_DOCUMENT_BUDGET)?;
        cancel.accept(drawer_core::book_production_verify::scan_image(10))?;
        app.book_workflow.scan = Some(cancel);
        app.production_action(&ctx, 14)?;
        if app.book_workflow.scan.is_some()
            || app.active_page_work().unwrap().pages != added.pages
            || std::fs::read(&source).unwrap() != original
            || app.documents[0].snapshot() != snapshot
            || app.documents[0].history_counts().0 != history.0
        {
            return Err("Scan abort/source document preservation failed".into());
        }
        let original_path = app.documents[0].path.clone();
        app.documents[0].path = Some(dir.join("missing-open.drawer"));
        app.set_project_structure_work(Work::new(
            "Missing source".into(),
            vec![dir.join("missing-closed.drawer")],
        )?)?;
        app.book_workflow.setup.front = None;
        app.prepare_book(&ctx)?;
        wait(&mut app, &mut time)?;
        if app.book_workflow.book.is_some() {
            return Err("Two missing paths incorrectly matched a different open document".into());
        }
        app.documents[0].path = original_path;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}

struct Book {
    work: Work,
    pages: Vec<Document>,
    stamps: Vec<Stamp>,
    sources: Vec<page_batch::Stamp>,
}
enum Source {
    Open(Box<Document>),
    File(PathBuf),
}
enum Value {
    Book(Box<Book>),
    Imported(Box<storyboard::Prepared>),
    Opened(
        Box<storyboard::Prepared>,
        drawer_core::page_resources::Prepared,
    ),
    Devices(Vec<drawer_core::acquisition::Device>),
    Scan(scan_batch::Image),
    Scanned(Work, drawer_core::page_resources::Prepared),
    Output(String),
}
struct Job {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Value, String>>,
    worker: Option<std::thread::JoinHandle<()>>,
    scan: bool,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
fn job(
    ctx: &egui::Context,
    scan: bool,
    f: impl FnOnce(&AtomicBool) -> Result<Value, String> + Send + 'static,
) -> Job {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (ctx, tx, rx) = {
        let (tx, rx) = mpsc::channel();
        (ctx.clone(), tx, rx)
    };
    let worker = std::thread::spawn(move || {
        let result = f(&flag);
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    Job {
        cancel,
        rx,
        worker: Some(worker),
        scan,
    }
}
pub(super) struct Panel {
    open: bool,
    section: u8,
    setup: Setup,
    book: Option<Book>,
    imported: Option<storyboard::Prepared>,
    job: Option<Job>,
    notice: String,
    path: String,
    directory: String,
    index: usize,
    closed: bool,
    opening: f64,
    yaw: f64,
    pitch: f64,
    textures: std::collections::BTreeMap<usize, egui::TextureHandle>,
    findings: Vec<book_production::Finding>,
    devices: Vec<drawer_core::acquisition::Device>,
    selected: usize,
    count: usize,
    dpi: f64,
    scan: Option<scan_batch::Session>,
    scan_work: Option<Work>,
    reviewed: bool,
    pub scan_apply: Option<egui::Rect>,
    pub story_apply: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            section: 0,
            setup: Setup::default(),
            book: None,
            imported: None,
            job: None,
            notice: String::new(),
            path: String::new(),
            directory: String::new(),
            index: 0,
            closed: false,
            opening: 155.,
            yaw: 15.,
            pitch: -15.,
            textures: Default::default(),
            findings: vec![],
            devices: vec![],
            selected: 0,
            count: 3,
            dpi: 300.,
            scan: None,
            scan_work: None,
            reviewed: false,
            scan_apply: None,
            story_apply: None,
        }
    }
}
fn publish(path: &Path, bytes: &[u8], cancel: &AtomicBool) -> Result<(), String> {
    book_io::publish(path, bytes, cancel, &mut |a, b| {
        drawer_platform::local_files::publish_no_replace(a, b).map_err(|e| e.to_string())
    })
}
fn load_book(
    work: Work,
    sources: Vec<Source>,
    stamps: Vec<Stamp>,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Book, String> {
    let mut book = Book {
        work,
        pages: vec![],
        stamps,
        sources: vec![],
    };
    let mut used = 0usize;
    for source in sources {
        book_io::check_cancel(cancel)?;
        let d = match source {
            Source::Open(d) => *d,
            Source::File(path) => {
                let mut batch = page_batch::prepare(
                    &[path],
                    page_batch::Size::Original,
                    budget.saturating_sub(used),
                    cancel,
                    &mut |_, _, _| Err("Native pages required".into()),
                )?;
                if !batch.rejected.is_empty() || batch.pages.len() != 1 {
                    return Err(format!("Cannot prepare book page: {:?}", batch.rejected));
                }
                book.sources.append(&mut batch.sources);
                batch.pages.remove(0)
            }
        };
        used = used.saturating_add(d.resident_bytes());
        if used > budget {
            return Err("Book preview exceeds memory budget".into());
        }
        book.pages.push(d);
    }
    Ok(book)
}
fn texture(ctx: &egui::Context, d: &Document) -> Result<egui::TextureHandle, String> {
    let r = drawer_core::render::Renderer::new(d);
    if let Some(e) = r.error() {
        return Err(e.into());
    }
    let scale = (d.width.max(d.height) as f64 / 256.).max(1.);
    let w = (d.width as f64 / scale).ceil() as usize;
    let h = (d.height as f64 / scale).ceil() as usize;
    let mut rgba = vec![];
    for y in 0..h {
        for x in 0..w {
            let p = r.pixel([
                (x as f64 + 0.5) * d.width as f64 / w as f64,
                (y as f64 + 0.5) * d.height as f64 / h as f64,
            ]);
            for v in &p[..3] {
                rgba.push(((*v as u16 * p[3] as u16 + 255 * (255 - p[3] as u16)) / 255) as u8);
            }
            rgba.push(255);
        }
    }
    Ok(ctx.load_texture(
        "local-book-page",
        egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba),
        egui::TextureOptions::LINEAR,
    ))
}
impl Drawer {
    pub(super) fn begin_book_workflow(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending edits first".into());
        }
        self.book_workflow.open = true;
        Ok(())
    }
    fn production_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        )
    }
    fn production_base(&self) -> PathBuf {
        self.storage
            .as_ref()
            .map(|s| s.root().join("acquisition-work"))
            .unwrap_or_else(|| std::env::temp_dir().join("Drawer-acquisition"))
    }
    fn check_book(&self, b: &Book) -> Result<(), String> {
        if self.active_page_work().as_ref() != Some(&b.work)
            || b.stamps.iter().any(|s| {
                self.documents
                    .iter()
                    .find(|d| d.id == s.id)
                    .is_none_or(|d| d.revision != s.revision || d.current_frame != s.frame)
            })
        {
            return Err("Book pages changed; prepare the preview again".into());
        }
        page_batch::Prepared {
            pages: vec![],
            sources: b.sources.clone(),
            rejected: vec![],
        }
        .fresh()
    }
    fn prepare_book(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let work = self
            .active_page_work()
            .ok_or("Open a local project in Page manager")?;
        let mut sources = vec![];
        let mut stamps = vec![];
        let budget = self.production_budget();
        let mut used = 0usize;
        for path in &work.pages {
            let d = self
                .page_work_document_id(path)
                .and_then(|id| self.documents.iter().find(|d| d.id == id))
                .or_else(|| {
                    self.documents.iter().find(|d| {
                        d.path.as_ref().is_some_and(|p| {
                            p == path
                                || p.canonicalize().ok().is_some_and(|actual| {
                                    path.canonicalize().ok().as_ref() == Some(&actual)
                                })
                        })
                    })
                });
            if let Some(d) = d {
                used = used.saturating_add(d.resident_bytes());
                if used > budget {
                    return Err("Book snapshot exceeds memory budget".into());
                }
                stamps.push(Stamp {
                    id: d.id,
                    revision: d.revision,
                    frame: d.current_frame,
                });
                sources.push(Source::Open(Box::new(drawer_core::layer_merge::preview(
                    &d.snapshot(),
                    d.current_frame,
                )?)));
            } else {
                sources.push(Source::File(path.clone()));
            }
        }
        self.book_workflow.book = None;
        self.book_workflow.reviewed = false;
        self.book_workflow.textures.clear();
        self.book_workflow.findings.clear();
        self.book_workflow.job = Some(job(ctx, false, move |cancel| {
            load_book(work, sources, stamps, budget, cancel).map(|b| Value::Book(Box::new(b)))
        }));
        Ok(())
    }
    fn add_scanned(
        &mut self,
        work: Work,
        owned: drawer_core::page_resources::Prepared,
    ) -> Result<(), String> {
        if self.active_page_work().as_ref() != Some(&work) {
            return Err("Project order changed during scanning; restore it before applying".into());
        }
        let count = owned.paths.len();
        self.edit_project_structure(drawer_core::page_work::Edit::Batch {
            paths: owned.paths.clone(),
            remove: vec![],
            index: work.pages.len(),
        })?;
        owned.keep();
        self.book_workflow.scan = None;
        self.book_workflow.scan_work = None;
        self.book_workflow.notice = format!(
            "Added {count} scanned pages with one project Undo. Native pages, original scan bytes and scans.json are permanent local resources. Save the project in Page manager."
        );
        Ok(())
    }
    fn next_scan(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let p = &mut self.book_workflow;
        let scan = p.scan.as_ref().ok_or("No scan batch")?;
        if scan.cancelled || scan.done() || scan.error.is_some() || p.job.is_some() {
            return Ok(());
        }
        let device = p
            .devices
            .get(p.selected)
            .cloned()
            .ok_or("Select a local WIA scanner")?;
        let budget = scan.remaining();
        let base = self.production_base();
        self.book_workflow.job = Some(job(ctx, true, move |cancel| {
            acquisition::book_capture(&base, device, budget, cancel).map(Value::Scan)
        }));
        Ok(())
    }
    fn poll_book_workflow(&mut self, ctx: &egui::Context) {
        let Some(j) = &self.book_workflow.job else {
            return;
        };
        let result = match j.rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("Book worker stopped".into())),
            Err(mpsc::TryRecvError::Empty) => None,
        };
        if let Some(result) = result {
            let j = self.book_workflow.job.take().unwrap();
            let cancelled = j.cancel.load(Ordering::Relaxed);
            let scan = j.scan;
            drop(j);
            if cancelled {
                self.book_workflow.notice = "Cancelled; project and source files unchanged".into();
                return;
            }
            let result=result.and_then(|v|match v {
                Value::Book(b)=>{self.check_book(&b)?;self.book_workflow.setup.validate(&b.work)?;self.book_workflow.findings=book_production::preflight(&b.work,&b.pages,&self.book_workflow.setup)?;self.book_workflow.index=self.book_workflow.index.min(b.pages.len()-1);self.book_workflow.book=Some(*b);self.book_workflow.notice="Prepared local book snapshot; run preflight after changing the setup".into();Ok(())},
                Value::Imported(p)=>{self.book_workflow.notice=format!("Prepared {} storyboard pages. Review before opening a separate local project; existing drawings remain open.",p.pages.len());self.book_workflow.imported=Some(*p);Ok(())},
                Value::Opened(p,owned)=>{p.fresh()?;let work=p.work(owned.paths.clone())?;self.set_project_structure_work(work)?;self.book_workflow.setup=p.file.setup;owned.keep();self.book_workflow.imported=None;self.book_workflow.book=None;self.book_workflow.textures.clear();self.book_workflow.notice="Opened an independent storyboard project. Original files and drawings remain; save this project in Page manager.".into();Ok(())},
                Value::Devices(d)=>{self.book_workflow.devices=d;self.book_workflow.selected=0;Ok(())},
                Value::Scan(image)=>{if let Some(s)=&mut self.book_workflow.scan{s.accept(image)?;}Ok(())},
                Value::Scanned(work,owned)=>self.add_scanned(work,owned),
                Value::Output(s)=>{self.book_workflow.notice=s;Ok(())}
            });
            if let Err(e) = result {
                if scan && let Some(s) = &mut self.book_workflow.scan {
                    s.fail(e.clone());
                }
                self.book_workflow.notice = e;
            }
        }
        if self.book_workflow.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }
    fn book_mesh(&mut self, ui: &mut egui::Ui) {
        let p = &mut self.book_workflow;
        let Some(b) = &p.book else {
            return;
        };
        let d = &b.pages[p.index];
        let size_mm = [
            d.width as f64 * 25.4 / d.dpi,
            d.height as f64 * 25.4 / d.dpi,
        ];
        let faces = match book_production::preview(
            &b.work, &p.setup, p.index, p.closed, p.opening, p.yaw, p.pitch, size_mm,
        ) {
            Ok(f) => f,
            Err(e) => {
                ui.label(e);
                return;
            }
        };
        let needed: std::collections::BTreeSet<_> = faces.iter().filter_map(|f| f.page).collect();
        p.textures.retain(|n, _| needed.contains(n));
        for n in needed {
            if let std::collections::btree_map::Entry::Vacant(e) = p.textures.entry(n) {
                match texture(ui.ctx(), &b.pages[n]) {
                    Ok(t) => {
                        e.insert(t);
                    }
                    Err(error) => {
                        p.notice = error;
                    }
                }
            }
        }
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(600.), 270.),
            egui::Sense::hover(),
        );
        let scale = rect.height() * 1.6;
        for f in faces {
            let points = f
                .points
                .map(|v| rect.center() + egui::vec2(v[0] as f32 * scale, v[1] as f32 * scale));
            let color = if f.spine {
                egui::Color32::from_rgb(80, 70, 65)
            } else {
                egui::Color32::WHITE
            };
            if let Some(t) = f.page.and_then(|n| p.textures.get(&n)) {
                let mut mesh = egui::Mesh::with_texture(t.id());
                for (pos, uv) in points
                    .into_iter()
                    .zip([[0., 0.], [1., 0.], [1., 1.], [0., 1.]])
                {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos,
                        uv: egui::pos2(uv[0], uv[1]),
                        color,
                    });
                }
                mesh.indices.extend([0, 1, 2, 0, 2, 3]);
                ui.painter().add(egui::Shape::mesh(mesh));
            } else {
                ui.painter().add(egui::Shape::convex_polygon(
                    points.to_vec(),
                    color,
                    egui::Stroke::NONE,
                ));
            }
            ui.painter().add(egui::Shape::closed_line(
                points.to_vec(),
                egui::Stroke::new(1., egui::Color32::GRAY),
            ));
        }
        let slots = book_production::spread(&b.work, p.index).unwrap_or([None, None]);
        ui.label(format!("Visual left {:?} / right {:?}; blank slots are unpaired pages. Spine {:.2}mm. {:?} binding.",slots[0].map(|n|n+1),slots[1].map(|n|n+1),p.setup.spine_mm,if b.work.right_binding{"right"}else{"left"}));
    }
    pub(super) fn book_workflow_window(&mut self, ctx: &egui::Context) {
        self.poll_book_workflow(ctx);
        if !self.book_workflow.open {
            return;
        }
        let previous_setup = self.book_workflow.setup.clone();
        let mut open = true;
        let mut action = 0u8;
        let mut cancel = false;
        egui::Window::new("Local book production").default_width(650.).open(&mut open).vscroll(true).show(ctx,|ui|{
            let p=&mut self.book_workflow;
            ui.horizontal(|ui|{ui.selectable_value(&mut p.section,0,"Book preview / preflight");ui.selectable_value(&mut p.section,1,"Storyboard exchange");ui.selectable_value(&mut p.section,2,"Continuous scan");});
            ui.label(&p.notice);
            if p.job.is_some(){ui.spinner();cancel=ui.button("Cancel current operation").clicked();return;}
            if p.section==0 {
                if ui.button("Prepare project preview").clicked(){action=1;}
                ui.horizontal(|ui|{ui.label("Minimum DPI");ui.add(egui::DragValue::new(&mut p.setup.minimum_dpi).range(1. ..=4800.));ui.label("Spine mm");ui.add(egui::DragValue::new(&mut p.setup.spine_mm).range(0. ..=100.));});
                ui.horizontal(|ui|{ui.label("Minimum bleed / safe mm");ui.add(egui::DragValue::new(&mut p.setup.bleed_mm).range(0. ..=100.));ui.add(egui::DragValue::new(&mut p.setup.safe_mm).range(0. ..=100.));});
                ui.horizontal(|ui|{ui.label("Expected trim mm (0 = any)");for v in &mut p.setup.trim_mm{ui.add(egui::DragValue::new(v).range(0. ..=10000.));}});
                if let Some(b)=&p.book {
                    for (label,value) in [("Front cover",&mut p.setup.front),("Back cover",&mut p.setup.back)]{egui::ComboBox::from_label(label).selected_text(value.map_or("None".into(),|id|format!("Page ID{id}"))).show_ui(ui,|ui|{ui.selectable_value(value,None,"None");for (n,id) in b.work.page_ids.iter().enumerate(){ui.selectable_value(value,Some(*id),format!("Page{} / ID{id}",n+1));}});}
                    let mut number=p.index+1;ui.add(egui::Slider::new(&mut number,1..=b.pages.len()).text("Preview page"));p.index=number-1;ui.checkbox(&mut p.closed,"Cover / spine view");
                    ui.add(egui::Slider::new(&mut p.opening,0. ..=170.).text("Opening"));ui.add(egui::Slider::new(&mut p.yaw,-75. ..=75.).text("Rotate"));ui.add(egui::Slider::new(&mut p.pitch,-60. ..=60.).text("Tilt"));
                    if ui.button("Run output preflight").clicked(){action=2;}
                }
                ui.horizontal(|ui|{if ui.button("Save book setup…").clicked(){action=3;}
if ui.button("Load book setup…").clicked(){action=4;}});
                if self.book_workflow.book.is_some(){ui.checkbox(&mut self.book_workflow.reviewed,"I reviewed the preflight warnings for this output");if ui.button("Export preflight-checked PDF…").clicked(){action=16;}}
                self.book_mesh(ui);
                for f in &self.book_workflow.findings{ui.colored_label(if f.blocking{egui::Color32::LIGHT_RED}else{egui::Color32::YELLOW},format!("Page ID{} · {} · {}",f.page,if f.blocking{"ERROR"}else{"REVIEW"},f.message));}
            } else if p.section==1 {
                ui.label("Drawer storyboard v1 only. External proprietary formats have no verified adapter. JSON carries ordered IDs, editable frames/dialogues and hashed local native/image references.");
                ui.text_edit_singleline(&mut p.path);
                ui.horizontal(|ui|{if ui.button("Choose storyboard…").clicked(){action=5;}
if ui.button("Prepare storyboard import").clicked(){action=6;}
if ui.button("Export prepared book…").clicked(){action=7;}});
                if let Some(s)=&p.imported {ui.label(format!("{} · {} pages · {} dialogues · {} frames",s.file.title,s.pages.len(),s.file.pages.iter().map(|p|p.dialogues.len()).sum::<usize>(),s.file.pages.iter().map(|p|p.frames.len()).sum::<usize>()));
                    for (n,page) in s.file.pages.iter().take(24).enumerate(){ui.label(format!("{}: ID{} {}",n+1,page.id,page.source.path.display()));}
                    let r=ui.button("Open storyboard as independent project");p.story_apply=Some(r.rect);if r.clicked(){action=8;}
                }
            } else {
                ui.label("Each local WIA acquisition becomes a reviewed page. A failure pauses at that slot: retry, skip, or abort. No scanner is accessed until you request it. Tests use a mock; physical scanning is unverified.");
                if p.scan.is_none(){
                    if ui.button("Refresh local scanners").clicked(){action=9;}
                    for (n,d) in p.devices.iter().enumerate(){ui.selectable_value(&mut p.selected,n,&d.name);}
                    ui.horizontal(|ui|{ui.label("Pages / DPI");ui.add(egui::DragValue::new(&mut p.count).range(1..=512));ui.add(egui::DragValue::new(&mut p.dpi).range(1. ..=4800.));});
                    if ui.add_enabled(!p.devices.is_empty(),egui::Button::new("Start continuous scan")).clicked(){action=10;}
                }
                if let Some(s)=&mut p.scan {
                    ui.label(format!("Slot {}/{} · accepted{} · skipped{}",(s.slot+1).min(s.count),s.count,s.accepted.len(),s.skipped.len()));
                    for image in &s.accepted {ui.label(format!("{}: {}×{} {}",image.slot+1,image.document.width,image.document.height,image.filename));}
                    if let Some(e)=&s.error{ui.colored_label(egui::Color32::LIGHT_RED,e);ui.horizontal(|ui|{if ui.button("Retry failed scan").clicked(){action=11;}
if ui.button("Skip failed scan").clicked(){action=12;}});}
                    if s.done(){ui.label("Review this batch before adding it. Original scan files are retained beside the native pages.");let r=ui.add_enabled(!s.accepted.is_empty(),egui::Button::new("Add scanned pages as one project edit"));p.scan_apply=Some(r.rect);if r.clicked(){action=13;}}
                    if ui.button("Abort scan batch").clicked(){action=14;}
                }
            }
            if self.book_workflow.section!=0 || action==8 || action==13 {ui.label("Local resource folder:");ui.text_edit_singleline(&mut self.book_workflow.directory);if ui.button("Choose resource folder…").clicked(){action=15;}}
        });
        if self.book_workflow.setup != previous_setup {
            self.book_workflow.reviewed = false;
            self.book_workflow.findings.clear();
        }
        if !open {
            cancel = true;
            if let Some(s) = &mut self.book_workflow.scan {
                s.cancel();
            }
            self.book_workflow.scan = None;
            self.book_workflow.imported = None;
            self.book_workflow.book = None;
            self.book_workflow.textures.clear();
            self.book_workflow.open = false;
        }
        if cancel {
            if let Some(j) = &self.book_workflow.job {
                j.cancel.store(true, Ordering::Relaxed);
            }
            if let Some(s) = &mut self.book_workflow.scan {
                s.cancel();
            }
            self.book_workflow.scan = None;
        }
        if action > 0
            && let Err(e) = self.production_action(ctx, action)
        {
            self.book_workflow.notice = e;
        }
        if self.book_workflow.open
            && let Err(e) = self.next_scan_if_ready(ctx)
        {
            self.book_workflow.notice = e;
        }
    }
    fn next_scan_if_ready(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.book_workflow.scan.is_some() {
            self.next_scan(ctx)
        } else {
            Ok(())
        }
    }
    fn production_action(&mut self, ctx: &egui::Context, action: u8) -> Result<(), String> {
        match action {
            1 => self.prepare_book(ctx)?,
            2 => {
                let b = self
                    .book_workflow
                    .book
                    .as_ref()
                    .ok_or("Prepare the book first")?;
                self.check_book(b)?;
                self.book_workflow.findings =
                    book_production::preflight(&b.work, &b.pages, &self.book_workflow.setup)?;
                self.book_workflow.notice = if self.book_workflow.findings.is_empty() {
                    "Preflight passed for this snapshot and these physical thresholds".into()
                } else {
                    format!(
                        "{} preflight findings; review before output",
                        self.book_workflow.findings.len()
                    )
                };
            }
            3 => {
                let setup = self.book_workflow.setup.clone();
                if !setup.valid() {
                    return Err("Invalid book setup".into());
                }
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Drawer book setup", &["drawer-book"])
                    .set_file_name("book.drawer-book")
                    .save_file()
                {
                    self.book_workflow.job = Some(job(ctx, false, move |cancel| {
                        publish(&path, &setup.encode()?, cancel)?;
                        Ok(Value::Output("Book setup saved locally".into()))
                    }));
                }
            }
            4 => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Drawer book setup", &["drawer-book"])
                    .pick_file()
                {
                    self.book_workflow.setup = Setup::decode(&book_io::read_local(&path, 16384)?)?;
                    self.book_workflow.findings.clear();
                }
            }
            5 => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Drawer storyboard v1", &["drawer-storyboard"])
                    .pick_file()
                {
                    self.book_workflow.path = path.to_string_lossy().into_owned();
                    self.book_workflow.imported = None;
                }
            }
            6 => {
                let path = PathBuf::from(self.book_workflow.path.trim());
                let budget = self.production_budget();
                self.book_workflow.imported = None;
                self.book_workflow.job = Some(job(ctx, false, move |cancel| {
                    storyboard::decode(&path, budget, cancel).map(|p| Value::Imported(Box::new(p)))
                }));
            }
            7 => {
                let b = self
                    .book_workflow
                    .book
                    .as_ref()
                    .ok_or("Prepare project preview first")?;
                self.check_book(b)?;
                self.book_workflow.setup.validate(&b.work)?;
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Drawer storyboard v1", &["drawer-storyboard"])
                    .set_file_name("story.drawer-storyboard")
                    .save_file()
                {
                    let b = self.book_workflow.book.take().unwrap();
                    let setup = self.book_workflow.setup.clone();
                    let budget = self.production_budget();
                    self.book_workflow.textures.clear();
                    self.book_workflow.job = Some(job(ctx, false, move |cancel| {
                        page_batch::Prepared {
                            pages: vec![],
                            sources: b.sources,
                            rejected: vec![],
                        }
                        .fresh()?;
                        storyboard::export(
                            &path,
                            &b.work,
                            &b.pages,
                            &setup,
                            budget,
                            cancel,
                            &mut |a, b| {
                                drawer_platform::local_files::publish_no_replace(a, b)
                                    .map_err(|e| e.to_string())
                            },
                        )?;
                        Ok(Value::Output("Storyboard and owned local references published; source files unchanged".into()))
                    }));
                }
            }
            8 => {
                let p = self
                    .book_workflow
                    .imported
                    .take()
                    .ok_or("Prepare a storyboard first")?;
                p.fresh()?;
                let dir = PathBuf::from(self.book_workflow.directory.trim());
                self.book_workflow.job = Some(job(ctx, false, move |cancel| {
                    let owned = drawer_core::page_resources::prepare(&p.pages, &dir, cancel)?;
                    p.fresh()?;
                    Ok(Value::Opened(Box::new(p), owned))
                }));
            }
            9 => {
                let base = self.production_base();
                self.book_workflow.job = Some(job(ctx, false, move |cancel| {
                    acquisition::book_devices(&base, cancel).map(Value::Devices)
                }));
            }
            10 => {
                let work = self
                    .active_page_work()
                    .ok_or("Open a local project before scanning")?;
                if work.pages.len() + self.book_workflow.count > 512 {
                    return Err("Scan batch would exceed512 project pages".into());
                }
                self.book_workflow.scan = Some(scan_batch::Session::new(
                    self.book_workflow.count,
                    self.book_workflow.dpi,
                    self.production_budget(),
                )?);
                self.book_workflow.scan_work = Some(work);
            }
            11 => self
                .book_workflow
                .scan
                .as_mut()
                .ok_or("No batch")?
                .retry()?,
            12 => self.book_workflow.scan.as_mut().ok_or("No batch")?.skip()?,
            13 => {
                let mut s = self.book_workflow.scan.as_ref().ok_or("No batch")?.clone();
                let work = self
                    .book_workflow
                    .scan_work
                    .clone()
                    .ok_or("No original project")?;
                if self.active_page_work().as_ref() != Some(&work) {
                    return Err("Project order changed; restore before applying".into());
                }
                s.bind(&work)?;
                let dir = PathBuf::from(self.book_workflow.directory.trim());
                self.book_workflow.job = Some(job(ctx, false, move |cancel| {
                    s.prepare(&dir, cancel)
                        .map(|owned| Value::Scanned(work, owned))
                }));
            }
            14 => {
                if let Some(s) = &mut self.book_workflow.scan {
                    s.cancel();
                }
                self.book_workflow.scan = None;
                self.book_workflow.scan_work = None;
                self.book_workflow.notice =
                    "Aborted; project and all source files unchanged".into();
            }
            15 => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.book_workflow.directory = dir.to_string_lossy().into_owned();
                }
            }
            16 => {
                let b = self
                    .book_workflow
                    .book
                    .as_ref()
                    .ok_or("Prepare the project preview first")?;
                self.check_book(b)?;
                self.book_workflow.findings =
                    book_production::preflight(&b.work, &b.pages, &self.book_workflow.setup)?;
                if self.book_workflow.findings.iter().any(|f| f.blocking)
                    || (!self.book_workflow.findings.is_empty() && !self.book_workflow.reviewed)
                {
                    return Err("Review preflight: fix errors and explicitly acknowledge warnings before PDF output".into());
                }
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Checked PDF", &["pdf"])
                    .set_file_name("book.pdf")
                    .save_file()
                {
                    let b = self.book_workflow.book.take().unwrap();
                    let setup = self.book_workflow.setup.clone();
                    let reviewed = self.book_workflow.reviewed;
                    let budget = self.production_budget();
                    self.book_workflow.textures.clear();
                    self.book_workflow.job = Some(job(ctx, false, move |cancel| {
                        page_batch::Prepared {
                            pages: vec![],
                            sources: b.sources,
                            rejected: vec![],
                        }
                        .fresh()?;
                        let bytes = book_production::checked_pdf(
                            &b.work, &b.pages, &setup, reviewed, budget, cancel,
                        )?;
                        publish(&path, &bytes, cancel)?;
                        Ok(Value::Output(
                            "Preflight-checked PDF published locally".into(),
                        ))
                    }));
                }
            }
            _ => return Err("Unknown book action".into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pointer_local_book_storyboard_and_mock_continuous_scan() {
        let root =
            std::env::temp_dir().join(format!("drawer-book-workflow-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let result = super::smoke(&root);
        std::fs::remove_dir(root).unwrap();
        result.unwrap();
    }
}
