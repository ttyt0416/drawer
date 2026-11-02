use super::*;
use drawer_core::model_library::{self as lib, AlignTarget, Anchor, Edit, Placement, Shape};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
struct Job {
    document: u64,
    revision: u64,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Box<drawer_core::world3d::Prepared>, String>>,
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
pub(super) struct Panel {
    pub open: bool,
    shape: Shape,
    detail: u16,
    placement: Placement,
    fit: bool,
    size: f64,
    path: String,
    ids: Vec<u64>,
    stamp: Option<u64>,
    offset: [f64; 3],
    axis: usize,
    anchor: Anchor,
    target: AlignTarget,
    job: Option<Job>,
    notice: String,
    pub controls: BTreeMap<&'static str, egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            shape: Shape::Box,
            detail: 12,
            placement: Default::default(),
            fit: true,
            size: 2.5,
            path: String::new(),
            ids: vec![],
            stamp: None,
            offset: [2., 0., 0.],
            axis: 0,
            anchor: Anchor::Center,
            target: AlignTarget::First,
            job: None,
            notice: String::new(),
            controls: BTreeMap::new(),
        }
    }
}
impl Drawer {
    pub(super) fn models_busy(&self) -> bool {
        self.world_models.job.is_some()
    }
    pub(super) fn begin_model_library(&mut self) -> Result<(), String> {
        if self.cameras_pending() {
            return Err("Apply/discard scene camera preview first".into());
        }
        self.scene_cameras.open = false;
        if self.sequence_pending() {
            return Err("Apply/discard pose sequence preview first".into());
        }
        self.pose_sequences.open = false;
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
        self.model_presets.open = false;
        if self.rig_pending() {
            return Err("Apply or discard the mannequin preview first".into());
        }
        self.human_rig.open = false;
        self.finish_stroke();
        if self.has_world_preview() {
            return Err("Apply or discard the current3D preview first".into());
        }
        self.hide_world_for_models();
        let n = self.active.ok_or("Choose a document")?;
        if self
            .world_models
            .job
            .as_ref()
            .is_some_and(|j| j.document != self.documents[n].id)
        {
            return Err("Cancel the current model preparation first".into());
        }
        self.world_models.open = true;
        Ok(())
    }
    fn stage_model(&mut self, ctx: &egui::Context, edit: Edit) -> Result<(), String> {
        if self.models_busy() {
            return Err("Wait for or cancel model preparation".into());
        }
        self.finish_stroke();
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending content edits before placing models".into());
        }
        if self.has_world_preview() {
            return Err("Apply or discard the current 3D preview first".into());
        }
        let n = self.active.ok_or("Choose a document")?;
        self.stop_timeline_playback(self.documents[n].id);
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let d = &self.documents[n];
        if d.memory_bytes().saturating_mul(3) > budget {
            return Err("Model preparation exceeds available memory".into());
        }
        let document = d.id;
        let revision = d.revision;
        let mut frozen = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        frozen.id = document;
        frozen.revision = revision;
        frozen.keyframe_content = d.keyframe_content;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = lib::prepare(&frozen, edit, budget, &flag).map(Box::new);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.hide_world_for_models();
        self.world_models.job = Some(Job {
            document,
            revision,
            cancel,
            rx,
            worker: Some(worker),
        });
        self.world_models.notice = "Preparing local model snapshot…".into();
        Ok(())
    }
    fn poll_models(&mut self, ctx: &egui::Context) {
        if self.world_models.job.as_ref().is_some_and(|j| {
            !self
                .documents
                .iter()
                .any(|d| d.id == j.document && d.revision == j.revision)
        }) {
            self.world_models.job = None;
            self.world_models.notice =
                "Document changed or closed; model preparation discarded".into();
        }
        let ready = self
            .world_models
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok());
        if let Some(result) = ready {
            let canceled = self
                .world_models
                .job
                .as_ref()
                .is_some_and(|j| j.cancel.load(Ordering::Relaxed));
            self.world_models.job = None;
            if canceled {
                self.world_models.notice =
                    "Cancelled; original document and files preserved".into();
            } else {
                match result {
                    Ok(p) => match self.adopt_model_preview(p) {
                        Ok(()) => self.world_models.open = false,
                        Err(e) => self.world_models.notice = e,
                    },
                    Err(e) => self.world_models.notice = e,
                }
            }
        }
        if self.models_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }
    pub(super) fn model_library_window(&mut self, ctx: &egui::Context) {
        self.poll_models(ctx);
        if !self.world_models.open {
            return;
        }
        let Some(n) = self.active else {
            return;
        };
        let d = &self.documents[n];
        let mut p = std::mem::take(&mut self.world_models);
        if p.stamp != Some(d.id) {
            p.ids.clear();
            p.stamp = Some(d.id);
        }
        p.ids
            .retain(|id| drawer_core::world3d::node(&d.scene, *id).is_some());
        let busy = p.job.is_some();
        let mut action = 0;
        let mut edit = None;
        let mut open = true;
        p.controls.clear();
        egui::Window::new("Local models / primitives, props and background").default_pos(egui::pos2(20.,20.)).default_width(740.).max_height(640.).vscroll(false).open(&mut open).show(ctx,|ui|{
            ui.add_sized([700.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);
            if let Some(job)=&p.job{ui.spinner();let r=ui.button("Cancel model preparation");p.controls.insert("cancel",r.rect);if r.clicked(){job.cancel.store(true,Ordering::Relaxed);}}
            ui.add_enabled_ui(!busy,|ui|{ui.horizontal_wrapped(|ui|{for(key,label,value)in[("primitive","Preview primitive",1),("import","Preview local model",3),("duplicate","Duplicate models",4),("align","Align models",5)]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){action=value;}}});});
            egui::ScrollArea::vertical().id_salt("model-library-settings").max_height(540.).show(ui,|ui|{ui.add_enabled_ui(!busy,|ui|{
                ui.label("Original built-in geometry; local OBJ/glTF2/GLB models. Preview first, then apply in the 3D window. Materials and missing references are listed before adoption.");
                ui.horizontal_wrapped(|ui|{for shape in Shape::ALL{ui.selectable_value(&mut p.shape,shape,shape.label());}});
                ui.horizontal(|ui|{ui.label("Round detail");ui.add(egui::DragValue::new(&mut p.detail).range(3..=64))});
                ui.horizontal(|ui|{ui.label("Model file");ui.text_edit_singleline(&mut p.path);if ui.button("Browse local file…").clicked(){action=2;}});
                ui.horizontal(|ui|{ui.label("Center X / Y / Z");for v in &mut p.placement.position{ui.add(egui::DragValue::new(v).speed(0.1));}ui.checkbox(&mut p.placement.hidden,"Place hidden");});
                ui.horizontal(|ui|{ui.label("Rotation X / Y / Z");for v in &mut p.placement.angles{ui.add(egui::DragValue::new(v));}ui.checkbox(&mut p.fit,"Fit longest side");ui.add(egui::DragValue::new(&mut p.size).range(0.001..=10000.));});
                ui.horizontal(|ui|{ui.label("Scale X / Y / Z (negative mirrors)");for v in &mut p.placement.scale{ui.add(egui::DragValue::new(v).speed(0.01));}});
                ui.separator();ui.label("Existing model trees: selecting a parent includes its children once.");
                egui::ScrollArea::vertical().id_salt("library-models").max_height(90.).show(ui,|ui|{ui.horizontal(|ui|{if ui.button("Select visible models").clicked(){p.ids=(0..d.scene.worlds.len()).map(|i|d.scene.catalog.worlds.id(i)).filter(|id|!drawer_core::layer_catalog::properties(d,drawer_core::layer_search::Key::World(*id)).hidden).collect();}
                    if ui.button("Clear models").clicked(){p.ids.clear();}});for i in 0..d.scene.worlds.len(){let id=d.scene.catalog.worlds.id(i);let info=d.scene.catalog.world_info.get(&id);let name=info.and_then(|i|i.name.as_deref()).unwrap_or("Model");let hidden=info.is_some_and(|i|i.properties.hidden);let mut selected=p.ids.contains(&id);if ui.checkbox(&mut selected,format!("{id}: {name}{}",if hidden{" (hidden)"}else{""})).changed(){if selected{p.ids.push(id);}else{p.ids.retain(|n|*n!=id);}}}});
                ui.horizontal(|ui|{ui.label("Duplicate offset X/Y/Z");for v in &mut p.offset{ui.add(egui::DragValue::new(v).speed(0.1));}});
                ui.horizontal_wrapped(|ui|{for i in 0..3{ui.selectable_value(&mut p.axis,i,["X","Y","Z"][i]);}for(a,label)in [(Anchor::Min,"Minimum"),(Anchor::Center,"Center"),(Anchor::Max,"Maximum")]{ui.selectable_value(&mut p.anchor,a,label);}ui.selectable_value(&mut p.target,AlignTarget::First,"First selected tree");ui.selectable_value(&mut p.target,AlignTarget::Selection,"Selection bounds");});
                if let Some(id)=p.ids.first()&&let Some(s)=d.scene.world3d.surfaces.get(id){ui.small(format!("{}: {} triangles, {} UV coordinates, {} normals",s.source,s.triangles.len(),s.texcoords.len(),s.normals.len()));if let Some(a)=&s.imported{ui.small(format!("{} imported material slots; {} retained skin joints (frozen default pose)",a.materials.len(),a.skin.as_ref().map_or(0,|r|r.names.len())));if let Some(r)=&a.skin{for (i,n) in r.names.iter().enumerate(){ui.small(format!("Joint {i}: {n}; parent {:?}",r.parents[i]));}}}for reference in &s.references{ui.small(format!("{} — {} at import",reference.name,if reference.present{"available"}else{"missing"}));}}
            });});
        });
        p.open = open;
        self.world_models = p;
        if !open {
            self.world_models.job = None;
            return;
        }
        let result = (|| {
            let mut placement = self.world_models.placement;
            placement.fit = self.world_models.fit.then_some(self.world_models.size);
            match action {
                1 => {
                    edit = Some(Edit::Primitive {
                        shape: self.world_models.shape,
                        detail: self.world_models.detail,
                        placement,
                    })
                }
                2 => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Local OBJ / glTF2 / GLB", &["obj", "gltf", "glb"])
                        .pick_file()
                    {
                        self.world_models.path = path.display().to_string();
                    }
                }
                3 => {
                    edit = Some(Edit::Import {
                        path: self.world_models.path.clone().into(),
                        placement,
                    })
                }
                4 => {
                    edit = Some(Edit::Duplicate {
                        ids: self.world_models.ids.clone(),
                        offset: self.world_models.offset,
                    })
                }
                5 => {
                    edit = Some(Edit::Align {
                        ids: self.world_models.ids.clone(),
                        axis: self.world_models.axis,
                        anchor: self.world_models.anchor,
                        target: self.world_models.target,
                    })
                }
                _ => {}
            }
            if let Some(edit) = edit {
                self.stage_model(ctx, edit)?;
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.world_models.notice = e;
        }
    }
}
#[path = "model_import_verify.rs"]
mod import_verify;
#[path = "world_models_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)?;
    import_verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_primitive_obj_review_duplicate_align_cancel_stale_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}

#[cfg(test)]
mod import_tests {
    #[test]
    fn pointer_gltf_review_apply_undo_cancel_stale_close_and_source_preservation() {
        let t = crate::tests::TestDirectory::new();
        super::import_verify::run(&t.0).unwrap();
    }
}
