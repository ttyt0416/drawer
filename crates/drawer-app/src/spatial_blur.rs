use super::*;
use drawer_core::{
    sizing::DocumentState,
    spatial_blur::{self as core, Mode, Settings},
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
#[path = "spatial_blur_verify.rs"]
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
struct Job {
    source: Source,
    settings: Settings,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    receiver: mpsc::Receiver<Result<DocumentState, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
pub(super) struct Draft {
    source: Source,
    settings: Settings,
    prepared: Option<(Settings, DocumentState)>,
    images: Option<[egui::TextureHandle; 2]>,
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
    pub(super) fn spatial_blur_save_guard(&self, index: usize) -> Result<(), String> {
        if self
            .spatial_blur
            .draft
            .as_ref()
            .is_some_and(|e| e.source.id == self.documents[index].id)
        {
            Err("Unconfirmed blur: apply it, or cancel the blur window, before saving. Save cancelled.".into())
        } else {
            Ok(())
        }
    }
    fn cancel_spatial_job(&mut self) {
        if let Some(job) = &self.spatial_blur.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) fn begin_spatial_blur(&mut self, mode: Mode) -> Result<(), String> {
        self.finish_stroke();
        if self.local_color.draft.is_some()
            || self.local_color.working()
            || self.batch_filters.draft.is_some()
            || self.batch_filters.working()
        {
            return Err("Apply or cancel the other correction/filter first".into());
        }
        if self.surface_effects.draft.is_some() || self.surface_effects.working() {
            return Err("Apply or cancel surface effects first".into());
        }
        if self.detail_filter.draft.is_some() || self.detail_filter.working() {
            return Err("Apply or cancel the detail filter first".into());
        }
        if self.spatial_blur.job.is_some() {
            self.cancel_spatial_job();
            return Err("Previous blur is stopping; reopen shortly".into());
        }
        let source = source(self)?;
        let d = &self.documents[self.active.unwrap()];
        drawer_core::layer_properties::paintable(d, source.layer)?;
        self.spatial_blur.draft = Some(Box::new(Draft {
            source,
            settings: self
                .workspaces
                .store
                .current
                .spatial_blur
                .get(mode, [d.width, d.height]),
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_spatial_blur(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if let Some(e) = &mut self.spatial_blur.draft {
            e.prepared = None;
            e.images = None;
        }
        if self.spatial_blur.job.is_some() {
            return Err("Wait for the previous blur to stop".into());
        }
        let current = source(self)?;
        let e = self
            .spatial_blur
            .draft
            .as_ref()
            .ok_or("Blur window closed")?;
        if e.source != current || !e.settings.valid() {
            return Err("Source changed or invalid settings; reopen blur".into());
        }
        let d = &self.documents[self.active.unwrap()];
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(7))
            .saturating_add(32 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Blur preview exceeds workspace memory".into());
        }
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
                    return Err("Blur cancelled".into());
                }
                let d = drawer_core::layer_merge::preview(&state, current.frame)?;
                core::prepare(&d, current.layer, settings, |done, total| {
                    if worker_cancel.load(Ordering::Relaxed) {
                        return Err("Blur cancelled".into());
                    }
                    worker_progress
                        .store(done.saturating_mul(100) / total.max(1), Ordering::Relaxed);
                    Ok(())
                })
            };
            let _ = sender.send(work());
            context.request_repaint();
        });
        self.spatial_blur.job = Some(Job {
            source: current,
            settings,
            cancel,
            progress,
            receiver,
        });
        self.status = None;
        Ok(())
    }
    fn poll_spatial_blur(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.spatial_blur.job else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(25));
                return;
            }
            Err(_) => Err("Blur worker stopped".into()),
        };
        let job = self.spatial_blur.job.take().unwrap();
        if job.cancel.load(Ordering::Relaxed) {
            return;
        }
        let current = source(self).ok();
        let Some(e) = &mut self.spatial_blur.draft else {
            return;
        };
        if current != Some(job.source) || e.source != job.source || e.settings != job.settings {
            e.prepared = None;
            e.images = None;
            self.status = Some("Source/settings changed; prepare a new blur".into());
            return;
        }
        let result = result.and_then(|state| {
            let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
            if used.saturating_add(state.memory_bytes().saturating_mul(4)) > DEFAULT_DOCUMENT_BUDGET
            {
                return Err("Blur output exceeds workspace memory".into());
            }
            let after = drawer_core::layer_merge::preview(&state, job.source.frame)?;
            e.images = Some([
                crate::layer_merge::image(
                    ctx,
                    &self.documents[self.active.unwrap()],
                    "spatial-before",
                ),
                crate::layer_merge::image(ctx, &after, "spatial-after"),
            ]);
            e.prepared = Some((job.settings, state));
            Ok(())
        });
        if let Err(error) = result {
            e.prepared = None;
            e.images = None;
            self.status = Some(error);
        }
    }
    fn apply_spatial_blur(&mut self) -> Result<(), String> {
        let current = source(self)?;
        if self.spatial_blur.job.is_some() {
            return Err("Wait for the blur preview".into());
        }
        let e = self
            .spatial_blur
            .draft
            .as_mut()
            .ok_or("Blur window closed")?;
        if current != e.source || e.prepared.as_ref().is_none_or(|(s, _)| *s != e.settings) {
            return Err("Prepare a fresh blur preview first".into());
        }
        let (settings, state) = e.prepared.take().unwrap();
        let n = self.active.unwrap();
        let changed = state != self.documents[n].snapshot();
        self.documents[n].commit_state(state)?;
        self.spatial_blur.draft = None;
        self.status = None;
        if changed {
            let old = self.workspaces.store.current.spatial_blur.clone();
            self.workspaces.store.current.spatial_blur.values[settings.mode.index()] =
                Some(settings);
            if let Err(error) = self.persist_workspace() {
                self.workspaces.store.current.spatial_blur = old;
                return Err(format!(
                    "Blur applied; local preset could not be saved: {error}"
                ));
            }
        }
        Ok(())
    }
    pub(super) fn spatial_blur_window(&mut self, ctx: &egui::Context) {
        self.poll_spatial_blur(ctx);
        let Some(mut e) = self.spatial_blur.draft.take() else {
            return;
        };
        let fresh = source(self).ok() == Some(e.source);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel, mut changed) = (false, false, false, false);
        let working = self.spatial_blur.job.is_some();
        egui::Window::new("Motion / radial / spin / lens blur").default_width(430.).default_pos(egui::pos2(35.,55.)).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Source changed; close and reopen.");}
            let old=e.settings;
            egui::ComboBox::from_id_salt("spatial-mode").selected_text(e.settings.mode.label()).show_ui(ui,|ui|{for mode in Mode::ALL{if ui.selectable_label(mode==e.settings.mode,mode.label()).clicked(){let size=self.active.map(|n|[self.documents[n].width,self.documents[n].height]).unwrap_or([1;2]);e.settings=self.workspaces.store.current.spatial_blur.get(mode,size);}}});
            controls(ui,&mut e.settings);
            ui.add(egui::Slider::new(&mut e.settings.amount,0. ..=1.).text("Blend amount"));ui.checkbox(&mut e.settings.selection,"Only within the area selection");
            if ui.button("Restore last applied settings").clicked()&&let Some(s)=self.active.and_then(|n|self.documents[n].scene.last_spatial_blur){e.settings=s;}
            changed=e.settings!=old;
            if changed{e.prepared=None;e.images=None;}
            let r=ui.add_enabled(fresh&&!working,egui::Button::new("Prepare blur preview"));self.spatial_blur.preview=Some(r.rect);preview=r.clicked();
            let progress=self.spatial_blur.job.as_ref().map_or(0,|j|j.progress.load(Ordering::Relaxed));
            ui.label(if working{format!("Calculating {progress}% — cancel remains available")}else{"Preview leaves the original unchanged".into()});
            let r=ui.button("Cancel preview and close");self.spatial_blur.cancel=Some(r.rect);cancel=r.clicked();
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(110.,110.)));});}});}
            let r=ui.add_enabled(fresh&&!working&&e.prepared.is_some(),egui::Button::new("Apply blur as one Undo"));self.spatial_blur.apply=Some(r.rect);apply=r.clicked();
            ui.small("Escape/close discards the preview. Settings are saved locally after Apply. Length/radius0 and spin0 leave pixels unchanged. Large sources/radii may exceed the processing limit.");
        });
        if changed || !fresh || !open || cancel {
            self.cancel_spatial_job();
        }
        if open && !cancel {
            self.spatial_blur.draft = Some(e);
        }
        if preview && let Err(error) = self.preview_spatial_blur(ctx) {
            self.status = Some(error);
        }
        if apply && let Err(error) = self.apply_spatial_blur() {
            self.status = Some(error);
        }
    }
}

