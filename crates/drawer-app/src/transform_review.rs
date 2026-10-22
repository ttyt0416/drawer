//! Shared cancellable transform previews and explicit save resolution.
use super::*;
use drawer_core::{
    interpolation::Filter, layer_search::Key, material_mapping::Warp, material_transform::Prepared,
    scene::Point, vector_scope::Transform,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Quality {
    Fast,
    #[default]
    Normal,
    Fine,
}
impl Quality {
    fn size(self) -> u32 {
        match self {
            Self::Fast => 48,
            Self::Normal => 96,
            Self::Fine => 192,
        }
    }
}
#[derive(Clone, PartialEq)]
enum Task {
    Affine {
        key: Key,
        pixels: bool,
        settings: Transform,
        bake: bool,
        filter: Filter,
    },
    Deform {
        layers: Vec<usize>,
        selection: bool,
        warp: Option<Warp>,
        dabs: Vec<drawer_core::deform::Dab>,
        only_refer: bool,
    },
    Mesh {
        layers: Vec<usize>,
        selection: bool,
        mesh: drawer_core::mesh_deform::Mesh,
    },
    Material {
        key: Key,
        settings: drawer_core::material_transform::Settings,
    },
}
#[derive(Clone, PartialEq)]
struct Request {
    task: Task,
    quality: Quality,
}
#[derive(Clone, Copy, PartialEq)]
struct Source {
    id: u64,
    revision: u64,
    frame: u32,
}
#[derive(Default)]
pub(super) struct Panel {
    job: Option<Job>,
    ready: Option<(u8, Source, Request)>,
    pending: Option<Box<Pending>>,
    apply: Option<egui::Rect>,
    discard: Option<egui::Rect>,
    keep: Option<egui::Rect>,
}
struct Preview {
    result: Prepared,
    images: [egui::ColorImage; 2],
}
struct Job {
    kind: u8,
    source: Source,
    request: Request,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<Preview, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
struct Pending {
    id: u64,
    path: Option<std::path::PathBuf>,
}
pub(super) fn quality(ui: &mut egui::Ui, q: &mut Quality) -> bool {
    let old = *q;
    ui.horizontal(|ui| {
        ui.label("Preview quality");
        for (value, label) in [
            (Quality::Fast, "Fast"),
            (Quality::Normal, "Normal"),
            (Quality::Fine, "Fine"),
        ] {
            ui.radio_value(q, value, label);
        }
    });
    *q != old
}
pub(super) fn scales(
    ui: &mut egui::Ui,
    scale: &mut Point,
    locked: &mut bool,
    range: std::ops::RangeInclusive<f64>,
) -> (bool, [egui::Rect; 2], egui::Rect) {
    let old = (*scale, *locked);
    let lock = ui.checkbox(locked, "Keep aspect ratio").rect;
    let mut axes = [egui::Rect::NOTHING; 2];
    ui.horizontal(|ui| {
        for (k, label) in ["Scale X: ", "Scale Y: "].iter().enumerate() {
            let before = *scale;
            let response = ui.add(
                egui::DragValue::new(&mut scale[k])
                    .speed(0.01)
                    .range(range.clone())
                    .prefix(*label),
            );
            axes[k] = response.rect;
            if response.changed() && *locked {
                let other = 1 - k;
                scale[other] = if before[k] == 0. {
                    scale[k].abs() * if before[other] < 0. { -1. } else { 1. }
                } else {
                    before[other] * (scale[k] / before[k]).abs()
                };
                let limited = scale[other].clamp(*range.start(), *range.end());
                if limited != scale[other] {
                    scale[k] *= limited / scale[other];
                    scale[other] = limited;
                }
            }
        }
    });
    ((*scale, *locked) != old, axes, lock)
}
pub(super) fn depth(quad: [Point; 4], depth: f64) -> Result<[Point; 4], String> {
    if !(-0.9..=0.9).contains(&depth) {
        return Err("Perspective depth must be between -0.9 and 0.9".into());
    }
    let q = quad.map(|p| {
        let denominator = 1. + depth * (2. * p[1] - 1.);
        [
            (p[0] - 0.5) / denominator + 0.5,
            (p[1] - 0.5) / denominator + 0.5,
        ]
    });
    if Warp::Perspective(q).valid() {
        Ok(q)
    } else {
        Err("Perspective depth crosses the horizon or folds the quadrilateral".into())
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
fn request(app: &Drawer, kind: u8) -> Result<Request, String> {
    let d = &app.documents[app.active.ok_or("Open a page first")?];
    let (task, quality) = match kind {
        0 => {
            let e = app
                .affine_transform
                .draft
                .as_ref()
                .ok_or("Transform closed")?;
            (
                Task::Affine {
                    key: e.target,
                    pixels: e.pixels,
                    settings: e.settings,
                    bake: e.bake,
                    filter: e.filter,
                },
                e.quality,
            )
        }
        1 => {
            let e = app.deform.draft.as_ref().ok_or("Deformation closed")?;
            let warp = match e.kind {
                0 => Some(Warp::Distort(e.quad)),
                1 => Some(Warp::Perspective(depth(e.quad, e.depth)?)),
                _ => None,
            };
            (
                Task::Deform {
                    layers: e.layers.clone(),
                    selection: e.selection,
                    warp,
                    dabs: e.dabs.clone(),
                    only_refer: e.only_refer,
                },
                e.quality,
            )
        }
        2 => {
            let e = app.mesh_deform.draft.as_ref().ok_or("Mesh closed")?;
            let area = drawer_core::deform::area(d, &e.layers, e.selection)?.unwrap_or(
                drawer_core::scene::Rect {
                    min: [0.; 2],
                    max: [d.width as f64, d.height as f64],
                },
            );
            (
                Task::Mesh {
                    layers: e.layers.clone(),
                    selection: e.selection,
                    mesh: e.geometry(area)?,
                },
                e.quality,
            )
        }
        3 => {
            let e = app
                .material_transform
                .draft
                .as_ref()
                .ok_or("Source transform closed")?;
            (
                Task::Material {
                    key: e.key,
                    settings: e.settings.clone(),
                },
                e.quality,
            )
        }
        _ => return Err("Unknown transform".into()),
    };
    match &task {
        Task::Affine { settings, .. } => {
            settings.matrix()?;
        }
        Task::Deform {
            layers, warp, dabs, ..
        } => {
            if layers.is_empty()
                || warp.as_ref().is_some_and(|w| !w.valid())
                || dabs.len() > 512
                || dabs.iter().any(|d| !d.valid())
            {
                return Err("Invalid quadrilateral/liquify parameters".into());
            }
        }
        Task::Mesh { layers, mesh, .. } => {
            if layers.is_empty() || !mesh.valid() {
                return Err("Invalid or folded mesh".into());
            }
        }
        Task::Material { settings, .. } => {
            settings.matrix()?;
        }
    }
    Ok(Request { task, quality })
}
fn image(d: &Document, q: Quality, cancel: &AtomicBool) -> Result<egui::ColorImage, String> {
    let tick = || {
        if cancel.load(Ordering::Relaxed) {
            Err("Transform preview cancelled".into())
        } else {
            Ok(())
        }
    };
    tick()?;
    let renderer =
        drawer_core::float_render::Renderer::with_effect_progress(d, None, &mut |_, _| tick())?;
    let size = [
        d.width.min(q.size()) as usize,
        d.height.min(q.size()) as usize,
    ];
    let mut pixels = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        for x in 0..size[0] {
            if x.is_multiple_of(128) {
                tick()?;
            }
            let p = renderer
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
    pub(super) fn cancel_transform_save(&mut self) {
        self.transform_review.pending = None;
    }
    pub(super) fn transform_save_pending(&self) -> bool {
        self.transform_review.pending.is_some()
    }
    pub(super) fn transform_apply_guard(&self, kind: u8) -> Result<(), String> {
        if self.transform_working()
            || self
                .transform_review
                .ready
                .as_ref()
                .is_none_or(|(k, s, r)| {
                    *k != kind
                        || source(self).ok() != Some(*s)
                        || request(self, kind).ok().as_ref() != Some(r)
                })
        {
            return Err("Prepare a fresh transform preview first".into());
        }
        Ok(())
    }
    pub(super) fn wait_transform_preview(
        &mut self,
        ctx: &egui::Context,
        time: &mut f64,
    ) -> Result<(), String> {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while self.transform_working() {
            if std::time::Instant::now() > until {
                return Err("Transform worker timeout".into());
            }
            *time += 0.1;
            crate::input_replay::frame(ctx, self, *time, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok(())
    }
    pub(super) fn transform_unconfirmed(&self) -> bool {
        self.transform_working()
            || self.affine_transform.draft.is_some()
            || self.deform.draft.is_some()
            || self.mesh_deform.draft.is_some()
            || self.material_transform.draft.is_some()
    }
    pub(super) fn transform_working(&self) -> bool {
        self.transform_review.job.is_some()
    }
    pub(super) fn cancel_transform_work(&mut self) {
        if let Some(j) = &self.transform_review.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) fn transform_begin_guard(&mut self, kind: u8) -> Result<(), String> {
        if self.local_color.draft.is_some()
            || self.local_color.working()
            || self.batch_filters.draft.is_some()
            || self.batch_filters.working()
            || self.detail_filter.draft.is_some()
            || self.detail_filter.working()
            || self.spatial_blur.draft.is_some()
            || self.spatial_blur.working()
            || self.surface_effects.draft.is_some()
            || self.surface_effects.working()
        {
            return Err("Apply or cancel the other filter first".into());
        }
        if self.transform_working() {
            self.cancel_transform_work();
            return Err("Previous transform is stopping; reopen shortly".into());
        }
        if (kind != 0 && self.affine_transform.draft.is_some())
            || (kind != 1 && self.deform.draft.is_some())
            || (kind != 2 && self.mesh_deform.draft.is_some())
            || (kind != 3 && self.material_transform.draft.is_some())
        {
            return Err("Apply or cancel the other transform first".into());
        }
        Ok(())
    }
    pub(super) fn start_transform_preview(
        &mut self,
        ctx: &egui::Context,
        kind: u8,
    ) -> Result<(), String> {
        self.transform_review.ready = None;
        if self.transform_working() {
            self.cancel_transform_work();
            return Err("Previous transform preview is stopping".into());
        }
        let source = source(self)?;
        let request = request(self, kind)?;
        let worker = request.clone();
        let d = &self.documents[self.active.unwrap()];
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(7))
            .saturating_add(160 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Transform snapshot exceeds workspace memory".into());
        }
        let state = d.snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let wc = cancel.clone();
        let context = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let work = || {
                let d = drawer_core::layer_merge::preview(&state, source.frame)?;
                let mut tick = || {
                    if wc.load(Ordering::Relaxed) {
                        Err("Transform cancelled".into())
                    } else {
                        Ok(())
                    }
                };
                tick()?;
                let result = match &worker.task {
                    Task::Affine {
                        key,
                        pixels,
                        settings,
                        bake,
                        filter,
                    } => drawer_core::affine_transform::prepare_with_progress(
                        &d, *key, *pixels, *settings, *bake, *filter, &mut tick,
                    )?,
                    Task::Deform {
                        layers,
                        selection,
                        warp,
                        dabs,
                        only_refer,
                    } => {
                        let change = if let Some(w) = warp {
                            drawer_core::deform::Change::Quad(w)
                        } else {
                            drawer_core::deform::Change::Liquify {
                                dabs,
                                only_refer: *only_refer,
                            }
                        };
                        let state = drawer_core::deform::prepare_with_progress(
                            &d, layers, *selection, change, &mut tick,
                        )?;
                        Prepared {
                            state,
                            selected: Key::Layer(drawer_core::layer_properties::raster(
                                &d, layers[0],
                            )),
                            notes: vec![],
                        }
                    }
                    Task::Mesh {
                        layers,
                        selection,
                        mesh,
                    } => {
                        let state = drawer_core::deform::prepare_with_progress(
                            &d,
                            layers,
                            *selection,
                            drawer_core::deform::Change::Mesh(mesh),
                            &mut tick,
                        )?;
                        Prepared {
                            state,
                            selected: Key::Layer(drawer_core::layer_properties::raster(
                                &d, layers[0],
                            )),
                            notes: vec![],
                        }
                    }
                    Task::Material { key, settings } => {
                        drawer_core::material_transform::prepare(&d, *key, settings)?
                    }
                };
                tick()?;
                let after = drawer_core::layer_merge::preview(&result.state, source.frame)?;
                let images = [
                    image(&d, worker.quality, &wc)?,
                    image(&after, worker.quality, &wc)?,
                ];
                tick()?;
                Ok(Preview { result, images })
            };
            let _ = sender.send(work());
            context.request_repaint();
        });
        self.transform_review.job = Some(Job {
            kind,
            source,
            request,
            cancel,
            receiver,
        });
        self.status = None;
        Ok(())
    }
    pub(super) fn poll_transform_preview(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.transform_review.job else {
            return;
        };
        if source(self).ok() != Some(job.source)
            || request(self, job.kind).ok().as_ref() != Some(&job.request)
        {
            job.cancel.store(true, Ordering::Relaxed);
        }
        let result = match job.receiver.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(20));
                return;
            }
            Err(_) => Err("Transform worker stopped".into()),
        };
        let job = self.transform_review.job.take().unwrap();
        if job.cancel.load(Ordering::Relaxed) {
            return;
        }
        match result {
            Ok(p) => {
                let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
                if used.saturating_add(p.result.state.memory_bytes().saturating_mul(4))
                    > DEFAULT_DOCUMENT_BUDGET
                {
                    self.status = Some("Transform output exceeds workspace memory".into());
                    return;
                }
                self.transform_review.ready = Some((job.kind, job.source, job.request.clone()));
                let images = p.images.map(|i| {
                    ctx.load_texture("transform-preview", i, egui::TextureOptions::NEAREST)
                });
                match job.kind {
                    0 => {
                        let e = self.affine_transform.draft.as_mut().unwrap();
                        e.prepared = Some(p.result);
                        e.images = Some(images);
                    }
                    1 => {
                        let e = self.deform.draft.as_mut().unwrap();
                        e.prepared = Some(p.result.state);
                        e.images = Some(images);
                    }
                    2 => {
                        let e = self.mesh_deform.draft.as_mut().unwrap();
                        e.prepared = Some(p.result.state);
                        e.images = Some(images);
                    }
                    3 => {
                        let e = self.material_transform.draft.as_mut().unwrap();
                        e.prepared = Some(p.result);
                        e.images = Some(images);
                        e.acknowledged = false;
                    }
                    _ => {}
                }
            }
            Err(error) => self.status = Some(error),
        }
    }
    pub(super) fn defer_transform_save(&mut self, index: usize, path: &std::path::Path) -> bool {
        self.defer_transform(index, Some(path.to_owned()))
    }
    pub(super) fn defer_transform_save_dialog(&mut self, index: usize) -> bool {
        self.defer_transform(index, None)
    }
    fn defer_transform(&mut self, index: usize, path: Option<std::path::PathBuf>) -> bool {
        let id = self.documents[index].id;
        let pending = self
            .affine_transform
            .draft
            .as_ref()
            .is_some_and(|e| e.id == id)
            || self.deform.draft.as_ref().is_some_and(|e| e.id == id)
            || self.mesh_deform.draft.as_ref().is_some_and(|e| e.id == id)
            || self
                .material_transform
                .draft
                .as_ref()
                .is_some_and(|e| e.id == id);
        if pending {
            self.status=Some("Unconfirmed deformation/transform: choose Apply, Discard, or Keep editing in the save window".into());
            self.transform_review.pending = Some(Box::new(Pending { id, path }));
        }
        pending
    }
    fn resolve_transform_save(&mut self, ctx: &egui::Context, apply: bool) -> Result<(), String> {
        self.resolve_transform_save_from(ctx, apply, || {
            rfd::FileDialog::new()
                .set_title("Save Drawer document")
                .add_filter("Drawer document", &["drawer"])
                .set_file_name("Drawing.drawer")
                .save_file()
        })
    }
    fn resolve_transform_save_from(
        &mut self,
        ctx: &egui::Context,
        apply: bool,
        choose: impl FnOnce() -> Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        let p = self
            .transform_review
            .pending
            .as_ref()
            .ok_or("Save decision closed")?;
        let index = self
            .documents
            .iter()
            .position(|d| d.id == p.id)
            .ok_or("Source document closed")?;
        if self.active != Some(index) {
            return Err("Activate the original document before resolving its transform".into());
        }
        if self.transform_working() {
            return Err("Wait for the preview or cancel it before saving".into());
        }
        let path = if let Some(path) = &p.path {
            path.clone()
        } else {
            let Some(path) = choose() else {
                self.transform_review.pending = None;
                return Ok(());
            };
            path
        };
        if apply {
            if self.affine_transform.draft.is_some() {
                self.apply_affine_transform()?;
            } else if self.deform.draft.is_some() {
                self.apply_deform()?;
            } else if self.mesh_deform.draft.is_some() {
                self.apply_mesh_deform()?;
            } else if self.material_transform.draft.is_some() {
                self.apply_material_transform()?;
            }
        } else {
            self.affine_transform.draft = None;
            self.deform.draft = None;
            self.mesh_deform.draft = None;
            self.material_transform.draft = None;
            self.cancel_transform_work();
        }
        self.transform_review.pending = None;
        let result = self.save_to(index, &path);
        ctx.request_repaint();
        result
    }
    pub(super) fn transform_save_window(&mut self, ctx: &egui::Context) {
        if self.transform_review.pending.is_none() {
            return;
        }
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut apply, mut discard, mut keep) = (false, false, false);
        egui::Window::new("Save with an unconfirmed transform").default_pos(egui::pos2(70.,40.)).collapsible(false).open(&mut open).show(ctx,|ui|{
            ui.label("Choose what to save. Keeping editing cancels this save and preserves the complete preview.");
            let r=ui.add_enabled(!self.transform_working(),egui::Button::new("Apply prepared transform and save"));self.transform_review.apply=Some(r.rect);apply=r.clicked();
            let r=ui.add_enabled(!self.transform_working(),egui::Button::new("Discard transform and save original"));self.transform_review.discard=Some(r.rect);discard=r.clicked();
            let r=ui.button("Keep editing — cancel save");self.transform_review.keep=Some(r.rect);keep=r.clicked();
            ui.small("Prepare the transform preview first. Source transforms also require the existing review checkbox. Invalid or stale results cannot be applied.");
        });
        if !open || keep {
            self.transform_review.pending = None;
            return;
        }
        if (apply || discard)
            && let Err(error) = self.resolve_transform_save(ctx, apply)
        {
            self.status = Some(error);
        }
    }
}

#[path = "transform_review_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)
}
