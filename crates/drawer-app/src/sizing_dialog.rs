use super::*;
use drawer_core::{
    scene::Rect,
    sizing::{self, Change, DocumentState, Interpolation},
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Mode {
    Resample,
    Dpi,
    Canvas,
    Crop,
}
impl Mode {
    fn title(self) -> &'static str {
        match self {
            Self::Resample => "Resample image",
            Self::Dpi => "Change DPI only",
            Self::Canvas => "Canvas size",
            Self::Crop => "Crop image",
        }
    }
}
struct Job {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<DocumentState, String>>,
}
pub(super) struct SizingDialog {
    id: u64,
    revision: u64,
    original: Arc<DocumentState>,
    mode: Mode,
    pub(super) width: String,
    pub(super) height: String,
    pub(super) dpi: String,
    pub(super) x: String,
    pub(super) y: String,
    anchor: [f64; 2],
    filter: Interpolation,
    pub(super) confirmed: bool,
    error: Option<String>,
    job: Option<Job>,
    #[cfg(test)]
    pub(super) apply_button: Option<egui::Rect>,
    #[cfg(test)]
    pub(super) cancel_button: Option<egui::Rect>,
}
impl Drop for SizingDialog {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl SizingDialog {
    fn start_job(&mut self, ctx: &egui::Context, budget: usize) -> Result<(), String> {
        let change = self.change()?;
        sizing::plan(&self.original, change, budget)?;
        let original = self.original.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = sizing::prepare(&original, change, budget, &worker_cancel);
            let _ = sender.send(result);
            repaint.request_repaint();
        });
        self.job = Some(Job { cancel, receiver });
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn job_is_idle(&self) -> bool {
        self.job.is_none()
    }
    fn new(document: &Document, mode: Mode) -> Self {
        Self {
            id: document.id,
            revision: document.revision,
            original: Arc::new(document.snapshot()),
            mode,
            width: document.width.to_string(),
            height: document.height.to_string(),
            dpi: document.dpi.to_string(),
            x: "0".into(),
            y: "0".into(),
            anchor: [0.5; 2],
            filter: Interpolation::Bilinear,
            confirmed: false,
            error: None,
            job: None,
            #[cfg(test)]
            apply_button: None,
            #[cfg(test)]
            cancel_button: None,
        }
    }
    fn change(&self) -> Result<Change, String> {
        let number = |text: &str| {
            text.trim()
                .parse::<u32>()
                .map_err(|_| "Use a whole number of pixels.".to_string())
        };
        let dpi = || {
            self.dpi
                .trim()
                .parse::<f64>()
                .map_err(|_| "Use a positive DPI value.".to_string())
        };
        match self.mode {
            Mode::Dpi => Ok(Change::Dpi(dpi()?)),
            Mode::Resample => Ok(Change::Resample {
                width: number(&self.width)?,
                height: number(&self.height)?,
                dpi: dpi()?,
                filter: self.filter,
            }),
            Mode::Canvas => Ok(Change::Canvas {
                width: number(&self.width)?,
                height: number(&self.height)?,
                anchor: self.anchor,
            }),
            Mode::Crop => Ok(Change::Crop {
                x: number(&self.x)?,
                y: number(&self.y)?,
                width: number(&self.width)?,
                height: number(&self.height)?,
            }),
        }
    }
    fn plan(&self, budget: usize) -> Result<sizing::Plan, String> {
        sizing::plan(&self.original, self.change()?, budget)
    }
}
impl Drawer {
    pub(super) fn smoke_start_sizing(&mut self, ctx: &egui::Context) {
        let budget = self.sizing_budget();
        self.sizing_dialog
            .as_mut()
            .unwrap()
            .start_job(ctx, budget)
            .unwrap();
    }
    pub(super) fn begin_sizing(&mut self, mode: Mode) {
        if self.sizing_dialog.is_none()
            && let Some(index) = self.active
        {
            let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
            if self.documents[index].memory_bytes() > DEFAULT_DOCUMENT_BUDGET.saturating_sub(used)
                || self.documents[index].memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 4 - 4
            {
                self.status=Some("Not enough memory to preserve this drawing for image sizing and undo. Close other drawings or reduce the document.".into());
                return;
            }
            self.sizing_dialog = Some(SizingDialog::new(&self.documents[index], mode));
        }
    }
    pub(super) fn sizing_controls(&mut self, ui: &mut egui::Ui) {
        if self.active.is_none() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for (label, mode) in [
                ("Resample…", Mode::Resample),
                ("DPI…", Mode::Dpi),
                ("Canvas size…", Mode::Canvas),
                ("Crop…", Mode::Crop),
            ] {
                if ui.button(label).clicked() {
                    self.begin_sizing(mode);
                }
            }
        });
    }
    fn sizing_budget(&self) -> usize {
        let Some(dialog) = &self.sizing_dialog else {
            return DEFAULT_DOCUMENT_BUDGET;
        };
        let other: usize = self
            .documents
            .iter()
            .map(|doc| {
                if doc.id == dialog.id {
                    doc.resident_bytes() - doc.memory_bytes()
                } else {
                    doc.resident_bytes()
                }
            })
            .sum();
        DEFAULT_DOCUMENT_BUDGET.saturating_sub(other)
    }
    pub(super) fn sizing_overlay(&self) -> Option<Rect> {
        let dialog = self.sizing_dialog.as_ref()?;
        if !matches!(dialog.mode, Mode::Canvas | Mode::Crop) {
            return None;
        }
        dialog
            .plan(self.sizing_budget())
            .ok()
            .map(|p| p.source_boundary)
    }
    pub(super) fn show_sizing(&mut self, ctx: &egui::Context) {
        let budget = self.sizing_budget();
        let Some(mut dialog) = self.sizing_dialog.take() else {
            return;
        };
        let mut keep = true;
        if let Some(job) = &dialog.job {
            let received = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Image calculation stopped unexpectedly.".into()))
                }
            };
            if let Some(result) = received {
                dialog.job = None;
                match result {
                    Ok(result) => {
                        if let Some(index) = self
                            .documents
                            .iter()
                            .position(|doc| doc.id == dialog.id && doc.revision == dialog.revision)
                        {
                            match self.documents[index].commit_state(result) {
                                Ok(_) => keep = false,
                                Err(error) => dialog.error = Some(error),
                            }
                        } else {
                            dialog.error =
                                Some("The drawing changed. Cancel and start again.".into());
                        }
                    }
                    Err(error) => dialog.error = Some(error),
                }
            }
        }
        if !keep {
            return;
        }
        let mut start = false;
        let mut cancel = false;
        let mut changed = false;
        egui::Modal::new(egui::Id::new("sizing-dialog")).show(ctx, |ui| {
            ui.heading(dialog.mode.title());
            ui.label("Finish or cancel this change before saving the drawing.");
            ui.add_enabled_ui(dialog.job.is_none(), |ui| {
                egui::Grid::new("sizing-fields").show(ui, |ui| {
                    if dialog.mode != Mode::Dpi {
                        for (label, value) in [("Width (pixels)", &mut dialog.width), ("Height (pixels)", &mut dialog.height)] {
                            ui.label(label);
                            changed |= ui.text_edit_singleline(value).changed();
                            ui.end_row();
                        }
                    }
                    if matches!(dialog.mode, Mode::Dpi | Mode::Resample) {
                        ui.label("DPI");
                        changed |= ui.text_edit_singleline(&mut dialog.dpi).changed();
                        ui.end_row();
                    }
                    if dialog.mode == Mode::Crop {
                        for (label, value) in [("Left (pixels)", &mut dialog.x), ("Top (pixels)", &mut dialog.y)] {
                            ui.label(label);
                            changed |= ui.text_edit_singleline(value).changed();
                            ui.end_row();
                        }
                    }
                });
                if dialog.mode == Mode::Canvas {
                    ui.label("Anchor");
                    egui::Grid::new("canvas-anchor").show(ui, |ui| {
                        for (row, y) in [0.0, 0.5, 1.0].into_iter().enumerate() {
                            for (column, x) in [0.0, 0.5, 1.0].into_iter().enumerate() {
                                let label = format!("{} {}", ["Top", "Middle", "Bottom"][row], ["Left", "Center", "Right"][column]);
                                if ui.selectable_label(dialog.anchor == [x, y], label).clicked() {
                                    dialog.anchor = [x, y];
                                    changed = true;
                                }
                            }
                            ui.end_row();
                        }
                    });
                }
                if dialog.mode == Mode::Resample {
                    ui.horizontal(|ui| {
                        changed |= ui.selectable_value(&mut dialog.filter, Interpolation::Nearest, "Nearest (pixel art)").changed();
                        changed |= ui.selectable_value(&mut dialog.filter, Interpolation::Bilinear, "Bilinear").changed();
                    });
                }
                if changed {
                    dialog.confirmed = false;
                    dialog.error = None;
                }
                match dialog.plan(budget) {
                    Ok(plan) => {
                        ui.label(format!("{} × {} pixels · {} DPI · {:.2} × {:.2} mm", plan.width, plan.height, plan.dpi, plan.width as f64*25.4/plan.dpi, plan.height as f64*25.4/plan.dpi));
                        if plan.removed_pixels > 0 {
                            ui.colored_label(egui::Color32::YELLOW, format!("{} raster pixels outside the outlined boundary will be removed.", plan.removed_pixels));
                            ui.label(format!("Affected layers: {}", dialog.original.layers.iter().map(|l|l.name.as_str()).collect::<Vec<_>>().join(", ")));
                            if !plan.clipped_objects.is_empty() {
                                ui.label(format!("Objects partly outside the boundary: {:?}. Geometry is retained.", plan.clipped_objects.iter().map(|i|i+1).collect::<Vec<_>>()));
                            }
                            ui.checkbox(&mut dialog.confirmed, "Confirm removal of pixels outside this boundary");
                        }
                        if !dialog.original.scene.objects.is_empty() {
                            ui.collapsing("Object sizes after this change", |ui| object_sizes(ui, &dialog, &plan));
                        }
                        if dialog.mode == Mode::Dpi {
                            ui.label("Pixels and object coordinates stay unchanged. Only physical size changes.");
                        }
                        if let Some(error) = &dialog.error {
                            ui.colored_label(egui::Color32::LIGHT_RED, error);
                        }
                        let apply = ui.add_enabled(plan.removed_pixels == 0 || dialog.confirmed, egui::Button::new("Apply"));
                        start = apply.clicked();
                        #[cfg(test)] { dialog.apply_button = Some(apply.rect); }
                    }
                    Err(error) => {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                }
            });
            if dialog.job.is_some() {
                ui.spinner();
                ui.label("Calculating image…");
            }
            let cancel_response = ui.button("Cancel");
            cancel = cancel_response.clicked();
            #[cfg(test)] { dialog.cancel_button = Some(cancel_response.rect); }
        });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if cancel {
            return;
        }
        if start && let Err(error) = dialog.start_job(ctx, budget) {
            dialog.error = Some(error);
        }
        if dialog.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        self.sizing_dialog = Some(dialog);
    }
}
fn object_sizes(ui: &mut egui::Ui, dialog: &SizingDialog, plan: &sizing::Plan) {
    use drawer_core::scene::{Affine, PageObject, Scene};
    let delta = if dialog.mode == Mode::Resample {
        Affine([
            plan.width as f64 / dialog.original.width as f64,
            0.0,
            0.0,
            plan.height as f64 / dialog.original.height as f64,
            0.0,
            0.0,
        ])
    } else {
        Affine::default()
    };
    let scene = Scene {
        transform: dialog.original.scene.transform.then(delta),
        ..Default::default()
    };
    for (index, object) in dialog.original.scene.objects.iter().enumerate() {
        let (name, size, unit) = match object {
            PageObject::Text { size_px, .. } => {
                ("Text", scene.physical_points(*size_px, plan.dpi), "pt")
            }
            PageObject::Line { width_px, .. } => {
                ("Line width", scene.physical_points(*width_px, 72.0), "px")
            }
            PageObject::Tone { period_px, .. } => {
                ("Tone", scene.tone_lpi(*period_px, plan.dpi), "lpi")
            }
            PageObject::Shadow { radius_px, .. } => (
                "Shadow radius",
                scene.physical_points(*radius_px, 72.0),
                "px",
            ),
        };
        ui.label(format!(
            "{} {}: {:.2} × {:.2} {} (horizontal / vertical)",
            name,
            index + 1,
            size[0],
            size[1],
            unit
        ));
    }
}