pub(super) fn controls(ui: &mut egui::Ui, settings: &mut Settings) {
    match settings.mode {
        Mode::Motion => {
            ui.add(egui::Slider::new(&mut settings.length, 0. ..=128.).text("Length (pixels)"));
            ui.add(
                egui::DragValue::new(&mut settings.angle)
                    .range(-36000. ..=36000.)
                    .prefix("Angle ° "),
            );
        }
        Mode::Radial | Mode::Spin => {
            if settings.mode == Mode::Radial {
                ui.add(
                    egui::Slider::new(&mut settings.radius, 0. ..=64.)
                        .text("Radial length (pixels)"),
                );
            } else {
                ui.add(egui::Slider::new(&mut settings.angle, -360. ..=360.).text("Spin angle °"));
            }
            ui.horizontal(|ui| {
                ui.label("Center X / Y");
                for v in &mut settings.center {
                    ui.add(egui::DragValue::new(v).range(-1e6..=1e6));
                }
            });
        }
        Mode::Lens => {
            ui.add(
                egui::Slider::new(&mut settings.radius, 0. ..=64.).text("Aperture radius (pixels)"),
            );
            ui.add(egui::Slider::new(&mut settings.sides, 3..=8).text("Aperture sides"));
            ui.add(egui::Slider::new(&mut settings.roundness, 0. ..=1.).text("Roundness"));
            ui.add(
                egui::DragValue::new(&mut settings.angle)
                    .range(-36000. ..=36000.)
                    .prefix("Rotation ° "),
            );
            ui.add(egui::Slider::new(&mut settings.highlight, 0. ..=4.).text("Highlight boost"));
        }
    }
}
