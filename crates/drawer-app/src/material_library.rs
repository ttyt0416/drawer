use super::*;
use drawer_core::material_library::{Kind, Library};
use std::{io, sync::Arc};
#[derive(Default)]
pub(super) struct Materials {
    pub library: Arc<Library>,
    history: MaterialHistory,
    confirm_remove: Option<(u32, usize)>,
    path: String,
    name: String,
    kind: Kind,
    density: drawer_core::material_library::Density,
    fibres: drawer_core::fibres::Set,
    blocked: Option<String>,
    message: Option<String>,
}
#[derive(Clone, Default, PartialEq)]
pub(super) struct MaterialHistory {
    undo: Vec<(Arc<Library>, usize)>,
    redo: Vec<(Arc<Library>, usize)>,
}
impl Drawer {
    pub(super) fn material_dependencies(&self, document: &Document) -> Vec<String> {
        let mut missing = document.scene.paper.missing();
        for record in &document.scene.mouse_strokes {
            for (source, kind) in drawer_core::tip_texture::references(
                &record.style.sequence,
                &record.style.variation.dual.controls,
                record.style.texture,
            ) {
                let builtin = match kind {
                    Kind::Tip => 7,
                    Kind::Texture => 3,
                };
                if source <= builtin || self.materials.library.get(source, kind).is_some() {
                    continue;
                }
                let frozen = record.material_versions.iter().any(|v| {
                    v.id == source
                        && v.kind == kind
                        && drawer_core::asset_catalog::valid_hash(&v.hash)
                });
                let note = match kind {
                    Kind::Tip => drawer_core::tip_sequence::Settings::label(source),
                    Kind::Texture => drawer_core::texture::Pattern {
                        source,
                        ..Default::default()
                    }
                    .label(),
                };
                let note = if frozen {
                    format!(
                        "{note} (definition unavailable for new drawing; saved pixels and frozen settings are preserved)"
                    )
                } else {
                    note
                };
                if missing.len() < 8 && !missing.contains(&note) {
                    missing.push(note);
                }
            }
        }
        missing
    }

