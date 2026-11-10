use super::*;
use drawer_core::{action_runner as core, auto_actions::Action};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
enum Run {
    Single(Box<core::Session>),
    Batch(Box<core::Batch>),
}
impl Run {
    fn complete(&self) -> bool {
        match self {
            Self::Single(s) => s.complete(),
            Self::Batch(b) => b.complete(),
        }
    }
}
struct Job {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<(Run, Result<(), String>)>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    action: Option<Action>,
    run: Option<Run>,
    job: Option<Job>,
    steps: core::Errors,
    files: core::Errors,
    inputs: Vec<PathBuf>,
    folder: String,
    pattern: String,
    message: String,
    controls: BTreeMap<&'static str, egui::Rect>,
}
impl Drawer {
    pub(super) fn begin_action_runner(&mut self, action: Action) {
        if self.action_runner.job.is_some() {
            self.action_runner.open = true;
            return;
        }
        self.action_runner = Panel {
            open: true,
            action: Some(action),
            pattern: "{title}-action-{index}".into(),
            ..Default::default()
        };
    }
    fn start_action_runner(&mut self, p: &mut Panel, kind: &str) -> Result<(), String> {
        if p.job.is_some() {
            return Err("Wait until playback pauses".into());
        }
        if self.drawing.has_path_draft() {
            return Err("Commit or cancel path draft first".into());
        }
        self.finish_stroke();
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        let action = p
            .action
            .clone()
            .ok_or("Select an action in Local auto actions")?;
        if kind == "document" {
            let d = &self.documents[self.active.ok_or("Open a document")?];
            p.run = Some(Run::Single(Box::new(core::Session::new(
                d,
                action,
                DEFAULT_DOCUMENT_BUDGET.saturating_sub(used.saturating_sub(d.resident_bytes())),
            )?)));
        } else if kind == "batch" {
            p.run = Some(Run::Batch(Box::new(core::Batch::new(
                p.inputs.clone(),
                std::path::Path::new(&p.folder),
                &p.pattern,
                action,
                DEFAULT_DOCUMENT_BUDGET.saturating_sub(used),
            )?)));
        }
        let mut run = p
            .run
            .take()
            .ok_or("Prepare a document or local file batch")?;
        let steps = p.steps;
        let files = p.files;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("resumable-auto-action".into())
            .spawn(move || {
                let result = match &mut run {
                    Run::Single(s) => s.run(steps, &flag),
                    Run::Batch(b) => b.run(steps, files, &flag),
                };
                let _ = tx.send((run, result));
            })
            .map_err(|e| e.to_string())?;
        p.job = Some(Job { cancel, rx });
        p.message = "Running frozen action snapshot on private copies…".into();
        Ok(())
    }
    fn runner_command(&mut self, p: &mut Panel, key: &str) -> Result<(), String> {
        match key {
            "document" | "batch" | "resume" => self.start_action_runner(p, key)?,
            "pause" => {
                if let Some(job) = &p.job {
                    job.cancel.store(true, Ordering::Relaxed);
                    p.message =
                        "Pausing at the current atomic step; completed outputs remain".into();
                }
            }
            "skip-step" => {
                if p.job.is_some() {
                    return Err("Pause first".into());
                }
                match p.run.as_mut() {
                    Some(Run::Single(s)) => s.skip()?,
                    Some(Run::Batch(b)) => {
                        b.current
                            .as_mut()
                            .ok_or("No current step; use Skip file")?
                            .skip()?;
                        b.checkpoint()?;
                    }
                    None => return Err("Prepare playback first".into()),
                }
            }
            "skip-file" => {
                if p.job.is_some() {
                    return Err("Pause first".into());
                }
                if let Some(Run::Batch(b)) = &mut p.run {
                    b.skip_file()?;
                } else {
                    return Err("A file batch is required".into());
                }
            }
            "apply" => {
                if p.job.is_some() {
                    return Err("Wait for playback".into());
                }
                if !matches!(&p.run,Some(Run::Single(s)) if s.complete()) {
                    return Err("Finish document playback first".into());
                }
                let resident: usize = self.documents.iter().map(Document::resident_bytes).sum();
                if let Some(Run::Single(s)) = &p.run
                    && resident.saturating_add(s.document().memory_bytes().saturating_mul(3))
                        > DEFAULT_DOCUMENT_BUDGET
                {
                    return Err("Action result exceeds current workspace memory".into());
                }
                let n = self.active.ok_or("Open original document")?;
                let Some(Run::Single(s)) = p.run.take() else {
                    unreachable!()
                };
                s.commit(&mut self.documents[n])?;
                p.message = "Applied once; document Undo restores the original".into();
            }
            "discard" => {
                if p.job.is_some() {
                    return Err("Pause first".into());
                }
                p.run = None;
                p.message="Session discarded; original documents unchanged. Completed batch outputs/log retained.".into();
            }
            "close" => {
                p.open = false;
                if let Some(j) = &p.job {
                    j.cancel.store(true, Ordering::Relaxed);
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn action_runner_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.action_runner);
        if let Some(j) = &p.job {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
            match j.rx.try_recv() {
                Ok((run, result)) => {
                    let paused = j.cancel.load(Ordering::Relaxed);
                    p.job = None;
                    let complete = run.complete();
                    p.run = Some(run);
                    p.message=match result{Err(e)=>format!("Stopped: {e}. Retry, skip the step/file, or discard."),Ok(())=>if complete{"Completed; document results require Apply. Batch outputs and log are saved."}else if paused{"Paused. Resume continues with the first incomplete step/file."}else{"Ready to continue"}.into()};
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    p.job = None;
                    p.message =
                        "Worker stopped; original documents unchanged. Keep completed outputs/log."
                            .into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !p.open {
            self.action_runner = p;
            return;
        }
        let mut open = true;
        let mut command = None;
        let mut pick = false;
        let mut folder = false;
        p.controls.clear();
        egui::Window::new("Action playback and local file batch").id(egui::Id::new("action-runner")).default_pos(egui::pos2(40.,40.)).default_width(610.).max_height(650.).vscroll(true).open(&mut open).show(ctx,|ui|{
            let r=ui.button("Close");p.controls.insert("close",r.rect);if r.clicked(){command=Some("close");}
            ui.label(p.action.as_ref().map_or("No action",|a|a.name.as_str()));
            ui.small("This is a frozen action snapshot. Discard and reopen from the editor to use edited parameters. Originals are never saved by file batches; new .drawer outputs only.");
            ui.horizontal(|ui|{ui.label("Step error:");ui.selectable_value(&mut p.steps,core::Errors::Stop,"Stop");ui.selectable_value(&mut p.steps,core::Errors::Skip,"Skip step");});
            ui.horizontal(|ui|{ui.label("File error:");ui.selectable_value(&mut p.files,core::Errors::Stop,"Stop batch");ui.selectable_value(&mut p.files,core::Errors::Skip,"Skip file");});
            ui.add_enabled_ui(p.job.is_none()&&p.run.is_none(),|ui|{
                ui.horizontal(|ui|{pick=ui.button("Select local input files…").clicked();folder=ui.button("Output folder…").clicked();});
                ui.text_edit_singleline(&mut p.folder);ui.text_edit_singleline(&mut p.pattern);ui.label(format!("{} input files · {{title}} / {{index}} naming",p.inputs.len()));
                for path in &p.inputs{ui.small(path.display().to_string());}
            });
            for(keys,enabled)in[(&[("document","Run on active document"),("batch","Run selected files")][..],p.job.is_none()&&p.run.is_none()),(&[("resume","Resume"),("skip-step","Skip current step"),("skip-file","Skip current file"),("apply","Apply document result"),("discard","Discard session")][..],p.job.is_none()&&p.run.is_some()),(&[("pause","Stop / pause")][..],p.job.is_some())]{ui.horizontal_wrapped(|ui|{for(key,label)in keys{let r=ui.add_enabled(enabled,egui::Button::new(*label));p.controls.insert(key,r.rect);if r.clicked(){command=Some(*key);}}});}
            if let Some(run)=&p.run{match run{Run::Single(s)=>{ui.label(format!("Step {} / {} · {} notices",s.cursor,s.action.steps.len(),s.issues.len()));for e in &s.issues{ui.small(e);}},Run::Batch(b)=>{ui.label(format!("File {} / {} · log {}",b.cursor,b.inputs.len(),b.log.display()));if let Some(s)=&b.current{ui.label(format!("Current step {} / {}",s.cursor,s.action.steps.len()));}for r in &b.results{ui.small(format!("{}: {}",r.input.display(),r.status));for e in &r.issues{ui.small(e);}}}}}
            ui.label(&p.message);
        });
        if pick && let Some(paths) = rfd::FileDialog::new().pick_files() {
            p.inputs = paths;
        }
        if folder && let Some(path) = rfd::FileDialog::new().pick_folder() {
            p.folder = path.display().to_string();
        }
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            command = Some("close");
        }
        if let Some(key) = command
            && let Err(e) = self.runner_command(&mut p, key)
        {
            p.message = e;
        }
        self.action_runner = p;
    }
}
#[path = "action_runner_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pause_resume_batch_and_apply_pointer_controls() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}
