use super::*;
use drawer_core::{
    local_color::{self as core, Correction},
    sizing::DocumentState,
};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
#[path = "local_color_verify.rs"]
mod verify;
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    verify::smoke(root)
}
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    job: Option<Job>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    input: Option<egui::TextureHandle>,
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
enum Config {
    Reference {
        path: String,
        page: usize,
        strength: f64,
    },
    Lighting(Correction),
}
pub(super) struct Draft {
    source: Source,
    config: Config,
    prepared: Option<(Config, DocumentState)>,
    images: Option<[egui::TextureHandle; 2]>,
}
struct ResultPreview {
    state: DocumentState,
    images: [egui::ColorImage; 3],
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
pub(super) fn controls(ui: &mut egui::Ui, value: &mut Correction) -> bool {
    let old = *value;
    match value {
        Correction::Distribution { strength, .. } => {
            ui.add(egui::Slider::new(strength, 0. ..=1.).text("Reference strength"));
            ui.small("Frozen color statistics are stored in this layer. A reference file is not required to reopen it.");
        }
        Correction::Lighting {
            light,
            shadow,
            strength,
        } => {
            for (label, color) in [("Light color", light), ("Shadow color", shadow)] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    let mut rgb = color.map(|v| v as f32);
                    if ui.color_edit_button_rgb(&mut rgb).changed() {
                        *color = rgb.map(|v| v as f64);
                    }
                });
            }
            ui.add(egui::Slider::new(strength, 0. ..=1.).text("Lighting strength"));
            ui.small("Neutral is 50% gray. Dark pixels use the shadow color; bright pixels use the light color. This is a tonal correction, not geometry-based relighting.");
        }
    }
    *value != old
}
impl Drawer {
    pub(super) fn local_color_save_guard(&self, index: usize) -> Result<(), String> {
        if self
            .local_color
            .draft
            .as_ref()
            .is_some_and(|e| e.source.id == self.documents[index].id)
        {
            Err("Unconfirmed local color correction: Apply or Cancel before saving. Save cancelled.".into())
        } else {
            Ok(())
        }
    }
    fn cancel_local_color(&mut self) {
        if let Some(j) = &self.local_color.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) fn begin_local_color(&mut self, reference: bool) -> Result<(), String> {
        self.finish_stroke();
        if self.local_color.working() {
            self.cancel_local_color();
            return Err("Previous color preview is stopping; reopen shortly".into());
        }
        if self.batch_filters.draft.is_some()
            || self.batch_filters.working()
            || self.detail_filter.draft.is_some()
            || self.detail_filter.working()
            || self.spatial_blur.draft.is_some()
            || self.spatial_blur.working()
            || self.surface_effects.draft.is_some()
            || self.surface_effects.working()
            || self.adjustments.draft.is_some()
        {
            return Err("Apply or cancel the other correction/filter first".into());
        }
        self.local_color.input = None;
        self.local_color.draft = Some(Box::new(Draft {
            source: source(self)?,
            config: if reference {
                Config::Reference {
                    path: String::new(),
                    page: 1,
                    strength: 1.,
                }
            } else {
                Config::Lighting(Correction::lighting())
            },
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_local_color(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.local_color.input = None;
        if let Some(e) = &mut self.local_color.draft {
            e.prepared = None;
            e.images = None;
        }
        if self.local_color.working() {
            return Err("Wait for the previous preview to stop".into());
        }
        let current = source(self)?;
        let e = self
            .local_color
            .draft
            .as_ref()
            .ok_or("Color window closed")?;
        if e.source != current {
            return Err("Page/frame changed; reopen the correction".into());
        }
        let d = &self.documents[self.active.unwrap()];
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(7))
            .saturating_add(160 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Local color preview exceeds workspace memory".into());
        }
        let state = d.snapshot();
        let config = e.config.clone();
        let worker = config.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let wc = cancel.clone();
        let progress = Arc::new(AtomicUsize::new(0));
        let wp = progress.clone();
        let context = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let work = || {
                let d = drawer_core::layer_merge::preview(&state, current.frame)?;
                let check = || {
                    if wc.load(Ordering::Relaxed) {
                        Err("Local color preview cancelled".to_string())
                    } else {
                        Ok(())
                    }
                };
                check()?;
                let (value, input) = match worker {
                    Config::Reference {
                        path,
                        page,
                        strength,
                    } => {
                        let input = drawer_core::image_import::ImageSource::read(
                            Path::new(&path),
                            64 * 1024 * 1024,
                            &wc,
                        )
                        .map_err(|e| e.to_string())?;
                        if page == 0 || page > input.pages.len() {
                            return Err("Reference page is outside the local image".into());
                        }
                        let info = &input.pages[page - 1];
                        if info.width as usize * info.height as usize > 4 * 1024 * 1024 {
                            return Err("Reference is limited to 4M pixels".into());
                        }
                        let reference = input
                            .decode(page - 1, true, 64 * 1024 * 1024, &wc)
                            .map_err(|e| e.to_string())?
                            .image;
                        let value = core::distribution(&d, &reference, strength, |done, total| {
                            check()?;
                            wp.store(done * 100 / total.max(1), Ordering::Relaxed);
                            Ok(())
                        })?;
                        let size = [
                            reference.width.min(64) as usize,
                            reference.height.min(64) as usize,
                        ];
                        let pixels = (0..size[1])
                            .flat_map(|y| (0..size[0]).map(move |x| (x, y)))
                            .map(|(x, y)| {
                                let p = reference.pixels[((y * reference.height as usize
                                    / size[1])
                                    * reference.width as usize)
                                    + (x * reference.width as usize / size[0])];
                                egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])
                            })
                            .collect();
                        (value, egui::ColorImage::new(size, pixels))
                    }
                    Config::Lighting(value) => (value, crate::surface_effects::thumbnail(&d, &wc)?),
                };
                check()?;
                let state = core::prepare(&d, value)?;
                let after = drawer_core::layer_merge::preview(&state, current.frame)?;
                let images = [
                    input,
                    crate::surface_effects::thumbnail(&d, &wc)?,
                    crate::surface_effects::thumbnail(&after, &wc)?,
                ];
                check()?;
                Ok(ResultPreview { state, images })
            };
            let _ = sender.send(work());
            context.request_repaint();
        });
        self.local_color.job = Some(Job {
            source: current,
            config,
            cancel,
            progress,
            receiver,
        });
        self.status = None;
        Ok(())
    }
    fn poll_local_color(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.local_color.job else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(25));
                return;
            }
            Err(_) => Err("Local color worker stopped".into()),
        };
        let job = self.local_color.job.take().unwrap();
        if job.cancel.load(Ordering::Relaxed) {
            return;
        }
        let fresh = source(self).ok() == Some(job.source);
        let Some(e) = &mut self.local_color.draft else {
            return;
        };
        if !fresh || e.source != job.source || e.config != job.config {
            e.prepared = None;
            e.images = None;
            self.status = Some("Page/frame/settings changed; prepare a new preview".into());
            return;
        }
        match result {
            Ok(result) => {
                let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
                if used.saturating_add(result.state.memory_bytes().saturating_mul(4))
                    > DEFAULT_DOCUMENT_BUDGET
                {
                    self.status = Some("Color output exceeds workspace memory".into());
                    return;
                }
                let [input, before, after] = result.images;
                self.local_color.input = Some(ctx.load_texture(
                    "local-color-input",
                    input,
                    egui::TextureOptions::NEAREST,
                ));
                e.images = Some([before, after].map(|i| {
                    ctx.load_texture("local-color-preview", i, egui::TextureOptions::NEAREST)
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
    fn apply_local_color(&mut self) -> Result<(), String> {
        let current = source(self)?;
        if self.local_color.working() {
            return Err("Wait for the color preview".into());
        }
        let e = self
            .local_color
            .draft
            .as_mut()
            .ok_or("Color window closed")?;
        if e.source != current || e.prepared.as_ref().is_none_or(|(c, _)| *c != e.config) {
            return Err("Prepare a fresh color preview first".into());
        }
        let (_, state) = e.prepared.take().unwrap();
        let n = self.active.unwrap();
        let created = state.layers.len() > self.documents[n].layers.len();
        self.documents[n].commit_state(state)?;
        self.local_color.draft = None;
        self.local_color.input = None;
        self.status = None;
        self.sync_layer_identity();
        if created {
            let d = &self.documents[n];
            self.panels.targets.insert(
                d.id,
                drawer_core::thumbnail::Target::Raster(d.layers.len() - 1),
            );
        }
        Ok(())
    }
    pub(super) fn local_color_window(&mut self, ctx: &egui::Context) {
        self.poll_local_color(ctx);
        let Some(mut e) = self.local_color.draft.take() else {
            return;
        };
        if self
            .active
            .is_none_or(|n| self.documents[n].id != e.source.id)
        {
            self.cancel_local_color();
            self.local_color.input = None;
            return;
        }
        let fresh = source(self).ok() == Some(e.source);
        let old = e.config.clone();
        let working = self.local_color.working();
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel) = (false, false, false);
        egui::Window::new("Local reference / light and shadow correction").default_width(530.).default_pos(egui::pos2(30.,20.)).collapsible(false).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Page/frame changed; cancel and reopen.");}
            ui.small("Creates a root correction layer above the raster stack. Page objects and animation cels are separate. Original pixels and alpha are preserved.");
            match &mut e.config {
                Config::Reference{path,page,strength}=>{
                    ui.label("Local reference file (PNG, JPEG, BMP or TIFF)");ui.text_edit_singleline(path);
                    if ui.button("Choose local reference…").clicked() && let Some(p)=rfd::FileDialog::new().add_filter("Local images",&["png","jpg","jpeg","bmp","tif","tiff"]).pick_file(){*path=p.to_string_lossy().into_owned();}
                    ui.horizontal(|ui|{ui.label("Page (TIFF)");ui.add(egui::DragValue::new(page).range(1..=256));});
                    ui.add(egui::Slider::new(strength,0. ..=1.).text("Reference strength"));
                    ui.small("Uses alpha-weighted RGB mean and covariance. Different dimensions are supported; transparent colors are excluded. No model or network is used.");
                },
                Config::Lighting(value)=>{controls(ui,value);},
            }
            if e.config!=old{e.prepared=None;e.images=None;self.local_color.input=None;}
            let r=ui.add_enabled(fresh&&!working,egui::Button::new("Preview local correction"));self.local_color.preview=Some(r.rect);preview=r.clicked();
            let r=ui.add_enabled(fresh&&!working&&e.prepared.is_some(),egui::Button::new("Apply correction as one Undo"));self.local_color.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel correction").clicked();
            if working{ui.label(format!("Calculating {}%",self.local_color.job.as_ref().unwrap().progress.load(Ordering::Relaxed)));}
            ui.horizontal(|ui|{if let Some(input)=&self.local_color.input{ui.vertical(|ui|{ui.label("Input reference");ui.add(egui::Image::new(input).fit_to_exact_size(egui::vec2(96.,96.)));});}
            if let Some(images)=&e.images{for(label,image)in[("Current",&images[0]),("Result",&images[1])]{ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}}});
            ui.small("Edit saved strength/light/shadow colors using Color correction layers. A fully transparent reference/backdrop is rejected. Cancel discards the complete draft.");
        });
        if e.config != old {
            self.cancel_local_color();
        }
        if open && !cancel {
            self.local_color.draft = Some(e);
        } else {
            self.cancel_local_color();
            self.local_color.input = None;
        }
        if preview && let Err(err) = self.preview_local_color(ctx) {
            self.status = Some(err);
        }
        if apply && let Err(err) = self.apply_local_color() {
            self.status = Some(err);
        }
    }
}
