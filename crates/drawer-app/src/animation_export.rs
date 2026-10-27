use super::*;
use drawer_core::animation_export::{self as core, Format, Options, Snapshot};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
};
pub(super) struct Panel {
    open: bool,
    path: String,
    options: Options,
    overwrite: bool,
    ack: bool,
    snapshot: Option<Snapshot>,
    source: String,
    job: Option<Job>,
    notice: String,
    pub prepare: Option<egui::Rect>,
    pub start: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            path: String::new(),
            options: Options {
                first: 0,
                last: 0,
                format: Format::PngSequence,
                plays: 0,
            },
            overwrite: false,
            ack: false,
            snapshot: None,
            source: String::new(),
            job: None,
            notice: String::new(),
            prepare: None,
            start: None,
        }
    }
}
struct Job {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    total: u32,
    receiver: mpsc::Receiver<Result<core::Report, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drawer {
    pub(super) fn begin_animation_export(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending drawing edits before capturing output".into());
        }
        let d = &self.documents[self.active.ok_or("Choose an animation")?];
        let a = d.animation.as_ref().ok_or("Choose an animation document")?;
        if self.animation_export.job.is_some() {
            self.animation_export.open = true;
            return Ok(());
        }
        self.animation_export.options.first = 0;
        self.animation_export.options.last = a.frames - 1;
        self.animation_export.snapshot = None;
        self.animation_export.ack = false;
        self.animation_export.open = true;
        Ok(())
    }
    fn prepare_animation_export(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending drawing edits first".into());
        }
        if self.animation_export.job.is_some() {
            return Err("Wait for or cancel the current output".into());
        }
        let d = &self.documents[self.active.ok_or("Choose an animation")?];
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        self.animation_export.snapshot =
            Some(core::prepare(d, self.animation_export.options, budget)?);
        self.animation_export.source = d.title.clone();
        self.animation_export.ack = false;
        self.animation_export.notice =
            "Snapshot captured. Later edits and playback changes will not affect this output."
                .into();
        Ok(())
    }
    fn start_animation_export(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.animation_export.job.is_some() || !self.animation_export.ack {
            return Err("Review and confirm RGBA8 output before starting".into());
        }
        if self.animation_export.path.trim().is_empty() {
            return Err("Choose a local output filename or new sequence folder".into());
        }
        let snapshot = self
            .animation_export
            .snapshot
            .take()
            .ok_or("Capture an output snapshot first")?;
        let total = snapshot.count();
        let destination = PathBuf::from(&self.animation_export.path);
        let overwrite = self.animation_export.overwrite;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let progress = Arc::new(AtomicU32::new(0));
        let progress_out = progress.clone();
        let (tx, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = core::write(&snapshot, &destination, overwrite, &flag, &progress_out);
            let _ = tx.send(result);
            repaint.request_repaint();
        });
        self.animation_export.job = Some(Job {
            cancel,
            progress,
            total,
            receiver,
        });
        Ok(())
    }
    pub(super) fn animation_export_window(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.animation_export.job {
            let result = match job.receiver.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Animation output worker stopped".into()))
                }
            };
            if let Some(result) = result {
                self.animation_export.job = None;
                self.animation_export.notice = match result {
                    Ok(r) => format!(
                        "Completed {} frames. Published {} files (showing up to4): {}",
                        r.frames,
                        r.files.len(),
                        r.files
                            .iter()
                            .take(4)
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                    Err(e) => format!("Output stopped: {e}. No partial batch was published."),
                };
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
            }
        }
        if !self.animation_export.open {
            return;
        }
        let mut p = std::mem::take(&mut self.animation_export);
        let mut open = true;
        let mut action = 0;
        egui::Window::new("Animation file output").default_pos(egui::pos2(15.,15.)).default_width(650.).open(&mut open).vscroll(true).show(ctx,|ui|{
            ui.label(&p.notice);
            ui.label("PNG sequence keeps each frame and a rational-FPS manifest. APNG keeps full RGBA alpha, ICC and exact supported rational timing. These image formats carry no audio. Both use8-bit RGBA; higher source precision is quantized only in the output.");
            ui.add_enabled_ui(p.job.is_none(),|ui|{
                let before=p.options;
                ui.horizontal(|ui|{ui.selectable_value(&mut p.options.format,Format::PngSequence,"PNG sequence folder");ui.selectable_value(&mut p.options.format,Format::Apng,"Animated PNG (APNG)");});
                ui.horizontal(|ui|{let mut first=p.options.first+1;let mut last=p.options.last+1;ui.label("Inclusive first / last frame");ui.add(egui::DragValue::new(&mut first).range(1..=10000));ui.add(egui::DragValue::new(&mut last).range(1..=10000));p.options.first=first-1;p.options.last=last-1;});
                if p.options.format==Format::Apng{ui.horizontal(|ui|{ui.label("Complete plays (0 = infinite)");ui.add(egui::DragValue::new(&mut p.options.plays).range(0..=100000));});}
                if before!=p.options{p.snapshot=None;p.ack=false;}
                ui.label(if p.options.format==Format::Apng{"Local .apng / .png filename"}else{"New local sequence folder (existing folders are preserved)"});
                ui.text_edit_singleline(&mut p.path);
                if p.options.format==Format::Apng{ui.checkbox(&mut p.overwrite,"Allow replacement of an existing APNG file");}
                let r=ui.button("Capture frame range for output");p.prepare=Some(r.rect);if r.clicked(){action=1;}
                if let Some(s)=&p.snapshot {
                    let d=s.document();let fps=d.animation.as_ref().unwrap().fps;
                    ui.label(format!("Captured {} · revision{} · {}×{} · {} frames ({}–{}) · {}/{} FPS",p.source,s.revision,d.width,d.height,s.count(),s.options.first+1,s.options.last+1,fps.numerator,fps.denominator));
                    ui.checkbox(&mut p.ack,"I reviewed the range, 8-bit RGBA precision and destination policy");
                    let r=ui.add_enabled(p.ack,egui::Button::new("Export captured animation"));p.start=Some(r.rect);if r.clicked(){action=2;}
                    if ui.button("Discard captured snapshot").clicked(){p.snapshot=None;p.ack=false;}
                }
            });
            if let Some(job)=&p.job {
                ui.label(format!("Rendered {} / {} frames. Private files are published only when the complete batch is ready.",job.progress.load(Ordering::Relaxed),job.total));
                if ui.button("Cancel output").clicked(){job.cancel.store(true,Ordering::Relaxed);}
            }
        });
        p.open = open;
        if !open {
            p.snapshot = None;
            if let Some(job) = &p.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.animation_export = p;
        let result = match action {
            1 => self.prepare_animation_export(),
            2 => self.start_animation_export(ctx),
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.animation_export.notice = e;
        }
    }
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::animation_export_verify::verify(root)?;
    let dir = root.join("animation-export-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.add_document(drawer_core::animation_export_verify::fixture());
        app.execute_command(&ctx, "animation-export");
        if !app.animation_export.open {
            return Err("Animation output command did not open".into());
        }
        app.animation_export.options = Options {
            first: 1,
            last: 2,
            format: Format::Apng,
            plays: 1,
        };
        let path = dir.join("검증.apng");
        app.animation_export.path = path.to_string_lossy().into_owned();
        let mut time = 0.;
        let frame = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.05;
            input_replay::frame(&ctx, app, *time, events);
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
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .animation_export
            .prepare
            .ok_or("Capture control missing")?;
        click(&mut app, &mut time, r);
        if app.animation_export.snapshot.is_none() || app.start_animation_export(&ctx).is_ok() {
            return Err("Capture pointer failed or output bypassed review".into());
        }
        app.documents[0].edit_pixel(0, 1, 0, [255, 200, 0, 255]);
        app.documents[0].current_frame = 0;
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let revision = app.documents[0].revision;
        let dirty = app.documents[0].dirty;
        app.animation_export.ack = true;
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app.animation_export.start.ok_or("Export control missing")?;
        click(&mut app, &mut time, r);
        let wait = |app: &mut Drawer, time: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.animation_export.job.is_some() {
                frame(app, time, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Animation output timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(())
        };
        wait(&mut app, &mut time)?;
        if !path.is_file() {
            return Err(format!(
                "Pointer output failed: {}",
                app.animation_export.notice
            ));
        }
        let mut reader = png_decoder(&path)?;
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut bytes).map_err(|e| e.to_string())?;
        if reader.info().animation_control.unwrap().num_frames != 2 || bytes[7] != 64 {
            return Err(
                "Output followed the changed live drawing or lost the inclusive range".into(),
            );
        }
        drop(reader);
        if app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != history
            || app.documents[0].revision != revision
            || app.documents[0].current_frame != 0
            || app.documents[0].dirty != dirty
        {
            return Err("Animation output changed the live document/view/history".into());
        }
        app.documents[0].animation.as_mut().unwrap().frames = 10000;
        app.animation_export.options.last = 9999;
        app.animation_export.path = dir.join("cancel.apng").to_string_lossy().into_owned();
        app.prepare_animation_export()?;
        app.animation_export.ack = true;
        app.start_animation_export(&ctx)?;
        app.animation_export
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        wait(&mut app, &mut time)?;
        if dir.join("cancel.apng").exists() || std::fs::read_dir(&dir).unwrap().count() != 1 {
            return Err("Cancelled output left published/private files".into());
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
fn png_decoder(path: &Path) -> Result<drawer_core::animation_export_verify::PngReader, String> {
    drawer_core::animation_export_verify::reader(path)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_range_capture_review_export_live_edits_and_cancel_are_isolated() {
        let root =
            std::env::temp_dir().join(format!("drawer-animation-output-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::smoke(&root);
        std::fs::remove_dir(&root).unwrap();
        r.unwrap();
    }
}
