use super::*;
use drawer_core::{
    scene::{Point, Rect},
    transform_handles::{self as core, Drag, Snap},
    vector_scope::Transform,
};
#[derive(Default)]
pub(super) struct Pad {
    drag: Option<Drag>,
    domain: Option<Rect>,
    bounds: Option<Rect>,
    pub rect: Option<egui::Rect>,
    pub fit_rect: Option<egui::Rect>,
    pub error: Option<String>,
}
impl Pad {
    pub(super) fn edited(&mut self) {
        if self.drag.is_none() {
            self.error = None;
        }
    }
    pub(super) fn fit(&mut self, bounds: Rect, t: Transform) {
        let points = core::box_corners(bounds, t).unwrap_or(core::corners(bounds));
        let mut r = Rect {
            min: [f64::INFINITY; 2],
            max: [f64::NEG_INFINITY; 2],
        };
        for p in points {
            for (n, v) in p.iter().enumerate() {
                r.min[n] = r.min[n].min(*v);
                r.max[n] = r.max[n].max(*v);
            }
        }
        let margin = ((r.max[0] - r.min[0]).max(r.max[1] - r.min[1]) * 0.3)
            .max(1e-12)
            .max(
                r.min
                    .iter()
                    .chain(&r.max)
                    .map(|v| v.abs())
                    .fold(1., f64::max)
                    * f64::EPSILON
                    * 8.,
            );
        r.min = r.min.map(|v| v - margin);
        r.max = r.max.map(|v| v + margin);
        self.domain = Some(r);
    }
    pub(super) fn point(&self, r: egui::Rect, p: Point) -> egui::Pos2 {
        let a = self.domain.unwrap();
        egui::pos2(
            r.min.x + ((p[0] - a.min[0]) / (a.max[0] - a.min[0])) as f32 * r.width(),
            r.min.y + ((p[1] - a.min[1]) / (a.max[1] - a.min[1])) as f32 * r.height(),
        )
    }
    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        bounds: Rect,
        t: &mut Transform,
        aspect: bool,
        snap: Snap,
        enabled: bool,
    ) -> (bool, bool) {
        if self.domain.is_none() || self.bounds != Some(bounds) {
            self.fit(bounds, *t);
            self.bounds = Some(bounds);
        }
        let (r, response) = ui.allocate_exact_size(egui::vec2(350., 170.), egui::Sense::drag());
        self.rect = Some(r);
        let domain = self.domain.unwrap();
        let to_doc = |p: egui::Pos2| {
            [
                domain.min[0]
                    + (p.x - r.min.x) as f64 / r.width() as f64 * (domain.max[0] - domain.min[0]),
                domain.min[1]
                    + (p.y - r.min.y) as f64 / r.height() as f64 * (domain.max[1] - domain.min[1]),
            ]
        };
        ui.painter()
            .rect_filled(r, 0., egui::Color32::from_gray(24));
        let mut changed = false;
        let mut released = false;
        if let Ok(q) = core::box_corners(bounds, *t) {
            let points = q.map(|p| self.point(r, p));
            for n in 0..4 {
                ui.painter_at(r).line_segment(
                    [points[n], points[(n + 1) % 4]],
                    egui::Stroke::new(1., egui::Color32::LIGHT_BLUE),
                );
                ui.painter()
                    .circle_filled(points[n], 4., egui::Color32::WHITE);
                let mid = points[n].lerp(points[(n + 1) % 4], 0.5);
                ui.painter_at(r).rect_filled(
                    egui::Rect::from_center_size(mid, egui::vec2(6., 6.)),
                    0.,
                    egui::Color32::WHITE,
                );
            }
        }
        if enabled
            && response.drag_started()
            && let Some(pos) = ui
                .input(|i| i.pointer.press_origin())
                .or_else(|| response.interact_pointer_pos())
        {
            let start = to_doc(pos);
            let radius = 6. * (domain.max[0] - domain.min[0]) / r.width() as f64;
            match core::pick(bounds, *t, start, radius) {
                Ok(handle) => {
                    self.drag = Some(Drag {
                        handle,
                        bounds,
                        source: *t,
                        start,
                    })
                }
                Err(err) => self.error = Some(err),
            }
        }
        if enabled
            && response.dragged()
            && let (Some(drag), Some(pos)) = (self.drag, response.interact_pointer_pos())
        {
            match drag.update(to_doc(pos), aspect, snap) {
                Ok(value) => {
                    changed = *t != value;
                    *t = value;
                    self.error = None;
                }
                Err(err) => self.error = Some(err),
            }
        }
        if response.drag_stopped() {
            self.drag = None;
            released = self.error.is_none();
        }
        let fit = ui.button("Fit transform handles");
        self.fit_rect = Some(fit.rect);
        if fit.clicked() {
            self.fit(bounds, *t);
        }
        ui.small("Inside: move · corners: aspect scale · side centers: one axis · outside: rotate. Handles use document coordinates, including off-canvas sources.");
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::YELLOW, error);
        }
        (changed, released)
    }
}
pub(super) fn quad_valid(q: [Point; 4], kind: u8, depth: f64) -> Result<(), String> {
    let valid = if kind == 1 {
        super::transform_review::depth(q, depth)
            .map(|q| drawer_core::material_mapping::Warp::Perspective(q).valid())?
    } else {
        drawer_core::material_mapping::Warp::Distort(q).valid()
    };
    if valid {
        Ok(())
    } else {
        Err("Invalid quadrilateral: concave, crossed, reversed or zero area. The last valid preview is retained.".into())
    }
}

#[path = "transform_handles_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)
}
