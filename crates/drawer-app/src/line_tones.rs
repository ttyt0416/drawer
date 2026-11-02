use super::*;
use drawer_core::line_tones::{self as core, Input, Settings};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
struct Job {
    id: u64,
    revision: u64,
    frame: u32,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Box<core::Prepared>, String>>,
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
    settings: Settings,
    job: Option<Job>,
    staged: Option<Box<core::Prepared>>,
    texture: Option<egui::TextureHandle>,
    review: bool,
    notice: String,
    pub controls: BTreeMap<String, egui::Rect>,
}
impl Panel {
    fn clear(&mut self) {
        self.job = None;
        self.staged = None;
        self.texture = None;
        self.review = false;
    }
}
impl Drawer {
    pub(super) fn line_tones_pending(&self) -> bool {
        self.line_tones.job.is_some() || self.line_tones.staged.is_some()
    }
    pub(super) fn begin_line_tones(&mut self) -> Result<(), String> {
        if self.environment_pending() {
            return Err("Apply/discard the environment preview first".into());
        }
        self.environment.open = false;
        self.finish_stroke();
        if self.models_busy()
            || self.has_world_preview()
            || self.rig_pending()
            || self.lighting_pending()
            || self.uv_pending()
        {
            return Err("Apply / discard existing3D previews first".into());
        }
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending content edits first".into());
        }
        let n = self.active.ok_or("Choose a document")?;
        self.stop_timeline_playback(self.documents[n].id);
        if !self.line_tones_pending() {
            self.line_tones.settings.input = if self.documents[n]
                .scene
                .world3d
                .surfaces
                .values()
                .any(|s| !s.triangles.is_empty())
            {
                Input::Scene
            } else {
                Input::Image(self.documents[n].scene.layer_ids.id(0))
            };
        }
        self.hide_world_for_models();
        self.world_models.open = false;
        self.model_presets.open = false;
        self.human_rig.open = false;
        self.lighting.open = false;
        self.uv_paint.open = false;
        self.world_physics.open = false;
        self.line_tones.open = true;
        Ok(())
    }
    fn stage_line_tones(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.line_tones.job.is_some() {
            return Err("Cancel the current extraction first".into());
        }
        self.finish_stroke();
        let n = self.active.ok_or("Document closed")?;
        if self.has_world_preview()
            || self.rig_pending()
            || self.lighting_pending()
            || self.uv_pending()
            || self.models_busy()
            || self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish current previews / content edits first".into());
        }
        self.stop_timeline_playback(self.documents[n].id);
        let d = &self.documents[n];
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        if d.memory_bytes().saturating_mul(3) > budget {
            return Err("Extraction snapshot exceeds remaining memory".into());
        }
        let mut frozen = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        frozen.id = d.id;
        frozen.revision = d.revision;
        frozen.keyframe_content = d.keyframe_content;
        let (id, revision, frame) = (d.id, d.revision, d.current_frame);
        let settings = self.line_tones.settings;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = core::prepare(&frozen, settings, budget, &flag).map(Box::new);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.line_tones.clear();
        self.line_tones.job = Some(Job {
            id,
            revision,
            frame,
            cancel,
            rx,
            worker: Some(worker),
        });
        self.line_tones.notice =
            "Preparing contour / tone layers; original content unchanged".into();
        Ok(())
    }
    pub(super) fn line_tones_window(&mut self, ctx: &egui::Context) {
        let Some(n) = self.active else {
            self.line_tones.clear();
            self.line_tones.open = false;
            return;
        };
        let d = &self.documents[n];
        if !self.line_tones.open {
            self.line_tones.clear();
            return;
        }
        if self
            .line_tones
            .job
            .as_ref()
            .is_some_and(|j| j.id != d.id || j.revision != d.revision || j.frame != d.current_frame)
            || self
                .line_tones
                .staged
                .as_ref()
                .is_some_and(|p| !p.matches(d))
        {
            self.line_tones.clear();
            self.line_tones.notice = "Source / frame changed; extraction discarded".into();
        }
        let ready = self
            .line_tones
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok());
        if let Some(result) = ready {
            let cancelled = self
                .line_tones
                .job
                .as_ref()
                .is_some_and(|j| j.cancel.load(Ordering::Relaxed));
            self.line_tones.job = None;
            if !cancelled {
                match result {
                    Ok(p) => {
                        if p.matches(d) {
                            self.line_tones.notice = p.summary.clone();
                            if !p.thumbnail.is_empty() {
                                self.line_tones.texture = Some(ctx.load_texture(
                                    "line-tone-preview",
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [128, 128],
                                        &p.thumbnail,
                                    ),
                                    egui::TextureOptions::NEAREST,
                                ));
                            }
                            self.line_tones.staged = Some(p);
                            self.line_tones.review = false;
                        }
                    }
                    Err(e) => self.line_tones.notice = e,
                }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            self.line_tones.clear();
        }
        let mut p = std::mem::take(&mut self.line_tones);
        p.controls.clear();
        let before = p.settings;
        let busy = p.job.is_some();
        let mut action = 0;
        let mut open = true;
        egui::Window::new("Contour / stepped-tone extraction").default_pos(egui::pos2(15.,15.)).default_width(730.).max_height(650.).vscroll(false).open(&mut open).show(ctx,|ui|{
 ui.add_sized([690.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);ui.horizontal(|ui|{for(key,label,a,enabled)in[("preview","Prepare preview",1,!busy),("apply","Apply new layers",2,p.staged.as_ref().is_some_and(|s|s.warnings.is_empty()||p.review)),("cancel","Cancel / discard",3,true)]{let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
 if busy{ui.spinner();}
 ui.add_enabled_ui(!busy,|ui|{ui.horizontal(|ui|{ui.selectable_value(&mut p.settings.input,Input::Scene,"3D surface render");let selected=if let Input::Image(id)=p.settings.input{id}else{d.scene.layer_ids.id(0)};ui.selectable_value(&mut p.settings.input,Input::Image(selected),"Raster image layer");});if let Input::Image(id)=&mut p.settings.input{egui::ComboBox::from_id_salt("line-tone-raster").selected_text(format!("Layer {id}")).show_ui(ui,|ui|{for i in 0..d.layers.len(){ui.selectable_value(id,d.scene.layer_ids.id(i),&d.layers[i].name);}});}ui.horizontal(|ui|{ui.checkbox(&mut p.settings.lines,"Contour layer");ui.checkbox(&mut p.settings.tones,"Stepped-tone layer");ui.checkbox(&mut p.settings.silhouette,"Alpha silhouette");});ui.horizontal(|ui|{ui.checkbox(&mut p.settings.luminance,"Luminance edges");ui.add_enabled_ui(p.settings.input==Input::Scene,|ui|{ui.checkbox(&mut p.settings.depth,"Depth edges");ui.checkbox(&mut p.settings.normals,"Normal edges");});});
 ui.horizontal(|ui|{ui.label("Luminance threshold");ui.add(egui::DragValue::new(&mut p.settings.luma_threshold).speed(0.01).range(0.0001..=1.));ui.label("Relative depth threshold");ui.add(egui::DragValue::new(&mut p.settings.depth_threshold).speed(0.01).range(0.0001..=1.));});ui.horizontal(|ui|{ui.label("Normal angle degrees");ui.add(egui::DragValue::new(&mut p.settings.normal_angle).range(1. ..=180.));ui.label("Contour radius");ui.add(egui::DragValue::new(&mut p.settings.radius).range(1..=4));ui.label("Tone bands");ui.add(egui::DragValue::new(&mut p.settings.bands).range(2..=16));});ui.horizontal(|ui|{ui.label("Result strength");let r=ui.add(egui::Slider::new(&mut p.settings.strength,0. ..=1.));p.controls.insert("strength".into(),r.rect);});});
 if let Some(s)=&p.staged{for warning in &s.warnings{ui.colored_label(egui::Color32::YELLOW,warning);}
if !s.warnings.is_empty(){let r=ui.checkbox(&mut p.review,"I reviewed the dense contour preview");p.controls.insert("review".into(),r.rect);}}
if let Some(texture)=&p.texture{ui.image((texture.id(),egui::vec2(230.,230.)));}else{ui.allocate_space(egui::vec2(230.,230.));}ui.small("Creates separate Normal /100% raster snapshots above the sources. Originals remain editable.3D uses visible front surfaces; wire views are sampled as Unlit. Strength0 keeps the document unchanged. Camera output requires explicit base-content editing.");
 });
        p.open = open;
        if !open {
            p.clear();
        }
        if p.settings != before {
            p.clear();
        }
        self.line_tones = p;
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
        let result = (|| {
            if !open {
                return Ok(());
            }
            match action {
                1 => self.stage_line_tones(ctx)?,
                2 => {
                    let p = self
                        .line_tones
                        .staged
                        .take()
                        .ok_or("Prepare the extraction preview first")?;
                    if !p.warnings.is_empty() && !self.line_tones.review {
                        self.line_tones.staged = Some(p);
                        return Err("Review dense contour notices before Apply".into());
                    }
                    p.commit(&mut self.documents[n])?;
                    self.line_tones.clear();
                    self.line_tones.notice =
                        "Created snapshot layers; one Undo removes the result".into();
                }
                3 => {
                    self.line_tones.clear();
                    self.line_tones.notice = "Cancelled; original content retained".into();
                }
                _ => {}
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.line_tones.notice = e;
        }
    }
}
#[path = "line_tones_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_extract_preview_dense_review_apply_cancel_and_stale() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
