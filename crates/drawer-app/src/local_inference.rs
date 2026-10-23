use super::*;
use drawer_core::{
    local_inference::{self as core, Output, Task},
    references::LocalImage,
    sizing::DocumentState,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    job: Option<Job>,
    input: Option<egui::Rect>,
    run: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
#[derive(Clone, Copy, PartialEq)]
struct Source {
    id: u64,
    revision: u64,
    frame: u32,
}
#[derive(Clone, Copy, Default, PartialEq)]
enum InputSource {
    #[default]
    Composite,
    Selection,
    File,
    Comparison,
}
#[derive(Clone, Default, PartialEq)]
struct Config {
    task: Task,
    source: InputSource,
    path: String,
    page: usize,
    manifest: String,
    hints: String,
}
#[derive(Clone)]
struct Frozen {
    config: Config,
    image: LocalImage,
    hints: Option<LocalImage>,
}
struct Draft {
    source: Source,
    config: Config,
    input: Option<Frozen>,
    images: Option<[egui::TextureHandle; 2]>,
    prepared: Option<(Config, DocumentState)>,
    result: Option<Output>,
    threshold: f32,
    accept_low: bool,
}
enum Preview {
    Input(Frozen),
    Result(Output, Box<DocumentState>),
}
struct Job {
    source: Source,
    config: Config,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<Preview, String>>,
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
fn texture(ctx: &egui::Context, i: &LocalImage) -> egui::TextureHandle {
    let scale = (128. / i.width.max(i.height) as f32).min(1.);
    let w = (i.width as f32 * scale).round().max(1.) as usize;
    let h = (i.height as f32 * scale).round().max(1.) as usize;
    let pixels = (0..h)
        .flat_map(|y| {
            (0..w).map(move |x| {
                let p = i.pixels
                    [(y * i.height as usize / h) * i.width as usize + x * i.width as usize / w];
                egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])
            })
        })
        .collect();
    ctx.load_texture(
        "local-inference",
        egui::ColorImage::new([w, h], pixels),
        egui::TextureOptions::NEAREST,
    )
}
fn read(path: &str, page: usize, cancel: &AtomicBool) -> Result<LocalImage, String> {
    let s = drawer_core::image_import::ImageSource::read(Path::new(path), 64 * 1024 * 1024, cancel)
        .map_err(|e| e.to_string())?;
    let info = s
        .pages
        .get(page.saturating_sub(1))
        .ok_or("Choose an existing image page")?;
    if info.width as usize * info.height as usize > core::MAX_INPUT {
        return Err("Input image exceeds 1,048,576 pixels; crop or resize first".into());
    }
    Ok(
        s.decode(page.saturating_sub(1), true, 64 * 1024 * 1024, cancel)
            .map_err(|e| e.to_string())?
            .image,
    )
}
fn executable() -> Result<PathBuf, String> {
    let p = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(test)]
    {
        Ok(p.parent().unwrap().parent().unwrap().join("drawer-app.exe"))
    }
    #[cfg(not(test))]
    {
        Ok(p)
    }
}
impl Drawer {
    pub(super) fn inference_save_guard(&self, index: usize) -> Result<(), String> {
        if self
            .local_inference
            .draft
            .as_ref()
            .is_some_and(|e| e.source.id == self.documents[index].id)
        {
            Err(
                "Unconfirmed local inference: Adopt or Cancel before saving. Save cancelled."
                    .into(),
            )
        } else {
            Ok(())
        }
    }
    fn cancel_inference(&mut self) {
        if let Some(j) = &self.local_inference.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) fn inference_unconfirmed(&self) -> bool {
        self.local_inference.draft.is_some() || self.local_inference.job.is_some()
    }
    pub(super) fn begin_local_inference(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.local_inference.job.is_some() {
            self.cancel_inference();
            return Err("Previous inference is stopping; reopen shortly".into());
        }
        if self.transform_unconfirmed()
            || self.local_color.working()
            || self.batch_filters.working()
            || self.detail_filter.working()
            || self.spatial_blur.working()
            || self.surface_effects.working()
            || self.local_color.draft.is_some()
            || self.batch_filters.draft.is_some()
            || self.detail_filter.draft.is_some()
            || self.spatial_blur.draft.is_some()
            || self.surface_effects.draft.is_some()
            || self.adjustments.draft.is_some()
        {
            return Err("Apply or cancel the other correction/transform first".into());
        }
        self.local_inference.draft = Some(Box::new(Draft {
            source: source(self)?,
            config: Config {
                page: 1,
                ..Default::default()
            },
            input: None,
            images: None,
            prepared: None,
            result: None,
            threshold: 0.5,
            accept_low: false,
        }));
        Ok(())
    }
    fn start_inference(&mut self, ctx: &egui::Context, run: bool) -> Result<(), String> {
        if self.local_inference.job.is_some() {
            return Err("Wait for the previous local task to stop".into());
        }
        let current = source(self)?;
        let e = self
            .local_inference
            .draft
            .as_mut()
            .ok_or("Local inference window closed")?;
        e.prepared = None;
        e.result = None;
        e.accept_low = false;
        if current != e.source {
            e.input = None;
            e.images = None;
            return Err("Source changed; reopen and select a fresh input".into());
        }
        let frozen = if run {
            Some(
                e.input
                    .clone()
                    .filter(|f| f.config == e.config)
                    .ok_or("Preview and confirm the input first")?,
            )
        } else {
            e.input = None;
            e.images = None;
            None
        };
        let cfg = e.config.clone();
        let d = &self.documents[self.active.unwrap()];
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(7))
            .saturating_add(96 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Insufficient workspace memory for local inference preview".into());
        }
        let snapshot = d.snapshot();
        let exe = executable()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let config = cfg.clone();
        let (tx, rx) = mpsc::channel();
        let context = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                core::checked(&c)?;
                if !run {
                    let (image, hints) = if config.source == InputSource::Comparison {
                        drawer_infer::comparison::input(config.task)
                    } else {
                        let image = if config.source == InputSource::File {
                            read(&config.path, config.page, &c)?
                        } else {
                            let d = drawer_core::layer_merge::preview(&snapshot, current.frame)?;
                            core::snapshot(&d, config.source == InputSource::Selection, &c)?
                        };
                        if image.pixels.iter().all(|p| p[3] == 0) {
                            return Err(
                                "Input/selection is fully transparent; choose visible pixels"
                                    .into(),
                            );
                        }
                        let hints = if config.task == Task::HintColor {
                            Some(read(&config.hints, 1, &c)?)
                        } else {
                            None
                        };
                        if hints.as_ref().is_some_and(|h| {
                            h.width != image.width
                                || h.height != image.height
                                || h.pixels.iter().all(|p| p[3] == 0)
                        }) {
                            return Err(
                                "Choose a same-size RGBA hint image with visible hints".into()
                            );
                        }
                        (image, hints)
                    };
                    core::checked(&c)?;
                    Ok(Preview::Input(Frozen {
                        config: config.clone(),
                        image,
                        hints,
                    }))
                } else {
                    let input = frozen.unwrap();
                    let mut out = drawer_infer::execute(
                        &std::env::temp_dir(),
                        &exe,
                        &drawer_infer::Request {
                            manifest: config.manifest.clone(),
                            task: config.task,
                        },
                        &input.image,
                        input.hints.as_ref(),
                        &c,
                        Duration::from_secs(120),
                    )?;
                    core::checked(&c)?;
                    let d = if config.source == InputSource::Comparison {
                        out.report.push_str(&format!(
                            "\n{}",
                            drawer_infer::comparison::report(config.task, &out)?
                        ));
                        input
                            .image
                            .document(DEFAULT_DOCUMENT_BUDGET)
                            .map_err(|e| e.to_string())?
                    } else {
                        drawer_core::layer_merge::preview(&snapshot, current.frame)?
                    };
                    // Preparation may contain uncertain joints. Actual adoption always rechecks consent.
                    let state = core::adopt(&d, &input.image, &out, 0.5, true)?;
                    core::checked(&c)?;
                    Ok(Preview::Result(out, Box::new(state)))
                }
            })();
            let _ = tx.send(result);
            context.request_repaint();
        });
        self.local_inference.job = Some(Job {
            source: current,
            config: cfg,
            cancel,
            receiver: rx,
        });
        self.status = None;
        Ok(())
    }
    fn poll_inference(&mut self, ctx: &egui::Context) {
        let Some(j) = &self.local_inference.job else {
            return;
        };
        let result = match j.receiver.try_recv() {
            Ok(v) => v,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(25));
                return;
            }
            Err(_) => Err("Local inference worker stopped".into()),
        };
        let j = self.local_inference.job.take().unwrap();
        if j.cancel.load(Ordering::Relaxed) {
            return;
        }
        let fresh = source(self).ok() == Some(j.source);
        let Some(e) = &mut self.local_inference.draft else {
            return;
        };
        if !fresh || e.source != j.source || e.config != j.config {
            e.prepared = None;
            e.result = None;
            e.input = None;
            e.images = None;
            self.status = Some("Page/frame/settings changed; stale local result discarded".into());
            return;
        }
        match result {
            Ok(Preview::Input(input)) => {
                let image = texture(ctx, &input.image);
                e.images = Some([image.clone(), image]);
                e.input = Some(input);
            }
            Ok(Preview::Result(out, state)) => {
                let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
                if used.saturating_add(state.memory_bytes().saturating_mul(4))
                    > DEFAULT_DOCUMENT_BUDGET
                {
                    self.status = Some("Inference output exceeds workspace memory".into());
                    return;
                }
                let preview = if let Some(image) = &out.image {
                    Ok(texture(ctx, image))
                } else {
                    drawer_core::layer_merge::preview(&state, j.source.frame).and_then(|d| {
                        let c = AtomicBool::new(false);
                        super::surface_effects::thumbnail(&d, &c).map(|i| {
                            ctx.load_texture("local-pose-result", i, egui::TextureOptions::NEAREST)
                        })
                    })
                };
                match preview {
                    Ok(image) => {
                        if let Some(images) = &mut e.images {
                            images[1] = image;
                        }
                        e.prepared = if j.config.source == InputSource::Comparison {
                            None
                        } else {
                            Some((j.config.clone(), *state))
                        };
                        e.result = Some(out);
                    }
                    Err(err) => self.status = Some(err),
                }
            }
            Err(err) => {
                e.prepared = None;
                e.result = None;
                self.status = Some(err);
            }
        }
    }
    fn adopt_inference(&mut self) -> Result<(), String> {
        let current = source(self)?;
        if self.local_inference.job.is_some() {
            return Err("Wait for the local preview".into());
        }
        let e = self
            .local_inference
            .draft
            .as_ref()
            .ok_or("Local inference window closed")?;
        if e.config.source == InputSource::Comparison
            || current != e.source
            || e.prepared.as_ref().is_none_or(|(cfg, _)| *cfg != e.config)
        {
            return Err("Prepare a fresh local inference preview".into());
        }
        let result = e.result.as_ref().ok_or("Inference result missing")?;
        // Only pose warning colors depend on the review threshold. Image preparation was done in the worker.
        let index = self.active.unwrap();
        let state = if result.image.is_some() {
            if result.low_confidence(e.threshold) && !e.accept_low {
                return Err("Review uncertain output first".into());
            }
            e.prepared.as_ref().unwrap().1.clone()
        } else {
            core::adopt(
                &self.documents[index],
                &e.input.as_ref().ok_or("Input missing")?.image,
                result,
                e.threshold,
                e.accept_low,
            )?
        };
        self.documents[index].commit_state(state)?;
        self.local_inference.draft = None;
        self.sync_layer_identity();
        self.status = None;
        Ok(())
    }
    pub(super) fn local_inference_window(&mut self, ctx: &egui::Context) {
        self.poll_inference(ctx);
        let Some(mut e) = self.local_inference.draft.take() else {
            return;
        };
        if self
            .active
            .is_none_or(|n| self.documents[n].id != e.source.id)
        {
            self.cancel_inference();
            return;
        }
        let fresh = source(self).ok() == Some(e.source);
        let old = e.config.clone();
        let busy = self.local_inference.job.is_some();
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut input, mut run, mut apply, mut cancel) = (false, false, false, false);
        egui::Window::new("Local models — pose / color / restoration").default_width(550.).vscroll(true).max_height(660.).default_pos(egui::pos2(25.,10.)).collapsible(false).open(&mut open).show(ctx,|ui|{
            ui.small(drawer_infer::GUIDE);
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Page/frame changed. Cancel and reopen to freeze a new input.");}
            ui.add_enabled_ui(!busy,|ui|{
                egui::ComboBox::from_label("Task").selected_text(e.config.task.label()).show_ui(ui,|ui|{for task in Task::ALL{ui.selectable_value(&mut e.config.task,task,task.label());}});
                ui.horizontal(|ui|{ui.radio_value(&mut e.config.source,InputSource::Composite,"Page composite");ui.radio_value(&mut e.config.source,InputSource::Selection,"Selected pixels");ui.radio_value(&mut e.config.source,InputSource::File,"Local file");ui.radio_value(&mut e.config.source,InputSource::Comparison,"Fixed comparison");});
                if e.config.source==InputSource::File{ui.text_edit_singleline(&mut e.config.path);if ui.button("Choose local input…").clicked()&&let Some(p)=rfd::FileDialog::new().add_filter("Local images",&["png","jpg","jpeg","tif","tiff","bmp"]).pick_file(){e.config.path=p.to_string_lossy().into_owned();}ui.add(egui::DragValue::new(&mut e.config.page).range(1..=256).prefix("TIFF page "));}
                if e.config.task==Task::HintColor && e.config.source!=InputSource::Comparison{ui.label("RGBA hints: same size; alpha marks hint strength");ui.text_edit_singleline(&mut e.config.hints);if ui.button("Choose local hints…").clicked()&&let Some(p)=rfd::FileDialog::new().add_filter("Hint image",&["png","tif","tiff"]).pick_file(){e.config.hints=p.to_string_lossy().into_owned();}}
                ui.label("Local model bundle manifest (no automatic installation)");ui.text_edit_singleline(&mut e.config.manifest);if ui.button("Choose model manifest…").clicked()&&let Some(p)=rfd::FileDialog::new().add_filter("Model manifest",&["json"]).pick_file(){e.config.manifest=p.to_string_lossy().into_owned();}
            });
            if old!=e.config{e.input=None;e.images=None;e.prepared=None;e.result=None;e.accept_low=false;}
            let r=ui.add_enabled(fresh&&!busy,egui::Button::new("1. Freeze and preview input"));self.local_inference.input=Some(r.rect);input=r.clicked();
            let r=ui.add_enabled(fresh&&!busy&&e.input.is_some(),egui::Button::new("2. Run verified local model"));self.local_inference.run=Some(r.rect);run=r.clicked();
            let can=e.result.as_ref().is_some_and(|r|!r.low_confidence(e.threshold)||e.accept_low);
            let r=ui.add_enabled(fresh&&!busy&&can&&e.prepared.is_some(),egui::Button::new("3. Adopt as one Undo"));self.local_inference.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel local inference").clicked();
            if busy{ui.horizontal(|ui|{ui.spinner();ui.label("Local task running · CPU · 512 MiB helper limit · 120 seconds maximum");});}
            if let Some(i)=&e.input{ui.small(format!("Frozen input {} × {}; transparent colors excluded; white matte for model RGB. Selection keeps page coordinates.",i.image.width,i.image.height));}
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in[("Frozen input",&images[0]),("Result",&images[1])]{ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(112.,112.)));});}});}
            if let Some(out)=&e.result{
                ui.small(&out.report);
                if !out.points.is_empty(){ui.add(egui::Slider::new(&mut e.threshold,0. ..=1.).text("Joint confidence threshold"));let low=out.points.iter().filter(|p|p[2]<e.threshold).count();ui.label(format!("2D pose, {} joints; {} below threshold. Preview colors use threshold 0.5; adopted colors use the current threshold.",out.points.len(),low));if low>0{ui.colored_label(egui::Color32::YELLOW,"Low confidence: review input and joints before adoption.");ui.checkbox(&mut e.accept_low,"I reviewed and accept uncertain joints");}}
            }
            ui.small("Pose becomes a new editable 2D vector layer. Image results become a new raster layer at native output size; the canvas and original layers stay unchanged. Models and file paths are not saved in the document.");
        });
        if old != e.config {
            self.cancel_inference();
        }
        if open && !cancel {
            self.local_inference.draft = Some(e);
        } else {
            self.cancel_inference();
        }
        if input && let Err(err) = self.start_inference(ctx, false) {
            self.status = Some(err);
        }
        if run && let Err(err) = self.start_inference(ctx, true) {
            self.status = Some(err);
        }
        if apply && let Err(err) = self.adopt_inference() {
            self.status = Some(err);
        }
    }
}

#[path = "local_inference_verify.rs"]
mod verify;
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    verify::smoke(root)
}