    pub(super) fn load_materials(&mut self, storage: &UserStorage) {
        let path = storage.root().join("materials/library.json");
        match Library::read(&path) {
            Ok(lib) => {
                self.materials.library = Arc::new(lib);
                self.materials.blocked = None;
                self.materials.history = Default::default();
                self.materials.confirm_remove = None;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.materials.library = Arc::default();
                self.materials.blocked = None;
                self.materials.history = Default::default();
                self.materials.confirm_remove = None;
            }
            Err(e) => {
                self.materials.blocked = Some(format!(
                    "Existing material library could not be read; file preserved: {e}"
                ));
            }
        }
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.starts_with("Missing tip #") || s.starts_with("Missing texture #"))
        {
            let notes = self
                .documents
                .iter()
                .flat_map(|d| self.material_dependencies(d))
                .take(8)
                .collect::<Vec<_>>();
            self.status = if notes.is_empty() {
                None
            } else {
                Some(notes.join("; "))
            };
        }
    }
    pub(super) fn publish_materials(&mut self, next: Library) -> Result<(), String> {
        self.save_materials(next, true)
    }
    fn material_library_bytes(library: &Library) -> usize {
        std::mem::size_of::<Library>()
            + 32 * std::mem::size_of::<drawer_core::material_library::Asset>()
            + library
                .assets()
                .iter()
                .map(|a| {
                    a.values.capacity()
                        + a.name.capacity()
                        + a.fibres.as_ref().map_or(0, |set| {
                            set.fibres.capacity()
                                * std::mem::size_of::<drawer_core::fibres::Fibre>()
                        })
                })
                .sum::<usize>()
    }
    pub(super) fn material_checkpoint(&self) -> MaterialHistory {
        self.materials.history.clone()
    }
    pub(super) fn rollback_materials(
        &mut self,
        next: Library,
        history: MaterialHistory,
    ) -> Result<(), String> {
        self.save_materials(next, false)?;
        self.materials.history = history;
        Ok(())
    }
    fn save_materials(&mut self, next: Library, record: bool) -> Result<(), String> {
        if let Some(e) = &self.materials.blocked {
            return Err(e.clone());
        }
        let storage = self
            .storage
            .as_ref()
            .ok_or("Local user storage is unavailable")?;
        let path = storage.root().join("materials/library.json");
        match Library::read(&path) {
            Ok(disk) if disk != *self.materials.library => {
                return Err("Library changed on disk. Reload before editing it.".into());
            }
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    && self.materials.library.assets().is_empty() => {}
            Err(e) => return Err(e.to_string()),
            _ => {}
        }
        next.save(&path).map_err(|e| e.to_string())?;
        if record && self.materials.library.as_ref() != &next {
            let before = self.materials.library.clone();
            let size = Self::material_library_bytes(&before);
            self.materials.history.undo.push((before, size));
            self.materials.history.redo.clear();
            while self.materials.history.undo.len() > 20
                || self
                    .materials
                    .history
                    .undo
                    .iter()
                    .map(|(_, n)| *n)
                    .sum::<usize>()
                    > 64 * 1024 * 1024
            {
                self.materials.history.undo.remove(0);
            }
        }
        self.materials.confirm_remove = None;
        self.materials.library = Arc::new(next);
        Ok(())
    }
    fn restore_material_history(&mut self, redo: bool) -> Result<(), String> {
        let target = if redo {
            self.materials.history.redo.last()
        } else {
            self.materials.history.undo.last()
        }
        .cloned();
        let Some((target, _)) = target else {
            return Ok(());
        };
        let current = self.materials.library.clone();
        let size = Self::material_library_bytes(&current);
        self.save_materials(target.as_ref().clone(), false)?;
        if redo {
            self.materials.history.redo.pop();
            self.materials.history.undo.push((current, size));
        } else {
            self.materials.history.undo.pop();
            self.materials.history.redo.push((current, size));
        }
        Ok(())
    }
    fn material_reference_count(&self, id: u32, kind: Kind) -> usize {
        self.documents
            .iter()
            .flat_map(|d| &d.scene.mouse_strokes)
            .filter(|m| {
                drawer_core::tip_texture::references(
                    &m.style.sequence,
                    &m.style.variation.dual.controls,
                    m.style.texture,
                )
                .contains(&(id, kind))
            })
            .fold(0usize, |n, m| n.saturating_add(m.count as usize))
    }
    fn fibre_material_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Owned fibre tip editor").show(ui,|ui|{
            ui.label("Normalized positions ±1, width 0–2, length 0–4, angle ±180°, density 0–1. Up to 128 round-capped fibres. Empty/zero width makes no ink. Registration creates a read-only material snapshot.");
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(self.materials.fibres.fibres.len()<128,egui::Button::new("Add fibre")).clicked(){self.materials.fibres.fibres.push(Default::default());}
                if ui.button("Clear fibre draft").clicked(){self.materials.fibres=Default::default();}
                if ui.button("Read own fibre JSON…").clicked() && let Some(path)=rfd::FileDialog::new().add_filter("Drawer fibre v1",&["json"]).pick_file(){
                    match drawer_core::fibres::Set::read(&path){Ok(set)=>self.materials.fibres=set,Err(e)=>self.materials.message=Some(e.to_string())}
                }
                if ui.add_enabled(self.storage.is_some() && self.materials.blocked.is_none(),egui::Button::new("Register fibre draft locally")).clicked(){
                    let r=self.materials.library.register_fibres(&self.materials.name,self.materials.fibres.clone()).map_err(|e|e.to_string()).and_then(|next|self.publish_materials(next));
                    self.materials.message=Some(match r{Ok(())=>"Fibre material registered; source and document preserved.".into(),Err(e)=>e});
                }
            });
            let mut remove=None;
            egui::ScrollArea::vertical().id_salt("fibre-rows").max_height(180.0).show(ui,|ui|{
                for (n,f) in self.materials.fibres.fibres.iter_mut().enumerate(){ui.push_id(n,|ui|{ui.horizontal_wrapped(|ui|{
                    ui.label(format!("{}",n+1));
                    let [x,y]=&mut f.position;
                    for (v,label,range) in [(x,"X",-1.0..=1.0),(y,"Y",-1.0..=1.0),(&mut f.thickness,"Width",0.0..=2.0),(&mut f.length,"Length",0.0..=4.0),(&mut f.angle,"Angle",-180.0..=180.0),(&mut f.density,"Density",0.0..=1.0)]{ui.add(egui::DragValue::new(v).prefix(format!("{label} ")).speed(0.01).range(range));}
                    if ui.button("Remove").clicked(){remove=Some(n);}
                });});}
            });
            if let Some(n)=remove{self.materials.fibres.fibres.remove(n);}
        });
    }
    pub(super) fn local_materials_ui(
        &mut self,
        ui: &mut egui::Ui,
        draft: &mut drawer_core::tool_input::Settings,
    ) {
        ui.label("PNG / JPEG / single-page TIFF, 1–512px per side. 32 entries / 2 MiB masks. Registration is saved immediately; tip/texture selection remains in this input draft.");
        super::edit::text(ui, &mut self.materials.path, "local-materials");
        if crate::help::button(ui, "Choose local material image", "local-materials").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Still image", &["png", "jpg", "jpeg", "tif", "tiff"])
                .pick_file()
        {
            if self.materials.name.is_empty() {
                self.materials.name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into();
            }
            self.materials.path = path.to_string_lossy().into();
        }
        ui.label("Material name");
        super::edit::text(ui, &mut self.materials.name, "local-materials");
        egui::ComboBox::from_id_salt("material-kind")
            .selected_text(self.materials.kind.label())
            .show_ui(ui, |ui| {
                for kind in Kind::ALL {
                    ui.selectable_value(&mut self.materials.kind, kind, kind.label());
                }
            });
        if self.materials.kind == Kind::Tip {
            use drawer_core::material_library::Density;
            egui::ComboBox::from_id_salt("material-density")
                .selected_text(match self.materials.density {
                    Density::DarknessOrAlpha => "Darkness / alpha density",
                    Density::LumaOrAlpha => "Gray value / alpha density",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.materials.density,
                        Density::DarknessOrAlpha,
                        "Darkness / alpha (legacy)",
                    );
                    ui.selectable_value(
                        &mut self.materials.density,
                        Density::LumaOrAlpha,
                        "Gray value / alpha (white = full, black = empty)",
                    );
                });
        }
        self.fibre_material_ui(ui);
        if let Some(e) = &self.materials.blocked {
            ui.colored_label(egui::Color32::YELLOW, e);
        }
        if ui
            .add_enabled(
                self.storage.is_some() && self.materials.blocked.is_none(),
                egui::Button::new("Register image locally"),
            )
            .clicked()
        {
            let result = self
                .materials
                .library
                .import_density(
                    PathBuf::from(&self.materials.path).as_path(),
                    &self.materials.name,
                    self.materials.kind,
                    self.materials.density,
                )
                .map_err(|e| e.to_string())
                .and_then(|next| self.publish_materials(next));
            self.materials.message = Some(match result {
                Ok(()) => "Material registered locally. The document is unchanged.".into(),
                Err(e) => format!("Registration rejected; previous materials preserved: {e}"),
            });
        }
        ui.horizontal(|ui| {
            for (redo, label, enabled) in [
                (
                    false,
                    "Undo mask library",
                    !self.materials.history.undo.is_empty(),
                ),
                (
                    true,
                    "Redo mask library",
                    !self.materials.history.redo.is_empty(),
                ),
            ] {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    self.materials.message = Some(match self.restore_material_history(redo) {
                        Ok(()) => "Local definitions restored; drawing history unchanged".into(),
                        Err(e) => e,
                    });
                }
            }
        });
        if crate::help::button(ui, "Reload local material library", "local-materials").clicked()
            && let Some(storage) = &self.storage
        {
            let path = storage.root().to_path_buf();
            match UserStorage::at(path) {
                Ok(storage) => self.load_materials(&storage),
                Err(e) => self.materials.message = Some(e.to_string()),
            }
        }
        if let Some(message) = &self.materials.message {
            ui.label(message);
        }
        let library = self.materials.library.clone();
        let mut remove = None;
        let n = BrushTip::ALL
            .iter()
            .position(|tip| *tip == self.selected)
            .unwrap();
        for asset in library.assets() {
            ui.push_id(asset.id, |ui| {
                ui.separator();
                ui.label(format!(
                    "{} · {} · {}×{} · #{}",
                    asset.name,
                    asset.kind.label(),
                    asset.width,
                    asset.height,
                    asset.id
                ));
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
                for y in 0..16 {
                    for x in 0..16 {
                        let q = [(x as f64 + 0.5) / 8.0 - 1.0, (y as f64 + 0.5) / 8.0 - 1.0];
                        let value = (asset.tip(q) * 255.0).round() as u8;
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                rect.min + egui::vec2(x as f32 * 3.0, y as f32 * 3.0),
                                egui::vec2(3.0, 3.0),
                            ),
                            0.0,
                            egui::Color32::from_gray(value),
                        );
                    }
                }
                if crate::help::button(
                    ui,
                    format!("Use on {}", self.selected.label()),
                    "local-materials",
                )
                .clicked()
                {
                    match asset.kind {
                        Kind::Tip => {
                            draft.sequences[n] = drawer_core::tip_sequence::Settings {
                                enabled: true,
                                count: 1,
                                tips: [asset.id, 0, 0, 0, 0, 0, 0, 0],
                                ..Default::default()
                            };
                        }
                        Kind::Texture => {
                            draft.textures[n].source = asset.id;
                        }
                    }
                }
                let references=self.material_reference_count(asset.id,asset.kind);
                if crate::help::button(ui,"Remove registration…","local-materials").clicked(){self.materials.confirm_remove=Some((asset.id,references));}
                if let Some((id,reviewed))=self.materials.confirm_remove&&id==asset.id{
                    if reviewed!=references{self.materials.confirm_remove=None;ui.label("Reference count changed; review deletion again");}
                    else{ui.label(format!("Remove owned {} #{}; {} frozen stroke reference(s) in open drawings. Source image files and already painted pixels stay unchanged. Library Undo restores the definition.",asset.kind.label(),asset.id,references));ui.horizontal(|ui|{if ui.button("Confirm removal of this definition").clicked(){remove=Some(asset.id);}
if ui.button("Cancel removal").clicked(){self.materials.confirm_remove=None;}});}
                }
            });
        }
        if let Some(id) = remove {
            let result = library
                .remove(id)
                .map_err(|e| e.to_string())
                .and_then(|next| self.publish_materials(next));
            self.materials.message=Some(match result{Ok(())=>"Registration removed. Frozen pixels/settings/versions remain; Undo mask library restores future-use definitions.".into(),Err(e)=>e});
        }
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::input_replay::Sequence;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    let library = drawer_core::material_library::verify(root).map_err(|e| e.to_string())?;
    let user = root.join("material-user");
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let before = app.documents[0].snapshot();
    app.publish_materials(library.clone())?;
    if app.documents[0].snapshot() != before || app.documents[0].history_counts() != (0, 0) {
        return Err("Registration changed document/history".into());
    }
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    if *restarted.materials.library != library {
        return Err("Local materials were not restored on restart".into());
    }
    let mut saved = drawer_core::files::open(&root.join("material-pixels.drawer"), 65536)
        .map_err(|e| e.to_string())?;
    if !restarted.material_dependencies(&saved).is_empty() {
        return Err("Registered native tip was reported missing".into());
    }
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut seq = Sequence::default();
    for sample in &mut seq.samples {
        sample.event.pressure = Some(1.0);
    }
    seq.pressure.tools[0].size = false;
    seq.pressure.tools[0].opacity = true;
    let tip = library
        .assets()
        .iter()
        .find(|a| a.kind == Kind::Tip)
        .unwrap()
        .id;
    let texture = library
        .assets()
        .iter()
        .find(|a| a.kind == Kind::Texture)
        .unwrap()
        .id;
    seq.sequences[0] = drawer_core::tip_sequence::Settings {
        enabled: true,
        count: 1,
        tips: [tip, 0, 0, 0, 0, 0, 0, 0],
        ..Default::default()
    };
    let custom = super::input_replay::run_with_materials(
        &seq,
        app.materials.library.clone(),
        &cancel,
        &progress,
    )?;
    let again = super::input_replay::run_with_materials(
        &seq,
        app.materials.library.clone(),
        &cancel,
        &progress,
    )?;
    let missing = super::input_replay::run(&seq, &cancel, &progress)?;
    if !custom.record.passed
        || !again.record.passed
        || !missing.record.passed
        || custom.pixels != again.pixels
        || custom.pixels == missing.pixels
    {
        return Err("Actual input did not use stable registered tip pixels/fallback".into());
    }
    seq.textures[0] = drawer_core::texture::Pattern {
        source: texture,
        strength: 1.0,
        ..Default::default()
    };
    let textured = super::input_replay::run_with_materials(
        &seq,
        app.materials.library.clone(),
        &cancel,
        &progress,
    )?;
    if !textured.record.passed || textured.pixels == custom.pixels {
        return Err("Actual input did not use registered texture".into());
    }
    seq.samples.pop();
    seq.expected_strokes = 0;
    let cancelled = super::input_replay::run_with_materials(
        &seq,
        app.materials.library.clone(),
        &cancel,
        &progress,
    )?;
    if !cancelled.record.passed || cancelled.pixels.iter().any(|p| p[3] != 0) {
        return Err("Cancelled image-material stroke changed original".into());
    }
    for record in &mut saved.scene.mouse_strokes {
        record.material_versions =
            drawer_core::asset_catalog::material_versions(&record.style, &library);
    }
    app.add_document(saved);
    let snapshots = app
        .documents
        .iter()
        .map(Document::snapshot)
        .collect::<Vec<_>>();
    let counts = app
        .documents
        .iter()
        .map(Document::history_counts)
        .collect::<Vec<_>>();
    if app.material_reference_count(tip, Kind::Tip) == 0 {
        return Err("Mask deletion reference count missed frozen strokes".into());
    }
    app.publish_materials(library.remove(tip).map_err(|e| e.to_string())?)?;
    let notices = app.material_dependencies(app.documents.last().unwrap());
    if notices.is_empty() || notices.iter().any(|n| !n.contains("saved pixels")) {
        return Err(
            "Missing future-use definition did not distinguish preserved frozen artwork".into(),
        );
    }
    app.restore_material_history(false)?;
    if *app.materials.library != library {
        return Err("Mask library Undo did not restore definitions".into());
    }
    app.restore_material_history(true)?;
    app.restore_material_history(false)?;
    if app
        .documents
        .iter()
        .map(Document::snapshot)
        .collect::<Vec<_>>()
        != snapshots
        || app
            .documents
            .iter()
            .map(Document::history_counts)
            .collect::<Vec<_>>()
            != counts
    {
        return Err("Mask definition deletion/history changed artwork or document Undo".into());
    }

    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_restart_native_references_production_pixels_and_cancel() {
        let root = crate::tests::TestDirectory::new();
        smoke(&root.0).unwrap();
    }
    #[test]
    fn damaged_or_externally_changed_library_is_preserved() {
        let root = crate::tests::TestDirectory::new();
        let lib = drawer_core::material_library::verify(&root.0).unwrap();
        let user = root.0.join("user");
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None).with_storage(UserStorage::at(user.clone()).unwrap());
        app.publish_materials(lib.clone()).unwrap();
        let path = user.join("materials/library.json");
        let external = lib.remove(lib.assets()[0].id).unwrap();
        external.save(&path).unwrap();
        assert!(app.publish_materials(lib.clone()).is_err());
        assert!(*app.materials.library == lib);
        assert!(Library::read(&path).unwrap() == external);
        std::fs::write(&path, b"damaged library").unwrap();
        let storage = UserStorage::at(user).unwrap();
        app.load_materials(&storage);
        assert!(app.materials.blocked.is_some());
        assert!(*app.materials.library == lib);
        assert!(app.publish_materials(lib).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"damaged library");
    }
}
