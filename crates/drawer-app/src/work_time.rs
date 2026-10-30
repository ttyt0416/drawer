use super::*;
use drawer_core::work_time::{self as core, Journal, Record, Snapshot};
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};

struct Entry {
    record: Record,
    journal: Option<Journal>,
    error: Option<String>,
    revision: u64,
}
impl Entry {
    fn flush(&mut self) {
        if self.error.is_some() {
            return;
        }
        if let Some(j) = &mut self.journal
            && let Err(e) = j.save(&self.record)
        {
            self.error = Some(e);
        }
    }
}
pub(super) struct Panel {
    pub open: bool,
    pub(super) report: Option<super::work_time_report::Report>,
    pub report_current: Option<egui::Rect>,
    pub report_open: Option<egui::Rect>,
    entries: HashMap<u64, Entry>,
    clock: core::Clock,
    awake_base: u64,
    last_flush: Instant,
    last_poll: Instant,
    confirm_reset: Option<String>,
    recover: Vec<Record>,
    recover_choice: Option<Record>,
    notice: String,
    pub toggle: Option<egui::Rect>,
    pub reset: Option<egui::Rect>,
    pub confirm: Option<egui::Rect>,
    pub cancel: Option<egui::Rect>,
    pub undo: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            report: None,
            report_current: None,
            report_open: None,
            entries: HashMap::new(),
            clock: Default::default(),
            awake_base: drawer_platform::work_clock::milliseconds().unwrap_or(0),
            last_flush: Instant::now(),
            last_poll: Instant::now(),
            confirm_reset: None,
            recover: vec![],
            recover_choice: None,
            notice: String::new(),
            toggle: None,
            reset: None,
            confirm: None,
            cancel: None,
            undo: None,
        }
    }
}
impl Panel {
    fn now(&self) -> u64 {
        drawer_platform::work_clock::milliseconds()
            .unwrap_or(self.awake_base)
            .saturating_sub(self.awake_base)
    }
    pub(super) fn attach(&mut self, d: &mut Document, storage: Option<&UserStorage>) {
        if let Some(e) = self.entries.get(&d.id) {
            d.work_time = Some(e.record.snapshot.clone());
            return;
        }
        let mut s = d.work_time.clone().unwrap_or_else(|| {
            Snapshot::new(
                d.path
                    .as_ref()
                    .map_or_else(core::identity, |p| core::legacy_identity(p)),
            )
        });
        if self.entries.values().any(|e| e.record.snapshot.id == s.id) {
            s.id = core::identity();
            s.sequence = 0;
            s.reset_undo.clear();
            self.notice="An independently opened copy has its own work-time identity; existing elapsed time was copied once".into();
        }
        let mut record = Record::new(
            s,
            d.title.clone(),
            d.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        );
        let (journal, error) = if let Some(storage) = storage {
            match Journal::open(&storage.root().join("work-time"), &mut record) {
                Ok(j) => (Some(j), None),
                Err(e) => (None, Some(e)),
            }
        } else {
            (None, None)
        };
        let saved = record.snapshot.clone();
        d.work_time = Some(saved.clone());
        self.entries.insert(
            d.id,
            Entry {
                record,
                journal,
                error,
                revision: d.revision,
            },
        );
    }
    fn reconcile(&mut self, documents: &mut [Document], storage: Option<&UserStorage>) {
        self.entries.retain(|id, e| {
            if documents.iter().any(|d| d.id == *id) {
                true
            } else {
                e.flush();
                false
            }
        });
        for d in documents {
            self.attach(d, storage);
        }
    }
    fn step(
        &mut self,
        now: u64,
        active: Option<u64>,
        focused: bool,
        editing: bool,
        documents: &mut [Document],
    ) {
        if self.last_poll.elapsed() > Duration::from_millis(core::MAX_GAP_MS) {
            self.clock.break_activity();
        }
        self.last_poll = Instant::now();
        let active = active.filter(|id| {
            self.entries
                .get(id)
                .is_some_and(|e| !e.record.snapshot.paused && e.error.is_none())
        });
        if let Some((id, ms)) = self.clock.step(now, active, focused, editing)
            && let Some(e) = self.entries.get_mut(&id)
            && let Err(error) = e.record.snapshot.add(ms)
        {
            e.error = Some(error);
        }
        for d in documents {
            if let Some(e) = self.entries.get_mut(&d.id) {
                e.record.title = d.title.chars().take(4096).collect();
                e.record.origin = d.path.as_ref().map(|p| p.to_string_lossy().into_owned());
                d.work_time = Some(e.record.snapshot.clone());
                e.revision = d.revision;
            }
        }
    }
    fn flush(&mut self) {
        for e in self.entries.values_mut() {
            e.flush();
        }
        self.last_flush = Instant::now();
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        self.flush();
    }
}
impl Drawer {
    /// Settle the previous owner before save, tab switch, close or time controls.
    pub(super) fn settle_work_time(&mut self) {
        self.work_time
            .reconcile(&mut self.documents, self.storage.as_ref());
        let id = self.active.map(|n| self.documents[n].id);
        self.work_time.step(
            self.work_time.now(),
            id,
            self.workspaces.is_focused(),
            false,
            &mut self.documents,
        );
        self.work_time.flush();
    }
    pub(super) fn work_time_tick(&mut self, ctx: &egui::Context) {
        self.work_time
            .reconcile(&mut self.documents, self.storage.as_ref());
        let id = self.active.map(|n| self.documents[n].id);
        let changed = id.is_some_and(|id| {
            self.documents
                .iter()
                .find(|d| d.id == id)
                .zip(self.work_time.entries.get(&id))
                .is_some_and(|(d, e)| d.revision != e.revision)
        });
        let viewport = self.canvas_view.viewport_rect();
        let over = ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| {
            viewport.is_some_and(|r| r.contains(p))
                && self.canvas_view.viewport_layer() == ctx.layer_id_at(p)
        });
        let canvas_input = ctx.input(|i| {
            over && !self.canvas_view.timeline_read_only
                && i.events.iter().any(|e| {
                    matches!(e, egui::Event::PointerButton { pressed: true, .. })
                        || (matches!(e, egui::Event::PointerMoved(_)) && i.pointer.primary_down())
                })
        });
        let now = self.work_time.now();
        self.work_time.step(
            now,
            id,
            self.workspaces.is_focused(),
            changed || canvas_input || id.is_some_and(|id| self.drawing.work_input == Some(id)),
            &mut self.documents,
        );
        if self.work_time.last_flush.elapsed() >= Duration::from_secs(1) {
            self.work_time.flush();
        }
        if self.work_time.clock.running(now) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
    fn change_work_time(&mut self, action: u8) -> Result<(), String> {
        self.settle_work_time();
        let n = self.active.ok_or("Choose a drawing")?;
        let e = self
            .work_time
            .entries
            .get_mut(&self.documents[n].id)
            .ok_or("Time record unavailable")?;
        if let Some(error) = &e.error {
            return Err(error.clone());
        }
        match action {
            1 => e.record.snapshot.pause(!e.record.snapshot.paused)?,
            2 => e.record.snapshot.reset()?,
            3 => e.record.snapshot.undo_reset()?,
            _ => return Err("Unknown time action".into()),
        }
        self.work_time.clock.break_activity();
        e.flush();
        self.documents[n].work_time = Some(e.record.snapshot.clone());
        if let Some(error) = &e.error {
            return Err(error.clone());
        }
        Ok(())
    }
    pub(super) fn work_time_records(&mut self, all: bool) -> Result<Vec<Record>, String> {
        self.settle_work_time();
        if all {
            Ok(self
                .documents
                .iter()
                .filter_map(|d| self.work_time.entries.get(&d.id).map(|e| e.record.clone()))
                .collect())
        } else {
            let n = self.active.ok_or("Choose a drawing")?;
            Ok(vec![
                self.work_time
                    .entries
                    .get(&self.documents[n].id)
                    .ok_or("Time record unavailable")?
                    .record
                    .clone(),
            ])
        }
    }
    fn export_work_time(&mut self, path: &Path) -> Result<(), String> {
        self.settle_work_time();
        let n = self.active.ok_or("Choose a drawing")?;
        let target = self.export_target(n, path)?;
        let r = &self
            .work_time
            .entries
            .get(&self.documents[n].id)
            .ok_or("Time record unavailable")?
            .record;
        core::export(r, &target)
    }
    fn recover_work_time(&mut self, record: &Record) -> Result<(), String> {
        let n = self
            .active
            .ok_or("Choose a drawing to attach recovered time")?;
        if self.work_time.entries.iter().any(|(id, e)| {
            *id != self.documents[n].id && e.record.snapshot.id == record.snapshot.id
        }) {
            return Err("This time record is already attached to an open document".into());
        }
        self.settle_work_time();
        let mut next = record.clone();
        next.title = self.documents[n].title.clone();
        next.origin = self.documents[n]
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let storage = self
            .storage
            .as_ref()
            .ok_or("Local user storage unavailable")?;
        if self.documents[n]
            .work_time
            .as_ref()
            .is_some_and(|s| s.id == next.snapshot.id)
        {
            return Err("This drawing already owns that time record".into());
        }
        let j = Journal::open(&storage.root().join("work-time"), &mut next)?;
        let saved = next.snapshot.clone();
        self.documents[n].work_time = Some(saved.clone());
        self.work_time.entries.insert(
            self.documents[n].id,
            Entry {
                record: next,
                journal: Some(j),
                error: None,
                revision: self.documents[n].revision,
            },
        );
        self.work_time.clock.break_activity();
        self.work_time.notice="Recovered time metadata attached. The drawing is unchanged; save the native document to include its identity".into();
        Ok(())
    }
    pub(super) fn work_time_window(&mut self, ctx: &egui::Context) {
        if !self.work_time.open {
            return;
        }
        let mut open = true;
        let mut action = 0;
        let mut recover = None;
        let record = self
            .active
            .and_then(|n| self.work_time.entries.get(&self.documents[n].id))
            .map(|e| (e.record.clone(), e.error.clone()));
        egui::Window::new("Document work time").default_pos(egui::pos2(30.,20.)).default_width(620.).max_height(600.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.label(&self.work_time.notice);
            ui.label("Active editing sessions only: 5 minutes idle stops counting. Hovering, background documents, manual pause, app shutdown and suspended/stalled gaps are excluded. Time metadata has its own reset history and never changes drawing Undo or its modified marker.");
            if let Some((r,error))=&record {
                let s=&r.snapshot;let seconds=s.milliseconds/1000;
                ui.heading(format!("{} · {}:{:02}:{:02}.{:03}",r.title,seconds/3600,seconds/60%60,seconds%60,s.milliseconds%1000));
                ui.label(format!("{} · {}",if s.paused{"Paused"}else if self.work_time.clock.running(self.work_time.now()){"Active editing session"}else{"Waiting for editing input"},s.id));
                if let Some(error)=error {ui.colored_label(egui::Color32::YELLOW,format!("Time tracking disabled: {error}. Existing records were preserved; close and reopen after resolving the storage error."));}
                ui.horizontal(|ui|{
                    let b=ui.add_enabled(error.is_none(),egui::Button::new(if s.paused{"Resume work time"}else{"Pause work time"}));self.work_time.toggle=Some(b.rect);if b.clicked(){action=1;}
                    let b=ui.add_enabled(error.is_none(),egui::Button::new("Reset work time…"));self.work_time.reset=Some(b.rect);if b.clicked(){self.work_time.confirm_reset=Some(s.id.clone());}
                    let b=ui.add_enabled(error.is_none()&&!s.reset_undo.is_empty(),egui::Button::new("Undo time reset"));self.work_time.undo=Some(b.rect);if b.clicked(){action=3;}
                });
                if self.work_time.confirm_reset.as_ref()==Some(&s.id){ui.label("Reset this drawing's elapsed time to zero? Undo time reset restores the removed duration, including after restart.");ui.horizontal(|ui|{
                    let b=ui.button("Confirm time reset");self.work_time.confirm=Some(b.rect);if b.clicked(){action=2;self.work_time.confirm_reset=None;}
                    let b=ui.button("Cancel time reset");self.work_time.cancel=Some(b.rect);if b.clicked(){self.work_time.confirm_reset=None;}
                });}else{self.work_time.confirm_reset=None;}
                if ui.button("Export local time JSON…").clicked(){action=4;}
                ui.horizontal(|ui|{
                    let r=ui.button("Prepare current document CSV");self.work_time.report_current=Some(r.rect);if r.clicked(){action=7;}
                    let r=ui.button("Prepare open documents CSV");self.work_time.report_open=Some(r.rect);if r.clicked(){action=8;}
                });
                ui.label("Normal native save includes the latest time snapshot. Local work-time journals recover newer time without changing the saved drawing. Save As keeps the identity. An independently opened copy receives a separate identity.");
            }else{ui.label("Choose a drawing to view its work time.");}
            ui.separator();if ui.button("Find local time records for unsaved/recovered drawings").clicked(){action=5;}
            for r in &self.work_time.recover {if ui.button(format!("Recover time: {} · {:.3}s · {}",r.title,r.snapshot.milliseconds as f64/1000.,&r.snapshot.id[..12])).clicked(){self.work_time.recover_choice=Some(r.clone());}}
            if let Some(r)=&self.work_time.recover_choice {ui.label(format!("Attach {}'s time metadata to the active drawing? This recovers duration only. Its original local record is preserved.",r.title));ui.horizontal(|ui|{if ui.button("Attach recovered time").clicked(){recover=Some(r.clone());}
if ui.button("Cancel recovery").clicked(){action=6;}});}
        });
        self.work_time.open = open;
        let result = match action {
            1..=3 => self.change_work_time(action),
            4 => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Export work time to a new local JSON file")
                    .add_filter("Time metadata", &["json"])
                    .set_file_name("work-time.json")
                    .save_file()
                {
                    self.export_work_time(&path)
                } else {
                    Ok(())
                }
            }
            5 => {
                let r = self
                    .storage
                    .as_ref()
                    .ok_or("Local user storage unavailable".to_string())
                    .and_then(|s| core::catalog(&s.root().join("work-time")));
                match r {
                    Ok(rows) => {
                        self.work_time.recover = rows;
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            7 => self.prepare_work_time_report(false),
            8 => self.prepare_work_time_report(true),
            6 => {
                self.work_time.recover_choice = None;
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.work_time.notice = e;
        }
        if let Some(r) = recover {
            match self.recover_work_time(&r) {
                Ok(()) => self.work_time.recover_choice = None,
                Err(e) => self.work_time.notice = e,
            }
        }
    }
}

pub(super) fn smoke(root: &Path) -> Result<(), String> {
    core::verify(root)?;
    let dir = root.join("work-time-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.settle_work_time();
        let id = app.documents[0].id;
        let mut t = 0.;
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
        app.execute_command(&ctx, "work-time");
        for _ in 0..3 {
            frame(&mut app, &mut t, vec![]);
        }
        let original = app.documents[0].snapshot();
        let dirty = app.documents[0].dirty;
        let counts = app.documents[0].history_counts();
        let revision = app.documents[0].revision;
        // Controlled monotonic times verify accounting without wall-clock sleeps.
        app.work_time.clock = Default::default();
        app.work_time
            .step(0, Some(id), true, true, &mut app.documents);
        app.work_time
            .step(1234, Some(id), true, false, &mut app.documents);
        app.work_time.clock = Default::default();
        let r = app.work_time.toggle.ok_or("Time pause control missing")?;
        click(&mut app, &mut t, r);
        if !app.documents[0].work_time.as_ref().unwrap().paused {
            return Err("Time pause pointer failed".into());
        }
        let r = app.work_time.reset.ok_or("Time reset control missing")?;
        click(&mut app, &mut t, r);
        let r = app.work_time.cancel.ok_or("Reset cancel missing")?;
        click(&mut app, &mut t, r);
        if app.documents[0].work_time.as_ref().unwrap().milliseconds != 1234 {
            return Err("Canceled reset changed time".into());
        }
        let r = app.work_time.reset.unwrap();
        click(&mut app, &mut t, r);
        let r = app.work_time.confirm.ok_or("Reset confirm missing")?;
        click(&mut app, &mut t, r);
        if app.documents[0].work_time.as_ref().unwrap().milliseconds != 0 {
            return Err("Confirmed time reset failed".into());
        }
        // The newly enabled Undo control must enter the previous-frame hit-test list.
        frame(&mut app, &mut t, vec![]);
        let r = app.work_time.undo.unwrap();
        click(&mut app, &mut t, r);
        if app.documents[0].work_time.as_ref().unwrap().milliseconds != 1234
            || app.documents[0].snapshot() != original
            || app.documents[0].dirty != dirty
            || app.documents[0].history_counts() != counts
            || app.documents[0].revision != revision
        {
            return Err(format!(
                "Time controls: ms={} expected1234, paused={}, reset_depth={}, same_state={}, dirty={} expected{}, counts={:?} expected{:?}, revision={} expected{}, notice={}",
                app.documents[0].work_time.as_ref().unwrap().milliseconds,
                app.documents[0].work_time.as_ref().unwrap().paused,
                app.documents[0]
                    .work_time
                    .as_ref()
                    .unwrap()
                    .reset_undo
                    .len(),
                app.documents[0].snapshot() == original,
                app.documents[0].dirty,
                dirty,
                app.documents[0].history_counts(),
                counts,
                app.documents[0].revision,
                revision,
                app.work_time.notice
            ));
        }
        let path = dir.join("drawing.drawer");
        app.save_to(0, &path)?;
        let stored = app.documents[0].work_time.clone();
        app.export_work_time(&dir.join("time.json"))?;
        // Newer unsaved time survives restart and supersedes the native snapshot.
        app.change_work_time(1)?;
        app.work_time.clock = Default::default();
        app.work_time
            .step(0, Some(id), true, true, &mut app.documents);
        app.work_time
            .step(1000, Some(id), true, false, &mut app.documents);
        app.work_time.clock = Default::default();
        app.work_time.flush();
        if app.documents[0].dirty {
            return Err("Automatic time marked drawing modified".into());
        }
        drop(app);
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        app.open_path(&path)?;
        let s = app.documents[0]
            .work_time
            .as_ref()
            .ok_or("Recovered native time absent")?;
        if s.milliseconds != 2234 || s.id != stored.unwrap().id || app.documents[0].dirty {
            return Err("Newer local time not recovered independently of drawing".into());
        }
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.duplicate_view();
        if app.work_time.entries.len() != 2 {
            return Err("Duplicate view created duplicate time owner".into());
        }
        let old = app.documents[0].work_time.clone();
        let new_id = app.documents[1].id;
        app.work_time.clock = Default::default();
        app.work_time
            .step(0, Some(new_id), true, true, &mut app.documents);
        app.work_time
            .step(1000, Some(new_id), true, false, &mut app.documents);
        if app.documents[0].work_time != old {
            return Err("Background time was double counted".into());
        }
        app.work_time.clock = Default::default();
        let before_recovery = app.documents[1].snapshot();
        let dirty_recovery = app.documents[1].dirty;
        let mut recovery = Record::new(
            Snapshot::new(core::identity()),
            "Unsaved lost drawing".into(),
            None,
        );
        recovery.snapshot.add(42)?;
        let journal = Journal::open(&dir.join("user/work-time"), &mut recovery)?;
        drop(journal);
        app.recover_work_time(&recovery)?;
        if app.documents[1].work_time.as_ref().unwrap().milliseconds != 42
            || app.documents[1].snapshot() != before_recovery
            || app.documents[1].dirty != dirty_recovery
        {
            return Err("Unsaved time recovery changed drawing".into());
        }
        let copy = dir.join("copy.drawer");
        std::fs::copy(&path, &copy).map_err(|e| e.to_string())?;
        app.open_path(&copy)?;
        if app.documents[2].work_time.as_ref().unwrap().id
            == app.documents[0].work_time.as_ref().unwrap().id
        {
            return Err("Independently opened native copy reused time owner".into());
        }
        if app.documents[2].work_time.as_ref().unwrap().paused {
            app.change_work_time(1)?;
        }
        app.drawing.work_input = Some(app.documents[2].id);
        app.work_time.clock = Default::default();
        app.work_time_tick(&ctx);
        if !app.work_time.clock.running(app.work_time.now()) {
            return Err("Native drawing input did not activate work time".into());
        }
        drop(app);
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_pause_reset_cancel_undo_native_save_local_recovery_and_multiple_views() {
        let dir = crate::tests::TestDirectory::new();
        super::smoke(&dir.0).unwrap();
    }
}
