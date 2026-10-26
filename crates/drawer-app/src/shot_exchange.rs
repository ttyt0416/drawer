use super::*;
use drawer_core::shot_exchange as core;
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
    job: Option<Job>,
    prepared: Option<core::Prepared>,
    notice: String,
    rows: Vec<(u64, String, String)>,
    stamp: Option<(u64, u64)>,
    pub apply: Option<egui::Rect>,
}
struct Job {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Value, String>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
enum Value {
    Prepared(Box<core::Prepared>),
    Output(String),
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
    work: impl FnOnce(&AtomicBool) -> Result<Value, String> + Send + 'static,
) -> Job {
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    let worker = std::thread::spawn(move || {
        let result = work(&signal);
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    Job {
        cancel,
        rx,
        worker: Some(worker),
    }
}
impl Drawer {
    pub(super) fn begin_shot_exchange(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending edits first".into());
        }
        self.shot_exchange.open = true;
        Ok(())
    }
    fn shot_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        )
    }
    fn prepare_shots(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let path = PathBuf::from(self.shot_exchange.path.trim());
        let budget = self.shot_budget();
        self.shot_exchange.prepared = None;
        self.shot_exchange.job = Some(job(ctx, move |cancel| {
            core::prepare(&path, budget, cancel).map(|p| Value::Prepared(Box::new(p)))
        }));
        Ok(())
    }
    fn accept_shots(&mut self) -> Result<(), String> {
        let p = self
            .shot_exchange
            .prepared
            .as_ref()
            .ok_or("Prepare the shot file first")?;
        p.fresh()?;
        if self.tabs.len() >= 256 || p.document.resident_bytes() > self.shot_budget() {
            return Err("Close views/documents to fit the prepared shot batch".into());
        }
        let p = self.shot_exchange.prepared.take().unwrap();
        self.protected_sources
            .extend(p.sources.into_iter().map(|s| s.path));
        self.add_document(p.document);
        self.shot_exchange.rows.clear();
        self.shot_exchange.stamp = None;
        self.shot_exchange.notice="Added an independent native shot document. One drawing Undo removes its imported timeline/metadata; Redo restores the batch. Save it as .drawer. Source files remain.".into();
        Ok(())
    }
    fn shot_rows(&mut self) -> Result<(), String> {
        let d = self
            .active
            .map(|n| &self.documents[n])
            .ok_or("Choose a document")?;
        self.shot_exchange.rows = d
            .scene
            .shots
            .entries
            .iter()
            .map(|b| (b.shot.id, b.shot.name.clone(), b.shot.dialogue.clone()))
            .collect();
        self.shot_exchange.stamp = Some((d.id, d.revision));
        Ok(())
    }
    fn apply_shot_rows(&mut self) -> Result<(), String> {
        let d = self
            .active
            .map(|n| &mut self.documents[n])
            .ok_or("Choose a document")?;
        if self.shot_exchange.stamp != Some((d.id, d.revision)) {
            return Err("Shot document changed; reload properties before applying".into());
        }
        d.commit_state(core::properties(d, &self.shot_exchange.rows)?)?;
        self.shot_exchange.stamp = Some((d.id, d.revision));
        Ok(())
    }
    fn output_shots(&mut self, ctx: &egui::Context, path: PathBuf) -> Result<(), String> {
        let d = self
            .active
            .map(|n| &self.documents[n])
            .ok_or("Choose a shot document")?;
        let budget = self.shot_budget();
        if d.resident_bytes() > budget {
            return Err("Shot output snapshot exceeds available memory".into());
        }
        let d = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        self.shot_exchange.job = Some(job(ctx, move |cancel| {
            core::export(&d, &path, budget, cancel, &mut |a, b| {
                drawer_platform::local_files::publish_no_replace(a, b).map_err(|e| e.to_string())
            })?;
            Ok(Value::Output(format!(
                "Saved local shot/cell exchange: {}",
                path.display()
            )))
        }));
        Ok(())
    }
    pub(super) fn shot_exchange_window(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.shot_exchange.job {
            let result = match j.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("Shot worker stopped".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(r) = result {
                let j = self.shot_exchange.job.take().unwrap();
                let cancelled = j.cancel.load(Ordering::Relaxed);
                drop(j);
                if cancelled {
                    self.shot_exchange.notice =
                        "Cancelled; source files and drawings unchanged".into();
                } else {
                    match r {
                        Ok(Value::Prepared(p)) => {
                            self.shot_exchange.prepared = Some(*p);
                            self.shot_exchange.notice="All local cells prepared. Review the batch before adding its independent document.".into();
                        }
                        Ok(Value::Output(s)) => self.shot_exchange.notice = s,
                        Err(e) => self.shot_exchange.notice = e,
                    }
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(25));
            }
        }
        if !self.shot_exchange.open {
            return;
        }
        let mut p = std::mem::take(&mut self.shot_exchange);
        let mut open = true;
        let mut action = 0u8;
        egui::Window::new("Local shot and dialogue exchange").default_width(640.).open(&mut open).vscroll(true).show(ctx,|ui|{
            ui.label("Drawer shots v1 JSON / CSV only. No verified proprietary animation adapter. Each independent imported document has one exact rational FPS; ranges use a zero-based inclusive start and exclusive end.");ui.label(&p.notice);
            if let Some(j)=&p.job{ui.spinner();if ui.button("Cancel shot operation").clicked(){j.cancel.store(true,Ordering::Relaxed);}return;}
            let old=p.path.clone();ui.text_edit_singleline(&mut p.path);if p.path!=old{p.prepared=None;}
            ui.horizontal(|ui|{if ui.button("Choose shot file…").clicked(){action=1;}
                if ui.button("Prepare local shots").clicked(){action=2;}});
            if let Some(prepared)=&p.prepared{ui.label(format!("{}×{} · {} shots",prepared.document.width,prepared.document.height,prepared.document.scene.shots.entries.len()));for b in &prepared.document.scene.shots.entries{ui.label(format!("ID{} · {} · [{}..{}) · {}/{} fps · {} cell references",b.shot.id,b.shot.name,b.shot.start,b.shot.end,b.shot.fps.numerator,b.shot.fps.denominator,b.cells.len()));ui.label(&b.shot.dialogue);}
                let r=ui.button("Add independent shot document");p.apply=Some(r.rect);if r.clicked(){action=3;}
            }
            ui.separator();ui.label("Active native animation / shot document");
            ui.horizontal_wrapped(|ui|{if ui.button("Create / refresh metadata from animation tracks").clicked(){action=4;}
                if ui.button("Reload shot properties").clicked(){action=5;}
                if ui.button("Export own JSON…").clicked(){action=6;}
                if ui.button("Export own CSV…").clicked(){action=7;}});
            for (id,name,dialogue) in &mut p.rows{ui.label(format!("Shot ID{id}"));ui.add(egui::TextEdit::singleline(name).char_limit(256));ui.add(egui::TextEdit::multiline(dialogue).desired_rows(2).char_limit(32768));}
            if !p.rows.is_empty() && ui.button("Apply shot names and dialogue as one edit").clicked(){action=8;}
        });
        p.open = open;
        if !open {
            if let Some(j) = &p.job {
                j.cancel.store(true, Ordering::Relaxed);
            }
            p.prepared = None;
            p.rows.clear();
            p.stamp = None;
        }
        self.shot_exchange = p;
        let result = (|| {
            match action {
                1 => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Drawer shots v1", &["drawer-shots", "csv"])
                        .pick_file()
                    {
                        self.shot_exchange.path = path.to_string_lossy().into_owned();
                        self.shot_exchange.prepared = None;
                    }
                }
                2 => self.prepare_shots(ctx)?,
                3 => self.accept_shots()?,
                4 => {
                    let d = self
                        .active
                        .map(|n| &mut self.documents[n])
                        .ok_or("Choose an animation document")?;
                    d.commit_state(core::from_tracks(d)?)?;
                    self.shot_rows()?;
                }
                5 => self.shot_rows()?,
                6 | 7 => {
                    let ext = if action == 6 { "drawer-shots" } else { "csv" };
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Drawer shots v1", &[ext])
                        .set_file_name(format!("shots.{ext}"))
                        .save_file()
                    {
                        self.output_shots(ctx, path)?;
                    }
                }
                8 => self.apply_shot_rows()?,
                _ => {}
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.shot_exchange.notice = e;
        }
    }
}

pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::publication_verify::exchange(root)?;
    let dir = root.join("shot-exchange-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let (path, _) = drawer_core::publication_verify::fixture(&dir)?;
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        let d = Document::create(
            CanvasSpec {
                width: 8,
                height: 8,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())?;
        let snapshot = d.snapshot();
        app.add_document(d);
        app.execute_command(&ctx, "shot-exchange");
        app.shot_exchange.path = path.to_string_lossy().into_owned();
        let mut time = 0.;
        let frame = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            input_replay::frame(&ctx, app, *time, events);
        };
        let wait = |app: &mut Drawer, time: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while app.shot_exchange.job.is_some() {
                frame(app, time, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Shot worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(())
        };
        app.prepare_shots(&ctx)?;
        wait(&mut app, &mut time)?;
        if app.shot_exchange.prepared.is_none() {
            return Err(format!(
                "Shot preparation failed: {}",
                app.shot_exchange.notice
            ));
        }
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .shot_exchange
            .apply
            .ok_or("Shot confirmation button absent")?;
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
        if app.documents.len() != 2 || app.documents[0].snapshot() != snapshot {
            return Err(format!(
                "Shot pointer confirmation changed originals/failed: {}",
                app.shot_exchange.notice
            ));
        }
        let n = app.active.unwrap();
        let imported = app.documents[n].snapshot();
        if app.documents[n].scene.shots.entries.len() != 2
            || !app.documents[n].undo()
            || app.documents[n].animation.is_some()
            || !app.documents[n].redo()
            || app.documents[n].snapshot() != imported
        {
            return Err("Shot batch document Undo/Redo failed".into());
        }
        app.shot_rows()?;
        app.shot_exchange.rows[0].2 = "포인터 가져오기 이후 대사\n,\"인용\"".into();
        app.apply_shot_rows()?;
        app.documents[n].edit_pixel(0, 0, 0, [40, 30, 20, 255]);
        if app.apply_shot_rows().is_ok() {
            return Err("Stale shot properties accepted".into());
        }
        app.documents[n].undo();
        let out = dir.join("export.csv");
        let before = app.documents[n].snapshot();
        let history = app.documents[n].history_counts();
        app.output_shots(&ctx, out.clone())?;
        wait(&mut app, &mut time)?;
        let prepared = core::prepare(&out, DEFAULT_DOCUMENT_BUDGET, &AtomicBool::new(false))?;
        if prepared.document.scene.shots.entries[0].shot.dialogue
            != "포인터 가져오기 이후 대사\n,\"인용\""
            || app.documents[n].snapshot() != before
            || app.documents[n].history_counts() != history
        {
            return Err("Shot export lost dialogue or changed source/history".into());
        }
        let native = dir.join("saved.drawer");
        drawer_core::files::save(&mut app.documents[n], &native).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != before
        {
            return Err("Saved UI shot document failed native reopen".into());
        }
        app.prepare_shots(&ctx)?;
        wait(&mut app, &mut time)?;
        std::fs::write(&path, b"changed after preparation").unwrap();
        if app.accept_shots().is_ok() || app.documents.len() != 2 {
            return Err("Stale shot input added a document".into());
        }
        std::fs::write(&path, &original).unwrap();
        app.shot_exchange.prepared = None;
        app.prepare_shots(&ctx)?;
        app.shot_exchange
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        wait(&mut app, &mut time)?;
        if app.shot_exchange.prepared.is_some()
            || app.documents.len() != 2
            || std::fs::read(&path).unwrap() != original
        {
            return Err("Cancelled shot batch changed source/documents".into());
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_shot_import_properties_native_undo_and_cancel() {
        let root = std::env::temp_dir().join(format!("drawer-shot-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::smoke(&root);
        std::fs::remove_dir(root).unwrap();
        r.unwrap();
    }
}
