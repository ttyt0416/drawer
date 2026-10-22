use super::*;
use drawer_core::{
    sizing::DocumentState,
    surface_effects::{self as core, Entry, Operation, Scope, Stack},
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
#[path = "surface_effects_verify.rs"]
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
#[derive(Clone, PartialEq)]
struct Config {
    stack: Stack,
    raster: bool,
}
pub(super) struct Draft {
    source: Source,
    config: Config,
    selected: usize,
    seed: crate::pixel_effects::Controls,
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
    let layer = match app.panels.targets.get(&d.id) {
        None => 0,
        Some(drawer_core::thumbnail::Target::Raster(n)) => *n,
        _ => return Err("Select a raster source layer".into()),
    };
    Ok(Source {
        id: d.id,
        revision: d.revision,
        frame: d.current_frame,
        layer,
    })
}
pub(super) fn thumbnail(d: &Document, cancel: &AtomicBool) -> Result<egui::ColorImage, String> {
    let r = drawer_core::float_render::Renderer::with_effect_progress(d, None, &mut |_, _| {
        if cancel.load(Ordering::Relaxed) {
            Err("Effects cancelled".into())
        } else {
            Ok(())
        }
    })?;
    let size = [d.width.min(64) as usize, d.height.min(64) as usize];
    let mut pixels = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        if cancel.load(Ordering::Relaxed) {
            return Err("Effects cancelled".into());
        }
        for x in 0..size[0] {
            let p = r
                .display_sample([
                    (x as f64 + 0.5) * d.width as f64 / size[0] as f64,
                    (y as f64 + 0.5) * d.height as f64 / size[1] as f64,
                ])?
                .rgba8();
            pixels.push(egui::Color32::from_rgba_unmultiplied(
                p[0], p[1], p[2], p[3],
            ));
        }
    }
    Ok(egui::ColorImage::new(size, pixels))
}
impl Drawer {
    pub(super) fn surface_effects_save_guard(&self, index: usize) -> Result<(), String> {
        if self
            .surface_effects
            .draft
            .as_ref()
            .is_some_and(|e| e.source.id == self.documents[index].id)
        {
            Err("Unconfirmed surface effects: Apply or Cancel the effects window before saving. Save cancelled.".into())
        } else {
            Ok(())
        }
    }
    fn cancel_surface_job(&mut self) {
        if let Some(j) = &self.surface_effects.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) fn begin_surface_effects(&mut self, add: Option<usize>) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed() {
            return Err("Apply or cancel the transform first".into());
        }
        if self.local_color.draft.is_some()
            || self.local_color.working()
            || self.batch_filters.draft.is_some()
            || self.batch_filters.working()
        {
            return Err("Apply or cancel the multiple-layer filter first".into());
        }
        if self.spatial_blur.draft.is_some()
            || self.spatial_blur.working()
            || self.detail_filter.draft.is_some()
            || self.detail_filter.working()
        {
            return Err("Apply or cancel the other filter first".into());
        }
        if self.surface_effects.working() {
            self.cancel_surface_job();
            return Err("Previous effects preview is stopping; reopen shortly".into());
        }
        let source = source(self)?;
        let d = &self.documents[self.active.unwrap()];
        drawer_core::layer_properties::paintable(d, source.layer)?;
        let mut stack = d
            .scene
            .surface_effects
            .rasters
            .get(&d.scene.layer_ids.id(source.layer))
            .cloned()
            .unwrap_or_default();
        if let Some(index) = add {
            if stack.effects.len() >= 16 {
                return Err("A layer supports up to16 ordered surface effects".into());
            }
            stack.effects.push(Entry {
                operation: self.workspaces.store.current.surface_effects.get(index),
                enabled: true,
                scope: None,
            });
        }
        let selected = stack.effects.len().saturating_sub(1);
        self.surface_effects.draft = Some(Box::new(Draft {
            source,
            config: Config {
                stack,
                raster: false,
            },
            selected,
            seed: Default::default(),
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_surface_effects(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if let Some(e) = &mut self.surface_effects.draft {
            e.prepared = None;
            e.images = None;
        }
        if self.surface_effects.working() {
            return Err("Wait for the previous effects preview".into());
        }
        let current = source(self)?;
        let e = self
            .surface_effects
            .draft
            .as_ref()
            .ok_or("Effects window closed")?;
        if e.source != current || !e.config.stack.valid() || !e.seed.valid {
            return Err("Source changed or invalid effect settings".into());
        }
        let d = &self.documents[self.active.unwrap()];
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(7))
            .saturating_add(96 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Effects preview exceeds workspace memory".into());
        }
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
                let mut tick = |done: usize, total: usize| {
                    if wc.load(Ordering::Relaxed) {
                        return Err("Effects cancelled".into());
                    }
                    wp.store(done.saturating_mul(100) / total.max(1), Ordering::Relaxed);
                    Ok(())
                };
                tick(0, 1)?;
                let d = drawer_core::layer_merge::preview(&state, current.frame)?;
                let state = if worker_config.raster {
                    core::rasterize(&d, current.layer, worker_config.stack.clone(), &mut tick)?
                } else {
                    core::prepare(&d, current.layer, worker_config.stack.clone(), &mut tick)?
                };
                let after = drawer_core::layer_merge::preview(&state, current.frame)?;
                let images = [thumbnail(&d, &wc)?, thumbnail(&after, &wc)?];
                tick(1, 1)?;
                Ok(ResultPreview { state, images })
            };
            let _ = sender.send(work());
            context.request_repaint();
        });
        self.surface_effects.job = Some(Job {
            source: current,
            config,
            cancel,
            progress,
            receiver,
        });
        self.status = None;
        Ok(())
    }
    fn poll_surface_effects(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.surface_effects.job else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(25));
                return;
            }
            Err(_) => Err("Effects worker stopped".into()),
        };
        let job = self.surface_effects.job.take().unwrap();
        if job.cancel.load(Ordering::Relaxed) {
            return;
        }
        let fresh = source(self).ok() == Some(job.source);
        let Some(e) = &mut self.surface_effects.draft else {
            return;
        };
        if !fresh || e.source != job.source || e.config != job.config {
            e.prepared = None;
            e.images = None;
            self.status = Some("Source/settings changed; prepare another preview".into());
            return;
        }
        match result {
            Ok(r) => {
                let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
                if used.saturating_add(r.state.memory_bytes().saturating_mul(4))
                    > DEFAULT_DOCUMENT_BUDGET
                {
                    self.status = Some("Effects output exceeds workspace memory".into());
                    return;
                }
                e.images = Some(r.images.map(|image| {
                    ctx.load_texture(
                        "surface-effects-preview",
                        image,
                        egui::TextureOptions::NEAREST,
                    )
                }));
                e.prepared = Some((job.config.clone(), r.state));
            }
            Err(error) => {
                e.prepared = None;
                e.images = None;
                self.status = Some(error);
            }
        }
    }
    fn apply_surface_effects(&mut self) -> Result<(), String> {
        let current = source(self)?;
        if self.surface_effects.working() {
            return Err("Wait for the effects preview".into());
        }
        let e = self
            .surface_effects
            .draft
            .as_mut()
            .ok_or("Effects window closed")?;
        if current != e.source
            || !e.seed.valid
            || e.prepared.as_ref().is_none_or(|(c, _)| *c != e.config)
        {
            return Err("Prepare a fresh effects preview first".into());
        }
        let (config, state) = e.prepared.take().unwrap();
        let n = self.active.unwrap();
        let changed = state != self.documents[n].snapshot();
        let result_layer = state.layers.len() - 1;
        self.documents[n].commit_state(state)?;
        self.surface_effects.draft = None;
        self.status = None;
        if config.raster {
            self.panels.targets.insert(
                self.documents[n].id,
                drawer_core::thumbnail::Target::Raster(result_layer),
            );
        }
        if changed {
            let old = self.workspaces.store.current.surface_effects.clone();
            for e in config.stack.effects {
                self.workspaces.store.current.surface_effects.values[e.operation.index()] =
                    Some(e.operation);
            }
            if let Err(err) = self.persist_workspace() {
                self.workspaces.store.current.surface_effects = old;
                return Err(format!(
                    "Effects applied; local preset could not be saved: {err}"
                ));
            }
        }
        Ok(())
    }
    pub(super) fn surface_effects_window(&mut self, ctx: &egui::Context) {
        self.poll_surface_effects(ctx);
        let Some(mut e) = self.surface_effects.draft.take() else {
            return;
        };
        let fresh = source(self).ok() == Some(e.source);
        let working = self.surface_effects.working();
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel) = (false, false, false);
        let old = e.config.clone();
        let mut colors = self.workspaces.store.current.color_modes;
        colors.observe(self.references.session.workspace.color);
        egui::Window::new("Ordered surface effects").default_width(490.).default_pos(egui::pos2(35.,20.)).collapsible(false).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Source changed; cancel and reopen.");}
            egui::ScrollArea::vertical().id_salt("surface-controls").max_height(330.).show(ui,|ui|{
                ui.horizontal_wrapped(|ui|{for op in Operation::defaults(){if ui.add_enabled(e.config.stack.effects.len()<16,egui::Button::new(format!("+ {}",op.label()))).clicked(){e.config.stack.effects.push(Entry{operation:self.workspaces.store.current.surface_effects.get(op.index()),enabled:true,scope:None});e.selected=e.config.stack.effects.len()-1;e.seed=Default::default();}}});
                for (i,entry) in e.config.stack.effects.iter_mut().enumerate(){ui.push_id(i,|ui|{ui.horizontal(|ui|{ui.checkbox(&mut entry.enabled,"");if ui.selectable_label(e.selected==i,format!("{}. {}",i+1,entry.operation.label())).clicked(){e.selected=i;e.seed=Default::default();}});});}
                if !e.config.stack.effects.is_empty(){e.selected=e.selected.min(e.config.stack.effects.len()-1);ui.horizontal(|ui|{if ui.add_enabled(e.selected>0,egui::Button::new("Earlier")).clicked(){e.config.stack.effects.swap(e.selected,e.selected-1);e.selected-=1;}
                    if ui.add_enabled(e.selected+1<e.config.stack.effects.len(),egui::Button::new("Later")).clicked(){e.config.stack.effects.swap(e.selected,e.selected+1);e.selected+=1;}
                    if ui.button("Remove effect").clicked(){e.config.stack.effects.remove(e.selected);e.selected=e.selected.saturating_sub(1);e.seed=Default::default();}
                });}
                if let Some(entry)=e.config.stack.effects.get_mut(e.selected){controls(ui,&mut entry.operation,&mut e.seed,colors.slots);let mut scoped=entry.scope.is_some();if ui.checkbox(&mut scoped,"Restrict this effect to a frozen copy of the current selection").changed(){entry.scope=scoped.then(||Scope::capture(&self.documents[self.active.unwrap()]));}
                    if ui.button("Restore last confirmed settings for this effect").clicked(){entry.operation=self.workspaces.store.current.surface_effects.get(entry.operation.index());e.seed=Default::default();}}
                ui.horizontal(|ui|{if ui.button("Output mask from current selection").clicked(){match core::capture_output_mask(&self.documents[self.active.unwrap()]){Ok(m)=>e.config.stack.output_mask=Some(m),Err(err)=>self.status=Some(err)}}
if ui.button("Remove output mask").clicked(){e.config.stack.output_mask=None;}});
                if let Some(mask)=&mut e.config.stack.output_mask{ui.checkbox(&mut mask.enabled,"Enable output mask (clips source and all effects)");ui.checkbox(&mut mask.inverted,"Invert output mask");}
                ui.checkbox(&mut e.config.raster,"Confirm as a separate raster result layer (preserve and hide source / full clipping run)");
            });
            if e.config!=old{e.prepared=None;e.images=None;}
            let r=ui.add_enabled(fresh&&!working&&e.seed.valid,egui::Button::new("Prepare effects preview"));self.surface_effects.preview=Some(r.rect);preview=r.clicked();
            let r=ui.add_enabled(fresh&&!working&&e.seed.valid&&e.prepared.is_some(),egui::Button::new("Apply effects as one Undo"));self.surface_effects.apply=Some(r.rect);apply=r.clicked();
            let r=ui.button("Cancel preview and close");self.surface_effects.cancel=Some(r.rect);cancel=r.clicked();
            if working{ui.label(format!("Calculating {}%",self.surface_effects.job.as_ref().unwrap().progress.load(Ordering::Relaxed)));}
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(100.,100.)));});}});}
            ui.small("Source mask and pre-effect clipping → ordered effects → output mask → layer opacity/blend. Parent folder masks clip the complete result. Reopen Edit surface effects to change the saved list. Escape discards the draft.");
        });
        if e.config != old || !fresh || !open || cancel {
            self.cancel_surface_job();
        }
        if open && !cancel {
            self.surface_effects.draft = Some(e);
        }
        if preview && let Err(err) = self.preview_surface_effects(ctx) {
            self.status = Some(err);
        }
        if apply && let Err(err) = self.apply_surface_effects() {
            self.status = Some(err);
        }
    }
}
pub(super) fn controls(
    ui: &mut egui::Ui,
    op: &mut Operation,
    state: &mut crate::pixel_effects::Controls,
    colors: [[u8; 4]; 2],
) {
    state.valid = true;
    match op {
        Operation::Shadow {
            blur,
            direction,
            distance,
            color,
            opacity,
        } => {
            ui.add(egui::Slider::new(blur, 0..=32).text("Shadow blur"));
            ui.add(
                egui::Slider::new(direction, -180. ..=180.)
                    .text("Direction ° (clockwise, right =0)"),
            );
            ui.add(egui::Slider::new(distance, 0. ..=256.).text("Distance (pixels)"));
            ui.add(egui::Slider::new(opacity, 0. ..=1.).text("Shadow opacity"));
            color_controls(ui, color, colors);
        }
        Operation::Glow {
            intensity,
            spread,
            blur,
            color,
        } => {
            ui.add(egui::Slider::new(intensity, 0. ..=4.).text("Glow intensity"));
            ui.add(egui::Slider::new(spread, 0..=32).text("Glow spread"));
            ui.add(egui::Slider::new(blur, 0..=32).text("Glow blur"));
            color_controls(ui, color, colors);
        }
        Operation::Emboss {
            shadow,
            gloss,
            height,
            smoothing,
            light,
        } => {
            ui.add(egui::Slider::new(shadow, 0. ..=2.).text("Shading strength"));
            ui.add(egui::Slider::new(gloss, 0. ..=2.).text("Gloss"));
            ui.add(egui::Slider::new(height, 0. ..=32.).text("Maximum height"));
            ui.add(egui::Slider::new(smoothing, 0..=32).text("Surface smoothing"));
            ui.add(egui::Slider::new(&mut light.x, -1. ..=1.).text("Light X"));
            ui.add(egui::Slider::new(&mut light.y, -1. ..=1.).text("Light Y"));
        }
        Operation::Crystal {
            cell,
            irregularity,
            offset,
            tiling,
            seed,
        } => {
            ui.add(egui::Slider::new(cell, 1..=256).text("Cell size"));
            ui.add(egui::Slider::new(irregularity, 0. ..=1.).text("Irregularity"));
            for (axis, v) in ["X", "Y"].into_iter().zip(offset) {
                ui.add(
                    egui::DragValue::new(v)
                        .range(-1e6..=1e6)
                        .prefix(format!("Pattern offset {axis} ")),
                );
            }
            ui.checkbox(tiling, "Periodic pattern / wrapped color sampling");
            state.seed(ui, seed);
        }
    }
}
fn color_controls(ui: &mut egui::Ui, color: &mut [u8; 3], colors: [[u8; 4]; 2]) {
    ui.horizontal(|ui| {
        ui.label("Direct color");
        ui.color_edit_button_srgb(color);
        for (label, c) in [
            ("Black", [0; 3]),
            ("Primary", [colors[0][0], colors[0][1], colors[0][2]]),
            ("Secondary", [colors[1][0], colors[1][1], colors[1][2]]),
        ] {
            if ui.button(label).clicked() {
                *color = c;
            }
        }
    });
}
