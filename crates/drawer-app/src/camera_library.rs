use super::*;
use drawer_core::{
    camera_library::{self as core, Edit},
    world_views::View,
    world3d::Projection,
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
type ImageResult = Result<(Box<core::Prepared>, Vec<u8>), String>;
struct Job {
    stamp: (u64, u64, u32),
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<ImageResult>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    view: Option<View>,
    name: String,
    selected: u64,
    ids: Vec<u64>,
    target: [f64; 3],
    margin: f64,
    stamp: Option<(u64, u64, u32)>,
    job: Option<Job>,
    staged: Option<Box<core::Prepared>>,
    texture: Option<egui::TextureHandle>,
    notice: String,
    pub controls: BTreeMap<String, egui::Rect>,
}
impl Panel {
    fn clear(&mut self) {
        self.job = None;
        self.staged = None;
        self.texture = None;
    }
    fn load(&mut self, d: &Document) {
        self.view = Some(View::scene(d));
        self.selected = d.scene.world3d.cameras.active;
        self.name = d
            .scene
            .world3d
            .cameras
            .entries
            .iter()
            .find(|e| e.id == self.selected)
            .map_or("Scene camera".into(), |e| e.name.clone());
        self.target = drawer_core::world_views::target(d);
        self.ids.retain(|id| {
            d.scene
                .catalog
                .worlds
                .index(*id, d.scene.worlds.len())
                .is_some()
        });
        if !(0.02..=0.4).contains(&self.margin) {
            self.margin = 0.1;
        }
        self.stamp = Some((d.id, d.revision, d.current_frame));
    }
}
impl Drawer {
    pub(super) fn cameras_pending(&self) -> bool {
        self.scene_cameras.job.is_some() || self.scene_cameras.staged.is_some()
    }
    pub(super) fn begin_scene_cameras(&mut self) -> Result<(), String> {
        if self.has_world_preview()
            || self.models_busy()
            || self.rig_pending()
            || self.lighting_pending()
            || self.uv_pending()
            || self.environment_pending()
            || self.line_tones_pending()
            || self.sequence_pending()
        {
            return Err("Apply/discard other3D previews first".into());
        }
        self.finish_stroke();
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending content edits first".into());
        }
        let n = self.active.ok_or("Choose a document")?;
        self.stop_timeline_playback(self.documents[n].id);
        if !self.cameras_pending() {
            self.scene_cameras.load(&self.documents[n]);
        }
        self.hide_world_for_models();
        self.world_models.open = false;
        self.human_rig.open = false;
        self.lighting.open = false;
        self.uv_paint.open = false;
        self.environment.open = false;
        self.line_tones.open = false;
        self.pose_sequences.open = false;
        self.model_presets.open = false;
        self.world_physics.open = false;
        self.world_views.open = false;
        self.scene_cameras.open = true;
        Ok(())
    }
    fn stage_camera_library(&mut self, ctx: &egui::Context, edit: Edit) -> Result<(), String> {
        if self.scene_cameras.job.is_some() {
            return Err("Wait for/cancel camera preparation first".into());
        }
        if self.has_world_preview()
            || self.models_busy()
            || self.rig_pending()
            || self.lighting_pending()
            || self.uv_pending()
            || self.environment_pending()
            || self.line_tones_pending()
            || self.sequence_pending()
            || self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish other previews/content edits first".into());
        }
        let n = self.active.ok_or("Document closed")?;
        self.stop_timeline_playback(self.documents[n].id);
        let d = &self.documents[n];
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        if d.memory_bytes()
            .saturating_mul(3)
            .saturating_add(4 * 1024 * 1024)
            > budget
        {
            return Err("Camera snapshot exceeds memory budget".into());
        }
        let stamp = (d.id, d.revision, d.current_frame);
        let mut frozen = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        frozen.id = d.id;
        frozen.revision = d.revision;
        frozen.keyframe_content = d.keyframe_content;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = (|| {
                let p = core::prepare(&frozen, edit, budget, &flag)?;
                let image = p.thumbnail(&frozen, budget, &flag)?;
                Ok((Box::new(p), image))
            })();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.scene_cameras.clear();
        self.scene_cameras.job = Some(Job {
            stamp,
            cancel,
            rx,
            worker: Some(worker),
        });
        Ok(())
    }
    pub(super) fn scene_cameras_window(&mut self, ctx: &egui::Context) {
        let Some(n) = self.active else {
            self.scene_cameras.clear();
            self.scene_cameras.open = false;
            return;
        };
        let d = &self.documents[n];
        let stamp = (d.id, d.revision, d.current_frame);
        if self.scene_cameras.stamp != Some(stamp) {
            self.scene_cameras.clear();
            self.scene_cameras.load(d);
        }
        if !self.scene_cameras.open {
            self.scene_cameras.clear();
            return;
        }
        if self
            .scene_cameras
            .job
            .as_ref()
            .is_some_and(|j| j.stamp != stamp)
        {
            self.scene_cameras.clear();
        }
        if let Some(result) = self
            .scene_cameras
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok())
        {
            let canceled = self
                .scene_cameras
                .job
                .as_ref()
                .is_some_and(|j| j.cancel.load(Ordering::Relaxed));
            self.scene_cameras.job = None;
            if !canceled {
                match result {
                    Ok((p, image)) => {
                        self.scene_cameras.notice = p.summary.clone();
                        self.scene_cameras.staged = Some(p);
                        self.scene_cameras.texture = Some(ctx.load_texture(
                            "scene-camera-preview",
                            egui::ColorImage::from_rgba_unmultiplied([128, 128], &image),
                            egui::TextureOptions::NEAREST,
                        ));
                    }
                    Err(e) => self.scene_cameras.notice = e,
                }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            self.scene_cameras.clear();
            self.scene_cameras.load(d);
        }
        let mut p = std::mem::take(&mut self.scene_cameras);
        let before = (
            p.view.clone(),
            p.name.clone(),
            p.selected,
            p.ids.clone(),
            p.target,
            p.margin,
        );
        let mut action = 0;
        let busy = p.job.is_some();
        let mut open = true;
        p.controls.clear();
        egui::Window::new("Scene cameras / framing").default_pos(egui::pos2(12.,12.)).default_width(770.).default_height(610.).min_height(520.).max_height(650.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.add_sized([730.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);
            ui.horizontal_wrapped(|ui|{for(key,label,a,enabled)in[("apply","Apply prepared camera",1,p.staged.is_some()),("cancel","Cancel / discard",2,true),("preview","Prepare current camera",3,!busy),("views","Open independent views",4,!busy),("undo","Undo document",5,!busy)]{let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
            ui.add_enabled_ui(!busy,|ui|{
                ui.horizontal_wrapped(|ui|{ui.label(format!("Active camera{}",d.scene.world3d.cameras.active));for e in &d.scene.world3d.cameras.entries{let r=ui.selectable_label(p.selected==e.id,format!("{}: {}",e.id,e.name));p.controls.insert(format!("camera-{}",e.id),r.rect);if r.clicked(){p.selected=e.id;}}});
                ui.horizontal_wrapped(|ui|{ui.text_edit_singleline(&mut p.name);for(key,label,a)in[("enable","Initialize list",6),("new","Create from draft",7),("duplicate","Duplicate active",8),("switch","Prepare switch",9),("rename","Rename active",10),("delete","Delete selected",11)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
                if let Some(v)=&mut p.view{
                    ui.horizontal_wrapped(|ui|{ui.label("Position X/Y/Z");for x in &mut v.camera.position{ui.add(egui::DragValue::new(x).speed(0.1));}ui.label("Rotation X/Y/Z");for(i,x)in v.lens.angles.iter_mut().enumerate(){ui.add_enabled(!v.lens.vertical||i==1,egui::DragValue::new(x));}});
                    ui.horizontal_wrapped(|ui|{for(projection,label,key)in[(Projection::Perspective,"Perspective","perspective"),(Projection::Orthographic,"Parallel","parallel")]{let r=ui.selectable_value(&mut v.lens.projection,projection,label);p.controls.insert(key.into(),r.rect);}ui.label("FOV / ortho pixels per unit");ui.add(egui::DragValue::new(&mut v.lens.fov).range(1. ..=179.));ui.add(egui::DragValue::new(&mut v.camera.pixels_per_unit).range(1e-6..=1e6));});
                    ui.horizontal_wrapped(|ui|{ui.label("Lens shift X/Y (page fraction)");for x in &mut v.lens.shift{ui.add(egui::DragValue::new(x).range(-4. ..=4.).speed(0.01));}ui.label("Near / far");ui.add(egui::DragValue::new(&mut v.lens.near).speed(0.01));ui.add(egui::DragValue::new(&mut v.lens.far).speed(1.));});
                    ui.horizontal_wrapped(|ui|{ui.label("Retain target X/Y/Z when leveling");for x in &mut p.target{ui.add(egui::DragValue::new(x).speed(0.1));}let r=ui.button("Level verticals, retain target");p.controls.insert("level".into(),r.rect);if r.clicked(){action=12;}let r=ui.button("Release vertical mode");p.controls.insert("release".into(),r.rect);if r.clicked(){v.lens.vertical=false;v.lens.level=None;}ui.label(if v.lens.vertical{"Vertical mode: pitch/roll locked"}else{"Free camera rotation"});});
                }
                ui.horizontal_wrapped(|ui|{ui.label("Fit margin per side");ui.add(egui::Slider::new(&mut p.margin,0.02..=0.4));let r=ui.button("Prepare fit selected models");p.controls.insert("fit".into(),r.rect);if r.clicked(){action=13;}
if ui.button("Select all models").clicked(){p.ids=(0..d.scene.worlds.len()).map(|i|d.scene.catalog.worlds.id(i)).collect();}});
                egui::ScrollArea::vertical().id_salt("camera-fit-models").max_height(55.).show(ui,|ui|{for i in 0..d.scene.worlds.len(){let id=d.scene.catalog.worlds.id(i);let mut selected=p.ids.contains(&id);let r=ui.checkbox(&mut selected,format!("Model{id}"));p.controls.insert(format!("model-{id}"),r.rect);if r.changed(){if selected{p.ids.push(id);}else{p.ids.retain(|n|*n!=id);}}}});
            });
            if let Some(texture)=&p.texture{ui.image((texture.id(),egui::vec2(140.,140.)));}ui.small("Named camera switches change the document. Independent editing views and their front cutaway planes are workspace state. Leveling retains the target position using lens shift; Undo restores the prior camera. Fit uses current posed model bounds and updates near/far.");
        });
        p.open = open;
        if !open {
            p.clear();
        }
        if before
            != (
                p.view.clone(),
                p.name.clone(),
                p.selected,
                p.ids.clone(),
                p.target,
                p.margin,
            )
        {
            p.clear();
        }
        self.scene_cameras = p;
        let result = (|| {
            if !open {
                return Ok(());
            }
            let p = &self.scene_cameras;
            let view = || p.view.clone().ok_or("Camera missing");
            let edit = match action {
                3 => Some(Edit::View(view()?)),
                6 => Some(Edit::Enable),
                7 => Some(Edit::New {
                    name: p.name.clone(),
                    view: view()?,
                }),
                8 => Some(Edit::Duplicate(p.name.clone())),
                9 => Some(Edit::Switch(p.selected)),
                10 => Some(Edit::Rename(p.name.clone())),
                11 => Some(Edit::Delete(p.selected)),
                12 => {
                    let v = core::straighten(&view()?, p.target)?;
                    self.scene_cameras.clear();
                    self.scene_cameras.view = Some(v);
                    None
                }
                13 => Some(Edit::Fit {
                    view: view()?,
                    ids: p.ids.clone(),
                    margin: p.margin,
                }),
                _ => None,
            };
            if let Some(edit) = edit {
                self.stage_camera_library(ctx, edit)?;
                return Ok(());
            }
            match action {
                1 => {
                    let prepared = self
                        .scene_cameras
                        .staged
                        .take()
                        .ok_or("Prepare a camera change first")?;
                    prepared.commit(&mut self.documents[n])?;
                    self.scene_cameras.clear();
                    self.scene_cameras.load(&self.documents[n]);
                }
                2 => {
                    self.scene_cameras.clear();
                    self.scene_cameras.load(&self.documents[n]);
                }
                4 => {
                    self.scene_cameras.clear();
                    self.scene_cameras.open = false;
                    self.begin_world_views()?;
                }
                5 => {
                    self.scene_cameras.clear();
                    self.documents[n].undo();
                    self.scene_cameras.load(&self.documents[n]);
                }
                _ => {}
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.scene_cameras.notice = e;
        }
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(20));
        }
    }
}
#[path = "camera_library_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_named_camera_framing_level_fit_undo_cancel_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
