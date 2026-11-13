use super::*;
use drawer_core::environment::{self as core, Edit, Settings};
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
    path: String,
    settings: Settings,
    stamp: Option<(u64, u64, u32)>,
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
    pub(super) fn environment_pending(&self) -> bool {
        self.environment.job.is_some() || self.environment.staged.is_some()
    }
    pub(super) fn begin_environment(&mut self) -> Result<(), String> {
        if self.model_edit_pending() {
            return Err("Apply/discard model controls preview first".into());
        }
        self.model_controls.open = false;
        if self.cameras_pending() {
            return Err("Apply/discard scene camera preview first".into());
        }
        self.scene_cameras.open = false;
        if self.sequence_pending() {
            return Err("Apply/discard pose sequence preview first".into());
        }
        self.pose_sequences.open = false;
        self.finish_stroke();
        if self.models_busy()
            || self.has_world_preview()
            || self.rig_pending()
            || self.lighting_pending()
            || self.uv_pending()
            || self.line_tones_pending()
        {
            return Err("Apply/discard the existing3D preview first".into());
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
        let d = &self.documents[n];
        if !self.environment_pending() {
            self.environment.settings = d
                .scene
                .world3d
                .environment
                .as_ref()
                .map_or_else(Settings::default, |e| e.settings);
            self.environment.stamp = Some((d.id, d.revision, d.current_frame));
        }
        self.hide_world_for_models();
        self.world_models.open = false;
        self.human_rig.open = false;
        self.lighting.open = false;
        self.uv_paint.open = false;
        self.line_tones.open = false;
        self.world_physics.open = false;
        self.model_presets.open = false;
        self.environment.open = true;
        Ok(())
    }
    fn stage_environment(&mut self, ctx: &egui::Context, edit: Edit) -> Result<(), String> {
        if self.environment.job.is_some() {
            return Err("Cancel the current panorama operation first".into());
        }
        if self.sequence_pending()
            || self.has_world_preview()
            || self.models_busy()
            || self.rig_pending()
            || self.lighting_pending()
            || self.uv_pending()
            || self.line_tones_pending()
            || self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish current previews/content edits first".into());
        }
        self.finish_stroke();
        let n = self.active.ok_or("Document closed")?;
        self.stop_timeline_playback(self.documents[n].id);
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let d = &self.documents[n];
        if d.memory_bytes()
            .saturating_mul(3)
            .saturating_add(4 * 1024 * 1024)
            > budget
        {
            return Err("Panorama snapshot exceeds available memory".into());
        }
        let (id, revision, frame) = (d.id, d.revision, d.current_frame);
        let mut frozen = drawer_core::layer_merge::preview(&d.snapshot(), frame)?;
        frozen.id = id;
        frozen.revision = revision;
        frozen.keyframe_content = d.keyframe_content;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = core::prepare(&frozen, edit, budget, &flag).map(Box::new);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.environment.clear();
        self.environment.job = Some(Job {
            id,
            revision,
            frame,
            cancel,
            rx,
            worker: Some(worker),
        });
        self.environment.notice = "Preparing local panorama snapshot…".into();
        Ok(())
    }
    pub(super) fn environment_window(&mut self, ctx: &egui::Context) {
        let Some(n) = self.active else {
            self.environment.clear();
            self.environment.open = false;
            self.environment.stamp = None;
            return;
        };
        let d = &self.documents[n];
        let stamp = (d.id, d.revision, d.current_frame);
        if self.environment.stamp != Some(stamp) {
            self.environment.clear();
            self.environment.stamp = Some(stamp);
            self.environment.settings = d
                .scene
                .world3d
                .environment
                .as_ref()
                .map_or_else(Settings::default, |e| e.settings);
            self.environment.notice =
                "Document/frame changed; unconfirmed panorama discarded".into();
        }
        if !self.environment.open {
            self.environment.clear();
            return;
        }
        if self
            .environment
            .job
            .as_ref()
            .is_some_and(|j| (j.id, j.revision, j.frame) != stamp)
        {
            self.environment.clear();
        }
        let ready = self
            .environment
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok());
        if let Some(result) = ready {
            let canceled = self
                .environment
                .job
                .as_ref()
                .is_some_and(|j| j.cancel.load(Ordering::Relaxed));
            self.environment.job = None;
            if !canceled {
                match result {
                    Ok(p) => {
                        if p.matches(d) {
                            self.environment.texture = Some(ctx.load_texture(
                                "environment-preview",
                                egui::ColorImage::from_rgba_unmultiplied([128, 128], &p.thumbnail),
                                egui::TextureOptions::NEAREST,
                            ));
                            self.environment.staged = Some(p);
                            self.environment.review = false;
                            self.environment.notice="Review the embedded image, rendering and source-transfer notices before Apply".into();
                        }
                    }
                    Err(e) => self.environment.notice = e,
                }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            self.environment.clear();
        }
        let mut p = std::mem::take(&mut self.environment);
        let before = (p.path.clone(), p.settings);
        let busy = p.job.is_some();
        let mut action = 0;
        let mut open = true;
        p.controls.clear();
        egui::Window::new("Local panorama / diffuse environment").default_pos(egui::pos2(15.,15.)).default_width(740.).max_height(650.).vscroll(false).open(&mut open).show(ctx,|ui|{
            ui.add_sized([700.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);
            ui.horizontal_wrapped(|ui|{for(key,label,a,enabled)in[("file","Preview local file",1,!busy),("values","Preview embedded values",2,!busy&&d.scene.world3d.environment.is_some()),("remove","Preview removal",3,!busy),("apply","Apply environment",4,p.staged.as_ref().is_some_and(|s|s.warnings().is_empty()||p.review)),("cancel","Cancel / discard",5,true)]{let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
            ui.add_enabled_ui(!busy,|ui|{ui.horizontal(|ui|{ui.label("PNG / JPEG / Radiance HDR");ui.text_edit_singleline(&mut p.path);if ui.button(crate::i18n::text("Browse…")).clicked(){action=6;}});ui.horizontal(|ui|{ui.checkbox(&mut p.settings.background,"Panorama background");ui.checkbox(&mut p.settings.illumination,"Diffuse image illumination");});ui.horizontal(|ui|{ui.label("Environment yaw");ui.add(egui::Slider::new(&mut p.settings.yaw,-360. ..=360.).suffix("°"));ui.label("Light strength");ui.add(egui::DragValue::new(&mut p.settings.strength).speed(0.05).range(0. ..=64.));});ui.horizontal(|ui|{ui.label("Background exposure EV");ui.add(egui::Slider::new(&mut p.settings.exposure,-16. ..=16.));});});
            if busy{ui.spinner();}
            if let Some(s)=&p.staged{if !s.warnings().is_empty(){let r=ui.checkbox(&mut p.review,"I reviewed transfer / capacity / renderer notices and the preview");p.controls.insert("review".into(),r.rect);}egui::ScrollArea::vertical().id_salt("environment-notices").max_height(105.).show(ui,|ui|{for warning in s.warnings(){ui.colored_label(egui::Color32::YELLOW,warning);}});}
            if let Some(texture)=&p.texture{ui.image((texture.id(),egui::vec2(180.,180.)));}else{ui.allocate_space(egui::vec2(180.,180.));}
            if let Some(e)=d.scene.world3d.environment.as_ref(){ui.small(format!("Committed: {} — {:?}, {}x{} linear embedded pixels",e.source,e.encoding,e.width,e.height));}
            ui.small("Local2:1 equirectangular PNG/JPEG or linear Radiance HDR. Image/yaw/strength stay in the document. Background precedes the World render group; advanced diffuse image light has no local occlusion/specular shadows. Source files are never modified or fetched.");
        });
        p.open = open;
        if !open || (p.path.clone(), p.settings) != before {
            p.clear();
        }
        self.environment = p;
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
        let result = (|| {
            if !open {
                return Ok(());
            }
            match action {
                1 => self.stage_environment(
                    ctx,
                    Edit::Load {
                        path: self.environment.path.clone().into(),
                        settings: self.environment.settings,
                    },
                )?,
                2 => self.stage_environment(ctx, Edit::Settings(self.environment.settings))?,
                3 => self.stage_environment(ctx, Edit::Remove)?,
                4 => {
                    let p = self
                        .environment
                        .staged
                        .take()
                        .ok_or("Prepare a panorama preview first")?;
                    if !p.warnings().is_empty() && !self.environment.review {
                        self.environment.staged = Some(p);
                        return Err("Review the environment notices before Apply".into());
                    }
                    let changed = p.commit(&mut self.documents[n])?;
                    self.environment.clear();
                    self.environment.stamp = Some((
                        self.documents[n].id,
                        self.documents[n].revision,
                        self.documents[n].current_frame,
                    ));
                    self.environment.notice = if changed {
                        "Environment applied; one Undo restores previous scene"
                    } else {
                        "No environment change; no Undo added"
                    }
                    .into();
                }
                5 => {
                    self.environment.clear();
                    self.environment.notice =
                        "Cancelled; original scene/source image retained".into();
                }
                6 => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Local panorama", &["png", "jpg", "jpeg", "hdr"])
                        .pick_file()
                    {
                        self.environment.clear();
                        self.environment.path = path.display().to_string();
                    }
                }
                _ => {}
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.environment.notice = e;
        }
    }
}
#[path = "environment_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_local_panorama_review_apply_cancel_values_stale_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
