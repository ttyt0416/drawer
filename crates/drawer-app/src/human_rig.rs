use super::*;
use drawer_core::{
    human_rig::{self as core, Edit, Kind, Rig},
    world3d,
};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    actor: Option<u64>,
    draft: Option<Rig>,
    staged: Option<Box<world3d::Prepared>>,
    stamp: Option<(u64, u64)>,
    joint: usize,
    notice: String,
    pub controls: BTreeMap<String, egui::Rect>,
}
impl Panel {
    fn load(&mut self, d: &Document, id: Option<u64>) {
        self.actor = id;
        self.draft = id.and_then(|id| d.scene.world3d.rigs.get(&id).cloned());
        self.staged = None;
        self.joint = 0;
        self.stamp = Some((d.id, d.revision));
        self.notice = if id.is_some() && self.draft.is_none() {
            "This model has no rig; joint pose editing is unavailable".into()
        } else {
            String::new()
        };
    }
    fn prepare(&mut self, d: &Document, budget: usize) -> Result<(), String> {
        self.staged = None;
        let rig = self.draft.clone().ok_or("Select a rigged mannequin")?;
        let e = if let Some(id) = self.actor {
            Edit::Replace { id, rig }
        } else {
            Edit::Create {
                rig,
                position: [0.; 3],
            }
        };
        self.staged = Some(Box::new(core::prepare(
            d,
            e,
            budget,
            &AtomicBool::new(false),
        )?));
        self.notice = "Preview only; Apply makes one undo step".into();
        Ok(())
    }
    fn preview(&self, ui: &mut egui::Ui, d: &Document) {
        let (r, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(650.), 185.),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(r);
        painter.rect_filled(r, 3., egui::Color32::from_rgb(25, 31, 32));
        let s = self.staged.as_ref().map_or(&d.scene, |p| p.scene());
        let Some(c) = s.camera.as_ref() else {
            return;
        };
        let scale = (r.width() as f64 / (c.page_bounds.max[0] - c.page_bounds.min[0]))
            .min(r.height() as f64 / (c.page_bounds.max[1] - c.page_bounds.min[1]));
        let screen = |p: [f64; 2]| {
            egui::pos2(
                r.center().x
                    + ((p[0] - (c.page_bounds.max[0] + c.page_bounds.min[0]) / 2.) * scale) as f32,
                r.center().y
                    + ((p[1] - (c.page_bounds.max[1] + c.page_bounds.min[1]) / 2.) * scale) as f32,
            )
        };
        for i in 0..s.worlds.len() {
            let id = s.catalog.worlds.id(i);
            if s.catalog
                .world_info
                .get(&id)
                .is_some_and(|p| p.properties.hidden)
            {
                continue;
            }
            for line in world3d::segments(s, i) {
                painter.line_segment(
                    line.map(screen),
                    egui::Stroke::new(0.8, egui::Color32::LIGHT_BLUE),
                );
            }
        }
        let id = self.actor.or_else(|| {
            self.staged
                .as_ref()
                .and_then(|p| p.affected.first().copied())
        });
        if let Some(id) = id
            && let Some(rig) = s.world3d.rigs.get(&id)
            && let Some(lens) = s.world3d.lens
            && let Some(node) = world3d::node(s, id)
        {
            let frames = rig.frames();
            for (i, j) in rig.joints().iter().enumerate() {
                let p = node.frame.map(frames[i].origin());
                let color = if i == self.joint {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::WHITE
                };
                if let Some(parent) = j.parent
                    && let Some(line) = lens.segment(c, p, node.frame.map(frames[parent].origin()))
                {
                    painter.line_segment(line.map(screen), egui::Stroke::new(1.3, color));
                }
                if let Some(p) = lens.project(c, p) {
                    painter.circle_filled(
                        screen(p),
                        if i == self.joint { 3.5 } else { 1.5 },
                        color,
                    );
                }
            }
        }
    }
}
impl Drawer {
    pub(super) fn rig_pending(&self) -> bool {
        self.human_rig.staged.is_some()
    }
    pub(super) fn begin_human_rig(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.models_busy() || self.has_world_preview() {
            return Err("Apply / discard 3D preview or cancel model preparation first".into());
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
        self.hide_world_for_models();
        self.world_models.open = false;
        if !self.rig_pending() {
            let id = self.documents[n].scene.world3d.rigs.keys().next().copied();
            self.human_rig.load(&self.documents[n], id);
        }
        self.human_rig.open = true;
        Ok(())
    }
    fn apply_human_rig(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Document closed")?;
        let p = self
            .human_rig
            .staged
            .take()
            .ok_or("Prepare a mannequin preview first")?;
        let id = p.affected.first().copied();
        p.commit(&mut self.documents[n])?;
        self.human_rig.load(&self.documents[n], id);
        self.human_rig.notice = "Applied mannequin; one Undo restores the prior state".into();
        Ok(())
    }
    pub(super) fn human_rig_window(&mut self, ctx: &egui::Context) {
        let Some(n) = self.active else {
            self.human_rig.staged = None;
            self.human_rig.draft = None;
            self.human_rig.stamp = None;
            self.human_rig.open = false;
            return;
        };
        if !self.human_rig.open {
            self.human_rig.staged = None;
            return;
        }
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let d = &self.documents[n];
        let mut p = std::mem::take(&mut self.human_rig);
        if p.stamp != Some((d.id, d.revision)) {
            let id = p.actor.filter(|id| world3d::node(&d.scene, *id).is_some());
            p.load(d, id);
            p.notice = "Document changed; unconfirmed mannequin edits discarded".into();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            let id = p.actor;
            p.load(d, id);
        }
        let mut open = true;
        let mut changed = false;
        let mut action = 0;
        p.controls.clear();
        egui::Window::new("Mannequins / body, head and hand joints").default_pos(egui::pos2(15.,15.)).default_width(680.).max_height(650.).vscroll(false).open(&mut open).show(ctx,|ui|{
   ui.add_sized([650.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);
   ui.horizontal_wrapped(|ui|{for(key,label,kind)in[("body","New body",Kind::Body),("head","New head",Kind::Head),("hand","New hand",Kind::Hand)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){p.actor=None;p.draft=Some(Rig::new(kind));p.joint=0;changed=true;}}
    for(key,label,a)in[("apply","Apply",1),("discard","Discard",2),("mirror","Mirror pose",3),("reset","Reset pose",4),("detach","Detach rig",5)]{let enabled=if a==1{p.staged.is_some()}else if a==5{p.actor.is_some()&&p.draft.is_some()}else{p.draft.is_some()};let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}
   });
   egui::ComboBox::from_id_salt("mannequin-actor").selected_text(p.actor.map_or("New mannequin".into(),|id|format!("Model {id}"))).show_ui(ui,|ui|{for i in 0..d.scene.worlds.len(){let id=d.scene.catalog.worlds.id(i);let label=format!("{id}: {}{}",d.scene.catalog.world_info.get(&id).and_then(|i|i.name.as_deref()).unwrap_or("Model"),if d.scene.world3d.rigs.contains_key(&id){""}else{" (no rig)"});let r=ui.selectable_label(p.actor==Some(id),label);p.controls.insert(format!("actor-{id}"),r.rect);if r.clicked(){p.load(d,Some(id));changed=false;action=0;}}});
   if let Some(rig)=&mut p.draft {
    let joints=rig.joints();p.joint=p.joint.min(joints.len()-1);
    egui::ComboBox::from_id_salt("mannequin-joint").selected_text(&joints[p.joint].name).show_ui(ui,|ui|{for(i,j)in joints.iter().enumerate(){ui.selectable_value(&mut p.joint,i,&j.name);}});
    ui.horizontal(|ui|{for axis in 0..3{let j=&joints[p.joint];let r=ui.add(egui::Slider::new(&mut rig.angles[p.joint][axis],j.min[axis]..=j.max[axis]).text(["X","Y","Z"][axis]));p.controls.insert(format!("angle-{axis}"),r.rect);changed|=r.changed();}});
   }else if p.actor.is_some(){ui.label("This model has no rig; joint pose editing is unavailable.");}
   p.preview(ui,d);
   egui::ScrollArea::vertical().id_salt("mannequin-shape").max_height(200.).show(ui,|ui|{if let Some(r)=&mut p.draft {
    ui.label("Original rigid skin mannequins. Joint rotations are local; limits are drawing controls.");
    let s=&mut r.shape;egui::Grid::new("mannequin-shape-grid").num_columns(2).show(ui,|ui|{for(name,v,range)in[("Height",&mut s.height,0.5..=3.),("Shoulders",&mut s.shoulders,0.5..=2.),("Hips",&mut s.hips,0.5..=2.),("Bulk",&mut s.bulk,0.5..=2.),("Head",&mut s.head,0.5..=2.),("Hands",&mut s.hands,0.5..=2.),("Face width",&mut s.face,0.5..=2.),("Jaw width",&mut s.jaw,0.5..=2.),("Palm width",&mut s.palm,0.5..=2.),("Finger length",&mut s.fingers,0.5..=2.)]{ui.label(name);let response=ui.add(egui::Slider::new(v,range));p.controls.insert(format!("shape-{name}"),response.rect);changed|=response.changed();ui.end_row();}});
   }});
  });
        p.open = open;
        if !open {
            p.staged = None;
        } else {
            match action {
                2 => {
                    let id = p.actor;
                    p.load(d, id);
                    changed = false;
                }
                3 => {
                    if let Some(r) = &mut p.draft {
                        r.mirror();
                        changed = true;
                    }
                }
                4 => {
                    if let Some(r) = &mut p.draft {
                        r.reset_pose();
                        changed = true;
                    }
                }
                5 => {
                    p.staged = None;
                    match core::prepare(
                        d,
                        Edit::Detach {
                            id: p.actor.unwrap(),
                        },
                        budget,
                        &AtomicBool::new(false),
                    ) {
                        Ok(s) => {
                            p.staged = Some(Box::new(s));
                            p.notice =
                                "Preview: detach rig, retain current mesh. Apply to confirm".into();
                        }
                        Err(e) => p.notice = e,
                    }
                    changed = false;
                }
                _ => {}
            }
            if changed && let Err(e) = p.prepare(d, budget) {
                p.notice = e;
            }
        }
        self.human_rig = p;
        if open
            && action == 1
            && let Err(e) = self.apply_human_rig()
        {
            self.human_rig.notice = e;
        }
    }
}
#[path = "human_rig_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_mannequin_pose_shape_apply_cancel_stale_and_rigless() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
