use super::*;
use drawer_core::{
    action_repeat as core,
    action_runner::{self as runner, Session},
    auto_actions::Action,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
enum Work {
    Document(Box<Session>),
    Batch(Box<runner::Batch>),
}
struct Job {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<(Work, Result<(), String>)>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
#[derive(Default)]
pub(super) struct Panel {
    pub(super) open: bool,
    action: Option<Action>,
    settings: core::Settings,
    blocked: bool,
    replace: bool,
    document: Option<core::Run>,
    batch: Option<Box<runner::Batch>>,
    job: Option<Job>,
    inputs: Vec<PathBuf>,
    folder: String,
    pattern: String,
    files: runner::Errors,
    message: String,
    controls: BTreeMap<&'static str, egui::Rect>,
}
fn job(mut work: Work, files: runner::Errors) -> Result<Job, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("repeated-action-round".into())
        .spawn(move || {
            let result = match &mut work {
                Work::Document(s) => s.run(runner::Errors::Stop, &flag),
                Work::Batch(b) => b.run(runner::Errors::Stop, files, &flag),
            };
            let _ = tx.send((work, result));
        })
        .map_err(|e| e.to_string())?;
    Ok(Job { cancel, rx })
}
impl Drawer {
    pub(super) fn begin_action_repeat(&mut self, action: Action) {
        if self.action_repeat.job.is_some() {
            self.action_repeat.open = true;
            return;
        }
        let loaded = self.storage.as_ref().map(core::Settings::load).transpose();
        let (settings, blocked, message) = match loaded {
            Ok(s) => (s.unwrap_or_default(), false, String::new()),
            Err(e) => (Default::default(), true, e),
        };
        self.action_repeat = Panel {
            open: true,
            action: Some(action),
            settings,
            blocked,
            message,
            pattern: "{title}-repeat-{index}".into(),
            ..Default::default()
        };
    }
    fn launch_action_round(&mut self, p: &mut Panel) -> Result<(), String> {
        if p.job.is_some() {
            return Ok(());
        }
        if let Some(run) = &p.document {
            let d =
                self.documents.iter().find(|d| d.id == run.source()).ok_or(
                    "Original document was closed; completed rounds remain in its history",
                )?;
            let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
            if used.saturating_add(d.memory_bytes().saturating_mul(10)) > DEFAULT_DOCUMENT_BUDGET {
                return Err("Round exceeds current workspace memory".into());
            }
            let s = run.prepare(d, core::now())?;
            p.job = Some(job(Work::Document(Box::new(s)), p.files)?);
        } else if let Some(batch) = p.batch.take() {
            p.job = Some(job(Work::Batch(batch), p.files)?);
        }
        Ok(())
    }
    fn repeat_command(&mut self, p: &mut Panel, key: &str) -> Result<(), String> {
        match key {
            "save" => {
                if p.blocked && !p.replace {
                    return Err("Confirm replacement of unreadable repeat settings".into());
                }
                p.settings
                    .save(self.storage.as_ref().ok_or("Local storage unavailable")?)?;
                p.blocked = false;
                p.replace = false;
                p.message = "Repeat options saved locally".into();
            }
            "document" => {
                self.finish_stroke();
                if self.drawing.has_path_draft() {
                    return Err("Commit or cancel path draft first".into());
                }
                let d = &self.documents[self.active.ok_or("Open a document")?];
                let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
                p.document = Some(core::Run::new(
                    d,
                    p.action.clone().ok_or("Select an action")?,
                    p.settings,
                    DEFAULT_DOCUMENT_BUDGET.saturating_sub(used.saturating_sub(d.resident_bytes())),
                )?);
                self.launch_action_round(p)?;
            }
            "batch" => {
                drawer_platform::file_browser::local_disk(Path::new(&p.folder))?;
                for path in &p.inputs {
                    drawer_platform::file_browser::local_disk(path)?;
                }
                let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
                let mut batch = runner::Batch::new(
                    p.inputs.clone(),
                    Path::new(&p.folder),
                    &p.pattern,
                    p.action.clone().ok_or("Select an action")?,
                    DEFAULT_DOCUMENT_BUDGET.saturating_sub(used),
                )?;
                batch.set_repeat(p.settings)?;
                p.batch = Some(Box::new(batch));
                self.launch_action_round(p)?;
            }
            "stop" => {
                if let Some(run) = &mut p.document {
                    run.schedule.stop();
                }
                if let Some(batch) = &mut p.batch
                    && let Some(r) = &mut batch.repeat
                {
                    r.stop();
                }
                if let Some(j) = &p.job {
                    j.cancel.store(true, Ordering::Relaxed);
                }
                p.message="Stopping: the current incomplete round is discarded; completed rounds/outputs remain".into();
            }
            "continue" => {
                if p.job.is_some() {
                    return Err("Wait until the current round stops".into());
                }
                if let Some(run) = &mut p.document {
                    run.schedule.resume();
                }
                if let Some(batch) = &mut p.batch {
                    batch.resume_repeat();
                }
            }
            "skip-file" => {
                if p.job.is_some() {
                    return Err("Stop first".into());
                }
                p.batch
                    .as_mut()
                    .ok_or("A file batch is required")?
                    .skip_file()?;
            }
            "discard" => {
                if p.job.is_some() {
                    return Err("Stop first".into());
                }
                p.document = None;
                p.batch = None;
                p.message =
                    "Session discarded; completed drawing rounds and new outputs remain".into();
            }
            "close" => {
                self.repeat_command(p, "stop")?;
                p.open = false;
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn action_repeat_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.action_repeat);
        if let Some(j) = &p.job {
            let stopped = j.cancel.load(Ordering::Relaxed);
            let received = j.rx.try_recv();
            if matches!(received, Err(mpsc::TryRecvError::Disconnected)) {
                p.job = None;
                if let Some(run) = &mut p.document {
                    run.schedule.stop();
                }
                p.message="Repeat worker stopped; source unchanged. Review the local batch log before starting another batch.".into();
            }
            if let Ok((work, result)) = received {
                p.job = None;
                match work {
                    Work::Document(s) => {
                        let outcome = if stopped {
                            Err("Current round canceled without changing its source".into())
                        } else {
                            result.and_then(|()| {
                                let used: usize =
                                    self.documents.iter().map(Document::resident_bytes).sum();
                                if used
                                    .saturating_add(s.document().memory_bytes().saturating_mul(3))
                                    > DEFAULT_DOCUMENT_BUDGET
                                {
                                    return Err(
                                        "Round result exceeds current workspace memory".into()
                                    );
                                }
                                let run = p.document.as_mut().ok_or("Round context missing")?;
                                let d = self
                                    .documents
                                    .iter_mut()
                                    .find(|d| d.id == run.source())
                                    .ok_or("Original document closed; result discarded")?;
                                run.commit(*s, d, core::now(), ctx.pixels_per_point() as f64)
                                    .map(|_| ())
                            })
                        };
                        if let Err(e) = outcome {
                            if let Some(run) = &mut p.document {
                                run.schedule.stop();
                            }
                            p.message = e;
                        } else {
                            p.message="Round committed once; Undo restores that round. View scroll is separate.".into();
                        }
                    }
                    Work::Batch(mut b) => {
                        if (stopped || result.is_err())
                            && let Some(r) = &mut b.repeat
                        {
                            r.stop();
                        }
                        p.message = result.map_or_else(
                            |e| e,
                            |()| {
                                if b.complete() {
                                    "File batch complete; new outputs and local log saved".into()
                                } else {
                                    "File round completed or stopped; source files unchanged".into()
                                }
                            },
                        );
                        p.batch = Some(b);
                    }
                }
            }
        }
        if !p.open {
            self.action_repeat = p;
            return;
        }
        let mut open = true;
        let mut command = None;
        let mut pick = false;
        let mut folder = false;
        p.controls.clear();
        egui::Window::new("Repeat action rounds")
            .id(egui::Id::new("action-repeat"))
            .default_pos(egui::pos2(80., 55.))
            .default_width(620.)
            .max_height(620.)
            .vscroll(true)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(p.action.as_ref().map_or("No action", |a| a.name.as_str()));
                let idle = p.job.is_none() && p.document.is_none() && p.batch.is_none();
                ui.add_enabled_ui(idle, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Rounds");
                        let r = ui.add(egui::DragValue::new(&mut p.settings.count).range(1..=1000));
                        p.controls.insert("count", r.rect);
                        ui.label("Wait ms");
                        ui.add(egui::DragValue::new(&mut p.settings.wait_ms).range(0..=60000));
                    });
                    ui.checkbox(
                        &mut p.settings.confirm,
                        "Ask to continue between rounds / files",
                    );
                    if p.blocked {
                        ui.checkbox(
                            &mut p.replace,
                            "Replace unreadable repeat options explicitly",
                        );
                    }
                    let r = ui.button("Save repeat options");
                    p.controls.insert("save", r.rect);
                    if r.clicked() {
                        command = Some("save");
                    }
                    ui.horizontal(|ui| {
                        pick = ui.button("Select input files…").clicked();
                        folder = ui.button("Output folder…").clicked();
                    });
                    ui.text_edit_singleline(&mut p.folder);
                    ui.text_edit_singleline(&mut p.pattern);
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut p.files,
                            runner::Errors::Stop,
                            "Stop on failed file",
                        );
                        ui.selectable_value(&mut p.files, runner::Errors::Skip, "Skip failed file");
                    });
                    ui.small(format!(
                        "{} files · outputs are new .drawer files; source replacement is refused",
                        p.inputs.len()
                    ));
                });
                ui.horizontal_wrapped(|ui| {
                    for (key, label, enabled) in [
                        ("document", "Run document rounds", idle),
                        ("batch", "Run file rounds", idle),
                        ("continue", "Continue / resume", p.job.is_none()),
                        ("stop", "Stop", true),
                        (
                            "skip-file",
                            "Skip current file",
                            p.job.is_none() && p.batch.is_some(),
                        ),
                        ("discard", "Discard session", p.job.is_none()),
                        ("close", "Close", true),
                    ] {
                        let r = ui.add_enabled(enabled, egui::Button::new(label));
                        p.controls.insert(key, r.rect);
                        if r.clicked() {
                            command = Some(key);
                        }
                    }
                });
                if let Some(r) = &p.document {
                    ui.label(format!(
                        "{} / {} rounds · {}{}{}",
                        r.schedule.completed,
                        r.schedule.settings.count,
                        if r.schedule.paused { "Stopped. " } else { "" },
                        if r.schedule.confirmation {
                            "Confirm continuation. "
                        } else {
                            ""
                        },
                        if core::now() < r.schedule.deadline {
                            "Waiting."
                        } else {
                            ""
                        }
                    ));
                }
                if let Some(b) = &p.batch {
                    ui.label(format!(
                        "File {} / {} · log {}",
                        b.cursor,
                        b.inputs.len(),
                        b.log.display()
                    ));
                    if let Some(r) = &b.repeat {
                        ui.label(format!(
                            "Round {} / {} · {}",
                            r.completed,
                            r.settings.count,
                            if r.confirmation {
                                "Confirm continuation"
                            } else if r.paused {
                                "Stopped"
                            } else {
                                "Ready / waiting"
                            }
                        ));
                    }
                    for result in &b.results {
                        ui.small(format!("{}: {}", result.input.display(), result.status));
                    }
                }
                if p.job.is_some() {
                    ui.label("Preparing a private round…");
                }
                ui.label(&p.message);
            });
        if pick && let Some(paths) = rfd::FileDialog::new().pick_files() {
            p.inputs = paths;
        }
        if folder && let Some(path) = rfd::FileDialog::new().pick_folder() {
            p.folder = path.display().to_string();
        }
        if !open {
            command = Some("close");
        }
        if let Some(command) = command
            && let Err(e) = self.repeat_command(&mut p, command)
        {
            p.message = e;
            if let Some(r) = &mut p.document {
                r.schedule.stop();
            }
        }
        let ready = p.job.is_none()
            && (p
                .document
                .as_ref()
                .is_some_and(|r| r.schedule.ready(core::now()))
                || p.batch.as_ref().is_some_and(|b| {
                    !b.complete()
                        && b.repeat
                            .as_ref()
                            .is_some_and(|r| !r.paused && (r.complete() || r.ready(core::now())))
                }));
        if ready
            && p.open
            && let Err(e) = self.launch_action_round(&mut p)
        {
            p.message = e;
            if let Some(r) = &mut p.document {
                r.schedule.stop();
            }
            if let Some(b) = &mut p.batch
                && let Some(r) = &mut b.repeat
            {
                r.stop();
            }
        }
        if p.job.is_some()
            || p.document.as_ref().is_some_and(|r| {
                !r.schedule.complete() && !r.schedule.paused && !r.schedule.confirmation
            })
            || p.batch.as_ref().is_some_and(|b| {
                !b.complete()
                    && b.repeat
                        .as_ref()
                        .is_some_and(|r| !r.paused && !r.confirmation)
            })
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
        self.action_repeat = p;
    }
    pub(super) fn quiesce_action_repeat(&mut self, ctx: &egui::Context) -> bool {
        let open = self.action_repeat.open;
        self.action_repeat.open = false;
        self.action_repeat_window(ctx);
        self.action_repeat.open = open;
        self.action_repeat.job.is_some()
    }
}
#[path = "action_repeat_verify.rs"]
mod verify;
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    verify::run(root)
}
