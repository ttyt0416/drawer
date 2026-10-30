use super::*;
use drawer_core::timelapse::{self as core, Clock, Store};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
struct Worker {
    handle: std::thread::JoinHandle<(Store, Result<bool, String>)>,
    cancel: Arc<AtomicBool>,
}
struct Session {
    store: Option<Store>,
    job: Option<Worker>,
    clock: Clock,
    epoch: Instant,
    last: Instant,
    doc: u64,
    label: String,
    revision: Option<u64>,
    notice: String,
}
impl Session {
    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }
    fn collect(&mut self, wait: bool) {
        if !self
            .job
            .as_ref()
            .is_some_and(|j| wait || j.handle.is_finished())
        {
            return;
        }
        let job = self.job.take().unwrap();
        match job.handle.join() {
            Ok((s, r)) => {
                self.store = Some(s);
                match r {
                    Ok(true) => self.notice = "Recorded a complete committed frame".into(),
                    Ok(false) => {
                        self.notice = "Visible pixels unchanged; elapsed time checkpointed".into()
                    }
                    Err(e) => {
                        self.clock.pause(self.now());
                        self.notice = format!("Recording paused: {e}");
                        self.revision = None;
                    }
                }
            }
            Err(_) => {
                self.clock.pause(self.now());
                self.notice =
                    "Recording worker failed; reopen the folder to recover committed frames".into();
            }
        }
    }
    fn checkpoint(&mut self) {
        self.collect(true);
        let ms = self.clock.at(self.now());
        if let Some(s) = &mut self.store
            && let Err(e) = s.checkpoint(ms)
        {
            self.notice = format!("Checkpoint failed; previous committed frames preserved: {e}");
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.clock.pause(self.now());
        self.checkpoint();
    }
}
pub(super) struct Panel {
    pub open: bool,
    path: String,
    limit_mib: u32,
    session: Option<Session>,
    first: usize,
    last: usize,
    latest: bool,
    fps_num: u32,
    fps_den: u32,
    export: Option<ExportJob>,
    pub(super) notice: String,
    pub toggle: Option<egui::Rect>,
    pub output: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            path: String::new(),
            limit_mib: 512,
            session: None,
            first: 0,
            last: 0,
            latest: true,
            fps_num: 24,
            fps_den: 1,
            export: None,
            notice: String::new(),
            toggle: None,
            output: None,
        }
    }
}
struct ExportJob {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<(Document, PathBuf), String>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for ExportJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(j) = self.worker.take() {
            let _ = j.join();
        }
    }
}
impl Drawer {
    fn timelapse_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        )
    }
    fn begin_timelapse(&mut self, resume: bool) -> Result<(), String> {
        self.finish_stroke();
        if self.timelapse.session.is_some() {
            return Err("Stop the existing recording before opening another folder".into());
        }
        let doc = self.documents[self.active.ok_or("Choose a drawing canvas")?].id;
        let path = PathBuf::from(self.timelapse.path.trim());
        let mut store = if resume {
            Store::open(&path, &AtomicBool::new(false))?
        } else {
            Store::create(&path, self.timelapse.limit_mib.max(1) as u64 * 1024 * 1024)?
        };
        if resume {
            self.timelapse.limit_mib = (store.data.limit / (1024 * 1024)) as u32;
        }
        let budget = self.timelapse_budget();
        let snapshot = core::capture(&self.documents[self.active.unwrap()], budget)?;
        store.append(
            &snapshot,
            store.data.active_ms,
            &AtomicBool::new(false),
            &mut |a, b| {
                drawer_platform::local_files::publish_no_replace(a, b).map_err(|e| e.to_string())
            },
        )?;
        let base = store.data.active_ms;
        self.timelapse.session=Some(Session{store:Some(store),job:None,clock:Clock::new(base,0.),epoch:Instant::now(),last:Instant::now(),doc,label:self.documents[self.active.unwrap()].title.clone(),revision:Some(self.documents[self.active.unwrap()].revision),notice:"Recording changed committed states, at most twice per second; pause/stop also flush the latest state".into()});
        self.timelapse.latest = true;
        self.timelapse.fps_num = 24;
        self.timelapse.fps_den = 1;
        Ok(())
    }
    fn pause_timelapse(&mut self) -> Result<(), String> {
        let mut s = self.timelapse.session.take().ok_or("No recording open")?;
        if s.clock.paused() {
            s.clock.resume(s.now());
            s.revision = None;
            s.notice = "Recording resumed on the bound canvas; paused time excluded".into();
        } else {
            s.clock.pause(s.now());
            s.collect(true);
            if let Some(d) = self.documents.iter().find(|d| d.id == s.doc)
                && s.revision != Some(d.revision)
            {
                match core::capture(d, self.timelapse_budget()) {
                    Ok(capture) => {
                        if let Some(store) = &mut s.store {
                            let result = store.append(
                                &capture,
                                s.clock.at(s.epoch.elapsed().as_secs_f64()),
                                &AtomicBool::new(false),
                                &mut |a, b| {
                                    drawer_platform::local_files::publish_no_replace(a, b)
                                        .map_err(|e| e.to_string())
                                },
                            );
                            if let Err(e) = result {
                                s.notice = e;
                            } else {
                                s.revision = Some(d.revision);
                            }
                        }
                    }
                    Err(e) => s.notice = format!("Latest state was not captured: {e}"),
                }
            }
            s.checkpoint();
        }
        self.timelapse.session = Some(s);
        Ok(())
    }
    pub(super) fn stop_timelapse(&mut self) {
        if self
            .timelapse
            .session
            .as_ref()
            .is_some_and(|s| !s.clock.paused())
        {
            let _ = self.pause_timelapse();
        }
        if let Some(mut s) = self.timelapse.session.take() {
            s.clock.pause(s.now());
            s.checkpoint();
            self.timelapse.notice = s.notice.clone();
        }
    }
    pub(super) fn timelapse_tick(&mut self, ctx: &egui::Context) {
        let Some(mut s) = self.timelapse.session.take() else {
            return;
        };
        s.collect(false);
        if !s.clock.paused() && s.job.is_none() && s.last.elapsed() >= Duration::from_millis(500) {
            if let Some(d) = self.documents.iter().find(|d| d.id == s.doc) {
                if s.revision != Some(d.revision) {
                    match core::capture(d, self.timelapse_budget()) {
                        Ok(capture) => {
                            if let Some(mut store) = s.store.take() {
                                let cancel = Arc::new(AtomicBool::new(false));
                                let flag = cancel.clone();
                                let elapsed = s.clock.at(s.now());
                                let worker = std::thread::spawn(move || {
                                    let r = store.append(&capture, elapsed, &flag, &mut |a, b| {
                                        drawer_platform::local_files::publish_no_replace(a, b)
                                            .map_err(|e| e.to_string())
                                    });
                                    (store, r)
                                });
                                s.job = Some(Worker {
                                    handle: worker,
                                    cancel,
                                });
                                s.revision = Some(d.revision);
                                s.last = Instant::now();
                            }
                        }
                        Err(e) => {
                            s.clock.pause(s.now());
                            s.notice = e;
                        }
                    }
                }
            } else {
                s.clock.pause(s.now());
                s.checkpoint();
                s.notice =
                    "Bound canvas closed; recording paused and committed frames preserved".into();
            }
        }
        if !s.clock.paused() || s.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        self.timelapse.session = Some(s);
    }
    fn prepare_timelapse_output(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.timelapse.export.is_some() {
            return Err("Wait for the current output capture".into());
        }
        let s = self
            .timelapse
            .session
            .as_mut()
            .ok_or("Open a recording folder first")?;
        s.collect(true);
        let store = s.store.as_ref().ok_or("Reopen the recording")?;
        let root = store.root.clone();
        let first = self.timelapse.first;
        let last = if self.timelapse.latest {
            store.data.frames.len().saturating_sub(1)
        } else {
            self.timelapse.last
        };
        let fps = drawer_core::animation::Fps {
            numerator: self.timelapse.fps_num,
            denominator: self.timelapse.fps_den,
        };
        let budget = self.timelapse_budget();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let r = core::animation(&root, first, last, fps, budget, &flag).map(|d| (d, root));
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.timelapse.export = Some(ExportJob {
            cancel,
            rx,
            worker: Some(worker),
        });
        Ok(())
    }
    pub(super) fn timelapse_window(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.timelapse.export {
            let r = match j.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Timelapse output worker stopped".into()))
                }
            };
            if let Some(r) = r {
                let j = self.timelapse.export.take().unwrap();
                let canceled = j.cancel.load(Ordering::Relaxed);
                drop(j);
                if !canceled {
                    match r {
                        Ok((d, root)) => {
                            if let Err(e) = self.begin_recorded_animation_export(d, root) {
                                self.timelapse.notice = e;
                            }
                        }
                        Err(e) => self.timelapse.notice = e,
                    }
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(30));
            }
        }
        if !self.timelapse.open {
            return;
        }
        let mut p = std::mem::take(&mut self.timelapse);
        let mut open = true;
        let mut action = 0;
        egui::Window::new("Local timelapse recording").default_pos(egui::pos2(25.,20.)).default_width(660.).max_height(600.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.label(&p.notice);ui.label("Independent local8-bit sRGB committed snapshots; no live stroke samples or audio. Original drawing precision, saving, dirty state and Undo remain independent. Closing this panel keeps recording; Stop or application exit flushes and closes it.");
            if let Some(s)=&mut p.session{
                ui.label(format!("Recording canvas {} · {} · recording elapsed {:.2}s",s.label,if s.clock.paused(){"paused"}else{"recording"},s.clock.at(s.now()) as f64/1000.));ui.label(&s.notice);
                if let Some(store)=&s.store{ui.label(format!("{} frames · {:.2}MiB PNG · limit {:.0}MiB · {}",store.data.frames.len(),store.data.bytes() as f64/(1024.*1024.),store.data.limit as f64/(1024.*1024.),store.root.display()));}
                let r=ui.button(if s.clock.paused(){"Resume bound canvas recording"}else{"Pause and flush latest committed state"});p.toggle=Some(r.rect);if r.clicked(){action=3;}
                if ui.button("Stop and keep recording files").clicked(){action=4;}
                ui.horizontal(|ui|{ui.label("Storage limit MiB");ui.add(egui::DragValue::new(&mut p.limit_mib).range(1..=2048));if ui.button("Apply storage limit").clicked(){action=5;}});
                if let Some(j)=&s.job && ui.button("Cancel current capture and pause").clicked(){j.cancel.store(true,Ordering::Relaxed);s.clock.pause(s.now());}
                let r=ui.add_enabled(p.export.is_none(),egui::Button::new("Prepare frozen timelapse animation output"));p.output=Some(r.rect);if r.clicked(){action=6;}
                ui.separator();ui.label("Output fixed-rate frames through the animation output window. Canvas size changes are centered, without resampling, on the largest selected capture. Later recording frames are excluded from the prepared cutoff.");
                ui.horizontal(|ui|{let mut first=p.first+1;let mut last=p.last+1;ui.label("First / last recorded frame");ui.add(egui::DragValue::new(&mut first).range(1..=10000));ui.add(egui::DragValue::new(&mut last).range(1..=10000));p.first=first-1;p.last=last-1;});ui.checkbox(&mut p.latest,"Through latest committed frame at capture");
                ui.horizontal(|ui|{ui.label("Output FPS numerator / denominator");ui.add(egui::DragValue::new(&mut p.fps_num).range(1..=100000));ui.add(egui::DragValue::new(&mut p.fps_den).range(1..=100000));});

            }else{ui.label("New recording folder, or an existing folder containing timelapse.json");ui.text_edit_singleline(&mut p.path);ui.horizontal(|ui|{if ui.button("Choose existing recording folder…").clicked(){action=7;}ui.label("Limit MiB");ui.add(egui::DragValue::new(&mut p.limit_mib).range(1..=2048));});
                ui.horizontal(|ui|{let r=ui.button("Start new recording on active canvas");p.toggle=Some(r.rect);if r.clicked(){action=1;}
                    if ui.button("Resume folder on active canvas").clicked(){action=2;}});
            }
            if let Some(j)=&p.export{ui.label("Freezing committed PNG frames for output…");if ui.button("Cancel output capture").clicked(){j.cancel.store(true,Ordering::Relaxed);}}
        });
        p.open = open;
        self.timelapse = p;
        let r = (|| {
            match action {
                1 => self.begin_timelapse(false)?,
                2 => self.begin_timelapse(true)?,
                3 => self.pause_timelapse()?,
                4 => self.stop_timelapse(),
                5 => {
                    let s = self.timelapse.session.as_mut().ok_or("No recording open")?;
                    s.collect(true);
                    s.store
                        .as_mut()
                        .ok_or("Reopen recording")?
                        .set_limit(self.timelapse.limit_mib.max(1) as u64 * 1024 * 1024)?;
                }
                6 => self.prepare_timelapse_output(ctx)?,
                7 => {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.timelapse.path = p.to_string_lossy().into_owned();
                    }
                }
                _ => {}
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = r {
            self.timelapse.notice = e;
        }
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    core::verify(root, &mut |a, b| {
        drawer_platform::local_files::publish_no_replace(a, b).map_err(|e| e.to_string())
    })?;
    let dir = root.join("timelapse-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 16,
            height: 12,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let before = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        app.execute_command(&ctx, "timelapse");
        let record = dir.join("record");
        app.timelapse.path = record.to_string_lossy().into_owned();
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
        let drain = |app: &mut Drawer, t: &mut f64| -> Result<(), String> {
            let end = Instant::now() + Duration::from_secs(30);
            while app
                .timelapse
                .session
                .as_ref()
                .is_some_and(|s| s.job.is_some())
                || app.timelapse.export.is_some()
            {
                frame(app, t, vec![]);
                if Instant::now() > end {
                    return Err("Timelapse worker timeout".into());
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(())
        };
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .timelapse
            .toggle
            .ok_or("Recording start control missing")?;
        click(&mut app, &mut time, r);
        if app.timelapse.session.is_none() {
            return Err(format!("Start pointer: {}", app.timelapse.notice));
        }
        if app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != counts
            || app.documents[0].dirty != dirty
        {
            return Err("Starting recording changed drawing".into());
        }
        app.documents[0].edit_pixel(0, 1, 1, [255, 0, 0, 128]);
        app.timelapse.session.as_mut().unwrap().last = Instant::now() - Duration::from_secs(1);
        app.timelapse_tick(&ctx);
        drain(&mut app, &mut time)?;
        if app
            .timelapse
            .session
            .as_ref()
            .unwrap()
            .store
            .as_ref()
            .unwrap()
            .data
            .frames
            .len()
            != 2
        {
            return Err("Committed drawing change was not recorded".into());
        }
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app.timelapse.toggle.unwrap();
        click(&mut app, &mut time, r);
        if !app.timelapse.session.as_ref().unwrap().clock.paused() {
            return Err("Pause pointer failed".into());
        }
        let paused = app.timelapse.session.as_ref().unwrap().clock.at(app
            .timelapse
            .session
            .as_ref()
            .unwrap()
            .now());
        app.documents[0].edit_pixel(0, 2, 1, [0, 255, 0, 255]);
        std::thread::sleep(Duration::from_millis(20));
        frame(&mut app, &mut time, vec![]);
        if app.timelapse.session.as_ref().unwrap().clock.at(app
            .timelapse
            .session
            .as_ref()
            .unwrap()
            .now())
            != paused
        {
            return Err("Paused time was counted".into());
        }
        let r = app.timelapse.toggle.unwrap();
        click(&mut app, &mut time, r);
        app.timelapse.session.as_mut().unwrap().last = Instant::now() - Duration::from_secs(1);
        app.timelapse_tick(&ctx);
        drain(&mut app, &mut time)?;
        if app
            .timelapse
            .session
            .as_ref()
            .unwrap()
            .store
            .as_ref()
            .unwrap()
            .data
            .frames
            .len()
            != 3
        {
            return Err("Resume did not record current committed state".into());
        }
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .timelapse
            .output
            .ok_or("Recorded output control missing")?;
        click(&mut app, &mut time, r);
        drain(&mut app, &mut time)?;
        app.documents[0].edit_pixel(0, 3, 1, [0, 0, 255, 255]);
        app.timelapse.session.as_mut().unwrap().last = Instant::now() - Duration::from_secs(1);
        app.timelapse_tick(&ctx);
        drain(&mut app, &mut time)?;
        let state = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        let current = app.documents[0].current_frame;
        app.timelapse.open = false;
        if animation_export::smoke_recorded_source(&mut app, &ctx, &dir)? != 3 {
            return Err("Output followed later recording frames".into());
        }
        if app.documents[0].snapshot() != state
            || app.documents[0].history_counts() != history
            || app.documents[0].dirty != dirty
            || app.documents[0].current_frame != current
        {
            return Err("Timelapse output changed live drawing/view/history".into());
        }
        app.documents[0].edit_pixel(0, 4, 1, [255, 0, 255, 255]);
        let final_state = app.documents[0].snapshot();
        eframe::App::on_exit(&mut app, None);
        if app.timelapse.session.is_some()
            || core::Store::open(&record, &AtomicBool::new(false))?
                .data
                .frames
                .len()
                != 5
            || app.documents[0].snapshot() != final_state
        {
            return Err("Application exit did not flush and preserve recording / drawing".into());
        }
        app.timelapse.path = record.to_string_lossy().into_owned();
        app.begin_timelapse(true)?;
        app.stop_timelapse();
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_record_pause_resume_frozen_output_shutdown_and_restart() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
