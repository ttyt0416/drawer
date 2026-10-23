use super::*;
use drawer_core::{
    batch_filters::{self as core, Task},
    layer_properties::{self as properties, Target},
    sizing::DocumentState,
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
#[path = "batch_filters_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)
}
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    job: Option<Job>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    cancel: Option<egui::Rect>,
    targets: std::collections::BTreeMap<Target, egui::Rect>,
}
impl Panel {
    pub(super) fn working(&self) -> bool {
        self.job.is_some()
    }
}
#[derive(Clone, Copy, PartialEq)]
struct Source {
    id: u64,
    revision: u64,
    frame: u32,
}
#[derive(Clone, PartialEq)]
struct Config {
    task: Task,
    targets: Vec<Target>,
}
pub(super) struct Draft {
    source: Source,
    config: Config,
    controls: crate::pixel_effects::Controls,
    prepared: Option<(Config, DocumentState)>,
    images: Option<[egui::TextureHandle; 2]>,
}
struct ResultPreview {
    state: DocumentState,
    images: [egui::ColorImage; 2],
}
struct Job {
    source: Source,
    config: Config,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    receiver: mpsc::Receiver<Result<ResultPreview, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn source(app: &Drawer) -> Result<Source, String> {
    let d = &app.documents[app.active.ok_or("Open a page first")?];
    Ok(Source {
        id: d.id,
        revision: d.revision,
        frame: d.current_frame,
    })
}
impl Drawer {
    pub(super) fn batch_filters_save_guard(&self, index: usize) -> Result<(), String> {
        if self
            .batch_filters
            .draft
            .as_ref()
            .is_some_and(|e| e.source.id == self.documents[index].id)
        {
            Err(
                "Unconfirmed batch filter: Apply all or Cancel before saving. Save cancelled."
                    .into(),
            )
        } else {
            Ok(())
        }
    }
    fn cancel_batch_job(&mut self) {
        if let Some(job) = &self.batch_filters.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) fn begin_batch_filters(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed() || self.inference_unconfirmed() {
            return Err("Apply or cancel the transform first".into());
        }
        if self.local_color.draft.is_some()
            || self.local_color.working()
            || self.surface_effects.draft.is_some()
            || self.surface_effects.working()
            || self.spatial_blur.draft.is_some()
            || self.spatial_blur.working()
            || self.detail_filter.draft.is_some()
            || self.detail_filter.working()
        {
            return Err("Apply or cancel the other filter first".into());
        }
        if self.batch_filters.working() {
            self.cancel_batch_job();
            return Err("Previous batch is stopping; reopen shortly".into());
        }
        let source = source(self)?;
        let d = &self.documents[self.active.unwrap()];
        let target = properties::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        );
        self.batch_filters.draft = Some(Box::new(Draft {
            source,
            config: Config {
                task: d
                    .scene
                    .last_batch_filter
                    .as_ref()
                    .map_or_else(Task::default, |r| r.task),
                targets: vec![target],
            },
            controls: Default::default(),
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_batch_filters(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if let Some(e) = &mut self.batch_filters.draft {
            e.prepared = None;
            e.images = None;
        }
        if self.batch_filters.working() {
            return Err("Wait for the previous batch to stop".into());
        }
        let current = source(self)?;
        let e = self
            .batch_filters
            .draft
            .as_ref()
            .ok_or("Batch window closed")?;
        if current != e.source || !e.config.task.valid() || !e.controls.valid {
            return Err("Source changed or invalid batch parameters".into());
        }
        let d = &self.documents[self.active.unwrap()];
        let report = core::inspect_for(d, &e.config.targets, e.config.task);
        if e.config.targets.is_empty() || report.iter().any(|s| s.reason.is_some()) {
            return Err(
                "Resolve the selected target list before calculating; no layers processed".into(),
            );
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(7))
            .saturating_add(160 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Batch preview exceeds workspace memory".into());
        }
        let budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(used)
            .saturating_sub(d.memory_bytes());
        let state = d.snapshot();
        let config = e.config.clone();
        let worker_config = config.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicUsize::new(0));
        let wc = cancel.clone();
        let wp = progress.clone();
        let context = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let work = || {
                let d = drawer_core::layer_merge::preview(&state, current.frame)?;
                let state = core::prepare(
                    &d,
                    &worker_config.targets,
                    worker_config.task,
                    budget,
                    |done, total| {
                        if wc.load(Ordering::Relaxed) {
                            return Err("Whole batch cancelled".into());
                        }
                        wp.store(done.saturating_mul(100) / total.max(1), Ordering::Relaxed);
                        Ok(())
                    },
                )?;
                let after = drawer_core::layer_merge::preview(&state, current.frame)?;
                let images = [
                    crate::surface_effects::thumbnail(&d, &wc)?,
                    crate::surface_effects::thumbnail(&after, &wc)?,
                ];
                Ok(ResultPreview { state, images })
            };
            let _ = sender.send(work());
            context.request_repaint();
        });
        self.batch_filters.job = Some(Job {
            source: current,
            config,
            cancel,
            progress,
            receiver,
        });
        self.status = None;
        Ok(())
    }
    fn poll_batch_filters(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.batch_filters.job else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(25));
                return;
            }
            Err(_) => Err("Batch worker stopped".into()),
        };
        let job = self.batch_filters.job.take().unwrap();
        if job.cancel.load(Ordering::Relaxed) {
            return;
        }
        let fresh = source(self).ok() == Some(job.source);
        let Some(e) = &mut self.batch_filters.draft else {
            return;
        };
        if !fresh || e.source != job.source || e.config != job.config {
            e.prepared = None;
            e.images = None;
            self.status =
                Some("Source/targets/settings changed; prepare another batch preview".into());
            return;
        }
        match result {
            Ok(result) => {
                let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
                if used.saturating_add(result.state.memory_bytes().saturating_mul(4))
                    > DEFAULT_DOCUMENT_BUDGET
                {
                    self.status = Some("Batch output exceeds workspace budget".into());
                    return;
                }
                e.images = Some(result.images.map(|image| {
                    ctx.load_texture("batch-preview", image, egui::TextureOptions::NEAREST)
                }));
                e.prepared = Some((job.config.clone(), result.state));
            }
            Err(error) => {
                e.prepared = None;
                e.images = None;
                self.status = Some(error);
            }
        }
    }
    fn apply_batch_filters(&mut self) -> Result<(), String> {
        let current = source(self)?;
        if self.batch_filters.working() {
            return Err("Wait for the batch preview".into());
        }
        let e = self
            .batch_filters
            .draft
            .as_mut()
            .ok_or("Batch window closed")?;
        if current != e.source
            || !e.controls.valid
            || e.prepared.as_ref().is_none_or(|(c, _)| *c != e.config)
        {
            return Err("Prepare a fresh whole-batch preview first".into());
        }
        let (config, state) = e.prepared.take().unwrap();
        let n = self.active.unwrap();
        let changed = state != self.documents[n].snapshot();
        self.documents[n].commit_state(state)?;
        self.batch_filters.draft = None;
        self.status = None;
        if changed && matches!(config.task, Task::Blur(_) | Task::Surface { .. }) {
            let blur = self.workspaces.store.current.spatial_blur.clone();
            let surface = self.workspaces.store.current.surface_effects.clone();
            match config.task {
                Task::Blur(s) => {
                    self.workspaces.store.current.spatial_blur.values[s.mode.index()] = Some(s)
                }
                Task::Surface { operation, .. } => {
                    self.workspaces.store.current.surface_effects.values[operation.index()] =
                        Some(operation)
                }
                _ => {}
            }
            if let Err(error) = self.persist_workspace() {
                self.workspaces.store.current.spatial_blur = blur;
                self.workspaces.store.current.surface_effects = surface;
                return Err(format!(
                    "Batch applied; previous local tool settings retained because saving failed: {error}"
                ));
            }
        }
        Ok(())
    }
    pub(super) fn batch_filters_window(&mut self, ctx: &egui::Context) {
        self.poll_batch_filters(ctx);
        let Some(mut e) = self.batch_filters.draft.take() else {
            return;
        };
        let Some(index) = self.active.filter(|n| self.documents[*n].id == e.source.id) else {
            self.cancel_batch_job();
            return;
        };
        let fresh = source(self).ok() == Some(e.source);
        let working = self.batch_filters.working();
        let old = e.config.clone();
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel) = (false, false, false);
        let d = &self.documents[index];
        let statuses = core::inspect_for(d, &properties::targets(d), e.config.task);
        let mut colors = self.workspaces.store.current.color_modes;
        colors.observe(self.references.session.workspace.color);
        self.batch_filters.targets.clear();
        egui::Window::new("Multiple-layer filter preview").default_width(560.).default_pos(egui::pos2(25.,15.)).collapsible(false).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Source changed; cancel and reopen.");}
            ui.label("Choose targets before calculation. Checked locked/unsupported targets block the entire batch.");
            egui::ScrollArea::vertical().id_salt("batch-targets").max_height(100.).show_rows(ui,22.,statuses.len(),|ui,rows|{for row in rows{let s=&statuses[row];let mut checked=e.config.targets.contains(&s.target);let reason=s.reason.as_deref().unwrap_or("Ready");let r=ui.checkbox(&mut checked,format!("{} · {reason}",crate::layer_properties::label(d,s.target)));self.batch_filters.targets.insert(s.target,r.rect);if r.changed(){if checked{e.config.targets.push(s.target);}else{e.config.targets.retain(|t|*t!=s.target);}}}});
            egui::ScrollArea::vertical().id_salt("batch-parameters").max_height(235.).show(ui,|ui|{
                let choices=[Task::default(),Task::Blur(Default::default()),Task::Gaussian{radius:3,selection:false},Task::Surface{operation:drawer_core::surface_effects::Operation::defaults()[0],selection:false}];
                egui::ComboBox::from_id_salt("batch-kind").selected_text(e.config.task.label()).show_ui(ui,|ui|{for task in choices{if ui.selectable_label(std::mem::discriminant(&task)==std::mem::discriminant(&e.config.task),task.label()).clicked(){e.config.task=task;e.controls=Default::default();}}});
                match &mut e.config.task{
                    Task::Detail(settings)=>{egui::ComboBox::from_id_salt("batch-detail").selected_text(settings.operation.label()).show_ui(ui,|ui|{for op in drawer_core::detail_filter::Operation::all(){if ui.selectable_label(op.same_kind(settings.operation),op.label()).clicked(){settings.operation=op;settings.options=Default::default();e.controls=Default::default();}}});crate::detail_filter::controls(ui,settings,&mut e.controls);},
                    Task::Blur(settings)=>{egui::ComboBox::from_id_salt("batch-blur").selected_text(settings.mode.label()).show_ui(ui,|ui|{for mode in drawer_core::spatial_blur::Mode::ALL{if ui.selectable_label(settings.mode==mode,mode.label()).clicked(){*settings=self.workspaces.store.current.spatial_blur.get(mode,[d.width,d.height]);}}});crate::spatial_blur::controls(ui,settings);ui.add(egui::Slider::new(&mut settings.amount,0. ..=1.).text("Blend amount"));},
                    Task::Gaussian{radius,..}=>{ui.add(egui::Slider::new(radius,0..=64).text("Gaussian radius"));},
                    Task::Surface{operation,..}=>{egui::ComboBox::from_id_salt("batch-surface").selected_text(operation.label()).show_ui(ui,|ui|{for op in drawer_core::surface_effects::Operation::defaults(){if ui.selectable_label(op.index()==operation.index(),op.label()).clicked(){*operation=self.workspaces.store.current.surface_effects.get(op.index());e.controls=Default::default();}}});crate::surface_effects::controls(ui,operation,&mut e.controls,colors.slots);},
                }
                ui.checkbox(e.config.task.selection(),"Limit every target to a frozen snapshot of the current area selection");
            });
            if e.config!=old{e.prepared=None;e.images=None;}
            let eligible=!e.config.targets.is_empty()&&core::inspect_for(d,&e.config.targets,e.config.task).iter().all(|s|s.reason.is_none())&&e.config.task.valid()&&e.controls.valid;
            ui.label(format!("{} selected targets · {}",e.config.targets.len(),if eligible{"Ready to preview"}else{"Resolve target list / parameters first"}));
            let r=ui.add_enabled(fresh&&!working&&eligible,egui::Button::new("Prepare all targets"));self.batch_filters.preview=Some(r.rect);preview=r.clicked();
            let r=ui.add_enabled(fresh&&!working&&eligible&&e.prepared.is_some(),egui::Button::new("Apply all as one Undo"));self.batch_filters.apply=Some(r.rect);apply=r.clicked();
            let r=ui.button("Cancel entire batch and close");self.batch_filters.cancel=Some(r.rect);cancel=r.clicked();
            if working{ui.label(format!("Calculating {}%",self.batch_filters.job.as_ref().unwrap().progress.load(Ordering::Relaxed)));}
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in["Original page","All targets after"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(100.,100.)));});}});}
            ui.small("No source layer changes during calculation. Failure, Escape or Cancel discards every result. Editable effects are added to each target's saved effect list. Saving requires Apply all or Cancel.");
        });
        if e.config != old || !fresh || !open || cancel {
            self.cancel_batch_job();
        }
        if open && !cancel {
            self.batch_filters.draft = Some(e);
        }
        if preview && let Err(error) = self.preview_batch_filters(ctx) {
            self.status = Some(error);
        }
        if apply && let Err(error) = self.apply_batch_filters() {
            self.status = Some(error);
        }
    }
}
