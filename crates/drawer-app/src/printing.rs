use super::*;
use drawer_core::{
    book_io::{self, Page},
    printing::{self as core, Paper, Settings},
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    draft: Settings,
    saved: Settings,
    blocked: bool,
    replace: bool,
    pages: Option<Arc<Vec<Page>>>,
    plan: Option<core::Plan>,
    page: usize,
    reviewed: bool,
    texture: Option<egui::TextureHandle>,
    job: Option<Job>,
    message: String,
    controls: BTreeMap<&'static str, egui::Rect>,
}
enum Value {
    Pages(Vec<Page>),
    Notice(String),
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
fn job(
    work: impl FnOnce(&AtomicBool) -> Result<Value, String> + Send + 'static,
) -> Result<Job, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("print-preview".into())
        .spawn(move || {
            let _ = tx.send(work(&flag));
        })
        .map_err(|e| e.to_string())?;
    Ok(Job { cancel, rx })
}
impl Drawer {
    fn print_panel(&self) -> Panel {
        let mut p = Panel {
            open: true,
            ..Default::default()
        };
        if let Some(s) = &self.storage {
            match Settings::load(s) {
                Ok(s) => {
                    p.saved = s.clone();
                    p.draft = s;
                }
                Err(e) => {
                    p.blocked = true;
                    p.message = format!(
                        "Saved print settings preserved: {e}. Confirm replacement before saving new preferences."
                    );
                }
            }
        }
        p
    }
    pub(super) fn begin_printing(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.printing.job.is_some() {
            return Err("Finish/cancel the current print preparation or printer dialog".into());
        }
        let d = &self.documents[self.active.ok_or("Open a source document")?];
        let available = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        if d.memory_bytes().saturating_mul(4) > available {
            return Err("Print snapshot exceeds workspace memory".into());
        }
        let copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        let dpi = d.dpi.clamp(1., 4800.);
        let budget = available.saturating_sub(copy.memory_bytes());
        let mut p = self.print_panel();
        p.message = "Preparing a read-only snapshot of the active page…".into();
        p.job = Some(job(move |c| {
            book_io::compose(&[&copy], dpi, budget, c).map(Value::Pages)
        })?);
        self.printing = p;
        Ok(())
    }
    pub(super) fn begin_print_pages(&mut self, pages: Vec<Page>) -> Result<(), String> {
        if self.printing.job.is_some() {
            return Err("Finish/cancel the current print preparation or printer dialog".into());
        }
        book_io::validate(&pages, book_io::MAX_BYTES)?;
        let held = pages.iter().map(|p| p.image.memory_bytes()).sum::<usize>();
        if held
            .saturating_mul(2)
            .saturating_add(self.documents.iter().map(Document::resident_bytes).sum())
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Prepared print pages exceed workspace memory".into());
        }
        let mut p = self.print_panel();
        p.pages = Some(Arc::new(pages));
        Self::refresh_print_plan(&mut p);
        self.printing = p;
        Ok(())
    }
    fn refresh_print_plan(p: &mut Panel) {
        p.reviewed = false;
        p.texture = None;
        p.plan = None;
        if let Some(pages) = &p.pages {
            match core::plan(pages, &p.draft) {
                Ok(plan) => {
                    p.page = p.page.min(plan.pages.len() - 1);
                    p.plan = Some(plan);
                    p.message="Snapshot ready. Review paper, margins, range and cropping before opening the Windows printer dialog.".into();
                }
                Err(e) => p.message = e,
            }
        }
    }
    fn save_printing(&self, p: &mut Panel) -> Result<(), String> {
        if p.blocked && !p.replace {
            return Err("Confirm replacement of the unreadable preference file".into());
        }
        let pages = p.pages.as_ref().ok_or("Prepare pages first")?;
        core::plan(pages, &p.draft)?;
        p.draft
            .save(self.storage.as_ref().ok_or("User storage unavailable")?)?;
        p.saved = p.draft.clone();
        p.blocked = false;
        p.replace = false;
        p.message = "Print preferences saved; document and Undo unchanged".into();
        Ok(())
    }
    fn print_action(&self, p: &mut Panel, pdf: Option<PathBuf>) -> Result<(), String> {
        if p.job.is_some() || !p.reviewed {
            return Err("Review the current snapshot/layout first".into());
        }
        let pages = p.pages.as_ref().ok_or("Prepare print pages first")?.clone();
        core::plan(&pages, &p.draft)?;
        let settings = p.draft.clone();
        if let Some(path) = pdf {
            drawer_core::file_objects::local_path(&path)?;
            if !path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("pdf"))
            {
                return Err("Choose a .pdf preview filename".into());
            }
            let budget = DEFAULT_DOCUMENT_BUDGET
                .saturating_sub(
                    self.documents
                        .iter()
                        .map(Document::resident_bytes)
                        .sum::<usize>(),
                )
                .saturating_sub(pages.iter().map(|p| p.image.memory_bytes()).sum::<usize>());
            p.job = Some(job(move |c| {
                let proof = core::virtual_pages(&pages, &settings, 96., budget / 2, c)?;
                let bytes = drawer_core::book_pdf::encode(&proof, c)?;
                book_io::publish(&path, &bytes, c, &mut |a, b| {
                    drawer_platform::local_files::publish_no_replace(a, b)
                        .map_err(|e| e.to_string())
                })?;
                Ok(Value::Notice("Virtual raster PDF saved; no OS spool/paper job. Physical dimensions/color validation is still pending.".into()))
            })?);
        } else {
            p.job = Some(job(move |c| {
                let sent = drawer_platform::book_print::show_with_settings(&pages, &settings, c)?;
                Ok(Value::Notice(if sent{"Spooler accepted the selected snapshot. Actual printer/paper/color validation is separate."}else{"Printer dialog canceled; no job submitted"}.into()))
            })?);
        }
        Ok(())
    }
    pub(super) fn printing_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.printing);

        if let Some(j) = &p.job {
            match j.rx.try_recv() {
                Ok(r) => {
                    let canceled = j.cancel.load(Ordering::Relaxed);
                    p.job = None;
                    match r {
                        Ok(Value::Pages(pages)) if !canceled => {
                            let held = pages.iter().map(|p| p.image.memory_bytes()).sum::<usize>();
                            if held.saturating_mul(2)
                                + self
                                    .documents
                                    .iter()
                                    .map(Document::resident_bytes)
                                    .sum::<usize>()
                                > DEFAULT_DOCUMENT_BUDGET
                            {
                                p.message =
                                    "Print snapshot/driver buffers exceed workspace memory".into();
                            } else {
                                p.pages = Some(Arc::new(pages));
                                Self::refresh_print_plan(&mut p);
                            }
                        }
                        Ok(Value::Notice(s)) => p.message = s,
                        Ok(_) => p.message = "Print preparation canceled".into(),
                        Err(e) => p.message = e,
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    p.job = None;
                    p.message = "Print worker stopped".into();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(25))
                }
            }
        }
        if !p.open {
            self.printing = p;
            return;
        }
        if p.texture.is_none()
            && let (Some(pages), Some(plan)) = (&p.pages, &p.plan)
        {
            let source = &pages[plan.pages[p.page].source].image;
            let step = source
                .width
                .div_ceil(400)
                .max(source.height.div_ceil(400))
                .max(1);
            let (w, h) = (source.width.div_ceil(step), source.height.div_ceil(step));
            let mut bytes = Vec::with_capacity(w as usize * h as usize * 4);
            for y in 0..h {
                for x in 0..w {
                    bytes.extend(source.pixels[(y * step * source.width + x * step) as usize]);
                }
            }
            p.texture = Some(ctx.load_texture(
                "print-source-preview",
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &bytes),
                egui::TextureOptions::LINEAR,
            ));
        }
        let before = p.draft.clone();
        let before_page = p.page;
        let mut open = true;
        let mut close = false;
        let mut action = "";
        p.controls.clear();
        egui::Window::new("Print layout and physical validation").default_pos(egui::pos2(25.,25.)).default_width(630.).max_height(700.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.horizontal(|ui|{for(key,label,enabled)in[("save","Save print settings",p.job.is_none()&&p.plan.is_some()),("print","Windows printer dialog…",p.job.is_none()&&p.plan.is_some()&&p.reviewed),("pdf","Virtual preview PDF…",p.job.is_none()&&p.plan.is_some()&&p.reviewed)]{let b=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key,b.rect);if b.clicked(){action=key;}}let b=ui.button("Close");p.controls.insert("close",b.rect);close=b.clicked();});
            ui.label("Preview is a captured page snapshot. Refresh from the drawing to include later edits. Virtual PDF is layout validation; actual printer/paper color must be checked separately.");
            ui.add_enabled_ui(p.job.is_none(),|ui|{
                ui.horizontal(|ui|{for(paper,label)in[(Paper::A4,"A4"),(Paper::Letter,"Letter"),(Paper::Custom,"Custom")]{let b=ui.selectable_value(&mut p.draft.paper,paper,label);if paper==Paper::Letter{p.controls.insert("letter",b.rect);}}ui.checkbox(&mut p.draft.landscape,"Landscape");});
                ui.add_enabled_ui(p.draft.paper==Paper::Custom,|ui|{ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut p.draft.custom_mm[0]).range(1.0..=2000.0).prefix("Paper mm "));ui.add(egui::DragValue::new(&mut p.draft.custom_mm[1]).range(1.0..=2000.0));});});
                ui.horizontal_wrapped(|ui|{for(n,name)in["Left mm ","Top mm ","Right mm ","Bottom mm "].into_iter().enumerate(){ui.add(egui::DragValue::new(&mut p.draft.margins[n]).range(0.0..=500.0).prefix(name));}});
                ui.checkbox(&mut p.draft.fit,"Fit to reviewed margins (otherwise physical size × percent)");ui.add_enabled(!p.draft.fit,egui::DragValue::new(&mut p.draft.percent).range(1.0..=400.0).suffix("%"));
                ui.horizontal(|ui|{ui.label("Range within prepared pages");ui.text_edit_singleline(&mut p.draft.range);});
                ui.checkbox(&mut p.draft.icm,"Windows ICM for explicit sRGB bitmap input");ui.small("Choose matching paper/orientation and driver options in the Windows dialog. Paper/printable-margin mismatches stop before any job is submitted. Monitor ICC is never used for print pixels.");
                if p.blocked{ui.checkbox(&mut p.replace,"Replace the unreadable saved print preferences");}
                let b=ui.checkbox(&mut p.reviewed,"Reviewed the snapshot, page range, physical size and clipped areas");p.controls.insert("review",b.rect);
            });
            if let(Some(plan),Some(texture))=(&p.plan,&p.texture){ui.horizontal(|ui|{let b=ui.button("Previous page");if b.clicked(){p.page=p.page.saturating_sub(1);}let b=ui.button("Next page");if b.clicked(){p.page=(p.page+1).min(plan.pages.len()-1);}ui.label(format!("{}/{} · source page{}",p.page+1,plan.pages.len(),plan.pages[p.page].source+1));});
                let scale=(390./plan.paper_mm[0]).min(330./plan.paper_mm[1]);let size=egui::vec2((plan.paper_mm[0]*scale) as f32,(plan.paper_mm[1]*scale) as f32);let(rect,_)=ui.allocate_exact_size(size,egui::Sense::hover());let painter=ui.painter();painter.rect_filled(rect,0.,egui::Color32::WHITE);
                let area=|a:core::Area|egui::Rect::from_min_size(rect.min+egui::vec2((a.x*scale) as f32,(a.y*scale) as f32),egui::vec2((a.width*scale) as f32,(a.height*scale) as f32));let item=&plan.pages[p.page];let clip=area(item.clip);painter.rect_stroke(clip,0.,egui::Stroke::new(1.,egui::Color32::GRAY),egui::StrokeKind::Inside);painter.with_clip_rect(clip.intersect(rect)).image(texture.id(),area(item.image),egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.,1.)),egui::Color32::WHITE);
                ui.label(format!("Paper {:.2}×{:.2}mm; content {:.2}×{:.2}mm",plan.paper_mm[0],plan.paper_mm[1],item.image.width,item.image.height));if item.clipped{ui.colored_label(egui::Color32::YELLOW,"This scale clips content at the reviewed margins. Reduce percent or choose Fit.");}
            }
            if p.job.is_some(){ui.spinner();if ui.button("Cancel job").clicked(){if let Some(j)=&p.job{j.cancel.store(true,Ordering::Relaxed);}p.message="Cancellation requested. Close any open Windows printer dialog with Cancel; the driver may finish an already-submitted page.".into();}}
            ui.label(&p.message);
        });
        if p.draft != before {
            Self::refresh_print_plan(&mut p);
        }
        if p.page != before_page {
            p.texture = None;
        }
        let result = match action {
            "save" => self.save_printing(&mut p),
            "print" => self.print_action(&mut p, None),
            "pdf" => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Virtual raster print preview PDF", &["pdf"])
                    .set_file_name("print-preview.pdf")
                    .save_file()
                {
                    self.print_action(&mut p, Some(path))
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            p.message = e;
        }
        if !open || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            p.open = false;
            p.job = None;
            p.pages = None;
            p.plan = None;
            p.texture = None;
            p.reviewed = false;
        }
        self.printing = p;
    }
}
#[path = "printing_verify.rs"]
mod verify;
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    verify::run(root)
}

impl Drawer {
    pub(super) fn quiesce_printing(&mut self, ctx: &egui::Context) -> bool {
        let open = self.printing.open;
        self.printing.open = false;
        self.printing_window(ctx);
        self.printing.open = open;
        self.printing.job.is_some()
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn paper_pointer_review_cancel_settings_and_virtual_pdf() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}
