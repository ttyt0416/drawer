use super::*;
use drawer_core::timesheet::{self as core, Naming, Options};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
};
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    path: String,
    folder: String,
    options: Options,
    ack: bool,
    prepared: Option<core::Prepared>,
    snapshot: Option<core::Snapshot>,
    job: Option<Job>,
    notice: String,
    pub capture: Option<egui::Rect>,
    pub start: Option<egui::Rect>,
    pub accept: Option<egui::Rect>,
}
struct Job {
    worker: Option<std::thread::JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    rx: mpsc::Receiver<Result<Value, String>>,
}
enum Value {
    Import(Box<core::Prepared>),
    Output(String),
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drawer {
    pub(super) fn begin_timesheet(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending edits before cel exchange".into());
        }
        self.timesheet.open = true;
        Ok(())
    }
    fn timesheet_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        )
    }
    fn capture_timesheet(&mut self) -> Result<(), String> {
        self.begin_timesheet()?;
        if self.timesheet.job.is_some() {
            return Err("Wait for or cancel the current exchange".into());
        }
        let d = &self.documents[self.active.ok_or("Choose an animation document")?];
        self.timesheet.snapshot = Some(core::capture(
            d,
            self.timesheet.options,
            self.timesheet_budget(),
        )?);
        self.timesheet.ack = false;
        self.timesheet.notice =
            "Captured committed cel sources and settings. Later edits will not change this bundle."
                .into();
        Ok(())
    }
    fn import_timesheet(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.timesheet.job.is_some() {
            return Err("Wait for the current exchange".into());
        }
        let path = PathBuf::from(self.timesheet.path.trim());
        let budget = self.timesheet_budget();
        self.timesheet.prepared = None;
        self.timesheet.job = Some(start(ctx, move |cancel, _| {
            core::prepare(&path, budget, cancel).map(|p| Value::Import(Box::new(p)))
        }));
        Ok(())
    }
    fn accept_timesheet(&mut self) -> Result<(), String> {
        let p = self
            .timesheet
            .prepared
            .as_ref()
            .ok_or("Prepare and review a timesheet first")?;
        p.fresh()?;
        if !self.timesheet.ack {
            return Err("Confirm flattened PNG precision and imported layout first".into());
        }
        if self.tabs.len() >= 256 || p.document.resident_bytes() > self.timesheet_budget() {
            return Err("Close other canvases to fit the imported document".into());
        }
        let p = self.timesheet.prepared.take().unwrap();
        self.protected_sources
            .extend(p.sources.into_iter().map(|s| s.path));
        self.add_document(p.document);
        self.timesheet.ack = false;
        self.timesheet.notice="Added an independent editable timeline. One Undo removes this import; Redo restores it. Source PNGs and sheets are preserved.".into();
        Ok(())
    }
    fn output_timesheet(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.timesheet.job.is_some() || !self.timesheet.ack {
            return Err("Review and confirm cel output first".into());
        }
        if self.timesheet.folder.trim().is_empty() {
            return Err("Choose a new local bundle folder".into());
        }
        let s = self
            .timesheet
            .snapshot
            .take()
            .ok_or("Capture the animation first")?;
        let destination = PathBuf::from(self.timesheet.folder.trim());
        self.timesheet.job = Some(start(ctx, move |cancel, progress| {
            let sheet = core::export(&s, &destination, cancel, progress, &mut |a, b| {
                drawer_platform::local_files::publish_no_replace(a, b).map_err(|e| e.to_string())
            })?;
            Ok(Value::Output(format!(
                "Published {} unique cel PNGs, {} tracks, JSON and CSV{}: {}",
                sheet.images.len(),
                sheet.tracks.len(),
                if sheet.common.is_some() {
                    " and a shared background PNG"
                } else {
                    ""
                },
                destination.display()
            )))
        }));
        Ok(())
    }
    fn timesheet_preset(&mut self, load: bool) -> Result<(), String> {
        let dialog = rfd::FileDialog::new().add_filter("Drawer timesheet preset", &["json"]);
        if load {
            if let Some(p) = dialog.pick_file() {
                let b = drawer_core::book_io::read_local(&p, 65536)?;
                self.timesheet.options = core::decode_options(&b)?;
                self.timesheet.snapshot = None;
                self.timesheet.ack = false;
                self.timesheet.notice =
                    "Loaded local cel output settings; capture and confirm again.".into();
            }
        } else if let Some(p) = dialog.set_file_name("cel-output-settings.json").save_file() {
            core::save_options(&p, self.timesheet.options)?;
            self.timesheet.notice = format!("Saved local output settings: {}", p.display());
        }
        Ok(())
    }
    pub(super) fn timesheet_window(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.timesheet.job {
            let r = match j.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Timesheet worker stopped".into()))
                }
            };
            if let Some(r) = r {
                let j = self.timesheet.job.take().unwrap();
                let canceled = j.cancel.load(Ordering::Relaxed);
                match r {
                    Ok(Value::Output(s)) => self.timesheet.notice = s,
                    Ok(Value::Import(p)) if !canceled => {
                        self.timesheet.prepared = Some(*p);
                        self.timesheet.ack = false;
                        self.timesheet.notice="Local PNGs and both reference graphs validated. Review before adding the document.".into();
                    }
                    Ok(_) => self.timesheet.notice = "Import canceled; drawings unchanged.".into(),
                    Err(e) => self.timesheet.notice = e,
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            }
        }
        if !self.timesheet.open {
            return;
        }
        let mut p = std::mem::take(&mut self.timesheet);
        let mut open = true;
        let mut action = 0;
        egui::Window::new("Local timesheet and shared cel exchange").default_pos(egui::pos2(15.,15.)).default_width(660.).max_height(625.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.label(&p.notice);
            if let Some(j)=&p.job{ui.label(format!("Prepared {} cel sources",j.progress.load(Ordering::Relaxed)));if ui.button("Cancel exchange").clicked(){j.cancel.store(true,Ordering::Relaxed);}return;}
            ui.label("Drawer timesheet v1 JSON / CSV only. No verified SAI / CSP / XDTS or other proprietary adapter. PNG cells are flattened8-bit sRGB; audio, keyframes, masks, layer structure and source precision are outside this exchange format.");
            if p.snapshot.is_some(){
                ui.label("Frozen output includes track order/visibility/opacity, cel references/offsets, exact rational FPS and exposures. Shared cells export once; unexposed cells follow the captured option. Common outside layers use one separate background PNG captured at the current frame, so reused cels keep their independent offsets.");
                ui.label(format!("Source revision {} · {:?}",p.snapshot.as_ref().unwrap().revision,p.options));
                ui.label("New local bundle folder (no merge or replacement)");ui.text_edit_singleline(&mut p.folder);
                ui.checkbox(&mut p.ack,"I reviewed the flattened precision, static common background and captured options");
                let r=ui.add_enabled(p.ack,egui::Button::new("Export captured cel bundle"));p.start=Some(r.rect);if r.clicked(){action=2;}
                if ui.button("Discard capture / change options").clicked(){p.snapshot=None;p.ack=false;}
                return;
            }
            if let Some(import)=&p.prepared{
                let s=&import.sheet;ui.label(format!("{}×{} · {} tracks · {} unique cel sources · {} frames · {}/{} FPS",s.width,s.height,s.tracks.len(),s.images.len(),s.frames,s.fps.numerator,s.fps.denominator));
                ui.label("The imported independent document restores cel sharing, frame exposure order and visibility. Each PNG is an editable raster cel; the common background remains separate.");
                ui.checkbox(&mut p.ack,"I reviewed this raster interchange and want an independent document");
                let r=ui.add_enabled(p.ack,egui::Button::new("Add independent timesheet document"));p.accept=Some(r.rect);if r.clicked(){action=4;}
                if ui.button("Discard prepared import").clicked(){p.prepared=None;p.ack=false;}return;
            }
            ui.collapsing("Import own JSON / CSV",|ui|{
                ui.text_edit_singleline(&mut p.path);
                ui.horizontal(|ui|{if ui.button("Choose timesheet…").clicked(){action=5;}
                    if ui.button("Prepare local timesheet").clicked(){action=3;}});
            });
            ui.separator();ui.label("Export active animation to a new bundle containing JSON, CSV and one PNG per source");
            ui.horizontal_wrapped(|ui|{for(v,label)in[(Naming::Cel,"Cel name"),(Naming::Serial,"Serial"),(Naming::FolderCel,"Folder + cel"),(Naming::FolderSerial,"Folder + serial")]{ui.selectable_value(&mut p.options.naming,v,label);}});
            ui.checkbox(&mut p.options.drafts,"Include drafts inside cels");ui.checkbox(&mut p.options.common,"Include outside layers as a separate common PNG");ui.checkbox(&mut p.options.overflow,"Include preserved overflow (otherwise full source canvas)");ui.checkbox(&mut p.options.unassigned,"Include unexposed slots and unassigned global cels");
            ui.horizontal(|ui|{if ui.button("Save local output preset…").clicked(){action=6;}
                if ui.button("Load local output preset…").clicked(){action=7;}});
            let r=ui.button("Capture cel exchange");p.capture=Some(r.rect);if r.clicked(){action=1;}
        });
        p.open = open;
        if !open {
            p.snapshot = None;
            p.prepared = None;
            if let Some(j) = &p.job {
                j.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.timesheet = p;
        let result = (|| {
            match action {
                1 => self.capture_timesheet()?,
                2 => self.output_timesheet(ctx)?,
                3 => self.import_timesheet(ctx)?,
                4 => self.accept_timesheet()?,
                5 => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Drawer timesheet v1", &["json", "csv"])
                        .pick_file()
                    {
                        self.timesheet.path = path.to_string_lossy().into_owned();
                    }
                }
                6 => self.timesheet_preset(false)?,
                7 => self.timesheet_preset(true)?,
                _ => {}
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.timesheet.notice = e;
        }
    }
}
fn start(
    ctx: &egui::Context,
    f: impl FnOnce(&AtomicBool, &AtomicU32) -> Result<Value, String> + Send + 'static,
) -> Job {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let progress = Arc::new(AtomicU32::new(0));
    let out = progress.clone();
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(f(&flag, &out));
        ctx.request_repaint();
    });
    Job {
        worker: Some(worker),
        cancel,
        progress,
        rx,
    }
}

pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::timesheet_verify::verify(root, &mut |a, b| {
        drawer_platform::local_files::publish_no_replace(a, b).map_err(|e| e.to_string())
    })?;
    let dir = root.join("timesheet-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.add_document(drawer_core::timesheet_verify::fixture());
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let revision = app.documents[0].revision;
        let dirty = app.documents[0].dirty;
        let current = app.documents[0].current_frame;
        app.execute_command(&ctx, "timesheet-exchange");
        if !app.timesheet.open {
            return Err("Timesheet command did not open".into());
        }
        app.timesheet.options.common = true;
        let bundle = dir.join("bundle");
        app.timesheet.folder = bundle.to_string_lossy().into_owned();
        let mut time = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, app, *t, events);
        };
        let click = |app: &mut Drawer, t: &mut f64, r: egui::Rect| {
            for pressed in [true, false] {
                frame(
                    app,
                    t,
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
        let wait = |app: &mut Drawer, t: &mut f64| -> Result<(), String> {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.timesheet.job.is_some() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > until {
                    return Err("Timesheet worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(())
        };
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let rect = app.timesheet.capture.ok_or("Capture control missing")?;
        click(&mut app, &mut time, rect);
        if app.timesheet.snapshot.is_none() || app.output_timesheet(&ctx).is_ok() {
            return Err("Capture pointer or output review gate failed".into());
        }
        app.timesheet.ack = true;
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let rect = app.timesheet.start.ok_or("Output control missing")?;
        click(&mut app, &mut time, rect);
        wait(&mut app, &mut time)?;
        if !bundle.join("timesheet.csv").is_file() {
            return Err(format!("Cel output pointer: {}", app.timesheet.notice));
        }
        if app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != history
            || app.documents[0].revision != revision
            || app.documents[0].dirty != dirty
            || app.documents[0].current_frame != current
        {
            return Err("Cel output changed live document".into());
        }
        app.timesheet.path = bundle.join("timesheet.csv").to_string_lossy().into_owned();
        app.import_timesheet(&ctx)?;
        wait(&mut app, &mut time)?;
        let prepared = app
            .timesheet
            .prepared
            .as_ref()
            .ok_or_else(|| format!("Import failed: {}", app.timesheet.notice))?;
        let png = bundle.join(&prepared.sheet.images[0].path);
        let bytes = std::fs::read(&png).map_err(|e| e.to_string())?;
        if app.accept_timesheet().is_ok() {
            return Err("Import bypassed review".into());
        }
        app.timesheet.ack = true;
        std::fs::write(&png, b"changed").map_err(|e| e.to_string())?;
        if app.accept_timesheet().is_ok() {
            return Err("Changed source accepted".into());
        }
        std::fs::write(&png, bytes).map_err(|e| e.to_string())?;
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let rect = app.timesheet.accept.ok_or("Import apply control missing")?;
        click(&mut app, &mut time, rect);
        if app.documents.len() != 2 || app.active != Some(1) {
            return Err(format!("Import apply pointer: {}", app.timesheet.notice));
        }
        if app.documents[0].snapshot() != before || app.documents[0].history_counts() != history {
            return Err("Import replaced existing drawing".into());
        }
        let imported = app.documents[1].snapshot();
        if !app.documents[1].undo()
            || app.documents[1].animation.is_some()
            || !app.documents[1].redo()
            || app.documents[1].snapshot() != imported
        {
            return Err("Imported timeline Undo/Redo".into());
        }
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
#[cfg(test)]
mod tests {
    #[test]
    fn real_pointer_capture_export_import_source_change_review_and_undo() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
