use super::*;
use drawer_core::{
    timeline_playback::{self as playback, Cache, Preferences, Stamp},
    timelines::{self as core, Edit},
};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc,
};
struct Prepared {
    id: u64,
    revision: u64,
    value: core::Prepared,
}
enum Done {
    Edit(Box<Prepared>),
    Cache(Cache),
}
struct Job {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    rx: mpsc::Receiver<Result<Done, String>>,
    worker: Option<std::thread::JoinHandle<()>>,
    stamp: Option<Stamp>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
pub(super) struct Panel {
    pub open: bool,
    pub preferences: Preferences,
    pub cache: Option<Cache>,
    draft: Preferences,
    custom: bool,
    fps: [u32; 2],
    name: String,
    selected: Option<u64>,
    frames: u32,
    rate: [u32; 2],
    job: Option<Job>,
    staged: Option<Box<Prepared>>,
    confirm: bool,
    notice: String,
    pub copy_shared: Option<egui::Rect>,
    pub copy_independent: Option<egui::Rect>,
    pub apply: Option<egui::Rect>,
    pub switch: Option<egui::Rect>,
    pub precompute: Option<egui::Rect>,
    pub cancel: Option<egui::Rect>,
    pub options: Option<egui::Rect>,
    pub review: Option<egui::Rect>,
    pub drop_frame: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        let p = Preferences::default();
        Self {
            open: false,
            preferences: p,
            cache: None,
            draft: p,
            custom: false,
            fps: [24, 1],
            name: "Timeline copy".into(),
            selected: None,
            frames: 24,
            rate: [24, 1],
            job: None,
            staged: None,
            confirm: false,
            notice: String::new(),
            copy_shared: None,
            copy_independent: None,
            apply: None,
            switch: None,
            precompute: None,
            cancel: None,
            options: None,
            review: None,
            drop_frame: None,
        }
    }
}
impl Drawer {
    pub(super) fn load_timeline_playback(&mut self, storage: &UserStorage) {
        match Preferences::load(storage.root()) {
            Ok(p) => {
                self.timeline_manager.preferences = p;
                self.timeline_manager.draft = p;
                self.timeline_manager.custom = p.fps.is_some();
                if let Some(f) = p.fps {
                    self.timeline_manager.fps = [f.numerator, f.denominator];
                }
            }
            Err(e) => self.timeline_manager.notice = e,
        }
    }
    pub(super) fn begin_timeline_manager(&mut self) -> Result<(), String> {
        self.begin_timeline_editor()?;
        self.timeline_manager.open = true;
        Ok(())
    }
    fn timeline_worker_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(
                self.documents
                    .iter()
                    .map(Document::resident_bytes)
                    .sum::<usize>(),
            )
            .saturating_sub(self.timeline_manager.cache.as_ref().map_or(0, |c| c.bytes))
    }
    fn timeline_stamp(&self) -> Option<Stamp> {
        let n = self.light_table_context()?;
        Stamp::of(
            &self.documents[n],
            self.timeline_preview_settings(n),
            self.timeline_manager.preferences,
            self.timeline_preview_repeat(self.documents[n].id),
        )
        .ok()
    }
    pub(super) fn poll_timeline_cache(&mut self, ctx: &egui::Context) {
        let current = self.timeline_stamp();
        if self
            .timeline_manager
            .cache
            .as_ref()
            .is_some_and(|c| Some(&c.stamp) != current.as_ref())
        {
            self.timeline_manager.cache = None;
            self.timeline_manager.notice =
                "Preview cache invalidated by drawing, source, timeline, range or view changes"
                    .into();
        }
        if self
            .timeline_manager
            .job
            .as_ref()
            .and_then(|j| j.stamp.as_ref())
            .is_some_and(|s| Some(s) != current.as_ref())
            && let Some(j) = &self.timeline_manager.job
        {
            j.cancel.store(true, Ordering::Relaxed);
        }
        let result = self
            .timeline_manager
            .job
            .as_ref()
            .and_then(|j| match j.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(Err("Timeline worker stopped".into())),
            });
        if let Some(r) = result {
            let j = self.timeline_manager.job.take().unwrap();
            let canceled = j.cancel.load(Ordering::Relaxed);
            drop(j);
            if canceled {
                self.timeline_manager.notice =
                    "Cancelled or stale; drawing unchanged and private RAM released".into();
            } else {
                match r {
                    Ok(Done::Cache(c)) if Some(&c.stamp) == current.as_ref() => {
                        self.timeline_manager.notice = format!(
                            "Prepared{} frames · {:.2}MiB RAM; no disk cache",
                            c.frames.len(),
                            c.bytes as f64 / (1024. * 1024.)
                        );
                        self.timeline_manager.cache = Some(c);
                    }
                    Ok(Done::Edit(p))
                        if self
                            .documents
                            .iter()
                            .any(|d| d.id == p.id && d.revision == p.revision) =>
                    {
                        self.timeline_manager.staged = Some(p);
                        self.timeline_manager.confirm = false;
                    }
                    Ok(_) => {
                        self.timeline_manager.notice =
                            "Prepared result is stale; drawing unchanged".into()
                    }
                    Err(e) => self.timeline_manager.notice = e,
                }
            }
        }
        if self.timeline_manager.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }
    fn stage_timeline(&mut self, ctx: &egui::Context, edit: Edit) -> Result<(), String> {
        if self.timeline_manager.job.is_some() {
            return Err("Wait for or cancel the current timeline operation".into());
        }
        if self.timeline_has_cel_binding() {
            return Err("Apply cel content or Return before changing timelines".into());
        }
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending edits first".into());
        }
        let n = self
            .light_table_context()
            .ok_or("Choose an animation document")?;
        self.stop_timeline_playback(self.documents[n].id);
        if self.documents[n].memory_bytes().saturating_mul(3) > self.timeline_worker_budget() {
            return Err("Timeline capture exceeds available working memory".into());
        }
        let mut d = drawer_core::layer_merge::preview(
            &self.documents[n].snapshot(),
            self.documents[n].current_frame,
        )?;
        d.id = self.documents[n].id;
        d.revision = self.documents[n].revision;
        d.keyframe_content = self.documents[n].keyframe_content;
        let budget = self.timeline_worker_budget();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let progress = Arc::new(AtomicU32::new(0));
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let r = core::prepare(&d, edit, budget, &flag).map(|value| {
                Done::Edit(Box::new(Prepared {
                    id: d.id,
                    revision: d.revision,
                    value,
                }))
            });
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.timeline_manager.staged = None;
        self.timeline_manager.job = Some(Job {
            cancel,
            progress,
            rx,
            worker: Some(worker),
            stamp: None,
        });
        Ok(())
    }
    fn apply_timeline_arrangement(&mut self) -> Result<(), String> {
        let p = self
            .timeline_manager
            .staged
            .take()
            .ok_or("Prepare a timeline change")?;
        let n = self
            .documents
            .iter()
            .position(|d| d.id == p.id && d.revision == p.revision)
            .ok_or("Prepared timeline is stale; drawing unchanged")?;
        self.stop_timeline_playback(p.id);
        self.documents[n].commit_state(p.value.state)?;
        self.documents[n].keyframe_content = None;
        self.timeline_manager.cache = None;
        self.reset_timeline_view(p.id);
        self.timeline_manager.notice = p.value.summary;
        self.timeline_manager.confirm = false;
        Ok(())
    }
    fn prepare_timeline_cache(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.timeline_manager.job.is_some() {
            return Err("Wait for or cancel the current timeline operation".into());
        }
        self.finish_stroke();
        let n = self
            .light_table_context()
            .ok_or("Choose an animation document")?;
        self.stop_timeline_playback(self.documents[n].id);
        let stamp = self
            .timeline_stamp()
            .ok_or("Choose a valid timeline, range and view")?;
        if self.documents[n].memory_bytes().saturating_mul(3) > self.timeline_worker_budget() {
            return Err("Timeline capture exceeds available working memory".into());
        }
        let mut d = drawer_core::layer_merge::preview(
            &self.documents[n].snapshot(),
            self.documents[n].current_frame,
        )?;
        d.id = self.documents[n].id;
        d.revision = self.documents[n].revision;
        d.keyframe_content = self.documents[n].keyframe_content;
        self.timeline_manager.cache = None;
        let budget = self.timeline_worker_budget();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let progress = Arc::new(AtomicU32::new(0));
        let done = progress.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let key = stamp.clone();
        let worker = std::thread::spawn(move || {
            let r = Cache::build(&d, key, budget, &flag, &done).map(Done::Cache);
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.timeline_manager.job = Some(Job {
            cancel,
            progress,
            rx,
            worker: Some(worker),
            stamp: Some(stamp),
        });
        Ok(())
    }
    fn apply_playback_preferences(&mut self) -> Result<(), String> {
        let mut p = self.timeline_manager.draft;
        p.fps = self
            .timeline_manager
            .custom
            .then_some(drawer_core::animation::Fps {
                numerator: self.timeline_manager.fps[0],
                denominator: self.timeline_manager.fps[1],
            });
        if !p.valid() {
            return Err("Use a valid rational preview FPS within240fps".into());
        }
        if let Some(s) = &self.storage {
            p.save(s.root())?;
        }
        if let Some(n) = self.light_table_context() {
            self.stop_timeline_playback(self.documents[n].id);
            self.reset_timeline_view(self.documents[n].id);
        }
        self.timeline_manager.preferences = p;
        self.timeline_manager.draft = p;
        self.timeline_manager.cache = None;
        if let Some(j) = &self.timeline_manager.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
        self.timeline_manager.notice="Local playback options applied; document, time placement and output settings are unchanged".into();
        Ok(())
    }
    pub(super) fn timeline_silent_preview(&self, n: usize) -> bool {
        self.timeline_manager.preferences.silent(&self.documents[n])
    }
    pub(super) fn timeline_manager_window(&mut self, ctx: &egui::Context) {
        self.poll_timeline_cache(ctx);
        if !self.timeline_manager.open {
            return;
        }
        let Some(n) = self.light_table_context() else {
            return;
        };
        let mut p = std::mem::take(&mut self.timeline_manager);
        let mut open = true;
        let mut action = 0;
        let mut edit = None;
        let d = &self.documents[n];
        egui::Window::new("Timelines / shots and playback").default_pos(egui::pos2(20.,20.)).default_width(700.).max_height(630.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.label(&p.notice);
            ui.horizontal(|ui|{let r=ui.add_enabled(p.job.is_none(),egui::Button::new("Precompute current playback range"));p.precompute=Some(r.rect);if r.clicked(){action=4;}let r=ui.button("Apply local playback options");p.options=Some(r.rect);if r.clicked(){action=3;}});
            if let Some(j)=&p.job{ui.label(format!("Preparing… {} frames complete",j.progress.load(Ordering::Relaxed)));let r=ui.button("Cancel preparation");p.cancel=Some(r.rect);if r.clicked(){j.cancel.store(true,Ordering::Relaxed);}}

            if let Some(prepared)=&p.staged{ui.label(&prepared.value.summary);let r=ui.checkbox(&mut p.confirm,"I reviewed this document-wide timeline change");p.review=Some(r.rect);let r=ui.add_enabled(p.confirm,egui::Button::new("Apply reviewed arrangement"));p.apply=Some(r.rect);if r.clicked(){action=1;}
            if ui.button("Discard arrangement change").clicked(){action=2;}}
            if let Some(a)=&d.animation{
                let data=&d.scene.timelines;ui.label(format!("Active: {} · {} frames · {}/{} FPS · {}",data.active_entry().map_or("Timeline 1",|e|e.name.as_str()),a.frames,a.fps.numerator,a.fps.denominator,core::timecode(d.current_frame as u64,a.fps,data.active_entry().is_some_and(|e|e.drop_frame)).unwrap_or_default()));
                ui.horizontal(|ui|{ui.label("New / copied / renamed timeline");ui.text_edit_singleline(&mut p.name);});
                ui.horizontal(|ui|{let r=ui.button("Duplicate: shared cels");p.copy_shared=Some(r.rect);if r.clicked(){edit=Some(Edit::Duplicate{name:p.name.clone(),independent:false});}let r=ui.button("Duplicate: independent cels");p.copy_independent=Some(r.rect);if r.clicked(){edit=Some(Edit::Duplicate{name:p.name.clone(),independent:true});}});
                for e in &data.entries{ui.selectable_value(&mut p.selected,Some(e.id),format!("{}{}",e.name,if e.id==data.active{" (active)"}else{""}));}
                if let Some(id)=p.selected{ui.horizontal(|ui|{let r=ui.button("Switch selected timeline");p.switch=Some(r.rect);if r.clicked(){edit=Some(Edit::Switch(id));}
                if ui.button("Rename selected").clicked(){edit=Some(Edit::Rename{id,name:p.name.clone()});}
                if ui.button("Delete selected arrangement…").clicked(){edit=Some(Edit::Delete(id));}});}
                ui.horizontal(|ui|{ui.label("Empty timeline: frames / FPS");ui.add(egui::DragValue::new(&mut p.frames).range(1..=10000));ui.add(egui::DragValue::new(&mut p.rate[0]).range(1..=100000));ui.add(egui::DragValue::new(&mut p.rate[1]).range(1..=100000));if ui.button("Prepare empty timeline").clicked(){edit=Some(Edit::New{name:p.name.clone(),frames:p.frames,fps:drawer_core::animation::Fps{numerator:p.rate[0],denominator:p.rate[1]}});}});
                let mut drop=data.active_entry().is_some_and(|e|e.drop_frame);let r=ui.add_enabled(core::drop_rate(a.fps).is_some(),egui::Checkbox::new(&mut drop,"Drop-frame timecode display (29.97 /59.94 only; no frames removed)"));p.drop_frame=Some(r.rect);if r.changed(){edit=Some(Edit::DropFrame(drop));}
                ui.label("Arrangements have independent exposures, ranges, keys, labels, audio and shot annotations. Layer/track structure and source layers are common to this document. Independent duplication creates new cel sources. Source-layer deletion/movement affects every saved arrangement and is restored together by Undo.");
            }else{ui.label("Create an animation in the timeline editor first.");}
            ui.separator();egui::CollapsingHeader::new("Local playback and precompute").default_open(false).show(ui,|ui|{
                ui.horizontal(|ui|{ui.selectable_value(&mut p.draft.mode,playback::Mode::RealTime,"Real-time (skip late screen frames)");ui.selectable_value(&mut p.draft.mode,playback::Mode::EveryFrame,"Every frame (may run slower)");});
                ui.horizontal(|ui|{ui.selectable_value(&mut p.draft.quality,playback::Quality::Quality,"Quality preview:256px");ui.selectable_value(&mut p.draft.quality,playback::Quality::Speed,"Speed preview:96px");});ui.checkbox(&mut p.draft.camera,"Apply filming camera in preview");
                ui.horizontal(|ui|{ui.checkbox(&mut p.custom,"Custom preview FPS");ui.add_enabled(p.custom,egui::DragValue::new(&mut p.fps[0]).range(1..=100000));ui.add_enabled(p.custom,egui::DragValue::new(&mut p.fps[1]).range(1..=100000));});
                ui.label("Every-frame and custom-rate playback are silent. Normal real-time playback uses the audio device clock when available. These options affect preview only; image/movie outputs still contain every requested frame at their saved FPS.");
                if let Some(c)=&p.cache{ui.label(format!("{} cached frames · {:.2}MiB RAM",c.frames.len(),c.bytes as f64/(1024.*1024.)));if ui.button("Release preview cache").clicked(){p.cache=None;}}
            });

        });
        p.open = open;
        self.timeline_manager = p;
        if !open {
            self.timeline_manager.job = None;
            self.timeline_manager.staged = None;
        }
        let r = match action {
            1 => self.apply_timeline_arrangement(),
            2 => {
                self.timeline_manager.staged = None;
                Ok(())
            }
            3 => self.apply_playback_preferences(),
            4 => self.prepare_timeline_cache(ctx),
            _ => Ok(()),
        };
        if let Err(e) = r {
            self.timeline_manager.notice = e;
        }
        if let Some(e) = edit
            && let Err(e) = self.stage_timeline(ctx, e)
        {
            self.timeline_manager.notice = e;
        }
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    core::verify(root)?;
    playback::verify(root)?;
    let dir = root.join("timelines-ui-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        use drawer_core::{
            animation::Fps,
            timeline_editor::{self, Edit as CelEdit},
            timeline_playback::{Mode, Quality},
        };
        let check = |ok: bool, s: &str| if ok { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 4,
            height: 3,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let state = timeline_editor::create(
            &app.documents[0],
            12,
            Fps {
                numerator: 30000,
                denominator: 1001,
            },
        )?;
        app.documents[0].commit_state(state)?;
        let state = timeline_editor::prepare(
            &app.documents[0],
            CelEdit::NewTrack("Cel track".into()),
            DEFAULT_DOCUMENT_BUDGET,
        )?
        .state;
        app.documents[0].commit_state(state)?;
        app.execute_command(&ctx, "timelines-playback");
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
            frame(app, t, vec![]);
        };
        let drain = |app: &mut Drawer, t: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.timeline_manager.job.is_some() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Timeline worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            frame(app, t, vec![]);
            Ok(())
        };
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let before = app.documents[0].snapshot();
        let r = app
            .timeline_manager
            .copy_shared
            .ok_or("Timeline copy control missing")?;
        click(&mut app, &mut time, r);
        drain(&mut app, &mut time)?;
        check(
            app.timeline_manager.staged.is_some(),
            &format!(
                "Pointer did not prepare a timeline: {}",
                app.timeline_manager.notice
            ),
        )?;
        let r = app
            .timeline_manager
            .apply
            .ok_or("Arrangement apply control missing")?;
        click(&mut app, &mut time, r);
        check(
            app.documents[0].snapshot() == before,
            "Unreviewed timeline change was applied",
        )?;
        let r = app
            .timeline_manager
            .review
            .ok_or("Timeline review checkbox missing")?;
        click(&mut app, &mut time, r);
        let r = app.timeline_manager.apply.unwrap();
        click(&mut app, &mut time, r);
        check(
            app.documents[0].scene.timelines.entries.len() == 2
                && app.documents[0].scene.cels.definitions.len() == 1,
            "Reviewed shared timeline copy failed",
        )?;
        app.timeline_manager.selected = Some(2);
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .timeline_manager
            .switch
            .ok_or("Timeline switch control missing")?;
        click(&mut app, &mut time, r);
        drain(&mut app, &mut time)?;
        let r = app.timeline_manager.review.unwrap();
        click(&mut app, &mut time, r);
        let r = app.timeline_manager.apply.unwrap();
        click(&mut app, &mut time, r);
        check(
            app.documents[0].scene.timelines.active == 2,
            "Pointer did not switch the timeline",
        )?;
        let original = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        let r = app
            .timeline_manager
            .precompute
            .ok_or("Precompute control missing")?;
        click(&mut app, &mut time, r);
        drain(&mut app, &mut time)?;
        check(
            app.timeline_manager
                .cache
                .as_ref()
                .is_some_and(|c| c.frames.len() == 12),
            &format!("Pointer precompute failed: {}", app.timeline_manager.notice),
        )?;
        app.documents[0].current_frame = 5;
        frame(&mut app, &mut time, vec![]);
        check(
            app.timeline_manager.cache.is_some()
                && app.documents[0].snapshot() == original
                && app.documents[0].history_counts() == counts
                && app.documents[0].dirty == dirty,
            "Precompute/playhead changed drawing or needlessly lost cache",
        )?;
        app.prepare_timeline_cache(&ctx)?;
        app.documents[0].edit_pixel(0, 3, 2, [1, 2, 3, 255]);
        let edited = app.documents[0].snapshot();
        drain(&mut app, &mut time)?;
        check(
            app.timeline_manager.cache.is_none() && app.documents[0].snapshot() == edited,
            "Editing during precompute applied stale pixels",
        )?;
        app.prepare_timeline_cache(&ctx)?;
        app.timeline_manager
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        drain(&mut app, &mut time)?;
        check(
            app.timeline_manager.cache.is_none() && app.documents[0].snapshot() == edited,
            "Cancelled cache changed drawing",
        )?;
        let before = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        app.timeline_manager.draft.mode = Mode::EveryFrame;
        app.timeline_manager.draft.quality = Quality::Speed;
        app.timeline_manager.draft.camera = false;
        app.timeline_manager.custom = true;
        app.timeline_manager.fps = [12, 1];
        let r = app
            .timeline_manager
            .options
            .ok_or("Playback options apply missing")?;
        click(&mut app, &mut time, r);
        check(
            app.timeline_manager.preferences.mode == Mode::EveryFrame
                && app.timeline_manager.preferences.fps
                    == Some(Fps {
                        numerator: 12,
                        denominator: 1,
                    })
                && app.documents[0].snapshot() == before
                && app.documents[0].history_counts() == counts
                && app.documents[0].dirty == dirty,
            "Local playback options changed drawing or failed to apply",
        )?;
        app.documents[0].current_frame = 0;
        app.toggle_audio_playback(&ctx, 0, false)?;
        let mut seen = vec![];
        for _ in 0..16 {
            time += 0.2;
            input_replay::frame(&ctx, &mut app, time, vec![]);
            let f = app.documents[0].current_frame;
            if seen.last() != Some(&f) {
                seen.push(f);
            }
            if !app.timeline_audio_playing(app.documents[0].id) {
                break;
            }
        }
        check(
            seen == (0..12).collect::<Vec<_>>()
                && app.audio_player.is_none()
                && app.documents[0].snapshot() == before
                && app.documents[0].history_counts() == counts,
            "Slow every-frame playback omitted frames, played audio or changed history",
        )?;
        let expected = app.timeline_manager.preferences;
        let restarted = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        check(
            restarted.timeline_manager.preferences == expected,
            "Local playback options did not survive restart",
        )?;
        drop(restarted);
        app.stage_timeline(
            &ctx,
            Edit::Duplicate {
                name: "Stale".into(),
                independent: false,
            },
        )?;
        app.documents[0].edit_pixel(0, 2, 2, [4, 5, 6, 255]);
        let edited = app.documents[0].snapshot();
        drain(&mut app, &mut time)?;
        check(
            app.timeline_manager.staged.is_none() && app.documents[0].snapshot() == edited,
            "Stale arrangement overwrote new edits",
        )?;
        let cancel = AtomicBool::new(false);
        while app.documents[0].scene.timelines.entries.len() < 32 {
            let n = app.documents[0].scene.timelines.entries.len();
            let s = core::prepare(
                &app.documents[0],
                Edit::Duplicate {
                    name: format!("Copy {n}"),
                    independent: false,
                },
                DEFAULT_DOCUMENT_BUDGET,
                &cancel,
            )?
            .state;
            app.documents[0].commit_state(s)?;
        }
        let max = app.documents[0].snapshot();
        check(
            core::prepare(
                &app.documents[0],
                Edit::Duplicate {
                    name: "Limit".into(),
                    independent: false,
                },
                DEFAULT_DOCUMENT_BUDGET,
                &cancel,
            )
            .is_err()
                && app.documents[0].snapshot() == max,
            "Timeline32 limit was not atomic",
        )?;
        let s = core::prepare(
            &app.documents[0],
            Edit::Rename {
                id: 2,
                name: "Renamed".into(),
            },
            DEFAULT_DOCUMENT_BUDGET,
            &cancel,
        )?
        .state;
        app.documents[0].commit_state(s)?;
        let prior = app.documents[0].snapshot();
        let sources = app.documents[0].scene.cels.definitions.len();
        let s = core::prepare(
            &app.documents[0],
            Edit::Delete(2),
            DEFAULT_DOCUMENT_BUDGET,
            &cancel,
        )?
        .state;
        app.documents[0].commit_state(s)?;
        check(
            app.documents[0].scene.timelines.entries.len() == 31
                && app.documents[0].scene.timelines.active != 2
                && app.documents[0].scene.cels.definitions.len() == sources,
            "Timeline deletion removed common sources",
        )?;
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == prior,
            "Timeline deletion Undo failed",
        )?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_copy_review_switch_options_cache_stale_cancel_restart_and_every_frame() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
