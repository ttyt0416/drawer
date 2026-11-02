use super::*;
use drawer_core::{
    keyframes::Curve,
    pose_sequence::{self as core, Edit, Pose},
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
enum ResultImage {
    Change(Box<core::Prepared>, Vec<u8>),
    View(Vec<u8>),
}
struct Job {
    stamp: (u64, u64, u32),
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<ResultImage, String>>,
    worker: Option<std::thread::JoinHandle<()>>,
    view: bool,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
pub(super) struct Panel {
    pub open: bool,
    rig: Option<u64>,
    asset: Option<u64>,
    pose: usize,
    joint: usize,
    draft: Option<Pose>,
    base: bool,
    name: String,
    path: String,
    output: String,
    step: u32,
    start: u32,
    repeats: u32,
    curve: Curve,
    stamp: Option<(u64, u64, u32)>,
    job: Option<Job>,
    staged: Option<Box<core::Prepared>>,
    texture: Option<egui::TextureHandle>,
    review: bool,
    notice: String,
    play: Option<f64>,
    loop_preview: bool,
    last_frame: Option<u32>,
    pub controls: BTreeMap<String, egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            rig: None,
            asset: None,
            pose: 0,
            joint: 0,
            draft: None,
            base: false,
            name: "Pose sequence".into(),
            path: String::new(),
            output: String::new(),
            step: 3,
            start: 0,
            repeats: 1,
            curve: Curve::Fixed,
            stamp: None,
            job: None,
            staged: None,
            texture: None,
            review: false,
            notice: String::new(),
            play: None,
            loop_preview: true,
            last_frame: None,
            controls: BTreeMap::new(),
        }
    }
}
impl Panel {
    fn clear(&mut self) {
        self.job = None;
        self.staged = None;
        self.texture = None;
        self.review = false;
        self.play = None;
        self.last_frame = None;
    }
    fn load(&mut self, d: &Document) {
        self.rig = self
            .rig
            .filter(|id| d.scene.world3d.rigs.contains_key(id))
            .or_else(|| d.scene.world3d.rigs.keys().next().copied());
        self.asset = self
            .asset
            .filter(|id| d.scene.world3d.sequences.assets.contains_key(id))
            .or_else(|| d.scene.world3d.sequences.assets.keys().next().copied());
        if self.base {
            self.draft = self
                .rig
                .and_then(|id| d.scene.world3d.rigs.get(&id))
                .map(Pose::of);
        } else {
            self.draft = self
                .asset
                .and_then(|id| d.scene.world3d.sequences.assets.get(&id))
                .and_then(|a| {
                    self.pose = self.pose.min(a.clip.poses.len().saturating_sub(1));
                    a.clip.poses.get(self.pose).cloned()
                });
        }
        if let Some(a) = self
            .asset
            .and_then(|id| d.scene.world3d.sequences.assets.get(&id))
        {
            self.name = a.clip.name.clone();
            self.step = a.clip.step;
        }
        self.joint = 0;
        self.stamp = Some((d.id, d.revision, d.current_frame));
    }
}
impl Drawer {
    pub(super) fn sequence_pending(&self) -> bool {
        self.pose_sequences.staged.is_some()
            || self.pose_sequences.job.as_ref().is_some_and(|j| !j.view)
    }
    pub(super) fn begin_pose_sequences(&mut self) -> Result<(), String> {
        if self.model_edit_pending() {
            return Err("Apply/discard model controls preview first".into());
        }
        self.model_controls.open = false;
        if self.cameras_pending() {
            return Err("Apply/discard scene camera preview first".into());
        }
        self.scene_cameras.open = false;
        self.finish_stroke();
        if self.has_world_preview()
            || self.models_busy()
            || self.rig_pending()
            || self.lighting_pending()
            || self.uv_pending()
            || self.line_tones_pending()
            || self.environment_pending()
        {
            return Err("Apply/discard existing3D previews first".into());
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
        if !self.sequence_pending() {
            self.pose_sequences.load(&self.documents[n]);
        }
        self.pose_sequences.open = true;
        self.hide_world_for_models();
        self.world_models.open = false;
        self.human_rig.open = false;
        self.model_presets.open = false;
        self.environment.open = false;
        self.lighting.open = false;
        self.uv_paint.open = false;
        self.line_tones.open = false;
        self.world_physics.open = false;
        Ok(())
    }
    fn stage_sequence(
        &mut self,
        ctx: &egui::Context,
        edit: Option<Edit>,
        preview: Option<Pose>,
    ) -> Result<(), String> {
        if edit.is_none() && self.pose_sequences.staged.is_some() {
            return Err("Apply/discard the prepared change before preview playback".into());
        }
        if self.pose_sequences.job.is_some() {
            return Err("Wait for/cancel sequence preparation first".into());
        }
        if edit.is_some()
            && (self.cameras_pending()
                || self.has_world_preview()
                || self.models_busy()
                || self.rig_pending()
                || self.lighting_pending()
                || self.uv_pending()
                || self.environment_pending()
                || self.line_tones_pending()
                || self.timeline_has_cel_binding()
                || self.transform_unconfirmed()
                || self.inference_unconfirmed()
                || self.text_editor.draft.is_some())
        {
            return Err("Finish other previews/content edits first".into());
        }
        let n = self.active.ok_or("Document closed")?;
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
            return Err("Pose sequence snapshot exceeds memory budget".into());
        }
        let stamp = (d.id, d.revision, d.current_frame);
        let mut frozen = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        frozen.id = d.id;
        frozen.revision = d.revision;
        frozen.keyframe_content = d.keyframe_content;
        let rig = self.pose_sequences.rig;
        let pose = self.pose_sequences.pose;
        let view = edit.is_none();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = (|| {
                if let Some(edit) = edit {
                    let p = core::prepare(&frozen, edit, budget, &flag)?;
                    let image = p.thumbnail(&frozen, rig, pose, budget, &flag)?;
                    Ok(ResultImage::Change(Box::new(p), image))
                } else {
                    Ok(ResultImage::View(core::preview_pose(
                        &frozen,
                        rig.ok_or("Choose a preview rig")?,
                        &preview.ok_or("Choose a pose")?,
                        budget,
                        &flag,
                    )?))
                }
            })();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.pose_sequences.staged = None;
        self.pose_sequences.review = false;
        self.pose_sequences.job = Some(Job {
            stamp,
            cancel,
            rx,
            worker: Some(worker),
            view,
        });
        Ok(())
    }
    pub(super) fn pose_sequences_window(&mut self, ctx: &egui::Context) {
        let Some(n) = self.active else {
            self.pose_sequences.clear();
            self.pose_sequences.open = false;
            return;
        };
        let d = &self.documents[n];
        let stamp = (d.id, d.revision, d.current_frame);
        if self.pose_sequences.stamp != Some(stamp) {
            self.pose_sequences.clear();
            self.pose_sequences.load(d);
        }
        if !self.pose_sequences.open {
            self.pose_sequences.clear();
            return;
        }
        if self
            .pose_sequences
            .job
            .as_ref()
            .is_some_and(|j| j.stamp != stamp)
        {
            self.pose_sequences.clear();
        }
        let result = self
            .pose_sequences
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok());
        if let Some(result) = result {
            let canceled = self
                .pose_sequences
                .job
                .as_ref()
                .is_some_and(|j| j.cancel.load(Ordering::Relaxed));
            self.pose_sequences.job = None;
            if !canceled {
                match result {
                    Ok(value) => {
                        let image = match value {
                            ResultImage::Change(p, image) => {
                                self.pose_sequences.notice = p.summary.clone();
                                self.pose_sequences.staged = Some(p);
                                image
                            }
                            ResultImage::View(image) => image,
                        };
                        self.pose_sequences.texture = Some(ctx.load_texture(
                            "pose-sequence-preview",
                            egui::ColorImage::from_rgba_unmultiplied([128, 128], &image),
                            egui::TextureOptions::NEAREST,
                        ));
                    }
                    Err(e) => {
                        self.pose_sequences.notice = e;
                        self.pose_sequences.play = None;
                    }
                }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            self.pose_sequences.clear();
            self.pose_sequences.load(d);
        }
        let mut p = std::mem::take(&mut self.pose_sequences);
        let old_selection = (p.rig, p.asset, p.pose, p.base);
        let old_config = (
            p.name.clone(),
            p.path.clone(),
            p.step,
            p.start,
            p.repeats,
            p.curve,
            p.draft.clone(),
        );
        let busy = p.job.is_some();
        let playing = p.play.is_some();
        let mut action = 0;
        let mut open = true;
        p.controls.clear();
        egui::Window::new("Local pose sequences / explicit timeline links").default_pos(egui::pos2(10.,10.)).default_width(790.).default_height(620.).min_height(520.).max_height(660.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.add_sized([750.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);
            ui.horizontal_wrapped(|ui|{for(key,label,a,enabled)in[("apply","Apply prepared change",1,p.staged.as_ref().is_some_and(|s|s.warnings.is_empty()||p.review)),("cancel","Cancel / discard",2,true),("preview","Preview draft pose",3,!busy&&!playing&&p.staged.is_none()&&p.draft.is_some()),("play","Play saved sequence preview",4,!busy&&!playing&&p.staged.is_none()&&p.asset.is_some()&&p.rig.is_some()),("stop","Stop preview",5,playing),("timeline","Open timeline playback",6,!busy&&!playing),("undo","Undo document",7,!busy&&!playing)]{let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}ui.checkbox(&mut p.loop_preview,"Preview loop (view only)");});
            if let Some(s)=&p.staged{if !s.warnings.is_empty(){let r=ui.checkbox(&mut p.review,"I reviewed placement range notices");p.controls.insert("review".into(),r.rect);}for w in &s.warnings{ui.colored_label(egui::Color32::YELLOW,w);}}
            ui.add_enabled_ui(!busy&&!playing,|ui|{
                ui.horizontal_wrapped(|ui|{egui::ComboBox::from_label("Original rig").selected_text(p.rig.map_or("Choose rig".into(),|id|format!("Model{id}"))).show_ui(ui,|ui|{for(id,r)in &d.scene.world3d.rigs{ui.selectable_value(&mut p.rig,Some(*id),format!("{id}: {:?}",r.kind));}});egui::ComboBox::from_label("Sequence").selected_text(p.asset.and_then(|id|d.scene.world3d.sequences.assets.get(&id)).map_or("None".into(),|a|a.clip.name.clone())).show_ui(ui,|ui|{for(id,a)in &d.scene.world3d.sequences.assets{ui.selectable_value(&mut p.asset,Some(*id),format!("{id}: {} /{} poses",a.clip.name,a.clip.poses.len()));}});ui.text_edit_singleline(&mut p.name);});
                ui.horizontal_wrapped(|ui|{for(key,label,a)in[("capture","Register base pose",8),("append","Append base pose",9),("append-sample","Append timeline pose",10),("timing","Save default timing",11),("rename","Rename sequence",12),("delete","Delete sequence",13)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
                ui.horizontal_wrapped(|ui|{ui.label("Local .drawerposes file");ui.text_edit_singleline(&mut p.path);if ui.button("Browse input…").clicked(){action=14;}for(key,label,a)in[("load","Register file snapshot",15),("reload","Reload chosen sequence",16)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
                ui.horizontal_wrapped(|ui|{ui.label("Output .drawerposes");ui.text_edit_singleline(&mut p.output);let r=ui.button("Export snapshot");p.controls.insert("export".into(),r.rect);if r.clicked(){action=17;}});
                ui.horizontal_wrapped(|ui|{ui.label("Pose");let count=p.asset.and_then(|id|d.scene.world3d.sequences.assets.get(&id)).map_or(0,|a|a.clip.poses.len());for i in 0..count.min(32){let r=ui.selectable_label(p.pose==i,format!("{}",i+1));p.controls.insert(format!("pose-{i}"),r.rect);if r.clicked(){p.pose=i;}}
if count>32{ui.add(egui::DragValue::new(&mut p.pose).range(0..=count-1));}for(key,label,a)in[("up","Move pose earlier",18),("down","Move pose later",19),("remove-pose","Remove pose",20)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
                ui.horizontal_wrapped(|ui|{let r=ui.selectable_value(&mut p.base,false,"Edit selected sequence pose");p.controls.insert("scope-pose".into(),r.rect);let r=ui.selectable_value(&mut p.base,true,"Edit base scene pose");p.controls.insert("scope-base".into(),r.rect);let r=ui.button("Prepare pose edit");p.controls.insert("pose-edit".into(),r.rect);if r.clicked(){action=21;}});
                if let Some(draft)=&mut p.draft{let bones=if p.base{p.rig.and_then(|id|d.scene.world3d.rigs.get(&id)).map(core::Skeleton::of).map(|s|s.bones)}else{p.asset.and_then(|id|d.scene.world3d.sequences.assets.get(&id)).map(|a|a.clip.skeleton.bones.clone())};if let Some(bones)=bones.filter(|b|!b.is_empty()){p.joint=p.joint.min(bones.len()-1);egui::ComboBox::from_label("Joint").selected_text(&bones[p.joint].name).show_ui(ui,|ui|{for(i,b)in bones.iter().enumerate(){ui.selectable_value(&mut p.joint,i,&b.name);}});ui.horizontal(|ui|{if let Some(angles)=draft.angles.get_mut(p.joint){for(i,v)in angles.iter_mut().enumerate(){ui.add(egui::DragValue::new(v).range(bones[p.joint].min[i]..=bones[p.joint].max[i]));}}ui.checkbox(&mut draft.mirrored,"Mirrored pose");});}}
                ui.horizontal_wrapped(|ui|{ui.label("Start frame (0-based)");ui.add(egui::DragValue::new(&mut p.start).range(0..=9999));ui.label("Frames per pose / default");ui.add(egui::DragValue::new(&mut p.step).range(1..=10000));ui.label("Placed repeats");ui.add(egui::DragValue::new(&mut p.repeats).range(1..=1000));for(c,label)in[(Curve::Fixed,"Fixed"),(Curve::Linear,"Linear")]{ui.selectable_value(&mut p.curve,c,label);}
if ui.button("Ease curve").clicked(){p.curve=Curve::Bezier{first:[0.25,0.],second:[0.75,1.]};}});
                p.controls.insert("bind-clip".into(),ui.clip_rect());
                ui.horizontal_wrapped(|ui|{for(key,label,a)in[("bind","Prepare explicit link to active timeline",22),("unbind","Prepare detach from active timeline",23)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
            });
            ui.horizontal(|ui|{if let Some(texture)=&p.texture{ui.image((texture.id(),egui::vec2(140.,140.)));}ui.vertical(|ui|{ui.small("Selected pose preview is independent of the document playhead. Registration never animates a rig; only an explicit active-timeline link does. Outside its range, endpoint poses hold. Preview loop does not change document time/history.");if let Some(id)=p.rig&&let Some(b)=core::active(&d.scene,id){ui.small(format!("Active link: asset{}; start{}; {} frames/pose; {} repeats; {:?}",b.asset,b.start,b.step,b.repeats,b.curve));}
if let Some(label)=d.undo_label(){ui.small(format!("Undo: {label}"));}
if let Some(label)=d.redo_label(){ui.small(format!("Redo: {label}"));}});});
        });
        p.open = open;
        if !open {
            p.clear();
        }
        if old_selection != (p.rig, p.asset, p.pose, p.base) {
            p.clear();
            p.load(d);
        } else if old_config
            != (
                p.name.clone(),
                p.path.clone(),
                p.step,
                p.start,
                p.repeats,
                p.curve,
                p.draft.clone(),
            )
        {
            p.clear();
        }
        self.pose_sequences = p;
        if busy || playing {
            ctx.request_repaint_after(std::time::Duration::from_millis(20));
        }
        let result = (|| {
            if !open {
                return Ok(());
            }
            let p = &self.pose_sequences;
            let rig = || p.rig.ok_or("Choose an original rig");
            let asset = || p.asset.ok_or("Choose a registered sequence");
            let edit = match action {
                8 => Some(Edit::Capture {
                    rig: rig()?,
                    name: p.name.clone(),
                    step: p.step,
                }),
                9 | 10 => Some(Edit::Append {
                    asset: asset()?,
                    rig: rig()?,
                    sampled: action == 10,
                }),
                11 => Some(Edit::Timing {
                    asset: asset()?,
                    step: p.step,
                }),
                12 => Some(Edit::Rename {
                    asset: asset()?,
                    name: p.name.clone(),
                }),
                13 => Some(Edit::Delete(asset()?)),
                15 => Some(Edit::Load(p.path.clone().into())),
                16 => Some(Edit::Reload {
                    asset: asset()?,
                    path: p.path.clone().into(),
                }),
                18 | 19 => {
                    let to = if action == 18 {
                        p.pose.saturating_sub(1)
                    } else {
                        p.pose + 1
                    };
                    Some(Edit::Order {
                        asset: asset()?,
                        from: p.pose,
                        to,
                    })
                }
                20 => Some(Edit::RemovePose {
                    asset: asset()?,
                    index: p.pose,
                }),
                21 => {
                    let pose = p.draft.clone().ok_or("Choose a pose")?;
                    Some(if p.base {
                        Edit::BasePose { rig: rig()?, pose }
                    } else {
                        Edit::Pose {
                            asset: asset()?,
                            index: p.pose,
                            pose,
                        }
                    })
                }
                22 => Some(Edit::Bind {
                    rig: rig()?,
                    asset: asset()?,
                    start: p.start,
                    step: p.step,
                    repeats: p.repeats,
                    curve: p.curve,
                }),
                23 => Some(Edit::Unbind(rig()?)),
                _ => None,
            };
            if let Some(edit) = edit {
                self.pose_sequences.play = None;
                self.stage_sequence(ctx, Some(edit), None)?;
                return Ok(());
            }
            match action {
                1 => {
                    let p = self
                        .pose_sequences
                        .staged
                        .take()
                        .ok_or("Prepare a sequence change first")?;
                    if !p.warnings.is_empty() && !self.pose_sequences.review {
                        self.pose_sequences.staged = Some(p);
                        return Err("Review pose placement notices".into());
                    }
                    let asset = p.asset;
                    let pose = p.pose;
                    p.commit(&mut self.documents[n])?;
                    self.pose_sequences.clear();
                    if let Some(asset) = asset {
                        self.pose_sequences.asset = Some(asset);
                    }
                    if let Some(pose) = pose {
                        self.pose_sequences.pose = pose;
                    }
                    self.pose_sequences.load(&self.documents[n]);
                }
                2 => {
                    self.pose_sequences.clear();
                    self.pose_sequences.load(&self.documents[n]);
                }
                5 => self.pose_sequences.clear(),
                3 => self.stage_sequence(ctx, None, self.pose_sequences.draft.clone())?,
                4 => {
                    self.pose_sequences.play = Some(ctx.input(|i| i.time));
                    self.pose_sequences.last_frame = None;
                }
                6 => {
                    self.pose_sequences.clear();
                    self.begin_timeline_manager()?;
                }
                7 => {
                    self.pose_sequences.clear();
                    self.documents[n].undo();
                    self.pose_sequences.load(&self.documents[n]);
                }
                14 => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Original pose sequence", &["drawerposes"])
                        .pick_file()
                    {
                        self.pose_sequences.clear();
                        self.pose_sequences.path = path.display().to_string();
                    }
                }
                17 => core::export(&self.documents[n], asset()?, Path::new(&p.output))?,
                _ => {}
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.pose_sequences.notice = e;
            self.pose_sequences.play = None;
        }
        if let Some(start) = self.pose_sequences.play
            && self.pose_sequences.job.is_none()
        {
            let p = &self.pose_sequences;
            let result = (|| {
                let a = self.documents[n]
                    .scene
                    .world3d
                    .sequences
                    .assets
                    .get(&p.asset.ok_or("Choose a sequence")?)
                    .ok_or("Sequence removed")?;
                if a.clip.poses.is_empty() {
                    return Err("Sequence has no poses".to_owned());
                }
                let fps = self.documents[n]
                    .animation
                    .as_ref()
                    .map_or(24., |a| a.fps.value());
                let elapsed = ((ctx.input(|i| i.time) - start).max(0.) * fps).floor() as u32;
                let cycle = a.clip.step * a.clip.poses.len() as u32;
                let frame = if p.loop_preview {
                    elapsed % cycle
                } else {
                    elapsed.min(cycle - 1)
                };
                let key = if p.curve == Curve::Fixed {
                    frame / a.clip.step * a.clip.step
                } else {
                    frame
                };
                let pose = core::at(&a.clip, frame, a.clip.step, p.curve)?;
                Ok((key, pose, !p.loop_preview && elapsed >= cycle))
            })();
            match result {
                Ok((key, pose, stop)) => {
                    if self.pose_sequences.last_frame != Some(key) {
                        self.pose_sequences.last_frame = Some(key);
                        if let Err(e) = self.stage_sequence(ctx, None, Some(pose)) {
                            self.pose_sequences.notice = e;
                            self.pose_sequences.play = None;
                        }
                    }
                    if stop {
                        self.pose_sequences.play = None;
                    }
                }
                Err(e) => {
                    self.pose_sequences.notice = e;
                    self.pose_sequences.play = None;
                }
            }
        }
    }
}
#[path = "pose_sequence_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_pose_assets_scopes_link_loop_undo_file_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
