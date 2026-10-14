use super::*;
use drawer_core::{
    references::{LocalImage, REFERENCE_BUDGET, ReferenceSession},
    view::View,
};
use std::path::Path;

#[derive(Default)]
pub(super) struct ReferencePane {
    pub session: ReferenceSession,
    image_texture: Option<(u64, egui::TextureHandle)>,
    checker: Option<egui::TextureHandle>,
    pick_mode: bool,
    pub error: Option<String>,
    window_epoch: u64,
    #[cfg(test)]
    pub viewport: Option<egui::Rect>,
    #[cfg(test)]
    pub ring: Option<(egui::Pos2, [u8; 4])>,
    #[cfg(test)]
    pub open_document_button: Option<egui::Rect>,
    #[cfg(test)]
    pub remove_button: Option<egui::Rect>,
}
enum Action {
    File,
    Clipboard,
    Remove,
    OpenDocument,
    Undo,
    Redo,
    Restore,
    Reset,
    Export,
    Import,
}
pub(super) fn clipboard_image() -> Result<LocalImage, String> {
    let data = arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_image())
        .map_err(|e| format!("No readable image on the clipboard: {e}"))?;
    LocalImage::from_rgba(data.width, data.height, &data.bytes, REFERENCE_BUDGET)
        .map_err(|e| e.to_string())
}
impl Drawer {
    pub(super) fn paste_reference_from(
        &mut self,
        read: impl FnOnce() -> Result<LocalImage, String>,
    ) -> Result<(), String> {
        self.add_reference_image(read()?, "Clipboard image".into(), None)
    }
    pub(super) fn add_reference_image(
        &mut self,
        image: LocalImage,
        name: String,
        source: Option<PathBuf>,
    ) -> Result<(), String> {
        let storage = self.storage.as_ref().ok_or("User storage is unavailable")?;
        self.references
            .session
            .add(storage, image, name, source)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub(super) fn add_reference_path(&mut self, path: &Path) -> Result<(), String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let budget = REFERENCE_BUDGET.saturating_sub(self.references.session.memory_bytes());
        let image = LocalImage::read(&path, budget).map_err(|e| e.to_string())?;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.add_reference_image(image, name, Some(path))
    }
    pub(super) fn open_reference_document(&mut self) -> Result<(), String> {
        let image = self
            .references
            .session
            .active_image()
            .ok_or("The reference is missing")?;
        let used = self.documents.iter().map(Document::resident_bytes).sum();
        let document = image
            .document(DEFAULT_DOCUMENT_BUDGET.saturating_sub(used))
            .map_err(|e| e.to_string())?;
        self.documents.try_reserve(1).map_err(|e| e.to_string())?;
        self.tabs.try_reserve(1).map_err(|e| e.to_string())?;
        self.add_document(document);
        Ok(())
    }
    pub(super) fn show_references(&mut self, ctx: &egui::Context) {
        if !self.references.session.workspace.visible {
            if !ctx.input(|i| i.pointer.any_down())
                && let Some(storage) = &self.storage
                && let Err(error) = self.references.session.persist(storage)
            {
                self.references.error = Some(error.to_string());
            }
            return;
        }
        let enabled = self.pending_close.is_none() && self.sizing_dialog.is_none();
        let pane = &mut self.references;
        let mut open = true;
        let mut action = None;
        let mut inner_size = None;
        let workspace = &pane.session.workspace;
        let response = egui::Window::new("Reference images")
            .id(egui::Id::new(("reference-window", pane.window_epoch)))
            .default_pos(egui::pos2(workspace.position[0], workspace.position[1]))
            .default_size(egui::vec2(workspace.size[0], workspace.size[1]))
            .min_size(egui::vec2(320.0, 280.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (label, event) in [
                            ("Add PNG…", Action::File),
                            ("Add clipboard image", Action::Clipboard),
                            ("Save set…", Action::Export),
                            ("Load set…", Action::Import),
                        ] {
                            if ui.button(label).clicked() {
                                action = Some(event);
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (label, event) in [
                            ("Undo settings", Action::Undo),
                            ("Redo settings", Action::Redo),
                            ("Restore previous", Action::Restore),
                            ("Reset references", Action::Reset),
                        ] {
                            if ui.button(label).clicked() {
                                action = Some(event);
                            }
                        }
                    });
                    if pane.session.workspace.entries.is_empty() {
                        ui.label("No reference images. Add a local PNG or clipboard image.");
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Previous").clicked() {
                                pane.session.workspace.step(-1);
                            }
                            egui::ComboBox::from_id_salt("reference-list")
                                .selected_text(
                                    pane.session
                                        .workspace
                                        .active_record()
                                        .map_or("Choose a reference", |r| r.name.as_str()),
                                )
                                .show_ui(ui, |ui| {
                                    for record in &pane.session.workspace.entries {
                                        ui.selectable_value(
                                            &mut pane.session.workspace.active,
                                            Some(record.id),
                                            &record.name,
                                        );
                                    }
                                });
                            if ui.button("Next").clicked() {
                                pane.session.workspace.step(1);
                            }
                            let remove = ui.button("Remove reference");
                            if remove.clicked() {
                                action = Some(Action::Remove);
                            }
                            let open_document = ui.add_enabled(
                                pane.session.active_image().is_some(),
                                egui::Button::new("Open as new drawing"),
                            );
                            if open_document.clicked() {
                                action = Some(Action::OpenDocument);
                            }
                            #[cfg(test)]
                            {
                                pane.remove_button = Some(remove.rect);
                                pane.open_document_button = Some(open_document.rect);
                            }
                        });
                        pane.image_ui(ui, enabled);
                    }
                    if let Some(error) = &pane.error {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                });
                inner_size = Some(ui.min_size());
            });
        if let Some(response) = response {
            let rect = response.response.rect;
            if action.is_none() {
                pane.session.workspace.position = [rect.min.x, rect.min.y];
                let size = inner_size.unwrap_or(rect.size());
                pane.session.workspace.size =
                    [size.x.clamp(240.0, 4000.0), size.y.clamp(240.0, 4000.0)];
            }
        }
        pane.session.workspace.visible = open;
        let attempted = action.is_some();
        let result: Result<(), String> = (|| {
            match action {
                Some(Action::File) => {
                    if let Some(paths) = rfd::FileDialog::new()
                        .add_filter("PNG", &["png"])
                        .pick_files()
                    {
                        for path in paths {
                            self.add_reference_path(&path)?;
                        }
                    }
                }
                Some(Action::Clipboard) => self.paste_reference_from(clipboard_image)?,
                Some(Action::OpenDocument) => self.open_reference_document()?,
                Some(Action::Export) => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Reference set", &["drawrefs"])
                        .set_file_name("references.drawrefs")
                        .save_file()
                    {
                        self.references
                            .session
                            .export(&path)
                            .map_err(|e| e.to_string())?;
                    }
                }
                Some(Action::Import) => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Reference set", &["drawrefs"])
                        .pick_file()
                    {
                        let storage = self.storage.as_ref().ok_or("User storage is unavailable")?;
                        self.references
                            .session
                            .import(storage, &path)
                            .map_err(|e| e.to_string())?;
                        self.references.image_texture = None;
                        self.references.window_epoch += 1;
                    }
                }
                Some(event) => {
                    let storage = self.storage.as_ref().ok_or("User storage is unavailable")?;
                    let session = &mut self.references.session;
                    match event {
                        Action::Remove => session.remove_active(storage),
                        Action::Undo => session.undo(storage),
                        Action::Redo => session.redo(storage),
                        Action::Restore => session.restore_previous(storage),
                        Action::Reset => session.reset(storage),
                        _ => unreachable!(),
                    }
                    .map_err(|e| e.to_string())?;
                    self.references.image_texture = None;
                    self.references.window_epoch += 1;
                }
                None => {}
            }
            if !ctx.input(|input| input.pointer.any_down())
                && let Some(storage) = &self.storage
            {
                self.references
                    .session
                    .persist(storage)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.references.error = Some(error);
        } else if attempted {
            self.references.error = None;
        }
    }
}
impl ReferencePane {
    fn image_ui(&mut self, ui: &mut egui::Ui, enabled: bool) {
        let Some(id) = self.session.workspace.active else {
            return;
        };
        let Some(image) = self.session.images.get(&id).cloned() else {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                self.session
                    .missing
                    .get(&id)
                    .map_or("Reference unavailable", String::as_str),
            );
            if let Some(source) = &self.session.workspace.active_record().unwrap().source {
                ui.label(source.display().to_string());
            }
            ui.label("Restore the source file, load a saved set, or remove and add this reference again.");
            return;
        };
        ui.horizontal_wrapped(|ui| {
            let view = &mut self.session.workspace.active_record_mut().unwrap().view;
            let mut percent = view.zoom * 100.0;
            if ui
                .add(
                    egui::DragValue::new(&mut percent)
                        .range(1.0..=6400.0)
                        .suffix("%"),
                )
                .changed()
                && percent.is_finite()
            {
                view.zoom = (percent / 100.0).clamp(0.01, 64.0);
            }
            let mut angle = view.angle;
            if ui
                .add(egui::DragValue::new(&mut angle).suffix("°"))
                .changed()
            {
                view.rotate(angle);
            }
            if ui.button("Reset view").clicked() {
                *view = View::default();
            }
            ui.checkbox(&mut self.pick_mode, "Pick color");
            ui.checkbox(&mut self.session.workspace.auto_pick, "Auto pick");
            ui.checkbox(&mut self.session.workspace.show_ring, "Color ring");
        });
        ui.label(format!("Selected RGBA: {:?}", self.session.workspace.color));
        if let Some(path) = &self.session.workspace.active_record().unwrap().source {
            ui.label(path.display().to_string());
        }
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(
                ui.available_width().max(1.0),
                ui.available_height().max(120.0),
            ),
            egui::Sense::click_and_drag(),
        );
        #[cfg(test)]
        {
            self.viewport = Some(rect);
            self.ring = None;
        }
        let size = [image.width as f64, image.height as f64];
        let center = [rect.center().x as f64, rect.center().y as f64];
        let ppp = ui.ctx().pixels_per_point() as f64;
        let view = &mut self.session.workspace.active_record_mut().unwrap().view;
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary)
                && ui.input(|i| i.key_down(egui::Key::Space)))
        {
            let delta = response.drag_delta();
            view.pan_by([delta.x as f64, delta.y as f64]);
        }
        if response.hovered() {
            let wheel = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta).y);
            if wheel != 0.0
                && let Some(point) = response.hover_pos()
            {
                view.zoom_about(
                    (wheel as f64 * 0.002).exp(),
                    [point.x as f64, point.y as f64],
                    size,
                    center,
                    ppp,
                );
            }
        }
        let view = *view;
        let pointer = response.hover_pos().or_else(|| {
            ui.input(|i| {
                i.raw.events.iter().rev().find_map(|e| match e {
                    egui::Event::Touch { pos, .. } if rect.contains(*pos) => Some(*pos),
                    _ => None,
                })
            })
        });
        let sampled = pointer.and_then(|point| {
            image
                .sample(view.screen_to_document(
                    [point.x as f64, point.y as f64],
                    size,
                    center,
                    ppp,
                ))
                .map(|color| (point, color))
        });
        if enabled
            && let Some((_, color)) = sampled
            && (self.session.workspace.auto_pick || (self.pick_mode && response.clicked()))
        {
            self.session.workspace.color = color;
        }
        if self.image_texture.as_ref().map(|(key, _)| *key) != Some(id) {
            let reduction = image.width.max(image.height).div_ceil(2048).max(1);
            let width = image.width.div_ceil(reduction);
            let height = image.height.div_ceil(reduction);
            let mut pixels = Vec::with_capacity((width * height) as usize);
            for y in 0..height {
                for x in 0..width {
                    let p = image.pixels[(y * reduction * image.width + x * reduction) as usize];
                    pixels.push(egui::Color32::from_rgba_unmultiplied(
                        p[0], p[1], p[2], p[3],
                    ));
                }
            }
            self.image_texture = Some((
                id,
                ui.ctx().load_texture(
                    "reference-image",
                    egui::ColorImage::new([width as usize, height as usize], pixels),
                    egui::TextureOptions::NEAREST,
                ),
            ));
        }
        if self.checker.is_none() {
            self.checker = Some(ui.ctx().load_texture(
                "reference-checker",
                egui::ColorImage::new(
                    [2, 2],
                    vec![
                        egui::Color32::from_gray(196),
                        egui::Color32::from_gray(224),
                        egui::Color32::from_gray(224),
                        egui::Color32::from_gray(196),
                    ],
                ),
                egui::TextureOptions {
                    wrap_mode: egui::TextureWrapMode::Repeat,
                    ..egui::TextureOptions::NEAREST
                },
            ));
        }
        let corners = [[0.0, 0.0], [size[0], 0.0], size, [0.0, size[1]]].map(|point| {
            let p = view.document_to_screen(point, size, center, ppp);
            egui::pos2(p[0] as f32, p[1] as f32)
        });
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(32));
        for (texture, uv_max) in [
            (
                self.checker.as_ref().unwrap().id(),
                egui::pos2(image.width as f32 / 32.0, image.height as f32 / 32.0),
            ),
            (
                self.image_texture.as_ref().unwrap().1.id(),
                egui::pos2(1.0, 1.0),
            ),
        ] {
            let mut mesh = egui::Mesh::with_texture(texture);
            for (pos, uv) in corners.into_iter().zip([
                egui::Pos2::ZERO,
                egui::pos2(uv_max.x, 0.0),
                uv_max,
                egui::pos2(0.0, uv_max.y),
            ]) {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos,
                    uv,
                    color: egui::Color32::WHITE,
                });
            }
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            painter.add(egui::Shape::mesh(mesh));
        }
        if enabled
            && self.session.workspace.show_ring
            && let Some((point, color)) = sampled
        {
            let center = point + egui::vec2(28.0, -28.0);
            painter.circle_filled(center, 18.0, egui::Color32::from_gray(196));
            painter.circle_filled(
                center,
                16.0,
                egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
            );
            painter.circle_stroke(center, 18.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
            #[cfg(test)]
            {
                self.ring = Some((point, color));
            }
        }
    }
}
