use super::*;
use drawer_core::recovery::{self as core, Candidate, Session};
use std::path::Path;
use std::time::{Duration, Instant};
pub(super) struct Panel {
    pub open: bool,
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
        self.init_recovery(storage.root());
    }
    fn init_recovery(&mut self, root: &Path) {
        self.recovery.root = Some(root.to_path_buf());
        match core::scan(root) {
            Ok(c) => {
                self.recovery.open = !c.is_empty();
                self.recovery.candidates = c;
            }
            Err(e) => self.recovery.error = Some(e),
        }
        match Session::create(root) {
            Ok(s) => self.recovery.session = Some(s),
            Err(e) => self.recovery.error = Some(e),
        }
    }
    pub(super) fn quiesce_recovery(&mut self) {
        // Retain the last complete snapshot while the data root is copied.
        self.recovery.session.take();
    }
    pub(super) fn resume_recovery(&mut self) {
        if self.recovery.root.is_none() || self.recovery.session.is_some() {
            return;
        }
        if let Some(storage) = self.storage.clone() {
            self.init_recovery(storage.root());
            if let Err(e) = self.checkpoint_recovery() {
                self.recovery.error = Some(e);
                self.recovery.open = true;
            }
        }
    }
    pub(super) fn finish_recovery(&mut self) {
        if (self.quit_ready || self.documents.iter().all(|d| !d.dirty))
            && let Some(s) = self.recovery.session.take()
            && let Err(e) = s.finish()
        {
            self.status = Some(e);
        }
    }
    fn checkpoint_recovery(&mut self) -> Result<(), String> {
        if self.drawing.pending() {
            return Err("Finish the current stroke/path before a recovery checkpoint".into());
        }
        let documents = self
            .documents
            .iter()
            .filter(|d| d.dirty)
            .collect::<Vec<_>>();
        let session = self
            .recovery
            .session
            .as_mut()
            .ok_or("Local recovery storage is unavailable")?;
        session.checkpoint(&documents)?;
        self.recovery.stamps = self
            .documents
            .iter()
            .map(|d| (d.id, d.revision, d.dirty))
            .collect();
        self.recovery.last = Instant::now();
        Ok(())
    }
    fn reopen_recovery(&mut self, index: usize) -> Result<(), String> {
        let d = self
            .recovery
            .candidates
            .get(index)
            .ok_or("Recovery entry removed")?
            .open()?;
        self.add_document(d);
        Ok(())
    }
    pub(super) fn recovery_window(&mut self, ctx: &egui::Context) {
        if self.recovery.session.is_some() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if self.recovery.session.is_some()
            && self.recovery.last.elapsed() >= Duration::from_secs(60)
            && !ctx.input(|i| i.pointer.any_down())
            && !self.drawing.pending()
        {
            let stamps = self
                .documents
                .iter()
                .map(|d| (d.id, d.revision, d.dirty))
                .collect::<Vec<_>>();
            if stamps != self.recovery.stamps
                && let Err(e) = self.checkpoint_recovery()
            {
                self.recovery.error = Some(e);
                self.recovery.open = true;
            }
            self.recovery.last = Instant::now();
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
                        let r=ui.add_enabled(c.error.is_none(),egui::Button::new("Open as recovered document"));if n==0{self.recovery.recover=Some(r.rect);}
if r.clicked(){recover=Some(n);}
                        let r=ui.button("Discard this session…");if n==0{self.recovery.discard=Some(r.rect);}
if r.clicked(){discard=Some(c.session.clone());}
                    });
                }
            });
            let r=ui.button("Cancel / keep snapshots");self.recovery.cancel=Some(r.rect);cancel=r.clicked();
        });
        self.recovery.open = open && !cancel;
        if rescan && let Some(root) = &self.recovery.root {
            match core::scan(root) {
                Ok(c) => self.recovery.candidates = c,
                Err(e) => self.recovery.error = Some(e),
            }
        }
        if checkpoint && let Err(e) = self.checkpoint_recovery() {
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
