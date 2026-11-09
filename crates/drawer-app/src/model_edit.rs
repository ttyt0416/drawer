use super::*;
use drawer_core::{
    model_edit::{self as core, Control, Edit, Keep, Side},
    world3d::V,
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
#[derive(Clone, PartialEq)]
struct Draft {
    actor: Option<u64>,
    joint: usize,
    angles: V,
    control: Option<Control>,
    expressions: [f64; 3],
    base: bool,
    side: Side,
    key: usize,
    weight: f64,
    name: String,
    scale: V,
    source: Option<u64>,
    parent: Option<u64>,
    bone: Option<usize>,
    keep: Keep,
    form: drawer_core::mannequin_fit::Form,
    target: drawer_core::human_rig::Shape,
    blend: f64,
    region: drawer_core::mannequin_fit::Region,
    floor: f64,
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            actor: None,
            joint: 0,
            angles: [0.; 3],
            control: None,
            expressions: [0.; 3],
            base: false,
            side: Side::LeftToRight,
            key: 0,
            weight: 0.,
            name: "Shape target".into(),
            scale: [1., 1.5, 1.],
            source: None,
            parent: None,
            bone: None,
            keep: Keep::World,
            form: Default::default(),
            target: Default::default(),
            blend: 0.5,
            region: drawer_core::mannequin_fit::Region::Body,
            floor: 0.,
        }
    }
}
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    draft: Draft,
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
        let p = &mut self.draft;
        p.actor = p
            .actor
            .filter(|id| {
                d.scene
                    .catalog
                    .worlds
                    .index(*id, d.scene.worlds.len())
                    .is_some()
            })
            .or_else(|| {
                (0..d.scene.worlds.len())
                    .next()
                    .map(|i| d.scene.catalog.worlds.id(i))
            });
        p.source = p
            .source
            .filter(|id| {
                Some(*id) != p.actor
                    && d.scene
                        .catalog
                        .worlds
                        .index(*id, d.scene.worlds.len())
                        .is_some()
            })
            .or_else(|| {
                (0..d.scene.worlds.len())
                    .map(|i| d.scene.catalog.worlds.id(i))
                    .find(|id| Some(*id) != p.actor)
            });
        if let Some(id) = p.actor {
            if let Some(r) = d.scene.world3d.rigs.get(&id) {
                p.joint = p.joint.min(r.angles.len() - 1);
                p.angles = r.angles[p.joint];
                p.control = Some(core::controls(r)[p.joint].clone());
                p.expressions = r.expressions;
                p.form = r.form.clone();
                p.target = r.shape;
                p.region = match r.kind {
                    drawer_core::human_rig::Kind::Body => drawer_core::mannequin_fit::Region::Body,
                    drawer_core::human_rig::Kind::Head => drawer_core::mannequin_fit::Region::Head,
                    drawer_core::human_rig::Kind::Hand => drawer_core::mannequin_fit::Region::Hand,
                };
            } else {
                p.control = None;
            }
            if let Some(m) = d.scene.world3d.editing.morphs.get(&id) {
                p.key = p.key.min(m.keys.len() - 1);
                p.weight = m.keys[p.key].weight;
            } else {
                p.key = 0;
                p.weight = 0.;
            }
            p.parent = drawer_core::world3d::node(&d.scene, id).and_then(|n| n.parent);
            p.bone = d
                .scene
                .world3d
                .editing
                .attachments
                .get(&id)
                .map(|a| a.joint);
        }
        self.stamp = Some((d.id, d.revision, d.current_frame));
    }
}
impl Drawer {
    pub(super) fn model_edit_pending(&self) -> bool {
        self.model_controls.job.is_some() || self.model_controls.staged.is_some()
    }
    pub(super) fn begin_model_controls(&mut self) -> Result<(), String> {
        if self.has_world_preview()
            || self.models_busy()
            || self.rig_pending()
            || self.lighting_pending()
            || self.uv_pending()
            || self.environment_pending()
            || self.line_tones_pending()
            || self.sequence_pending()
            || self.cameras_pending()
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
        if !self.model_edit_pending() {
            self.model_controls.load(&self.documents[n]);
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
        self.scene_cameras.open = false;
        self.world_views.open = false;
        self.model_controls.open = true;
        Ok(())
    }
    fn stage_model_controls(&mut self, ctx: &egui::Context, edit: Edit) -> Result<(), String> {
        if self.model_controls.job.is_some() {
            return Err("Wait for/cancel model preparation first".into());
        }
        if self.has_world_preview()
            || self.models_busy()
            || self.rig_pending()
            || self.lighting_pending()
            || self.uv_pending()
            || self.environment_pending()
            || self.line_tones_pending()
            || self.sequence_pending()
            || self.cameras_pending()
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
            return Err("Model snapshot exceeds memory budget".into());
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
        self.model_controls.clear();
        self.model_controls.job = Some(Job {
            stamp,
            cancel,
            rx,
            worker: Some(worker),
        });
        Ok(())
    }
    pub(super) fn model_controls_window(&mut self, ctx: &egui::Context) {
        let Some(n) = self.active else {
            self.model_controls.clear();
            self.model_controls.open = false;
            return;
        };
        let d = &self.documents[n];
        let stamp = (d.id, d.revision, d.current_frame);
        if self.model_controls.stamp != Some(stamp) {
            self.model_controls.clear();
            self.model_controls.load(d);
        }
        if !self.model_controls.open {
            self.model_controls.clear();
            return;
        }
        if self
            .model_controls
            .job
            .as_ref()
            .is_some_and(|j| j.stamp != stamp)
        {
            self.model_controls.clear();
        }
        if let Some(result) = self
            .model_controls
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok())
        {
            let canceled = self
                .model_controls
                .job
                .as_ref()
                .is_some_and(|j| j.cancel.load(Ordering::Relaxed));
            self.model_controls.job = None;
            if !canceled {
                match result {
                    Ok((p, image)) => {
                        self.model_controls.notice = p.summary.clone();
                        self.model_controls.staged = Some(p);
                        self.model_controls.texture = Some(ctx.load_texture(
                            "model-control-preview",
                            egui::ColorImage::from_rgba_unmultiplied([128, 128], &image),
                            egui::TextureOptions::NEAREST,
                        ));
                    }
                    Err(e) => self.model_controls.notice = e,
                }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            self.model_controls.clear();
            self.model_controls.load(d);
        }
        let mut p = std::mem::take(&mut self.model_controls);
        let before = p.draft.clone();
        let selection = (p.draft.actor, p.draft.joint, p.draft.key);
        let mut action = 0;
        let busy = p.job.is_some();
        let mut open = true;
        p.controls.clear();
        egui::Window::new("Model controls / shapes, joints and attachments").default_pos(egui::pos2(12.,12.)).default_width(810.).default_height(620.).min_height(540.).max_height(650.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.add_sized([770.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);
            ui.horizontal_wrapped(|ui|{for(key,label,a,enabled)in[("apply","Apply prepared change",1,p.staged.is_some()),("cancel","Cancel / discard",2,true),("undo","Undo document",3,!busy)]{let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
            ui.add_enabled_ui(!busy,|ui|{let v=&mut p.draft;
                ui.horizontal_wrapped(|ui|{ui.label("Model");for i in 0..d.scene.worlds.len(){let id=d.scene.catalog.worlds.id(i);let r=ui.selectable_label(v.actor==Some(id),format!("{id}{}",if d.scene.world3d.rigs.contains_key(&id){" (rig)"}else{""}));p.controls.insert(format!("actor-{id}"),r.rect);if r.clicked(){v.actor=Some(id);v.joint=0;v.key=0;}}});
                if let Some(id)=v.actor{
                    if d.scene.world3d.rigs.contains_key(&id){
                        let r=ui.checkbox(&mut v.base,"Explicit base-scene pose/controls (sequence snapshots stay unchanged)");p.controls.insert("base".into(),r.rect);
                        let joints=d.scene.world3d.rigs[&id].joints();let _=r;ui.horizontal_wrapped(|ui|{egui::ComboBox::from_label("Joint").selected_text(&joints[v.joint.min(joints.len()-1)].name).show_ui(ui,|ui|{for(i,j)in joints.iter().enumerate(){ui.selectable_value(&mut v.joint,i,&j.name);}});for(i,x)in v.angles.iter_mut().enumerate(){let enabled=v.control.as_ref().is_none_or(|c|!c.locked[i]);ui.add_enabled(enabled,egui::DragValue::new(x).speed(1.));}let r=ui.button("Prepare joint pose");p.controls.insert("joint".into(),r.rect);if r.clicked(){action=4;}});
                        if let Some(c)=&mut v.control{ui.horizontal_wrapped(|ui|{for i in 0..3{let r=ui.checkbox(&mut c.locked[i],format!("Lock {}",["X","Y","Z"][i]));p.controls.insert(format!("lock-{i}"),r.rect);if r.changed(){c.hold=d.scene.world3d.rigs[&id].angles[v.joint];}ui.add(egui::DragValue::new(&mut c.min[i]).speed(1.));ui.add(egui::DragValue::new(&mut c.max[i]).speed(1.));}let r=ui.button("Prepare locks / ranges");p.controls.insert("limits".into(),r.rect);if r.clicked(){action=5;}});}
                        ui.horizontal_wrapped(|ui|{for(side,label)in[(Side::LeftToRight,"Left → right"),(Side::RightToLeft,"Right → left"),(Side::Swap,"Swap sides")]{ui.selectable_value(&mut v.side,side,label);}let r=ui.button("Prepare verified pose copy");p.controls.insert("copy".into(),r.rect);if r.clicked(){action=6;}});
                        if d.scene.world3d.rigs[&id].kind!=drawer_core::human_rig::Kind::Hand{ui.horizontal_wrapped(|ui|{for(i,label)in["Blink","Surprise","Cheek puff"].into_iter().enumerate(){ui.label(label);ui.add(egui::DragValue::new(&mut v.expressions[i]).range(0. ..=1.).speed(0.01));}let r=ui.button("Prepare expression morphs");p.controls.insert("expressions".into(),r.rect);if r.clicked(){action=7;}});}
                        let advanced=egui::CollapsingHeader::new("Mannequin form / perspective / pose assistance").id_salt("mannequin-fit").show(ui,|ui|{
                            use drawer_core::mannequin_fit::Region;
                            ui.small("Local part axes; X/Y/Z0.5..2. Scale changes the selected part and the child joint offsets. Manga perspective freezes a local viewer direction; moving the camera does not deform the model.");
                            ui.horizontal_wrapped(|ui|{ui.label("Selected part X/Y/Z");if v.form.parts.is_empty(){v.form.parts=vec![[1.;3];joints.len()];}for x in &mut v.form.parts[v.joint]{ui.add(egui::DragValue::new(x).range(0.5..=2.).speed(0.01));}ui.label("Perspective0..2");ui.add(egui::DragValue::new(&mut v.form.strength).range(0. ..=2.).speed(0.01));for(dir,label)in[([1.,0.,0.],"+X"),([-1.,0.,0.],"−X"),([0.,1.,0.],"+Y"),([0.,-1.,0.],"−Y"),([0.,0.,1.],"+Z"),([0.,0.,-1.],"−Z")]{ui.selectable_value(&mut v.form.toward,dir,label);}});
                            ui.horizontal_wrapped(|ui|{for(key,label,a)in[("fit-form","Prepare part / perspective",14),("fit-reset","Reset all proportions / perspective",15)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
                            ui.horizontal_wrapped(|ui|{ui.label("Blend region");for(region,label)in[(Region::Body,"Body + head + hands"),(Region::Head,"Head only"),(Region::Hand,"Hands only")]{ui.selectable_value(&mut v.region,region,label);}ui.add(egui::Slider::new(&mut v.blend,0. ..=1.).text("Blend"));});
                            ui.horizontal_wrapped(|ui|{ui.label("Target height");ui.add(egui::DragValue::new(&mut v.target.height).range(0.5..=3.).speed(0.01));for(label,x)in[("Shoulders",&mut v.target.shoulders),("Hips",&mut v.target.hips),("Bulk",&mut v.target.bulk),("Head",&mut v.target.head),("Face",&mut v.target.face),("Jaw",&mut v.target.jaw),("Hands",&mut v.target.hands),("Palm",&mut v.target.palm),("Fingers",&mut v.target.fingers)]{ui.label(label);ui.add(egui::DragValue::new(x).range(0.5..=2.).speed(0.01));}});
                            let r=ui.button("Prepare regional shape blend");p.controls.insert("fit-blend".into(),r.rect);if r.clicked(){action=16;}
                            ui.horizontal_wrapped(|ui|{ui.label("World floor Y");ui.add(egui::DragValue::new(&mut v.floor).speed(0.01));for(key,label,a)in[("fit-collision","Separate limb capsules",17),("fit-ground","Ground feet",18),("fit-upright","Stand upright",19),("fit-stand-ground","Stand + ground",20)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
                            ui.small("Assistances are explicit, each Apply is one Undo. Capsule separation excludes adjacent links and fingers; an unresolved constrained pose is rejected without changing it. Ground uses actual foot vertices; upright leaves locked joint axes fixed. Imported meshes need their original editor.");
                        });p.controls.insert("fit-expand".into(),advanced.header_response.rect);
                    }else{
                        ui.horizontal_wrapped(|ui|{ui.label("Target name / local target scale X/Y/Z");ui.text_edit_singleline(&mut v.name);for x in &mut v.scale{ui.add(egui::DragValue::new(x).range(0.1..=4.).speed(0.01));}let r=ui.button("Register shape target");p.controls.insert("add-key".into(),r.rect);if r.clicked(){action=8;}});
                        ui.horizontal_wrapped(|ui|{egui::ComboBox::from_label("Matching topology source").selected_text(v.source.map_or("None".into(),|id|format!("Model{id}"))).show_ui(ui,|ui|{for i in 0..d.scene.worlds.len(){let source=d.scene.catalog.worlds.id(i);if source!=id{ui.selectable_value(&mut v.source,Some(source),format!("Model{source}"));}}});let r=ui.button("Capture frozen target");p.controls.insert("capture-key".into(),r.rect);if r.clicked(){action=9;}});
                        if let Some(m)=d.scene.world3d.editing.morphs.get(&id){ui.horizontal_wrapped(|ui|{for(i,k)in m.keys.iter().enumerate(){ui.selectable_value(&mut v.key,i,&k.name);}ui.add(egui::Slider::new(&mut v.weight,0. ..=1.).text("Weight"));for(key,label,a)in[("weight","Prepare key weight",10),("delete-key","Delete selected key",11)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});}
                    }
                    ui.separator();ui.horizontal_wrapped(|ui|{egui::ComboBox::from_label("Parent model").selected_text(v.parent.map_or("No parent".into(),|id|format!("Model{id}"))).show_ui(ui,|ui|{if ui.selectable_value(&mut v.parent,None,"No parent").changed(){v.bone=None;}for i in 0..d.scene.worlds.len(){let parent=d.scene.catalog.worlds.id(i);if parent!=id&&ui.selectable_value(&mut v.parent,Some(parent),format!("Model{parent}")).changed(){v.bone=None;}}});if let Some(r)=v.parent.and_then(|id|d.scene.world3d.rigs.get(&id)){let joints=r.joints();egui::ComboBox::from_label("Parent bone / part").selected_text(v.bone.and_then(|i|joints.get(i)).map_or("Model axes",|b|b.name.as_str())).show_ui(ui,|ui|{ui.selectable_value(&mut v.bone,None,"Model axes");for(i,j)in joints.iter().enumerate(){ui.selectable_value(&mut v.bone,Some(i),&j.name);}});}for(keep,label)in[(Keep::World,"Keep current world pose"),(Keep::Local,"Keep current local pose")]{ui.selectable_value(&mut v.keep,keep,label);}});
                    ui.horizontal_wrapped(|ui|{for(key,label,a)in[("parent","Prepare parent / attachment",12),("detach","Detach parent (keep world)",13)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
                }
            });
            if let Some(texture)=&p.texture{ui.image((texture.id(),egui::vec2(140.,140.)));}ui.small("Each applied model edit is one Undo. Bone links follow base edits and explicit pose-sequence placement. Shape targets are independent local vertex snapshots. Locks/ranges reject incompatible pose clips; they never silently clamp keys. Side copying needs the complete original skeleton mapping.");
        });
        p.open = open;
        if !open {
            p.clear();
        }
        if before != p.draft {
            p.clear();
        }
        if selection != (p.draft.actor, p.draft.joint, p.draft.key) {
            p.load(d);
        }
        self.model_controls = p;
        let result = (|| {
            if !open {
                return Ok(());
            }
            let p = &self.model_controls.draft;
            let model = || p.actor.ok_or("Choose a model");
            let edit = match action {
                14..=20 => {
                    use drawer_core::mannequin_fit::{Change, Form};
                    let change = match action {
                        14 => Change::Form(p.form.clone()),
                        15 => Change::Form(Form::default()),
                        16 => Change::Blend {
                            target: p.target,
                            weight: p.blend,
                            region: p.region,
                        },
                        _ => Change::Assist {
                            collision: action == 17,
                            upright: action == 19 || action == 20,
                            ground: if action == 18 || action == 20 {
                                Some(p.floor)
                            } else {
                                None
                            },
                        },
                    };
                    Some(Edit::Fit {
                        rig: model()?,
                        change,
                        base: p.base,
                    })
                }
                4 => Some(Edit::Joint {
                    rig: model()?,
                    joint: p.joint,
                    angles: p.angles,
                    base: p.base,
                }),
                5 => Some(Edit::Control {
                    rig: model()?,
                    joint: p.joint,
                    value: p.control.clone().ok_or("Choose a rig joint")?,
                    base: p.base,
                }),
                6 => Some(Edit::Copy {
                    rig: model()?,
                    side: p.side,
                    base: p.base,
                }),
                7 => Some(Edit::Expressions {
                    rig: model()?,
                    weights: p.expressions,
                    base: p.base,
                }),
                8 => Some(Edit::AddKey {
                    model: model()?,
                    name: p.name.clone(),
                    scale: p.scale,
                }),
                9 => Some(Edit::CaptureKey {
                    model: model()?,
                    source: p.source.ok_or("Choose a shape source")?,
                    name: p.name.clone(),
                }),
                10 => Some(Edit::Weight {
                    model: model()?,
                    key: p.key,
                    value: p.weight,
                }),
                11 => Some(Edit::DeleteKey {
                    model: model()?,
                    key: p.key,
                }),
                12 => Some(Edit::Parent {
                    child: model()?,
                    parent: p.parent,
                    joint: p.bone,
                    keep: p.keep,
                }),
                13 => Some(Edit::Parent {
                    child: model()?,
                    parent: None,
                    joint: None,
                    keep: Keep::World,
                }),
                _ => None,
            };
            if let Some(edit) = edit {
                self.stage_model_controls(ctx, edit)?;
                return Ok(());
            }
            match action {
                1 => {
                    let p = self
                        .model_controls
                        .staged
                        .take()
                        .ok_or("Prepare a model change first")?;
                    p.commit(&mut self.documents[n])?;
                    self.model_controls.clear();
                    self.model_controls.load(&self.documents[n]);
                }
                2 => {
                    self.model_controls.clear();
                    self.model_controls.load(&self.documents[n]);
                }
                3 => {
                    self.model_controls.clear();
                    self.documents[n].undo();
                    self.model_controls.load(&self.documents[n]);
                }
                _ => {}
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.model_controls.notice = e;
        }
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(20));
        }
    }
}
#[path = "model_edit_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)?;
    fit_verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_morph_joint_constraints_bone_parent_apply_undo_cancel_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}

#[path = "mannequin_fit_verify.rs"]
mod fit_verify;
#[cfg(test)]
mod fit_tests {
    #[test]
    fn pointer_original_form_blend_preview_apply_undo_cancel_stale_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::fit_verify::run(&t.0).unwrap();
    }
}
