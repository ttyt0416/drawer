use super::*;
use drawer_core::book_io::{self as core, Page};
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
    path: String,
    import_range: String,
    export_range: String,
    dpi: f64,
    title: String,
    author: String,
    language: String,
    cover: usize,
    right_binding: bool,
    blank_after: String,
    import_meta: Option<drawer_core::epub_metadata::Metadata>,
    project: bool,
    prepared: Option<(PathBuf, Vec<Page>)>,
    job: Option<Job>,
    notice: String,
    texture: Option<egui::TextureHandle>,
    pub apply: Option<egui::Rect>,
}
struct Job {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<ResultValue, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
enum ResultValue {
    Pages(
        PathBuf,
        Vec<Page>,
        Option<drawer_core::epub_metadata::Metadata>,
    ),
    Output(String),
}
fn job(
    ctx: &egui::Context,
    work: impl FnOnce(&AtomicBool) -> Result<ResultValue, String> + Send + 'static,
) -> Job {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (tx, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = work(&flag);
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    Job { cancel, receiver }
}
fn import(
    path: &Path,
    range: &str,
    dpi: f64,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Page>, String> {
    let bytes = core::read_local(path, budget / 2)?;
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "epub" => drawer_core::book_epub::decode(&bytes, range, budget, cancel),
        "pdf" => {
            #[cfg(windows)]
            {
                drawer_platform::book_pdf::render(&bytes, range, dpi, budget, cancel)
            }
            #[cfg(not(windows))]
            {
                let _ = (bytes, range, dpi, budget, cancel);
                Err("PDF raster import requires Windows".into())
            }
        }
        _ => Err("Choose a local .pdf or fixed-layout image .epub".into()),
    }
}
fn publish(path: &Path, bytes: &[u8], cancel: &AtomicBool) -> Result<(), String> {
    core::publish(path, bytes, cancel, &mut |a, b| {
        drawer_platform::local_files::publish_no_replace(a, b).map_err(|e| e.to_string())
    })
}
enum Source {
    Open(Box<Document>),
    Closed(PathBuf),
}
fn output(
    sources: Vec<Source>,
    dpi: f64,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Page>, String> {
    let held = sources
        .iter()
        .map(|s| match s {
            Source::Open(d) => d.resident_bytes(),
            _ => 0,
        })
        .sum::<usize>();
    let mut pages = vec![];
    let mut used = held;
    for source in sources {
        core::check_cancel(cancel)?;
        let already_held = matches!(&source, Source::Open(_));
        let d = match source {
            Source::Open(d) => d,
            Source::Closed(p) => Box::new(
                drawer_core::files::open(&p, budget.saturating_sub(used))
                    .map_err(|e| e.to_string())?,
            ),
        };
        let remaining = budget.saturating_sub(used).saturating_sub(if already_held {
            0
        } else {
            d.resident_bytes()
        });
        let mut page = core::compose(&[&d], dpi, remaining, cancel)?;
        used = used.saturating_add(page[0].image.memory_bytes());
        if already_held {
            used = used.saturating_sub(d.resident_bytes());
        }
        pages.append(&mut page);
    }
    core::validate(&pages, budget.saturating_sub(held))?;
    Ok(pages)
}
impl Drawer {
    pub(super) fn begin_book_io(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Apply or cancel the pending edit first".into());
        }
        self.book_io.open = true;
        if self.book_io.dpi == 0. {
            self.book_io.dpi = 300.;
            self.book_io.title = "Local image book".into();
            self.book_io.language = "ko".into();
            self.book_io.project = self.active_page_work().is_some();
        }
        Ok(())
    }
    fn book_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum())
    }
    fn apply_book_pages(&mut self) -> Result<(), String> {
        let (path, pages) = self
            .book_io
            .prepared
            .as_ref()
            .ok_or("Prepare the import first")?;
        if self.tabs.len().saturating_add(pages.len()) > 256 {
            return Err("Close some views before importing pages (256-view limit)".into());
        }
        let budget = self.book_budget();
        let mut docs = vec![];
        let mut used = pages.iter().map(|p| p.image.memory_bytes()).sum::<usize>();
        for (n, p) in pages.iter().enumerate() {
            let mut d = p.document(budget.saturating_sub(used))?;
            d.source_path = Some(path.clone());
            if let Some(meta) = &self.book_io.import_meta {
                let info = &mut d.scene.comics.page;
                info.title = meta.title.clone();
                info.author = meta.author.clone();
                info.language = meta.language.clone();
                info.right_binding = meta.right_binding;
                info.number = n as u32 + 1;
                info.total = pages.len() as u32;
            }
            used = used.saturating_add(d.resident_bytes());
            if used > budget {
                return Err("Imported document memory limit; choose fewer pages".into());
            }
            docs.push(d);
        }
        self.protected_sources.insert(path.clone());
        self.book_io.prepared = None;
        self.book_io.texture = None;
        let count = docs.len();
        for d in docs {
            self.add_document(d);
        }
        self.book_io.notice = format!(
            "Imported {count} independent raster pages. Save each as .drawer to retain edits."
        );
        Ok(())
    }
    fn book_sources(&self) -> Result<Vec<Source>, String> {
        let paths = self
            .book_io
            .project
            .then(|| self.active_page_work())
            .flatten();
        if self.book_io.project && paths.is_none() {
            return Err("Open a local project or choose open documents".into());
        }
        let count = paths
            .as_ref()
            .map_or(self.documents.len(), |w| w.pages.len());
        let chosen = core::selection(&self.book_io.export_range, count)?;
        let mut sources = vec![];
        let mut held = 0usize;
        for n in chosen {
            let d = if let Some(w) = &paths {
                let path = &w.pages[n];
                self.page_work_document_id(path)
                    .and_then(|id| self.documents.iter().find(|d| d.id == id))
            } else {
                self.documents.get(n)
            };
            if let Some(d) = d {
                held = held.saturating_add(d.resident_bytes());
                if held > self.book_budget() / 2 {
                    return Err("Output snapshot memory limit; choose fewer pages".into());
                }
                sources.push(Source::Open(Box::new(d.clone())));
            } else {
                sources.push(Source::Closed(paths.as_ref().unwrap().pages[n].clone()));
            }
        }
        Ok(sources)
    }
    fn start_book_import(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.book_io.job.is_some() {
            return Err("Wait for the current page operation".into());
        }
        let path = PathBuf::from(&self.book_io.path)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let range = self.book_io.import_range.clone();
        let dpi = self.book_io.dpi;
        let budget = self.book_budget();
        self.book_io.prepared = None;
        self.book_io.texture = None;
        self.book_io.notice = "Preparing every selected page before adding documents…".into();
        self.book_io.job = Some(job(ctx, move |cancel| {
            let (pages, metadata) = if path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("epub"))
            {
                let bytes = core::read_local(&path, budget / 2)?;
                let decoded = drawer_core::epub_metadata::decode(&bytes, &range, budget, cancel)?;
                (decoded.pages, Some(decoded.metadata))
            } else {
                (import(&path, &range, dpi, budget, cancel)?, None)
            };
            core::check_cancel(cancel)?;
            Ok(ResultValue::Pages(path, pages, metadata))
        }));
        Ok(())
    }
    fn start_book_output(
        &mut self,
        ctx: &egui::Context,
        kind: u8,
        path: Option<PathBuf>,
    ) -> Result<(), String> {
        if self.book_io.job.is_some() {
            return Err("Wait for the current page operation".into());
        }
        let sources = self.book_sources()?;
        let dpi = self.book_io.dpi;
        let budget = self.book_budget();
        let metadata = drawer_core::epub_metadata::Metadata {
            title: self.book_io.title.clone(),
            author: self.book_io.author.clone(),
            language: self.book_io.language.clone(),
            identifier: String::new(),
            cover: self.book_io.cover.checked_sub(1),
            right_binding: self.book_io.right_binding,
        };
        let blank = self.book_io.blank_after.trim();
        let blank = if kind != 2 || blank.is_empty() {
            None
        } else {
            Some(
                blank
                    .parse::<usize>()
                    .map_err(|_| "Blank position must be an integer or empty")?,
            )
        };
        self.book_io.notice = "Preparing selected page composites…".into();
        self.book_io.job = Some(job(ctx, move |cancel| {
            let mut pages = output(sources, dpi, budget, cancel)?;
            if kind == 2
                && let Some(after) = blank
            {
                drawer_core::epub_metadata::blank_after(&mut pages, after, budget)?;
            }
            if kind == 3 {
                #[cfg(windows)]
                {
                    let sent = drawer_platform::book_print::show(&pages, cancel)?;
                    return Ok(ResultValue::Output(
                        if sent {
                            "Selected pages submitted to the chosen printer"
                        } else {
                            "Print dialog cancelled"
                        }
                        .into(),
                    ));
                }
                #[cfg(not(windows))]
                {
                    return Err("Printing requires Windows".into());
                }
            }
            let path = path.ok_or("Choose an output filename")?;
            let bytes = if kind == 1 {
                drawer_core::book_pdf::encode(&pages, cancel)?
            } else {
                drawer_core::book_epub::encode_metadata(&pages, &metadata, cancel)?
            };
            publish(&path, &bytes, cancel)?;
            Ok(ResultValue::Output(format!(
                "Saved {} composite pages to {}",
                pages.len(),
                path.display()
            )))
        }));
        Ok(())
    }
    pub(super) fn book_io_window(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.book_io.job {
            let result = match j.receiver.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("Page worker stopped".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let cancelled = j.cancel.load(Ordering::Relaxed);
                self.book_io.job = None;
                match result {
                    Ok(ResultValue::Pages(path, pages, metadata)) if !cancelled => {
                        if let Some(m) = &metadata {
                            self.book_io.title = m.title.clone();
                            self.book_io.author = m.author.clone();
                            self.book_io.language = m.language.clone();
                            self.book_io.right_binding = m.right_binding;
                            self.book_io.cover = m.cover.map_or(0, |n| n + 1);
                        }
                        self.book_io.import_meta = metadata;
                        self.book_io.notice = format!(
                            "{} raster pages prepared. Review, then add all pages.",
                            pages.len()
                        );
                        self.book_io.prepared = Some((path, pages));
                    }
                    Ok(ResultValue::Output(s)) => self.book_io.notice = s,
                    Ok(_) => self.book_io.notice = "Import cancelled".into(),
                    Err(e) => self.book_io.notice = e,
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        if !self.book_io.open {
            return;
        }
        let mut p = std::mem::take(&mut self.book_io);
        let mut open = true;
        let mut action = 0;
        let mut browse = false;
        let working = p.job.is_some();
        egui::Window::new("PDF, image EPUB and printing").id(egui::Id::new("local-book-io")).open(&mut open).default_width(660.).show(ctx,|ui|{
            ui.label("Pages are raster composites. Original layers, text and timeline structure remain in .drawer files.");
            ui.add_enabled_ui(!working,|ui|{
                let old=(p.path.clone(),p.import_range.clone(),p.dpi);
                ui.horizontal(|ui|{ui.label("Raster DPI");ui.add(egui::DragValue::new(&mut p.dpi).range(1. ..=4800.));});
                ui.separator();ui.label("Import local PDF / fixed-layout image EPUB");
                ui.horizontal(|ui|{ui.text_edit_singleline(&mut p.path);if ui.button("Browse file…").clicked(){browse=true;}});
                ui.horizontal(|ui|{ui.label("File pages (blank = all)");ui.text_edit_singleline(&mut p.import_range);});
                if old!=(p.path.clone(),p.import_range.clone(),p.dpi){p.prepared=None;p.texture=None;}
                if ui.button("Prepare selected file pages").clicked(){action=1;}
                if let Some((_,pages))=&p.prepared{
                    if p.texture.is_none(){let i=&pages[0].image;let scale=(i.width.max(i.height) as f64/256.).max(1.);let w=(i.width as f64/scale).round().max(1.)as usize;let h=(i.height as f64/scale).round().max(1.)as usize;let pixels=(0..h).flat_map(|y|(0..w).map(move|x|i.pixels[((y*i.height as usize/h)*i.width as usize)+(x*i.width as usize/w)])).flatten().collect::<Vec<_>>();p.texture=Some(ctx.load_texture("book-first-page",egui::ColorImage::from_rgba_unmultiplied([w,h],&pixels),egui::TextureOptions::LINEAR));}
                    if let Some(t)=&p.texture{ui.image((t.id(),t.size_vec2()));}ui.label(format!("First page shown · {} selected pages",pages.len()));
                    if let Some(m)=&p.import_meta{ui.label(format!("{} · {} · {} · {} · cover {:?}",m.title,m.author,m.language,m.identifier,m.cover.map(|n|n+1)));}
                    let r=ui.button("Add all prepared pages");p.apply=Some(r.rect);if r.clicked(){action=2;}
                    if ui.button("Discard prepared import").clicked(){p.prepared=None;p.texture=None;}
                }
                ui.separator();ui.label("Export / print selected page composites");
                ui.checkbox(&mut p.project,"Use current local project order (otherwise open documents)");
                ui.horizontal(|ui|{ui.label("Output pages (e.g. 1,3-5)");ui.text_edit_singleline(&mut p.export_range);});
                ui.horizontal(|ui|{ui.label("EPUB title");ui.text_edit_singleline(&mut p.title);});
                ui.horizontal(|ui|{ui.label("Author / language");ui.text_edit_singleline(&mut p.author);ui.text_edit_singleline(&mut p.language);});
                ui.checkbox(&mut p.right_binding,"EPUB reads right to left");
                ui.horizontal(|ui|{ui.label("Cover page in final EPUB (0 = none)");ui.add(egui::DragValue::new(&mut p.cover).range(0..=512));});
                ui.horizontal(|ui|{ui.label("Insert one blank after selected page (0 = before first; empty = none)");ui.text_edit_singleline(&mut p.blank_after);});
                ui.label("EPUB identifier is generated locally from metadata and pixels. Cover numbering includes the inserted blank.");
                ui.horizontal_wrapped(|ui|{if ui.button("Save composite PDF…").clicked(){action=3;}
if ui.button("Save image EPUB…").clicked(){action=4;}
if ui.button("Print selected pages…").clicked(){action=5;}});
                ui.label("Print scales each page to the printable area on white paper. Choose the printer and copies in the Windows dialog.");
            });
            if working{ui.spinner();if ui.button("Cancel page operation").clicked(){p.job.as_ref().unwrap().cancel.store(true,Ordering::Relaxed);}}
            if !p.notice.is_empty(){ui.label(&p.notice);}
if ui.button("Close").clicked(){action=9;}
        });
        if action == 9 {
            open = false;
        }
        p.open = open;
        if !open {
            p.prepared = None;
            p.texture = None;
            if let Some(j) = &p.job {
                j.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.book_io = p;
        if browse
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Local page books", &["pdf", "epub"])
                .pick_file()
        {
            self.book_io.path = path.to_string_lossy().into_owned();
            self.book_io.prepared = None;
            self.book_io.texture = None;
        }
        let result = match action {
            1 => self.start_book_import(ctx),
            2 => self.apply_book_pages(),
            3 | 4 => {
                let ext = if action == 3 { "pdf" } else { "epub" };
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Composite pages", &[ext])
                    .set_file_name(format!("pages.{ext}"))
                    .save_file()
                {
                    if !path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
                    {
                        Err(format!("Choose a .{ext} filename"))
                    } else {
                        self.start_book_output(ctx, if action == 3 { 1 } else { 2 }, Some(path))
                    }
                } else {
                    Ok(())
                }
            }
            5 => self.start_book_output(ctx, 3, None),
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.book_io.notice = e;
        }
    }
}

pub(super) fn smoke(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    drawer_core::book_verify::verify(root)?;
    #[cfg(windows)]
    drawer_platform::book_pdf::verify()?;
    let dir = root.join("book-app-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let cancel = AtomicBool::new(false);
        let pages = drawer_core::book_verify::pages();
        let bytes = drawer_core::book_epub::encode(&pages, "로컬 책", &cancel)?;
        let path = dir.join("source.epub");
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.documents[0].edit_pixel(0, 0, 0, [12, 34, 56, 255]);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.execute_command(&ctx, "page-exchange");
        app.book_io.path = path.to_string_lossy().into_owned();
        app.book_io.import_range = "2-3".into();
        app.start_book_import(&ctx)?;
        let mut time = 0.;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.book_io.job.is_some() {
            time += 0.1;
            input_replay::frame(&ctx, &mut app, time, vec![]);
            if std::time::Instant::now() > deadline {
                return Err("Book import worker timed out".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app
            .book_io
            .prepared
            .as_ref()
            .is_none_or(|(_, p)| p != &pages[1..])
            || app.documents.len() != 1
            || app.documents[0].snapshot() != before
        {
            return Err(format!(
                "Book preview partial import/source mutation: {}",
                app.book_io.notice
            ));
        }
        for _ in 0..3 {
            time += 0.1;
            input_replay::frame(&ctx, &mut app, time, vec![]);
        }
        let r = app
            .book_io
            .apply
            .ok_or("Prepared page confirmation button missing")?;
        for pressed in [true, false] {
            time += 0.1;
            input_replay::frame(
                &ctx,
                &mut app,
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
        if app.documents.len() != 3
            || app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != history
            || app.documents[1].layers[0].pixels != pages[1].image.pixels
            || app.documents[2].layers[0].pixels != pages[2].image.pixels
            || std::fs::read(&path).unwrap() != bytes
        {
            return Err(format!(
                "Book confirmation, independent pixels or original history/source bytes: {}",
                app.book_io.notice
            ));
        }
        app.documents[1].edit_pixel(0, 0, 0, [255; 4]);
        if app.documents[2].layers[0].pixels[0] != pages[2].image.pixels[0] {
            return Err("Imported pages shared mutable pixels".into());
        }
        let native = dir.join("imported.drawer");
        drawer_core::files::save(&mut app.documents[1], &native).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[1].snapshot()
        {
            return Err("Imported page native reopen".into());
        }
        app.book_io.export_range = "2-3".into();
        let out = output(app.book_sources()?, 96., app.book_budget(), &cancel)?;
        let pdf = drawer_core::book_pdf::encode(&out, &cancel)?;
        let dest = dir.join("output.pdf");
        publish(&dest, &pdf, &cancel)?;
        if drawer_core::book_pdf::validate(&std::fs::read(&dest).unwrap())? != 2 {
            return Err("Selected app PDF output page count".into());
        }
        let initial = app.documents.len();
        let mut invalid = pages.clone();
        invalid[2].dpi = 0.;
        app.book_io.prepared = Some((path.clone(), invalid));
        if app.apply_book_pages().is_ok()
            || app.documents.len() != initial
            || app.documents[0].snapshot() != before
        {
            return Err("Late invalid page added a partial import".into());
        }
        app.book_io.prepared = None;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
pub(super) fn metadata_smoke(root: &Path) -> Result<(), String> {
    drawer_core::publication_verify::epub()?;
    let dir = root.join("epub-metadata-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        let mut d = Document::create(
            CanvasSpec {
                width: 12,
                height: 18,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())?;
        d.edit_pixel(0, 2, 2, [90, 30, 120, 255]);
        let snapshot = d.snapshot();
        let history = d.history_counts();
        app.add_document(d);
        app.execute_command(&ctx, "page-exchange");
        app.book_io.project = false;
        app.book_io.title = "한글 도서".into();
        app.book_io.author = "로컬 저자".into();
        app.book_io.language = "ko-KR".into();
        app.book_io.right_binding = true;
        app.book_io.cover = 2;
        app.book_io.blank_after = "0".into();
        let mut time = 0.;
        let frame = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            input_replay::frame(&ctx, app, *time, events);
        };
        let wait = |app: &mut Drawer, time: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while app.book_io.job.is_some() {
                frame(app, time, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Metadata EPUB worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(())
        };
        let path = dir.join("book.epub");
        app.start_book_output(&ctx, 2, Some(path.clone()))?;
        wait(&mut app, &mut time)?;
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {}", e, app.book_io.notice))?;
        let decoded = drawer_core::epub_metadata::decode(
            &bytes,
            "",
            DEFAULT_DOCUMENT_BUDGET,
            &AtomicBool::new(false),
        )?;
        if decoded.pages.len() != 2
            || decoded.metadata.cover != Some(1)
            || decoded.metadata.author != "로컬 저자"
            || !decoded.pages[0].image.pixels.iter().all(|p| *p == [255; 4])
        {
            return Err("UI metadata EPUB output failed".into());
        }
        app.book_io.path = path.to_string_lossy().into_owned();
        app.book_io.import_range = String::new();
        app.start_book_import(&ctx)?;
        wait(&mut app, &mut time)?;
        if app.book_io.prepared.is_none() {
            return Err(format!(
                "Metadata import preparation failed: {}",
                app.book_io.notice
            ));
        }
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .book_io
            .apply
            .ok_or("Metadata EPUB confirmation missing")?;
        for pressed in [true, false] {
            frame(
                &mut app,
                &mut time,
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
        if app.documents.len() != 3
            || app.documents[0].snapshot() != snapshot
            || app.documents[0].history_counts() != history
            || std::fs::read(&path).unwrap() != bytes
        {
            return Err("Metadata EPUB pointer import changed source or failed".into());
        }
        for d in &app.documents[1..] {
            let p = &d.scene.comics.page;
            if p.author != "로컬 저자" || p.language != "ko-KR" || !p.right_binding || p.total != 2
            {
                return Err("EPUB bibliography lost on imported native pages".into());
            }
        }
        app.book_io.blank_after = "unused invalid EPUB field".into();
        let pdf = dir.join("independent.pdf");
        app.start_book_output(&ctx, 1, Some(pdf.clone()))?;
        wait(&mut app, &mut time)?;
        if !pdf.is_file() {
            return Err(format!(
                "EPUB-only option blocked PDF output: {}",
                app.book_io.notice
            ));
        }
        let invalid_epub = dir.join("invalid.epub");
        if app
            .start_book_output(&ctx, 2, Some(invalid_epub.clone()))
            .is_ok()
            || invalid_epub.exists()
        {
            return Err("Invalid EPUB blank position was accepted".into());
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}

#[cfg(test)]
mod tests {
    #[test]
    fn pointer_epub_bibliography_blank_cover_import() {
        let root = std::env::temp_dir().join(format!("drawer-epub-meta-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::metadata_smoke(&root);
        std::fs::remove_dir(root).unwrap();
        r.unwrap();
    }
    #[test]
    fn book_preview_pointer_confirm_selected_output_and_atomic_partial_failure() {
        let root = std::env::temp_dir().join(format!("drawer-book-app-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        super::smoke(&root).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
