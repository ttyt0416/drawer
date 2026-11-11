use super::*;
use drawer_core::recovery::{self as core, Candidate, Session};
use std::path::Path;
use std::time::{Duration, Instant};
pub(super) struct Panel {
    pub open: bool,
    last_success: bool,
    retire: std::collections::BTreeSet<u64>,
    work_stamp: Option<String>,
    session: Option<Session>,
    candidates: Vec<Candidate>,
    root: Option<PathBuf>,
    last: Instant,
    stamps: Vec<(u64, u64, bool)>,
    error: Option<String>,
    confirm: Option<PathBuf>,
    pub recover: Option<egui::Rect>,
    pub cancel: Option<egui::Rect>,
    pub discard: Option<egui::Rect>,
    pub accept: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            last_success: false,
            retire: Default::default(),
            work_stamp: None,
            session: None,
            candidates: vec![],
            root: None,
            last: Instant::now(),
            stamps: vec![],
            error: None,
            confirm: None,
            recover: None,
            cancel: None,
            discard: None,
            accept: None,
        }
    }
}
impl Drawer {
    pub(super) fn load_recovery(&mut self, storage: &UserStorage) {
        if cfg!(test)
            || self.smoke_output.is_some()
            || std::env::args_os().any(|v| v == "--smoke-test")
        {
            return;
        }
        let root = self.resources.current.root(storage).to_owned();
        self.init_recovery(&root);
    }
    pub(super) fn install_resource_recovery(&mut self, root: &Path, session: Session) {
        if let Some(old) = self.recovery.session.take()
            && let Err(e) = old.finish()
        {
            self.status = Some(format!(
                "Old scratch session retained after cleanup error: {e}"
            ));
        }
        self.recovery.root = Some(root.to_owned());
        self.recovery.session = Some(session);
        self.recovery.last = Instant::now();
        self.recovery.stamps.clear();
    }
    fn resource_recovery_candidates(&mut self, root: &Path) -> Vec<Candidate> {
        let mut roots = self.resources.current.retained.clone();
        roots.push(root.to_owned());
        roots.sort();
        roots.dedup();
        let mut out = vec![];
        for p in roots {
            match drawer_platform::file_browser::local_disk(&p).and_then(|_| core::scan(&p)) {
                Ok(mut c) => out.append(&mut c),
                Err(e) => {
                    self.recovery.error = Some(format!(
                        "Scratch unavailable; recovery copies preserved: {e}"
                    ))
                }
            }
        }
        out
    }
    pub(super) fn init_recovery(&mut self, root: &Path) {
        self.recovery.root = Some(root.to_path_buf());
        self.recovery.candidates = self.resource_recovery_candidates(root);
        self.recovery.open = !self.recovery.candidates.is_empty();
        match Session::create(root) {
            Ok(s) => self.recovery.session = Some(s),
            Err(e) => self.recovery.error = Some(e),
        }
    }
    pub(super) fn retire_recovery_document(&mut self, id: u64) {
        if self.background.busy() && self.recovery.session.is_none() {
            self.recovery.retire.insert(id);
        }
        if let Some(session) = &mut self.recovery.session
            && let Err(e) = session.forget(id)
        {
            self.status = Some(format!(
                "Document decision completed; recovery record retained after cleanup error: {e}"
            ));
        }
    }
    pub(super) fn quiesce_recovery(&mut self) {
        self.drain_background();
        // Retain the last complete snapshot while the data root is copied.
        self.recovery.session.take();
    }
    pub(super) fn resume_recovery(&mut self) {
        if self.recovery.root.is_none() || self.recovery.session.is_some() {
            return;
        }
        if let Some(storage) = self.storage.clone() {
            let root = self.resources.current.root(&storage).to_owned();
            self.init_recovery(&root);
            if let Err(e) = self.checkpoint_recovery() {
                self.recovery.error = Some(e);
                self.recovery.open = true;
            }
        }
    }
    pub(super) fn finish_recovery(&mut self) {
        if (self.quit_ready || self.documents.iter().all(|d| !d.dirty))
            && !self.project_needs_recovery()
            && let Some(s) = self.recovery.session.take()
            && let Err(e) = s.finish()
        {
            self.status = Some(e);
        }
    }
    pub(super) fn checkpoint_recovery(&mut self) -> Result<(), String> {
        self.start_recovery_job()?;
        self.drain_background();
        if !self.recovery.last_success {
            return Err(self
                .recovery
                .error
                .clone()
                .unwrap_or("Recovery did not complete".into()));
        }
        Ok(())
    }
    fn reopen_recovery(&mut self, index: usize) -> Result<(), String> {
        let c = self
            .recovery
            .candidates
            .get(index)
            .ok_or("Recovery entry removed")?;
        if c.project {
            let work = c.open_project()?;
            self.open_recovered_project(work)
        } else {
            let d = c.open()?;
            self.add_document(d);
            Ok(())
        }
    }
    pub(super) fn recovery_window(&mut self, ctx: &egui::Context) {
        if self.recovery.session.is_some() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if self.recovery.session.is_some()
            && !self.background.busy()
            && !ctx.input(|i| i.pointer.any_down())
            && !self.drawing.pending()
        {
            let stamps = self
                .documents
                .iter()
                .map(|d| (d.id, d.revision, d.dirty))
                .collect::<Vec<_>>();
            let commands = stamps
                .iter()
                .map(|(id, rev, _)| {
                    self.recovery
                        .stamps
                        .iter()
                        .find(|(old, _, _)| old == id)
                        .map_or(rev.saturating_add(1), |(_, old, _)| rev.abs_diff(*old))
                })
                .sum::<u64>();
            let work = self.recovery_project_stamp();
            let changed = stamps != self.recovery.stamps
                || work.as_ref().is_ok_and(|w| *w != self.recovery.work_stamp);
            let elapsed = self.recovery.last.elapsed();
            if changed
                && (elapsed
                    >= Duration::from_secs(u64::from(self.resources.current.recovery_seconds))
                    || commands >= u64::from(self.resources.current.recovery_commands)
                        && elapsed >= Duration::from_secs(1))
                && let Err(e) = self.start_recovery_job()
            {
                self.recovery.error = Some(e);
                self.recovery.open = true;
                self.recovery.last = Instant::now();
            }
        }
        self.recovery.recover = None;
        self.recovery.cancel = None;
        self.recovery.discard = None;
        self.recovery.accept = None;
        if !self.recovery.open {
            return;
        }
        let mut open = true;
        let mut recover = None;
        let mut discard = None;
        let mut rescan = false;
        let mut checkpoint = false;
        let mut cancel = false;
        egui::Window::new("Local recovery").open(&mut open).default_width(570.).show(ctx,|ui|{
            ui.label("Snapshots are current editable state; session Undo is not stored. Original files are never overwritten by recovery.");
            ui.horizontal(|ui|{rescan=ui.button("Rescan closed sessions").clicked();checkpoint=ui.add_enabled(self.recovery.session.is_some(),egui::Button::new("Checkpoint now")).clicked();});
            if let Some(e)=&self.recovery.error{ui.colored_label(egui::Color32::LIGHT_RED,e);}
            if self.recovery.candidates.is_empty(){ui.label("No unresolved closed-session snapshots.");}
            egui::ScrollArea::vertical().max_height(350.).show(ui,|ui|{
                for (n,c) in self.recovery.candidates.iter().enumerate(){
                    ui.group(|ui|{ui.label(&c.title);if let Some(p)=&c.original{ui.small(format!("Original (display only): {}",p.display()));}
                        if let Some(e)=&c.error{ui.colored_label(egui::Color32::LIGHT_RED,e);}
                        let r=ui.add_enabled(c.error.is_none(),egui::Button::new(if c.project{"Open as recovered project"}else{"Open as recovered document"}));if n==0{self.recovery.recover=Some(r.rect);}
if r.clicked(){recover=Some(n);}
                        let r=ui.button("Discard this session…");if n==0{self.recovery.discard=Some(r.rect);}
if r.clicked(){discard=Some(c.session.clone());}
                    });
                }
            });
            let r=ui.button("Cancel / keep snapshots");self.recovery.cancel=Some(r.rect);cancel=r.clicked();
        });
        self.recovery.open = open && !cancel;
        if rescan && let Some(root) = self.recovery.root.clone() {
            self.recovery.candidates = self.resource_recovery_candidates(&root);
        }
        if checkpoint && let Err(e) = self.start_recovery_job() {
            self.recovery.error = Some(e);
        }
        if let Some(n) = recover {
            match self.reopen_recovery(n){Ok(())=>self.status=Some("Recovered into a separate unsaved document; snapshots retained until your decision".into()),Err(e)=>self.recovery.error=Some(e)}
        }
        if let Some(path) = discard {
            self.recovery.confirm = Some(path);
        }
        if let Some(path) = self.recovery.confirm.clone() {
            let mut confirmed = false;
            let mut keep = false;
            egui::Window::new("Discard recovery session?").collapsible(false).show(ctx,|ui|{
                ui.label("Delete all recovery snapshots in this closed session? This cannot be undone. Open originals and recovered drawings are preserved.");ui.small(path.display().to_string());
                let r=ui.button("Discard snapshots permanently");self.recovery.accept=Some(r.rect);confirmed=r.clicked();keep=ui.button("Keep snapshots").clicked();
            });
            if confirmed {
                if self.project_uses_recovery_session(&path) {
                    self.recovery.error=Some("Save the recovered project to a separate local project before discarding its backing recovery session".into());
                    self.recovery.confirm = None;
                    return;
                }
                match core::discard(&path) {
                    Ok(()) => {
                        self.recovery.candidates.retain(|c| c.session != path);
                        self.recovery.confirm = None;
                    }
                    Err(e) => self.recovery.error = Some(e),
                }
            }
            if keep {
                self.recovery.confirm = None;
            }
        }
    }
}

#[path = "recovery_verify.rs"]
pub(super) mod verify;

#[path = "background_save.rs"]
pub(super) mod jobs;
