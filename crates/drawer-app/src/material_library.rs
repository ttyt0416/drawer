use super::*;
use drawer_core::material_library::{Kind, Library};
use std::{io, sync::Arc};
#[derive(Default)]
pub(super) struct Materials {
    pub library: Arc<Library>,
    path: String,
    name: String,
    kind: Kind,
    blocked: Option<String>,
    message: Option<String>,
}
impl Drawer {
    pub(super) fn material_dependencies(&self, document: &Document) -> Vec<String> {
        let mut missing = document.scene.paper.missing();
        for record in &document.scene.mouse_strokes {
            for source in record
                .style
                .sequence
                .missing()
                .into_iter()
                .filter(|id| self.materials.library.get(*id, Kind::Tip).is_none())
            {
                let note = drawer_core::tip_sequence::Settings::label(source);
                if missing.len() < 8 && !missing.contains(&note) {
                    missing.push(note);
                }
            }
            if record.style.texture.missing()
                && self
                    .materials
                    .library
                    .get(record.style.texture.source, Kind::Texture)
                    .is_none()
            {
                let note = record.style.texture.label();
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
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.materials.library = Arc::default();
                self.materials.blocked = None;
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
        self.materials.library = Arc::new(next);
        Ok(())
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
                .import(
                    PathBuf::from(&self.materials.path).as_path(),
                    &self.materials.name,
                    self.materials.kind,
                )
                .map_err(|e| e.to_string())
                .and_then(|next| self.publish_materials(next));
            self.materials.message = Some(match result {
                Ok(()) => "Material registered locally. The document is unchanged.".into(),
                Err(e) => format!("Registration rejected; previous materials preserved: {e}"),
            });
        }
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
                if crate::help::button(ui, "Remove registration", "local-materials").clicked() {
                    remove = Some(asset.id);
                }
            });
        }
        if let Some(id) = remove {
            let result = library
                .remove(id)
                .map_err(|e| e.to_string())
                .and_then(|next| self.publish_materials(next));
            self.materials.message=Some(match result{Ok(())=>"Registration removed. Stored references keep their IDs and use their missing-material fallback.".into(),Err(e)=>e});
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
    let saved = drawer_core::files::open(&root.join("material-pixels.drawer"), 65536)
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
