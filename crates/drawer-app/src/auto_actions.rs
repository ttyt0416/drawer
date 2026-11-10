use super::*;
use drawer_core::auto_actions::{self as core, Canvas, Mode, Operation, Settings};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
struct Job {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<core::Prepared, String>>,
    document: u64,
    apply: bool,
}
struct Import {
    before: Settings,
    after: Settings,
    issues: Vec<String>,
}
pub(super) struct Panel {
    open: bool,
    pub(super) draft: Settings,
    saved: Settings,
    blocked: bool,
    undo: Vec<Settings>,
    redo: Vec<Settings>,
    set: Option<u64>,
    action: Option<u64>,
    recording: Option<u64>,
    operation: Operation,
    edit_step: Option<u64>,
    name: String,
    path: String,
    replace: bool,
    import: Option<Import>,
    acknowledge: bool,
    job: Option<Job>,
    prepared: Option<core::Prepared>,
    message: String,
    pub(super) controls: BTreeMap<&'static str, egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            draft: Default::default(),
            saved: Default::default(),
            blocked: false,
            undo: vec![],
            redo: vec![],
            set: None,
            action: None,
            recording: None,
            operation: Operation::Canvas(Canvas::FlipHorizontal),
            edit_step: None,
            name: "Action".into(),
            path: String::new(),
            replace: false,
            import: None,
            acknowledge: false,
            job: None,
            prepared: None,
            message: String::new(),
            controls: BTreeMap::new(),
        }
    }
}
impl Panel {
    fn changed(&mut self, before: Settings) {
        if self.draft != before {
            if self.draft.mode != before.mode {
                self.recording = None;
            }
            if !self.draft.valid() {
                self.draft = before;
                self.message = "Invalid name, parameters or action capacity".into();
                return;
            }
            if self.undo.len() >= 20 {
                self.undo.remove(0);
            }
            self.undo.push(before);
            self.redo.clear();
            self.import = None;
            self.prepared = None;
            if let Some(job) = &self.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
        if self
            .action
            .is_some_and(|id| self.draft.action(id).is_none())
        {
            self.action = None;
            self.recording = None;
        }
    }
    fn restore(&mut self, redo: bool) {
        self.recording = None;
        self.import = None;
        self.prepared = None;
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        let next = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        if let Some(next) = next {
            let before = std::mem::replace(&mut self.draft, next);
            if redo {
                self.undo.push(before)
            } else {
                self.redo.push(before)
            }
        }
    }
}
impl Drawer {
    pub(super) fn load_auto_actions(&mut self, storage: &UserStorage) {
        match Settings::load(storage) {
            Ok(s) => {
                self.auto_actions.draft = s.clone();
                self.auto_actions.saved = s;
            }
            Err(e) => {
                self.auto_actions.blocked = true;
                self.auto_actions.message =
                    format!("Settings unreadable; original file preserved: {e}");
            }
        }
    }
    pub(super) fn begin_auto_actions(&mut self) {
        self.auto_actions.open = true;
    }
    pub(super) fn auto_actions_nonrecordable(&mut self, command: &str) {
        if self.auto_actions.recording.is_some() && command != "auto-actions" {
            self.auto_actions.message = format!(
                "Not recorded: {command}. Only commands executed in Auto actions are recordable."
            );
        }
    }
    fn save_auto_actions(&self, p: &mut Panel) -> Result<(), String> {
        if p.blocked {
            return Err(
                "Unreadable settings file is protected; repair or move it before restarting".into(),
            );
        }
        p.draft.save(
            self.storage
                .as_ref()
                .ok_or("Local settings folder unavailable")?,
        )?;
        p.saved = p.draft.clone();
        p.message = "Action settings saved locally; document history unchanged".into();
        Ok(())
    }
    fn execute_auto_operation(&mut self, p: &mut Panel) -> Result<(), String> {
        if self.drawing.has_path_draft() {
            return Err("Commit or cancel the path draft before running action commands".into());
        }
        if p.job.is_some() {
            return Err("Wait for or cancel playback".into());
        }
        let recording = p.recording;
        let mut recorded = p.draft.clone();
        if let Some(id) = recording {
            recorded.append(id, p.operation.clone())?;
        }
        self.finish_stroke();
        let n = self.active.ok_or("Open a document first")?;
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(self.documents[n].memory_bytes().saturating_mul(6))
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Command exceeds workspace memory".into());
        }
        let state =
            core::prepare_operation(&self.documents[n], &p.operation, &AtomicBool::new(false))?;
        let changed = if let Operation::Scroll { document } = p.operation {
            let before = self.documents[n].view;
            let size = [
                self.documents[n].width as f64,
                self.documents[n].height as f64,
            ];
            core::scroll(
                &mut self.documents[n].view,
                size,
                document,
                self.interface.ppp,
            )?;
            before != self.documents[n].view
        } else {
            self.documents[n].commit_state_named(state, "Auto action recording command")?
        };
        if changed && recording.is_some() {
            p.draft = recorded;
            p.message =
                "Command applied and recorded with its parameters; Save settings to retain it"
                    .into();
        } else {
            p.message = if changed {
                "Command applied"
            } else {
                "No document change; no step recorded"
            }
            .into();
        }
        Ok(())
    }
    fn start_auto_action(&mut self, p: &mut Panel, id: u64, apply: bool) -> Result<(), String> {
        if p.job.is_some() {
            return Err("Playback is already running".into());
        }
        if self.drawing.has_path_draft() {
            return Err("Commit or cancel the path draft before playback".into());
        }
        p.recording = None;
        self.finish_stroke();
        let n = self.active.ok_or("Open a document first")?;
        let d = &self.documents[n];
        let action = p.draft.action(id).ok_or("Select an action")?.clone();
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        let available =
            DEFAULT_DOCUMENT_BUDGET.saturating_sub(used.saturating_sub(d.resident_bytes()));
        if d.memory_bytes()
            .saturating_mul(10)
            .saturating_add(d.resident_bytes())
            > available
        {
            return Err("Playback exceeds workspace memory".into());
        }
        let mut copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        copy.id = d.id;
        copy.revision = d.revision;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = cancel.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("local-auto-action".into())
            .spawn(move || {
                let _ = tx.send(core::prepare(&copy, &action, available, &worker));
            })
            .map_err(|e| e.to_string())?;
        p.job = Some(Job {
            cancel,
            rx,
            document: d.id,
            apply,
        });
        p.prepared = None;
        p.message = "Preparing commands on a private copy…".into();
        Ok(())
    }
    fn apply_auto_action(&mut self, p: &mut Panel) -> Result<(), String> {
        let prepared = p.prepared.take().ok_or("Prepare playback first")?;
        let n = self.active.ok_or("Open the original document")?;
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(prepared.state.memory_bytes().saturating_mul(3))
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Playback result exceeds workspace memory".into());
        }
        let changed = prepared.commit_with_scale(&mut self.documents[n], self.interface.ppp)?;
        p.message = if changed {
            "Action applied as one document Undo"
        } else {
            "Action completed without a document change"
        }
        .into();
        Ok(())
    }
    fn read_auto_sets(&self, p: &mut Panel) -> Result<(), String> {
        p.import = None;
        let source = Settings::read(std::path::Path::new(&p.path))?;
        let after = p.draft.import(&source)?;
        let tools = self
            .workspaces
            .store
            .current
            .user_tools
            .tools
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>();
        let issues = source.issues(self.active.map(|n| self.documents[n].id), &tools);
        p.import = Some(Import {
            before: p.draft.clone(),
            after,
            issues,
        });
        p.acknowledge = false;
        Ok(())
    }
    fn import_auto_sets(&self, p: &mut Panel) -> Result<(), String> {
        let s = p.import.as_ref().ok_or("Read action sets first")?;
        if s.before != p.draft {
            return Err("Action settings changed; read sets again".into());
        }
        if !s.issues.is_empty() && !p.acknowledge {
            return Err("Review the listed unavailable commands/tools before importing".into());
        }
        p.draft = s.after.clone();
        p.import = None;
        Ok(())
    }
    fn poll_auto_action(&mut self, p: &mut Panel, ctx: &egui::Context) {
        let Some(job) = &p.job else {
            return;
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(25));
        let result = match job.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(_) => Err("Playback worker stopped; document unchanged".into()),
        };
        let job = p.job.take().unwrap();
        if job.cancel.load(Ordering::Relaxed) {
            p.message = "Playback canceled; document unchanged".into();
            return;
        }
        if self
            .active
            .is_none_or(|n| self.documents[n].id != job.document)
        {
            p.message = "Active document changed; playback discarded".into();
            return;
        }
        match result {
            Err(e) => p.message = e,
            Ok(result) => {
                p.prepared = Some(result);
                if job.apply {
                    if let Err(e) = self.apply_auto_action(p) {
                        p.message = e;
                    }
                } else {
                    p.message =
                        "Playback prepared. Apply commits once; Discard changes nothing.".into();
                }
            }
        }
    }
    pub(super) fn auto_actions_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.auto_actions);
        self.poll_auto_action(&mut p, ctx);
        if !p.open {
            self.auto_actions = p;
            return;
        }
        let before = p.draft.clone();
        let mut open = true;
        let mut command = None;
        let mut run = None;
        let mut browse = false;
        let mut save_as = false;
        p.controls.clear();
        egui::Window::new("Local auto actions").id(egui::Id::new("auto-actions")).default_pos(egui::pos2(25.,25.)).default_width(650.).max_height(640.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{for(key,label)in[("save","Save settings"),("revert","Revert unsaved"),("undo","Undo settings"),("redo","Redo settings")]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}});
            ui.horizontal(|ui|{ui.selectable_value(&mut p.draft.mode,Mode::Edit,"Edit mode");let r=ui.selectable_value(&mut p.draft.mode,Mode::Buttons,"Button mode");p.controls.insert("buttons",r.rect);});
            ui.small("Only commands executed here are recorded. Other tools, dialogs, shell and network commands are not recorded. Raster targets are zero-based layer slots, resolved in the current document.");
            ui.label(&p.message);
            if p.job.is_some(){let r=ui.button("Cancel playback");p.controls.insert("cancel-play",r.rect);if r.clicked(){command=Some("cancel-play");}}
            if let Some(s)=&p.prepared {ui.label(format!("Prepared: {} × {}, {} layers",s.state.width,s.state.height,s.state.layers.len()));ui.horizontal(|ui|{for key in["apply","discard"]{let r=ui.button(key);p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}});}
            ui.separator();
            ui.horizontal_wrapped(|ui|{ui.label("Name");ui.text_edit_singleline(&mut p.name);for(key,label)in[("new-set","New set"),("new-action","New action")]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}});
            for set in &mut p.draft.sets {
                ui.horizontal(|ui|{if ui.selectable_label(p.set==Some(set.id),format!("Set {}",set.id)).clicked(){p.set=Some(set.id);p.action=None;p.recording=None;}ui.text_edit_singleline(&mut set.name);});
                if p.set==Some(set.id){
                    for a in &mut set.actions {
                        if p.draft.mode==Mode::Buttons {let r=ui.button(format!("▶ {}",a.name));p.controls.insert("play-button",r.rect);if r.clicked(){run=Some((a.id,true));}}
                        else{ui.horizontal(|ui|{if ui.selectable_label(p.action==Some(a.id),format!("Action {}",a.id)).clicked(){p.action=Some(a.id);p.recording=None;}ui.text_edit_singleline(&mut a.name);});}
                    }
                }
            }
            if p.draft.mode==Mode::Edit {
                ui.horizontal_wrapped(|ui|{for(key,label)in[("copy-set","Duplicate set"),("delete-set","Delete set"),("copy-action","Duplicate action"),("delete-action","Delete action")]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}});
                ui.horizontal_wrapped(|ui|{for(key,label)in[("record","Start recording"),("stop","Stop recording"),("prepare","Prepare playback"),("runner","Resumable playback / file batch")]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}if let Some(id)=p.recording{ui.label(format!("Recording action {id}"));}});
                if let Some(a)=p.action.and_then(|id|p.draft.action_mut(id).ok()) {
                    let mut edit=None;
                    egui::ScrollArea::vertical().id_salt("action-steps").max_height(150.).show(ui,|ui|{for(i,step)in a.steps.iter_mut().enumerate(){ui.horizontal_wrapped(|ui|{ui.checkbox(&mut step.enabled,format!("{} · #{}",i+1,step.id));ui.label(format!("{:?}",step.operation));for key in["↑","↓","Copy","Delete","Edit"]{if ui.small_button(key).clicked(){edit=Some((i,key));}}});}});
                    if let Some((i,key))=edit{match key{"↑" if i>0=>a.steps.swap(i,i-1),"↓" if i+1<a.steps.len()=>a.steps.swap(i,i+1),"Delete"=>{a.steps.remove(i);},"Edit"=>{p.operation=a.steps[i].operation.clone();p.edit_step=Some(a.steps[i].id);p.message="Edit the command fields, then Replace edited step".into();},"Copy"=>{command=Some("copy-step");p.edit_step=Some(a.steps[i].id);},_=>{}}}
                    for issue in a.steps.iter().filter_map(|s|s.operation.issue(self.active.map(|n|self.documents[n].id),&[])){ui.colored_label(egui::Color32::YELLOW,issue);}
                }
                operation_fields(ui,&mut p.operation);
                ui.horizontal_wrapped(|ui|{for(key,label)in[("execute","Execute command / record"),("append","Append command without execution"),("replace-step","Replace edited step")]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}});
            }
            ui.collapsing("Local set import / export",|ui|{
                if ui.text_edit_singleline(&mut p.path).changed(){p.import=None;p.replace=false;p.acknowledge=false;}
                ui.horizontal_wrapped(|ui|{browse=ui.button("Browse…").clicked();save_as=ui.button("Save as…").clicked();for(key,label)in[("read","Read sets"),("import","Apply independent sets"),("cancel-import","Cancel import"),("export","Export selected set")]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}});
                ui.checkbox(&mut p.replace,"Replace existing export file");
                if let Some(s)=&p.import{ui.label("Validated and staged; all imported IDs will be new.");for issue in &s.issues{ui.colored_label(egui::Color32::YELLOW,issue);}if !s.issues.is_empty(){ui.checkbox(&mut p.acknowledge,"Keep these unavailable steps for editing; playback will refuse enabled unavailable steps");}}
            });
        });
        p.changed(before);
        if browse
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Drawer action sets", &["json"])
                .pick_file()
        {
            p.path = path.display().to_string();
            p.import = None;
            p.replace = false;
        }
        if save_as
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Drawer action sets", &["json"])
                .set_file_name("actions.json")
                .save_file()
        {
            p.path = path.display().to_string();
            p.import = None;
            p.replace = false;
        }
        let before = p.draft.clone();
        let result = (|| -> Result<(), String> {
            match command {
                Some("save") => self.save_auto_actions(&mut p)?,
                Some("revert") => {
                    p.draft = p.saved.clone();
                    p.recording = None;
                }
                Some("undo") => p.restore(false),
                Some("redo") => p.restore(true),
                Some("new-set") => {
                    p.set = Some(p.draft.add_set(p.name.clone())?);
                    p.action = None;
                }
                Some("new-action") => {
                    p.action = Some(
                        p.draft
                            .add_action(p.set.ok_or("Select a set")?, p.name.clone())?,
                    );
                }
                Some("copy-set") => {
                    p.set = Some(p.draft.duplicate_set(p.set.ok_or("Select a set")?)?);
                    p.action = None;
                }
                Some("delete-set") => {
                    let id = p.set.ok_or("Select a set")?;
                    p.draft.sets.retain(|s| s.id != id);
                    p.set = None;
                    p.action = None;
                    p.recording = None;
                }
                Some("copy-action") => {
                    p.action = Some(
                        p.draft
                            .duplicate_action(p.action.ok_or("Select an action")?)?,
                    );
                }
                Some("delete-action") => {
                    let id = p.action.ok_or("Select an action")?;
                    for s in &mut p.draft.sets {
                        s.actions.retain(|a| a.id != id);
                    }
                    p.action = None;
                    p.recording = None;
                }
                Some("record") => {
                    let id = p.action.ok_or("Select an action")?;
                    p.draft.action(id).ok_or("Select an action")?;
                    p.recording = Some(id);
                    p.message="Recording commands run here. Other editor operations are explicitly not recorded.".into();
                }
                Some("stop") => {
                    p.recording = None;
                    p.message = "Recording stopped; Save settings to retain the sequence".into();
                }
                Some("execute") => self.execute_auto_operation(&mut p)?,
                Some("append") => {
                    p.draft
                        .append(p.action.ok_or("Select an action")?, p.operation.clone())?;
                }
                Some("copy-step") => {
                    let id = p.action.ok_or("Select an action")?;
                    let i = p
                        .draft
                        .action(id)
                        .and_then(|a| a.steps.iter().position(|s| Some(s.id) == p.edit_step))
                        .ok_or("Select a step")?;
                    p.draft.duplicate_step(id, i)?;
                }
                Some("replace-step") => {
                    let a = p.draft.action_mut(p.action.ok_or("Select an action")?)?;
                    let step = a
                        .steps
                        .iter_mut()
                        .find(|s| Some(s.id) == p.edit_step)
                        .ok_or("Click Edit next to a step first")?;
                    step.operation = p.operation.clone();
                }
                Some("prepare") => run = Some((p.action.ok_or("Select an action")?, false)),
                Some("apply") => self.apply_auto_action(&mut p)?,
                Some("discard") => {
                    p.prepared = None;
                    p.message = "Playback discarded; document unchanged".into();
                }
                Some("cancel-play") => {
                    if let Some(job) = &p.job {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    p.prepared = None;
                }
                Some("read") => self.read_auto_sets(&mut p)?,
                Some("import") => self.import_auto_sets(&mut p)?,
                Some("cancel-import") => p.import = None,
                Some("export") => {
                    let set = p
                        .draft
                        .sets
                        .iter()
                        .find(|s| Some(s.id) == p.set)
                        .ok_or("Select a set to export")?
                        .clone();
                    Settings {
                        sets: vec![set],
                        ..p.draft.clone()
                    }
                    .write(std::path::Path::new(&p.path), p.replace)?;
                    p.message = "Selected action set exported locally".into();
                }
                _ => {}
            }
            Ok(())
        })();
        if !matches!(command, Some("undo" | "redo")) {
            p.changed(before);
        }
        if let Err(e) = result {
            p.message = e;
        }
        if let Some((id, apply)) = run
            && let Err(e) = self.start_auto_action(&mut p, id, apply)
        {
            p.message = e;
        }
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            p.open = false;
            p.recording = None;
            p.prepared = None;
            p.import = None;
            if let Some(job) = &p.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
        if command == Some("runner") {
            if let Some(action) = p.action.and_then(|id| p.draft.action(id)).cloned() {
                self.begin_action_runner(action);
            } else {
                p.message = "Select an action first".into();
            }
        }
        self.auto_actions = p;
    }
}
fn operation_fields(ui: &mut egui::Ui, op: &mut Operation) {
    let mut kind = match op {
        Operation::Canvas(_) => 0,
        Operation::Dpi(_) => 1,
        Operation::NewRaster { .. } => 2,
        Operation::RenameRaster { .. } => 3,
        Operation::Opacity { .. } => 4,
        Operation::Fill { .. } => 5,
        Operation::Unsupported { .. } => 6,
        Operation::Transform { .. } => 7,
        Operation::Scroll { .. } => 8,
    };
    let old = kind;
    egui::ComboBox::from_id_salt("action-command")
        .selected_text(
            [
                "Canvas",
                "DPI",
                "New raster",
                "Rename raster",
                "Layer opacity",
                "Fill selection",
                "Unavailable command",
                "Document transform",
                "View scroll in document pixels",
            ][kind],
        )
        .show_ui(ui, |ui| {
            for (i, name) in [
                "Canvas",
                "DPI",
                "New raster",
                "Rename raster",
                "Layer opacity",
                "Fill selection",
                "Unavailable command",
                "Document transform",
                "View scroll in document pixels",
            ]
            .iter()
            .enumerate()
            {
                ui.selectable_value(&mut kind, i, *name);
            }
        });
    if kind != old {
        *op = match kind {
            1 => Operation::Dpi(300.),
            2 => Operation::NewRaster {
                name: "Raster".into(),
            },
            3 => Operation::RenameRaster {
                layer: 0,
                name: "Raster".into(),
            },
            4 => Operation::Opacity {
                layer: 0,
                percent: 100,
            },
            5 => Operation::Fill {
                layer: 0,
                rgba: [0, 0, 0, 255],
                erase: false,
                alpha_lock: false,
            },
            7 => Operation::Transform {
                layer: 0,
                vector: false,
                settings: Default::default(),
                filter: drawer_core::interpolation::Filter::Nearest,
            },
            8 => Operation::Scroll { document: [0.; 2] },
            6 => Operation::Unsupported {
                command: "Unavailable".into(),
                tool: None,
            },
            _ => Operation::Canvas(Canvas::FlipHorizontal),
        };
    }
    ui.horizontal_wrapped(|ui| match op {
        Operation::Canvas(c) => {
            for (v, n) in [
                (Canvas::FlipHorizontal, "Flip H"),
                (Canvas::FlipVertical, "Flip V"),
                (Canvas::Clockwise, "90° CW"),
                (Canvas::Counterclockwise, "90° CCW"),
            ] {
                ui.selectable_value(c, v, n);
            }
        }
        Operation::Dpi(dpi) => {
            ui.label("DPI");
            ui.add(egui::DragValue::new(dpi).range(1.0..=9600.0));
        }
        Operation::NewRaster { name } => {
            ui.text_edit_singleline(name);
        }
        Operation::RenameRaster { layer, name } => {
            ui.label("Raster slot");
            ui.add(egui::DragValue::new(layer).range(0..=255));
            ui.text_edit_singleline(name);
        }
        Operation::Opacity { layer, percent } => {
            ui.label("Raster slot");
            ui.add(egui::DragValue::new(layer).range(0..=255));
            ui.add(egui::Slider::new(percent, 0..=100).text("Opacity %"));
        }
        Operation::Fill {
            layer,
            rgba,
            erase,
            alpha_lock,
        } => {
            ui.label("Raster slot");
            ui.add(egui::DragValue::new(layer).range(0..=255));
            ui.color_edit_button_srgba_unmultiplied(rgba);
            ui.checkbox(erase, "Erase");
            ui.checkbox(alpha_lock, "Lock alpha");
        }
        Operation::Transform {
            layer,
            vector,
            settings,
            filter,
        } => {
            ui.checkbox(vector, "Vector");
            ui.label("Layer slot (0-based)");
            ui.add(egui::DragValue::new(layer).range(0..=255));
            for (label, values) in [
                ("Document pivot", &mut settings.pivot),
                ("Document movement", &mut settings.translation),
                ("Scale", &mut settings.scale),
            ] {
                ui.label(label);
                for value in values {
                    ui.add(egui::DragValue::new(value).speed(0.1));
                }
            }
            ui.label("Rotation degrees");
            ui.add(egui::DragValue::new(&mut settings.degrees));
            ui.checkbox(&mut settings.preserve_width, "Preserve stroke width");
            for f in drawer_core::interpolation::Filter::ALL {
                ui.selectable_value(filter, f, format!("{f:?}"));
            }
        }
        Operation::Scroll { document } => {
            ui.label("Move viewport center by document pixels");
            for value in document {
                ui.add(egui::DragValue::new(value).range(-1e6..=1e6));
            }
            ui.small("View command only; no drawing Undo entry");
        }
        Operation::Unsupported { command, .. } => {
            ui.label(format!("Unavailable: {command}"));
        }
    });
}
#[path = "auto_actions_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}

impl Drawer {
    pub(super) fn quiesce_auto_actions(&mut self, ctx: &egui::Context) -> bool {
        let open = self.auto_actions.open;
        self.auto_actions.open = false;
        self.auto_actions_window(ctx);
        self.auto_actions.open = open;
        self.auto_actions.job.is_some()
    }
}

impl Drawer {
    pub(super) fn rebase_auto_actions(
        &mut self,
        old: &std::path::Path,
        new: &std::path::Path,
    ) -> Result<(), String> {
        let p = &mut self.auto_actions;
        for s in std::iter::once(&mut p.draft)
            .chain(std::iter::once(&mut p.saved))
            .chain(&mut p.undo)
            .chain(&mut p.redo)
        {
            *s = drawer_core::storage_migration::rebase_settings(s, old, new)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn editor_record_replay_cancel_exchange_restart() {
        let dir = crate::tests::TestDirectory::new();
        super::verify::run(&dir.0).unwrap();
    }
}
