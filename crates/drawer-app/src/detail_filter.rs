use super::*;
use drawer_core::{
    detail_filter::{self as core, Operation, Settings},
    sizing::DocumentState,
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
#[path = "pixel_effects_verify.rs"]
mod pixel_verify;
#[path = "detail_filter_verify.rs"]
mod verify;
pub(super) fn pixel_smoke(root: &std::path::Path) -> Result<(), String> {
    pixel_verify::smoke(root)
}
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
    layer: usize,
}
struct PreviewResult {
    state: DocumentState,
    candidates: Option<egui::ColorImage>,
}
struct Job {
    source: Source,
    settings: Settings,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    receiver: mpsc::Receiver<Result<PreviewResult, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
pub(super) struct Draft {
    pixel_controls: crate::pixel_effects::Controls,
    source: Source,
    settings: Settings,
    prepared: Option<(Settings, DocumentState)>,
    images: Option<[egui::TextureHandle; 2]>,
    candidates: Option<egui::TextureHandle>,
}
fn source(app: &Drawer) -> Result<Source, String> {
    let d = &app.documents[app.active.ok_or("Open a page first")?];
    let layer = match app.panels.targets.get(&d.id) {
        None => 0,
        Some(drawer_core::thumbnail::Target::Raster(n)) => *n,
        _ => return Err("Select a raster layer".into()),
    };
    Ok(Source {
        id: d.id,
        revision: d.revision,
        frame: d.current_frame,
        layer,
    })
}
impl Drawer {
    pub(super) fn detail_filter_save_guard(&self, index: usize) -> Result<(), String> {
        if self
            .detail_filter
            .draft
            .as_ref()
            .is_some_and(|e| e.source.id == self.documents[index].id)
        {
            Err("Unconfirmed filter: apply it, or cancel the filter window, before saving. Save cancelled.".into())
        } else {
            Ok(())
        }
    }
    fn cancel_detail_job(&mut self) {
        if let Some(job) = &self.detail_filter.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) fn begin_detail_filter(&mut self, operation: Operation) -> Result<(), String> {
        self.finish_stroke();
        if self.spatial_blur.draft.is_some() || self.spatial_blur.working() {
            return Err("Apply or cancel the spatial blur first".into());
        }
        if self.detail_filter.job.is_some() {
            self.cancel_detail_job();
            return Err("Previous filter is stopping; reopen shortly".into());
        }
        let source = source(self)?;
        let d = &self.documents[self.active.unwrap()];
        drawer_core::layer_properties::paintable(d, source.layer)?;
        self.detail_filter.draft = Some(Box::new(Draft {
            pixel_controls: Default::default(),
            source,
            settings: Settings {
                operation,
                selection: false,
            },
            prepared: None,
            images: None,
            candidates: None,
        }));
        Ok(())
    }
    fn preview_detail_filter(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if let Some(e) = &mut self.detail_filter.draft {
            e.prepared = None;
            e.images = None;
            e.candidates = None;
        }
        if self.detail_filter.job.is_some() {
            return Err("Wait for the previous filter to stop".into());
        }
        let current = source(self)?;
        let e = self
            .detail_filter
            .draft
            .as_ref()
            .ok_or("Filter window closed")?;
        if e.source != current || !e.settings.valid() || !e.pixel_controls.valid {
            return Err("Source changed or invalid settings; reopen filter".into());
        }
        let d = &self.documents[self.active.unwrap()];
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(7))
            .saturating_add(32 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Filter preview exceeds workspace memory".into());
        }
        let budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(used)
            .saturating_sub(d.memory_bytes());
        let state = d.snapshot();
        let settings = e.settings;
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicUsize::new(0));
        let worker_cancel = cancel.clone();
        let worker_progress = progress.clone();
        let context = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let work = || {
                if worker_cancel.load(Ordering::Relaxed) {
                    return Err("Filter cancelled".into());
                }
                let d = drawer_core::layer_merge::preview(&state, current.frame)?;
                let state = core::prepare_with_budget(
                    &d,
                    current.layer,
                    settings,
                    budget,
                    |done, total| {
                        if worker_cancel.load(Ordering::Relaxed) {
                            return Err("Filter cancelled".into());
                        }
                        worker_progress
                            .store(done.saturating_mul(100) / total.max(1), Ordering::Relaxed);
                        Ok(())
                    },
                )?;
                let candidates = if matches!(
                    settings.operation,
                    Operation::Pixels(drawer_core::pixel_effects::Operation::Dust { .. })
                ) {
                    Some(crate::pixel_effects::candidate_image(
                        &d,
                        &state,
                        current.layer,
                        &worker_cancel,
                    )?)
                } else {
                    None
                };
                Ok(PreviewResult { state, candidates })
            };
            let _ = sender.send(work());
            context.request_repaint();
        });
        self.detail_filter.job = Some(Job {
            source: current,
            settings,
            cancel,
            progress,
            receiver,
        });
        self.status = None;
        Ok(())
    }
    fn poll_detail_filter(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.detail_filter.job else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(25));
                return;
            }
            Err(_) => Err("Filter worker stopped".into()),
        };
        let job = self.detail_filter.job.take().unwrap();
        if job.cancel.load(Ordering::Relaxed) {
            return;
        }
        let current = source(self).ok();
        let Some(e) = &mut self.detail_filter.draft else {
            return;
        };
        if current != Some(job.source) || e.source != job.source || e.settings != job.settings {
            e.prepared = None;
            e.images = None;
            e.candidates = None;
            self.status = Some("Source/settings changed; prepare a new filter".into());
            return;
        }
        let result = result.and_then(|result| {
            let state = result.state;
            e.candidates = result.candidates.map(|image| {
                ctx.load_texture("speck-candidates", image, egui::TextureOptions::NEAREST)
            });
            let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
            if used.saturating_add(state.memory_bytes().saturating_mul(4)) > DEFAULT_DOCUMENT_BUDGET
            {
                return Err("Filter output exceeds workspace memory".into());
            }
            let after = drawer_core::layer_merge::preview(&state, job.source.frame)?;
            e.images = Some([
                crate::layer_merge::image(
                    ctx,
                    &self.documents[self.active.unwrap()],
                    "detail-before",
                ),
                crate::layer_merge::image(ctx, &after, "detail-after"),
            ]);
            e.prepared = Some((job.settings, state));
            Ok(())
        });
        if let Err(error) = result {
            e.prepared = None;
            e.images = None;
            e.candidates = None;
            self.status = Some(error);
        }
    }
    fn apply_detail_filter(&mut self) -> Result<(), String> {
        let current = source(self)?;
        if self.detail_filter.job.is_some() {
            return Err("Wait for the filter preview".into());
        }
        let e = self
            .detail_filter
            .draft
            .as_mut()
            .ok_or("Filter window closed")?;
        if current != e.source
            || !e.pixel_controls.valid
            || e.prepared.as_ref().is_none_or(|(s, _)| *s != e.settings)
        {
            return Err("Prepare a fresh filter preview first".into());
        }
        let (_, state) = e.prepared.take().unwrap();
        self.documents[self.active.unwrap()].commit_state(state)?;
        self.detail_filter.draft = None;
        self.status = None;
        Ok(())
    }
    pub(super) fn detail_filter_window(&mut self, ctx: &egui::Context) {
        self.poll_detail_filter(ctx);
        let Some(mut e) = self.detail_filter.draft.take() else {
            return;
        };
        let fresh = source(self).ok() == Some(e.source);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel, mut changed) = (false, false, false, false);
        let working = self.detail_filter.job.is_some();
        egui::Window::new("Detail and pixel effects").default_width(430.).default_pos(egui::pos2(35.,55.)).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Source changed; close and reopen.");}
            let old=e.settings;
            egui::ComboBox::from_id_salt("detail-operation").selected_text(e.settings.operation.label()).show_ui(ui,|ui|{for op in Operation::all(){if ui.selectable_label(op.same_kind(e.settings.operation),op.label()).clicked(){e.settings.operation=op;e.pixel_controls=Default::default();}}});
            match &mut e.settings.operation{
                Operation::Pixels(op)=>crate::pixel_effects::controls(ui,op,&mut e.pixel_controls),
                Operation::Sharpen{strength}=>{ui.add(egui::Slider::new(strength,0. ..=5.).text("Strength"));},
                Operation::Unsharp{radius,amount,threshold}=>{ui.add(egui::Slider::new(radius,0..=32).text("Radius (pixels)"));ui.add(egui::Slider::new(amount,0. ..=5.).text("Amount"));ui.add(egui::Slider::new(threshold,0. ..=1.).text("Threshold"));},
                Operation::Denoise{luminance,chroma,detail}=>{ui.add(egui::Slider::new(luminance,0. ..=1.).text("Luminance noise reduction"));ui.add(egui::Slider::new(chroma,0. ..=1.).text("Color noise reduction"));ui.add(egui::Slider::new(detail,0. ..=1.).text("Line continuity tolerance"));}
            }
            ui.checkbox(&mut e.settings.selection,"Only within the area selection");
            if ui.button("Restore last applied settings").clicked()&&let Some(s)=self.active.and_then(|n|self.documents[n].scene.last_detail_filter){e.settings=s;e.pixel_controls=Default::default();}
            changed=e.settings!=old;if changed{e.prepared=None;e.images=None;e.candidates=None;}
            let r=ui.add_enabled(fresh&&!working&&e.pixel_controls.valid,egui::Button::new("Prepare filter preview"));self.detail_filter.preview=Some(r.rect);preview=r.clicked();
            let progress=self.detail_filter.job.as_ref().map_or(0,|j|j.progress.load(Ordering::Relaxed));ui.label(if working{format!("Calculating {progress}% — cancel remains available")}else{"Preview leaves the original unchanged".into()});
            let r=ui.button("Cancel preview and close");self.detail_filter.cancel=Some(r.rect);cancel=r.clicked();
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(110.,110.)));});}if let Some(image)=&e.candidates{ui.vertical(|ui|{ui.label("Red: changes");ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(110.,110.)));});}});}
            let r=ui.add_enabled(fresh&&!working&&e.pixel_controls.valid&&e.prepared.is_some(),egui::Button::new("Apply filter as one Undo"));self.detail_filter.apply=Some(r.rect);apply=r.clicked();
            ui.small("Keeps8/16-bit precision and the document profile. Apply or cancel before saving. Escape/close discards the preview.");
        });
        if changed || !fresh || !open || cancel {
            self.cancel_detail_job();
        }
        if open && !cancel {
            self.detail_filter.draft = Some(e);
        }
        if preview && let Err(error) = self.preview_detail_filter(ctx) {
            self.status = Some(error);
        }
        if apply && let Err(error) = self.apply_detail_filter() {
            self.status = Some(error);
        }
    }
}
