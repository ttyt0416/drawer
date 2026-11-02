use super::*;
use drawer_core::model_presets::{Library, Preset, Scope};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
pub(super) struct Panel {
    pub open: bool,
    library: Option<Library>,
    actor: Option<u64>,
    selected: Option<u64>,
    scope: Scope,
    name: String,
    notice: String,
    pub controls: BTreeMap<String, egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            library: None,
            actor: None,
            selected: None,
            scope: Scope::Pose,
            name: "My pose".into(),
            notice: String::new(),
            controls: BTreeMap::new(),
        }
    }
}
impl Drawer {
    pub(super) fn begin_model_presets(&mut self) -> Result<(), String> {
        if self.environment_pending() {
            return Err("Apply/discard the environment preview first".into());
        }
        self.environment.open = false;
        if self.line_tones_pending() {
            return Err("Apply / discard contour and tone preview first".into());
        }
        self.line_tones.open = false;
        if self.uv_pending() {
            return Err("Apply or discard the UV stroke first".into());
        }
        self.uv_paint.open = false;
        self.world_physics.open = false;
        if self.lighting_pending() {
            return Err("Apply or discard lighting preview first".into());
        }
        self.lighting.open = false;
        self.finish_stroke();
        if self.models_busy() || self.has_world_preview() || self.rig_pending() {
            return Err("Apply / discard3D preview or cancel model preparation first".into());
        }
        let n = self.active.ok_or("Choose a document")?;
        self.model_presets.open = false;
        self.model_presets.library = None;
        let root = self
            .storage
            .as_ref()
            .ok_or("Local storage unavailable")?
            .root();
        let library = Library::load(root)?;
        self.model_presets.selected = self
            .model_presets
            .selected
            .filter(|id| library.entries.iter().any(|e| e.id == *id))
            .or_else(|| library.entries.first().map(|e| e.id));
        self.model_presets.library = Some(library);
        self.model_presets.actor = self.documents[n].scene.world3d.rigs.keys().next().copied();
        self.model_presets.notice="Presets are local assets. Capture preserves document history; preview before adoption.".into();
        self.model_presets.open = true;
        self.hide_world_for_models();
        self.world_models.open = false;
        self.human_rig.open = false;
        Ok(())
    }
    fn save_model_preset(&mut self) -> Result<(), String> {
        if self.has_world_preview() || self.rig_pending() || self.models_busy() {
            return Err("Apply / discard the preview before capturing a preset".into());
        }
        let n = self.active.ok_or("Document closed")?;
        let p = &self.model_presets;
        let preset = Preset::capture(&self.documents[n], p.actor, p.scope, p.name.clone())?;
        let mut lib = p
            .library
            .clone()
            .ok_or("Reload the local preset library first")?;
        let id = lib.add(preset)?;
        lib.save(
            self.storage
                .as_ref()
                .ok_or("Local storage unavailable")?
                .root(),
        )?;
        self.model_presets.library = Some(lib);
        self.model_presets.selected = Some(id);
        self.model_presets.notice = "Saved local preset; drawing / undo history unchanged".into();
        Ok(())
    }
    fn preview_model_preset(&mut self) -> Result<(), String> {
        if self.has_world_preview() || self.rig_pending() || self.models_busy() {
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
        let id = self.model_presets.selected.ok_or("Select a saved preset")?;
        let preset = &self
            .model_presets
            .library
            .as_ref()
            .ok_or("Reload presets first")?
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or("Preset removed")?
            .preset;
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let p = preset.prepare(
            &self.documents[n],
            self.model_presets.actor,
            budget,
            &AtomicBool::new(false),
        )?;
        self.adopt_model_preview(Box::new(p))?;
        self.model_presets.open = false;
        Ok(())
    }
    fn delete_model_preset(&mut self) -> Result<(), String> {
        let mut lib = self
            .model_presets
            .library
            .clone()
            .ok_or("Reload presets first")?;
        lib.remove(self.model_presets.selected.ok_or("Select a saved preset")?)?;
        lib.save(
            self.storage
                .as_ref()
                .ok_or("Local storage unavailable")?
                .root(),
        )?;
        self.model_presets.selected = lib.entries.first().map(|e| e.id);
        self.model_presets.library = Some(lib);
        self.model_presets.notice = "Deleted local preset; drawing unchanged".into();
        Ok(())
    }
    pub(super) fn model_presets_window(&mut self, ctx: &egui::Context) {
        if !self.model_presets.open {
            return;
        }
        let Some(n) = self.active else {
            self.model_presets.open = false;
            return;
        };
        let d = &self.documents[n];
        let mut p = std::mem::take(&mut self.model_presets);
        let mut action = 0;
        let mut open = true;
        p.controls.clear();
        p.actor = p.actor.filter(|id| {
            d.scene
                .catalog
                .worlds
                .index(*id, d.scene.worlds.len())
                .is_some()
        });
        egui::Window::new("Local3D presets / pose, shape and camera").default_pos(egui::pos2(20.,20.)).default_width(650.).max_height(600.).open(&mut open).show(ctx,|ui|{
 ui.add_sized([620.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);ui.horizontal(|ui|{for(key,label,a)in[("save","Capture / save preset",1),("preview","Preview selected preset",2),("delete","Delete selected preset",3)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
 ui.horizontal(|ui|{for scope in [Scope::Pose,Scope::Shape,Scope::Camera]{ui.selectable_value(&mut p.scope,scope,format!("{scope:?}"));}ui.label("New preset name");ui.text_edit_singleline(&mut p.name);});
 egui::ComboBox::from_id_salt("preset-model").selected_text(p.actor.map_or("Select mannequin (camera needs none)".into(),|id|format!("Model {id}"))).show_ui(ui,|ui|{for i in 0..d.scene.worlds.len(){let id=d.scene.catalog.worlds.id(i);ui.selectable_value(&mut p.actor,Some(id),format!("Model {id}{}",if d.scene.world3d.rigs.contains_key(&id){""}else{" (no rig)"}));}});
 ui.label("Pose changes joint angles / hand side; Shape changes proportions. Other values and object placement remain. Different skeletons are rejected. Camera retains the saved page frame. Apply / Discard in the 3D preview window.");
 egui::ScrollArea::vertical().max_height(320.).show(ui,|ui|{if let Some(lib)=&p.library{if lib.entries.is_empty(){ui.label("No saved presets. Capture the current committed mannequin or camera.");}for e in &lib.entries{let r=ui.selectable_label(p.selected==Some(e.id),format!("{}: {} / {:?}",e.id,e.preset.name,e.preset.scope()));p.controls.insert(format!("entry-{}",e.id),r.rect);if r.clicked(){p.selected=Some(e.id);}}}});
 });
        p.open = open;
        self.model_presets = p;
        let result = match action {
            1 => self.save_model_preset(),
            2 => self.preview_model_preset(),
            3 => self.delete_model_preset(),
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.model_presets.notice = e;
        }
    }
}
#[path = "model_presets_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_preset_capture_preview_cancel_apply_restart_and_mismatch() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
