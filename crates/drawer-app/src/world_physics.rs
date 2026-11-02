use super::*;
use drawer_core::world_physics::{self as core, Edit};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
pub(super) struct Panel {
    pub open: bool,
    ids: Vec<u64>,
    floor: f64,
    distance: f64,
    counts: [u16; 3],
    spacing: [f64; 3],
    gravity: f64,
    seconds: f64,
    timestep: f64,
    notice: String,
    pub controls: BTreeMap<String, egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            ids: vec![],
            floor: 0.,
            distance: 1000.,
            counts: [2, 1, 1],
            spacing: [2., 0., 0.],
            gravity: 9.81,
            seconds: 2.,
            timestep: 1. / 120.,
            notice: String::new(),
            controls: BTreeMap::new(),
        }
    }
}
impl Drawer {
    pub(super) fn begin_world_physics(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.models_busy()
            || self.has_world_preview()
            || self.rig_pending()
            || self.lighting_pending()
        {
            return Err("Apply / discard current3D preview first".into());
        }
        if self.active.is_none() {
            return Err("Choose a document".into());
        }
        self.hide_world_for_models();
        self.world_models.open = false;
        self.human_rig.open = false;
        self.model_presets.open = false;
        self.lighting.open = false;
        self.world_physics.open = true;
        Ok(())
    }
    fn stage_world_physics(&mut self, edit: Edit) -> Result<(), String> {
        if self.models_busy()
            || self.has_world_preview()
            || self.rig_pending()
            || self.lighting_pending()
        {
            return Err("Finish the current3D preview first".into());
        }
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending content edits first".into());
        }
        let n = self.active.ok_or("Document closed")?;
        self.stop_timeline_playback(self.documents[n].id);
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let p = core::prepare(&self.documents[n], edit, budget, &AtomicBool::new(false))?;
        self.adopt_model_preview(Box::new(p))?;
        self.world_physics.open = false;
        Ok(())
    }
    pub(super) fn world_physics_window(&mut self, ctx: &egui::Context) {
        if !self.world_physics.open {
            return;
        }
        let Some(n) = self.active else {
            self.world_physics.open = false;
            return;
        };
        let d = &self.documents[n];
        let mut p = std::mem::take(&mut self.world_physics);
        p.ids.retain(|id| {
            d.scene
                .catalog
                .worlds
                .index(*id, d.scene.worlds.len())
                .is_some()
        });
        p.controls.clear();
        let mut action = 0;
        let mut open = true;
        egui::Window::new("3D snap / array and explicit gravity settle").default_pos(egui::pos2(15.,15.)).default_width(710.).max_height(640.).open(&mut open).show(ctx,|ui|{
 ui.add_sized([680.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);ui.horizontal_wrapped(|ui|{for(key,label,a)in[("floor","Preview floor snap",1),("surface","Preview surface snap",2),("array","Preview array",3),("settle","Preview gravity settle",4),("collider","Mark colliders",5),("unmark","Unmark colliders",6)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
 ui.label("All commands preview first; Apply / Discard in the 3D window. Review warnings before Apply. Simulation runs only on request, with no live motion. Colliders use world axis-aligned bounding boxes; surface snap uses actual polygons below a bottom-center anchor.");
 egui::ScrollArea::vertical().id_salt("physics-models").max_height(140.).show(ui,|ui|{for i in 0..d.scene.worlds.len(){let id=d.scene.catalog.worlds.id(i);let mut selected=p.ids.contains(&id);let flags=drawer_core::layer_catalog::properties(d,drawer_core::layer_search::Key::World(id));let r=ui.checkbox(&mut selected,format!("{id}: {}{}{}",d.scene.catalog.world_info.get(&id).and_then(|i|i.name.as_deref()).unwrap_or("Model"),if d.scene.world3d.colliders.contains(&id){" [collider]"}else{""},if flags.hidden||flags.locked||flags.position_locked||flags.pixels_locked{" [hidden / locked]"}else{""}));p.controls.insert(format!("select-{id}"),r.rect);if r.changed(){if selected{p.ids.push(id);}else{p.ids.retain(|n|*n!=id);}}}});
 ui.horizontal(|ui|{ui.label("World Y floor");ui.add(egui::DragValue::new(&mut p.floor).speed(0.1));ui.label("Surface search distance");ui.add(egui::DragValue::new(&mut p.distance).range(0.0001..=1e6));});
 ui.horizontal(|ui|{ui.label("Grid counts X/Y/Z (includes original)");for n in &mut p.counts{ui.add(egui::DragValue::new(n).range(1..=16));}});ui.horizontal(|ui|{ui.label("World spacing X/Y/Z");for n in &mut p.spacing{ui.add(egui::DragValue::new(n).speed(0.1));}});
 ui.horizontal(|ui|{ui.label("Gravity");ui.add(egui::DragValue::new(&mut p.gravity).range(0. ..=100.));ui.label("Duration seconds");ui.add(egui::DragValue::new(&mut p.seconds).range(0. ..=10.));ui.label("Requested timestep");ui.add(egui::DragValue::new(&mut p.timestep).range(1e-6..=1e6).speed(0.001));});ui.small("Locked colliders remain stationary. Selected parent / children move once. Settle discards velocities; only accepted poses and collider roles are stored.");
 });
        p.open = open;
        let ids = p.ids.clone();
        let edit = match action {
            1 => Some(Edit::Floor {
                ids,
                height: p.floor,
            }),
            2 => Some(Edit::Surface {
                ids,
                distance: p.distance,
            }),
            3 => Some(Edit::Array {
                ids,
                counts: p.counts,
                spacing: p.spacing,
            }),
            4 => Some(Edit::Settle {
                ids,
                floor: p.floor,
                gravity: p.gravity,
                seconds: p.seconds,
                timestep: p.timestep,
            }),
            5 | 6 => Some(Edit::Collider {
                ids,
                enabled: action == 5,
            }),
            _ => None,
        };
        self.world_physics = p;
        if open
            && let Some(edit) = edit
            && let Err(e) = self.stage_world_physics(edit)
        {
            self.world_physics.notice = e;
        }
    }
}
#[path = "world_physics_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_snap_surface_array_colliders_settle_review_cancel_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
