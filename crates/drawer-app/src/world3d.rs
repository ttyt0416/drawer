use super::*;
use drawer_core::{
    scene::{Camera, Scene},
    world3d::{self as core, Axes, Edit, Mode, Pivot, Preferences, Projection, V},
};
use std::collections::BTreeMap;
struct Drag {
    source: Document,
    start: egui::Pos2,
    options: Preferences,
    ids: Vec<u64>,
    axis: usize,
    units: f64,
    direction: egui::Vec2,
}
pub(super) struct Panel {
    pub open: bool,
    pub preferences: Preferences,
    stamp: Option<(u64, u64)>,
    ids: Vec<u64>,
    axis: usize,
    value: V,
    parent: Option<u64>,
    camera: Option<Camera>,
    lens: core::Lens,
    staged: Option<Box<core::Prepared>>,
    drag: Option<Box<Drag>>,
    notice: String,
    review_models: bool,
    pub controls: BTreeMap<String, egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            preferences: Default::default(),
            stamp: None,
            ids: vec![],
            axis: 0,
            value: [0.; 3],
            parent: None,
            camera: None,
            lens: Default::default(),
            staged: None,
            drag: None,
            notice: String::new(),
            review_models: false,
            controls: BTreeMap::new(),
        }
    }
}
fn frozen(d: &Document) -> Result<Document, String> {
    let mut copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
    copy.id = d.id;
    copy.revision = d.revision;
    copy.keyframe_content = d.keyframe_content;
    Ok(copy)
}
fn screen(c: &Camera, p: [f64; 2], r: egui::Rect) -> egui::Pos2 {
    let scale = (r.width() as f64 / (c.page_bounds.max[0] - c.page_bounds.min[0]))
        .min(r.height() as f64 / (c.page_bounds.max[1] - c.page_bounds.min[1]));
    egui::pos2(
        r.center().x + ((p[0] - (c.page_bounds.min[0] + c.page_bounds.max[0]) / 2.) * scale) as f32,
        r.center().y + ((p[1] - (c.page_bounds.min[1] + c.page_bounds.max[1]) / 2.) * scale) as f32,
    )
}
impl Panel {
    fn preview(&mut self, d: &Document, e: Edit, budget: usize) {
        match core::prepare(d, e, budget, &std::sync::atomic::AtomicBool::new(false)) {
            Ok(p) => {
                self.notice = p.summary.clone();
                self.review_models = false;
                self.staged = Some(Box::new(p));
            }
            Err(e) => {
                self.notice = e;
                self.staged = None;
            }
        }
    }
    fn pad(&mut self, ui: &mut egui::Ui, d: &Document, budget: usize) {
        let (r, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(700.), 235.),
            egui::Sense::drag(),
        );
        self.controls.insert("pad".into(), r);
        let scene = self
            .staged
            .as_ref()
            .map_or(&d.scene as &Scene, |s| s.scene());
        let mut axes = vec![];
        let painter = ui.painter_at(r);
        painter.rect_filled(r, 3., egui::Color32::from_rgb(25, 31, 32));
        if let Some(c) = &scene.camera {
            for (i, m) in scene.worlds.iter().enumerate() {
                let id = scene.catalog.worlds.id(i);
                if scene
                    .catalog
                    .world_info
                    .get(&id)
                    .is_some_and(|i| i.properties.hidden)
                {
                    continue;
                }
                let color = if self.ids.contains(&id) {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::from_rgba_unmultiplied(
                        m.color[0], m.color[1], m.color[2], m.color[3],
                    )
                };
                for line in core::segments(scene, i) {
                    painter
                        .line_segment(line.map(|p| screen(c, p, r)), egui::Stroke::new(1., color));
                }
            }
            if let Some(lens) = scene.world3d.lens
                && let Ok(pivot) = core::pivot(scene, &self.ids, self.preferences.pivot)
                && let Some(origin) = lens.project(c, pivot)
            {
                let origin = screen(c, origin, r);
                let basis = if self.preferences.axes == Axes::Object {
                    core::node(scene, self.ids[0]).unwrap().frame.basis()
                } else {
                    core::Matrix::default()
                };
                for i in 0..3 {
                    let axis = basis.axis(i);
                    let end = std::array::from_fn(|k| pivot[k] + axis[k]);
                    if let Some(end) = lens.project(c, end) {
                        let tip = screen(c, end, r);
                        let delta = tip - origin;
                        let color = [
                            egui::Color32::LIGHT_RED,
                            egui::Color32::LIGHT_GREEN,
                            egui::Color32::LIGHT_BLUE,
                        ][i];
                        let dir = if delta.length() > 3. {
                            delta.normalized()
                        } else {
                            egui::vec2(1., 0.)
                        };
                        let handle = origin + dir * (if i == self.axis { 75. } else { 58. });
                        painter.line_segment(
                            [origin, handle],
                            egui::Stroke::new(if i == self.axis { 3. } else { 1.5 }, color),
                        );
                        painter.text(
                            handle,
                            egui::Align2::CENTER_BOTTOM,
                            ["X", "Y", "Z"][i],
                            egui::FontId::proportional(13.),
                            color,
                        );
                        axes.push((i, origin, handle, delta.length().max(1.), dir));
                    }
                }
            }
        } else {
            painter.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "Place a local reference model to begin",
                egui::FontId::proportional(14.),
                egui::Color32::LIGHT_GRAY,
            );
        }
        // Axes and pivots remain fixed to the captured gesture source.
        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
        {
            if self.staged.is_some() {
                self.notice = "Apply or discard the previous 3D preview before another drag".into();
            } else {
                let nearest = axes
                    .iter()
                    .filter_map(|(i, a, b, _, _)| {
                        let ab = *b - *a;
                        let t = ((pos - *a).dot(ab) / ab.length_sq().max(1.)).clamp(0., 1.);
                        let distance = pos.distance(*a + ab * t);
                        (distance < 10.).then_some((*i, distance))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((i, _)) = nearest {
                    self.axis = i;
                }
                if let Some((_, _, _, units, direction)) = axes.iter().find(|a| a.0 == self.axis) {
                    if d.memory_bytes().saturating_mul(3) > budget {
                        self.notice = "3D gesture snapshot exceeds available memory".into();
                    } else {
                        match frozen(d) {
                            Ok(source) => {
                                self.staged = None;
                                self.drag = Some(Box::new(Drag {
                                    source,
                                    start: ui.input(|i| i.pointer.press_origin()).unwrap_or(pos),
                                    options: self.preferences,
                                    ids: self.ids.clone(),
                                    axis: self.axis,
                                    units: *units as f64,
                                    direction: *direction,
                                }));
                            }
                            Err(e) => self.notice = e,
                        }
                    }
                } else {
                    self.notice =
                        "Select a visible model / pivot inside the camera depth range".into();
                }
            }
        }
        if response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
            && let Some(drag) = &self.drag
        {
            let delta = pos - drag.start;
            let mut v = if drag.options.mode == Mode::Scale {
                [1.; 3]
            } else {
                [0.; 3]
            };
            v[drag.axis] = match drag.options.mode {
                Mode::Move => delta.dot(drag.direction) as f64 / drag.units,
                Mode::Rotate => (delta.x - delta.y) as f64 * 0.5,
                Mode::Scale => {
                    let n = 1. + (delta.x - delta.y) as f64 / 100.;
                    if n.abs() < 0.01 { 0.01 } else { n }
                }
            };
            match core::prepare(
                &drag.source,
                Edit::Transform {
                    ids: drag.ids.clone(),
                    options: drag.options,
                    value: v,
                },
                budget,
                &std::sync::atomic::AtomicBool::new(false),
            ) {
                Ok(p) => {
                    self.notice = p.summary.clone();
                    self.staged = Some(Box::new(p));
                }
                Err(e) => {
                    self.staged = None;
                    self.notice = e;
                }
            }
        }
        if response.drag_stopped() {
            self.drag = None;
        }
    }
}
impl Drawer {
    pub(super) fn load_world_preferences(&mut self, storage: &UserStorage) {
        match Preferences::load(storage.root()) {
            Ok(p) => self.world3d.preferences = p,
            Err(e) => self.world3d.notice = e,
        }
    }
    pub(super) fn has_world_preview(&self) -> bool {
        self.world3d.staged.is_some() || self.world3d.drag.is_some()
    }
    pub(super) fn hide_world_for_models(&mut self) {
        self.world3d.open = false;
    }
    pub(super) fn adopt_model_preview(&mut self, p: Box<core::Prepared>) -> Result<(), String> {
        let n = self.active.ok_or("Model document closed")?;
        if !p.matches(&self.documents[n]) {
            return Err("Model preview became stale; prepare again".into());
        }
        if self.has_world_preview() {
            return Err("Another3D preview is open; prepare models again".into());
        }
        self.world3d.ids = p
            .affected
            .iter()
            .copied()
            .filter(|id| core::node(&self.documents[n].scene, *id).is_some())
            .collect();
        self.world3d.notice = p.summary.clone();
        self.world3d.staged = Some(p);
        self.world3d.review_models = false;
        self.world3d.open = true;
        self.world3d.stamp = None;
        Ok(())
    }
    pub(super) fn begin_world3d(&mut self) -> Result<(), String> {
        if self.uv_pending() {
            return Err("Apply or discard the UV stroke first".into());
        }
        self.uv_paint.open = false;
        self.world_physics.open = false;
        if self.lighting_pending() {
            return Err("Apply or discard lighting preview first".into());
        }
        self.lighting.open = false;
        self.model_presets.open = false;
        if self.rig_pending() {
            return Err("Apply or discard the mannequin preview first".into());
        }
        self.human_rig.open = false;
        if self.models_busy() {
            return Err("Wait for or cancel model preparation first".into());
        }
        self.finish_stroke();
        if self.active.is_none() {
            return Err("Choose a document".into());
        }
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.text_editor.draft.is_some()
            || self.inference_unconfirmed()
        {
            return Err("Finish pending content edits first".into());
        }
        if self
            .world3d
            .stamp
            .is_some_and(|(id, _)| id != self.documents[self.active.unwrap()].id)
        {
            self.world3d.ids.clear();
        }
        self.world3d.staged = None;
        self.world3d.drag = None;
        self.world3d.open = true;
        self.world3d.stamp = None;
        Ok(())
    }
    fn apply_world3d(&mut self) -> Result<(), String> {
        if self
            .world3d
            .staged
            .as_ref()
            .is_some_and(|p| !p.warnings.is_empty())
            && !self.world3d.review_models
        {
            return Err("Review model import notices before applying".into());
        }
        if self.world3d.drag.is_some() {
            return Err("Release the 3D gesture before applying".into());
        }
        let n = self.active.ok_or("Choose a document")?;
        let p = self
            .world3d
            .staged
            .take()
            .ok_or("Prepare a 3D preview first")?;
        let affected = p.affected.clone();
        let summary = p.summary.clone();
        p.commit(&mut self.documents[n])?;
        self.world3d.notice = summary;
        if self.world3d.ids.is_empty() {
            self.world3d.ids = affected;
        }
        self.world3d.stamp = None;
        Ok(())
    }
    pub(super) fn world3d_window(&mut self, ctx: &egui::Context) {
        if self
            .world3d
            .staged
            .as_ref()
            .is_some_and(|p| !self.documents.iter().any(|d| p.matches(d)))
        {
            self.world3d.staged = None;
            self.world3d.notice = "Document changed or closed; 3D preview discarded".into();
        }
        if self.world3d.drag.as_ref().is_some_and(|p| {
            !self
                .documents
                .iter()
                .any(|d| d.id == p.source.id && d.revision == p.source.revision)
        }) {
            self.world3d.drag = None;
        }
        if !self.world3d.open {
            return;
        }
        let Some(n) = self.active else {
            return;
        };
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let d = &self.documents[n];
        let mut p = std::mem::take(&mut self.world3d);
        p.controls.clear();
        if p.stamp != Some((d.id, d.revision)) {
            if p.stamp.is_some_and(|(id, _)| id != d.id) {
                p.ids.clear();
                p.staged = None;
                p.drag = None;
            }
            p.ids.retain(|id| core::node(&d.scene, *id).is_some());
            let (c, l) = core::default_camera(d);
            p.camera = Some(c);
            p.lens = l;
            p.stamp = Some((d.id, d.revision));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            p.staged = None;
            p.drag = None;
        }
        let mut edit = None;
        let mut action = 0;
        let mut open = true;
        egui::Window::new("3D objects / articulated pose and camera").default_pos(egui::pos2(25.,20.)).default_width(730.).max_height(640.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.add_sized([690.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);
            ui.horizontal(|ui|{for(key,label,a,enabled)in [("reference","Place local reference model",1,true),("apply","Apply 3D preview",2,p.staged.as_ref().is_some_and(|s|s.warnings.is_empty()||p.review_models)),("discard","Discard 3D preview",3,true),("save","Save gizmo defaults",4,true)]{let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
            if let Some(staged)=&p.staged && !staged.warnings.is_empty(){
                egui::ScrollArea::vertical().id_salt("model-import-notices").max_height(85.).show(ui,|ui|{for warning in &staged.warnings{ui.add(egui::Label::new(warning).truncate()).on_hover_text(warning);}});
                let r=ui.checkbox(&mut p.review_models,"I reviewed the notices and preview result");p.controls.insert("model-review".into(),r.rect);
            }
            ui.horizontal(|ui|{ui.label("Relative X / Y / Z");for v in &mut p.value{ui.add(egui::DragValue::new(v).speed(0.1));}let r=ui.button("Preview transform / pose");p.controls.insert("transform".into(),r.rect);if r.clicked(){edit=Some(Edit::Transform{ids:p.ids.clone(),options:p.preferences,value:p.value});}});
            p.pad(ui,d,budget);
            ui.small("Drag a colored axis (or choose X/Y/Z below). Release keeps a preview; Apply commits one Undo. Esc / close discards. Wireframe gizmos; page output follows scene rendering settings.");
            egui::ScrollArea::vertical().id_salt("world-objects").max_height(65.).show(ui,|ui|{ui.horizontal(|ui|{if ui.button("Select all").clicked(){p.ids=(0..d.scene.worlds.len()).map(|i|d.scene.catalog.worlds.id(i)).collect();p.staged=None;}
if ui.button("Clear selection").clicked(){p.ids.clear();p.staged=None;}ui.label(format!("Selection order: {:?}",p.ids));});for i in 0..d.scene.worlds.len(){let id=d.scene.catalog.worlds.id(i);let name=d.scene.catalog.world_info.get(&id).and_then(|i|i.name.as_deref()).unwrap_or("3D model");let mut selected=p.ids.contains(&id);let r=ui.checkbox(&mut selected,format!("{id}: {name} (parent {:?})",core::node(&d.scene,id).and_then(|n|n.parent)));p.controls.insert(format!("select-{id}"),r.rect);if r.changed(){if selected{p.ids.push(id);}else{p.ids.retain(|n|*n!=id);}p.staged=None;}}});
            let old_mode=p.preferences.mode;
            ui.horizontal_wrapped(|ui|{for(mode,label)in [(Mode::Move,"Move"),(Mode::Rotate,"Rotate / pose"),(Mode::Scale,"Scale")]{ui.selectable_value(&mut p.preferences.mode,mode,label);}for i in 0..3{ui.selectable_value(&mut p.axis,i,["X","Y","Z"][i]);}for(axes,label)in[(Axes::Floor,"Floor axes"),(Axes::Object,"Object axes")]{ui.selectable_value(&mut p.preferences.axes,axes,label);}});
            if old_mode!=p.preferences.mode{p.value=if p.preferences.mode==Mode::Scale{[1.;3]}else{[0.;3]};}
            ui.horizontal_wrapped(|ui|{for(mode,label)in [(Pivot::Individual,"Individual pivots"),(Pivot::SelectionCenter,"Selection bounds center"),(Pivot::FirstSelected,"First selected pivot")]{ui.selectable_value(&mut p.preferences.pivot,mode,label);}ui.label("Rotation step (0 disables)");ui.add(egui::DragValue::new(&mut p.preferences.step).range(0. ..=180.));});
            ui.collapsing("Articulated parent links (keep world pose)",|ui|{egui::ComboBox::from_label("Parent").selected_text(p.parent.map_or("No parent".into(),|id|format!("Model{id}"))).show_ui(ui,|ui|{ui.selectable_value(&mut p.parent,None,"No parent");for i in 0..d.scene.worlds.len(){let id=d.scene.catalog.worlds.id(i);ui.selectable_value(&mut p.parent,Some(id),format!("Model{id}"));}});if ui.button("Preview parent relation").clicked(){edit=Some(Edit::Parent{ids:p.ids.clone(),parent:p.parent});}});
            ui.collapsing("Document 3D camera / depth clipping",|ui|{let c=p.camera.as_mut().unwrap();ui.horizontal(|ui|{ui.label("Position X/Y/Z");for v in &mut c.position{ui.add(egui::DragValue::new(v).speed(0.1));}});ui.horizontal(|ui|{ui.label("Rotation X/Y/Z degrees");for v in &mut p.lens.angles{ui.add(egui::DragValue::new(v));}});ui.horizontal(|ui|{ui.selectable_value(&mut p.lens.projection,Projection::Perspective,"Perspective");ui.selectable_value(&mut p.lens.projection,Projection::Orthographic,"Orthographic");ui.label("FOV / ortho pixels per unit");ui.add(egui::DragValue::new(&mut p.lens.fov).range(1. ..=179.));ui.add(egui::DragValue::new(&mut c.pixels_per_unit).range(1e-6..=1e6));});ui.horizontal(|ui|{ui.label("Near / far");ui.add(egui::DragValue::new(&mut p.lens.near).speed(0.01));ui.add(egui::DragValue::new(&mut p.lens.far).speed(1.));ui.label("Page left / top / right / bottom");for v in c.page_bounds.min.iter_mut().chain(c.page_bounds.max.iter_mut()){ui.add(egui::DragValue::new(v));}});let r=ui.button("Preview document camera");p.controls.insert("camera".into(),r.rect);if r.clicked(){edit=Some(Edit::Camera{camera:c.clone(),lens:p.lens});}});
        });
        p.open = open;
        if !open {
            p.staged = None;
            p.drag = None;
        }
        self.world3d = p;
        let result = (|| {
            if !open {
                return Ok(());
            }
            match action {
                1 => edit = Some(Edit::Reference),
                2 => self.apply_world3d()?,
                3 => {
                    self.world3d.staged = None;
                    self.world3d.drag = None;
                }
                4 => {
                    if let Some(s) = &self.storage {
                        self.world3d.preferences.save(s.root())?;
                    }
                    self.world3d.notice = "Local gizmo defaults saved".into();
                }
                _ => {}
            }
            if let Some(edit) = edit {
                self.finish_stroke();
                if self.timeline_has_cel_binding()
                    || self.transform_unconfirmed()
                    || self.inference_unconfirmed()
                    || self.text_editor.draft.is_some()
                {
                    return Err("Finish pending edits before preparing 3D changes".into());
                }
                self.world3d.preview(&self.documents[n], edit, budget);
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.world3d.notice = e;
        }
    }
}
#[path = "world3d_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_preview_gizmo_apply_cancel_prefs_stale_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
