use super::*;
use drawer_core::{
    image_import::{ImageSource, ImportedPage},
    references::LocalImage,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

enum ResultValue {
    Source(ImageSource),
    Page(ImportedPage),
}
struct Job {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<ResultValue, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
pub(super) struct ImportDialog {
    destination: Option<(u64, u64, u32)>,
    context: egui::Context,
    source: Option<Arc<ImageSource>>,
    pub page: usize,
    pub orient: bool,
    preview: Option<ImportedPage>,
    texture: Option<egui::TextureHandle>,
    job: Option<Job>,
    error: Option<String>,
    #[cfg(test)]
    pub import_button: Option<egui::Rect>,
    #[cfg(test)]
    pub cancel_button: Option<egui::Rect>,
    #[cfg(test)]
    pub orientation_button: Option<egui::Rect>,
}
fn job(
    ctx: &egui::Context,
    work: impl FnOnce(&AtomicBool) -> Result<ResultValue, String> + Send + 'static,
) -> Job {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (sender, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = work(&worker_cancel);
        let _ = sender.send(result);
        ctx.request_repaint();
    });
    Job { cancel, receiver }
}
impl ImportDialog {
    pub fn working(&self) -> bool {
        self.job.is_some()
    }
    pub(super) fn start_page(&mut self, ctx: &egui::Context, budget: usize) {
        let Some(source) = self.source.clone() else {
            return;
        };
        self.preview = None;
        self.texture = None;
        self.error = None;
        let page = self.page;
        let orient = self.orient;
        self.job = Some(job(ctx, move |cancel| {
            source
                .decode(page, orient, budget, cancel)
                .map(ResultValue::Page)
                .map_err(|e| e.to_string())
        }));
    }
    #[cfg(test)]
    pub fn ready(&self) -> bool {
        self.preview.is_some() && self.job.is_none()
    }
    #[cfg(test)]
    pub fn preview_image(&self) -> &LocalImage {
        &self.preview.as_ref().unwrap().image
    }
    #[cfg(test)]
    pub fn failed(&self) -> bool {
        self.error.is_some() && self.job.is_none()
    }
}
impl Drawer {
    fn import_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum())
    }
    pub(super) fn begin_import(&mut self, ctx: &egui::Context, path: &Path) {
        let budget = self.import_budget();
        let path = path.to_owned();
        self.import_dialog = Some(ImportDialog {
            destination: None,
            context: ctx.clone(),
            source: None,
            page: 0,
            orient: true,
            preview: None,
            texture: None,
            error: None,
            job: Some(job(ctx, move |cancel| {
                ImageSource::read(&path, budget, cancel)
                    .map(ResultValue::Source)
                    .map_err(|e| e.to_string())
            })),
            #[cfg(test)]
            import_button: None,
            #[cfg(test)]
            cancel_button: None,
            #[cfg(test)]
            orientation_button: None,
        });
    }
    pub(super) fn begin_import_layer(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
    ) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let destination = (d.id, d.revision, d.current_frame);
        self.begin_import(ctx, path);
        self.import_dialog.as_mut().unwrap().destination = Some(destination);
        Ok(())
    }
    pub(super) fn choose_image_layer(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.active.is_none() {
            return Err("Open a page first".into());
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Local image", &["png", "jpg", "jpeg", "tif", "tiff"])
            .pick_file()
        {
            self.begin_import_layer(ctx, &path)?;
        }
        Ok(())
    }
    pub(super) fn document_from_clipboard(
        &mut self,
        read: impl FnOnce() -> Result<LocalImage, String>,
    ) -> Result<(), String> {
        let image = read()?;
        self.add_image_document(image, "Clipboard image".into())
    }
    pub(super) fn add_image_document(
        &mut self,
        image: LocalImage,
        title: String,
    ) -> Result<(), String> {
        let mut document = image
            .document(self.import_budget())
            .map_err(|e| e.to_string())?;
        document.title = title;
        document.view = self.default_view;
        self.documents.try_reserve(1).map_err(|e| e.to_string())?;
        self.tabs.try_reserve(1).map_err(|e| e.to_string())?;
        self.add_document(document);
        Ok(())
    }
    pub(super) fn accept_import(&mut self) -> Result<(), String> {
        let Some(dialog) = self.import_dialog.as_mut() else {
            return Err("No import preview".into());
        };
        if dialog.job.is_some() {
            return Err("Wait for the image preview".into());
        }
        if let Some((id, revision, frame)) = dialog.destination {
            let d = &self.documents[self.active.ok_or("Source page was closed")?];
            if (d.id, d.revision, d.current_frame) != (id, revision, frame) {
                return Err("Page/frame changed; reopen image layer import".into());
            }
        }
        let destination = dialog.destination;
        let context = dialog.context.clone();
        let preview = dialog.preview.take().ok_or("No decoded image to import")?;
        let source = dialog.source.clone().ok_or("No input image")?;
        let title = format!(
            "{}{}",
            source
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            if source.pages.len() > 1 {
                format!(" — page {}", dialog.page + 1)
            } else {
                String::new()
            }
        );
        if destination.is_some() {
            self.begin_paste_reserved(
                &context,
                || {
                    Ok(drawer_core::paste::ClipboardImage {
                        color: None,
                        image: preview.image,
                        origin: None,
                    })
                },
                source.encoded_snapshot().len(),
            )?;
            if let Some(p) = self.paste_dialog.as_mut() {
                p.set_layer_name(title.chars().take(64).collect());
            }
        } else {
            self.add_image_document(preview.image, title)?;
        }

        self.protected_sources.insert(source.path.clone());
        self.remember_file(&source.path);
        self.import_dialog = None;
        Ok(())
    }
    pub(super) fn show_import(&mut self, ctx: &egui::Context) {
        let budget = self.import_budget();
        let Some(dialog) = &mut self.import_dialog else {
            return;
        };
        if let Some(job) = &dialog.job {
            match job.receiver.try_recv() {
                Ok(result) => {
                    dialog.job = None;
                    match result {
                        Ok(ResultValue::Source(source)) => {
                            dialog.source = Some(Arc::new(source));
                            dialog.start_page(ctx, budget);
                        }
                        Ok(ResultValue::Page(page)) => {
                            let image = &page.image;
                            let scale = (512.0 / image.width.max(image.height) as f64).min(1.0);
                            let w = (image.width as f64 * scale).round().max(1.0) as usize;
                            let h = (image.height as f64 * scale).round().max(1.0) as usize;
                            let pixels = (0..h)
                                .flat_map(|y| {
                                    (0..w).map(move |x| {
                                        let p = image.pixels[(y * image.height as usize / h)
                                            * image.width as usize
                                            + x * image.width as usize / w];
                                        egui::Color32::from_rgba_unmultiplied(
                                            p[0], p[1], p[2], p[3],
                                        )
                                    })
                                })
                                .collect();
                            dialog.texture = Some(ctx.load_texture(
                                "image-import-preview",
                                egui::ColorImage::new([w, h], pixels),
                                egui::TextureOptions::NEAREST,
                            ));
                            dialog.preview = Some(page);
                        }
                        Err(error) => dialog.error = Some(error),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    dialog.job = None;
                    dialog.error = Some("Image decoder stopped before producing a preview".into());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(25))
                }
            }
        }
        let mut import = false;
        let mut cancel = false;
        let mut changed = false;
        let mut switch_screen = false;
        egui::Modal::new(egui::Id::new("import-image")).show(ctx, |ui| {
            let switch = super::screen_mode::preview_switch(ui, self.simple_view);
            #[cfg(test)]
            {
                self.screen_button = Some(switch.rect);
            }
            switch_screen = switch.clicked();

            ui.heading(if dialog.destination.is_some() {
                "Image as a new local layer"
            } else {
                "Image as new document"
            });
            ui.label("PNG / JPEG / TIFF · independent copy · source kept unchanged");
            if let Some(source) = &dialog.source {
                ui.label(source.path.display().to_string());
                ui.add_enabled_ui(dialog.job.is_none(), |ui| {
                    if source.pages.len() > 1 {
                        egui::ComboBox::from_id_salt("import-page")
                            .selected_text(format!(
                                "Page {} of {}",
                                dialog.page + 1,
                                source.pages.len()
                            ))
                            .show_ui(ui, |ui| {
                                for (n, page) in source.pages.iter().enumerate() {
                                    changed |= ui
                                        .selectable_value(
                                            &mut dialog.page,
                                            n,
                                            format!(
                                                "Page {} · {}×{}",
                                                n + 1,
                                                page.width,
                                                page.height
                                            ),
                                        )
                                        .changed();
                                }
                            });
                    }
                    let orientation = crate::help::checkbox(
                        ui,
                        &mut dialog.orient,
                        "Apply orientation metadata",
                        "files",
                    );
                    changed |= orientation.changed();
                    #[cfg(test)]
                    {
                        dialog.orientation_button = Some(orientation.rect);
                    }
                });
            }
            if dialog.job.is_some() {
                ui.spinner();
                ui.label("Reading image… Cancel discards the result.");
            }
            if let Some(page) = &dialog.preview {
                ui.label(format!(
                    "{} × {} pixels · metadata orientation {}",
                    page.image.width,
                    page.image.height,
                    page.orientation.to_exif()
                ));
                ui.label(if page.profile_applied {
                    "Embedded ICC converted to sRGB (RGBA8)"
                } else {
                    "No ICC profile: sRGB assumed (RGBA8)"
                });
                if let Some(texture) = &dialog.texture {
                    let size = texture.size_vec2().min(egui::vec2(420.0, 300.0));
                    let ratio =
                        (size.x / texture.size_vec2().x).min(size.y / texture.size_vec2().y);
                    let (rect, _) =
                        ui.allocate_exact_size(texture.size_vec2() * ratio, egui::Sense::hover());
                    let painter = ui.painter();
                    for y in 0..(rect.height() / 12.0).ceil() as i32 {
                        for x in 0..(rect.width() / 12.0).ceil() as i32 {
                            let tile = egui::Rect::from_min_size(
                                rect.min + egui::vec2(x as f32 * 12.0, y as f32 * 12.0),
                                egui::vec2(12.0, 12.0),
                            )
                            .intersect(rect);
                            painter.rect_filled(
                                tile,
                                0.0,
                                egui::Color32::from_gray(if (x + y) % 2 == 0 { 70 } else { 110 }),
                            );
                        }
                    }
                    painter.image(
                        texture.id(),
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
            }
            if let Some(error) = &dialog.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.horizontal(|ui| {
                let accept = ui.add_enabled(
                    dialog.preview.is_some() && dialog.job.is_none() && !changed,
                    egui::Button::new(if dialog.destination.is_some() {
                        "Place image layer…"
                    } else {
                        "Create document"
                    }),
                );
                import = accept.clicked();
                let dismiss = crate::help::button(ui, "Cancel", "files");
                cancel = dismiss.clicked();
                #[cfg(test)]
                {
                    dialog.import_button = Some(accept.rect);
                    dialog.cancel_button = Some(dismiss.rect);
                }
            });
        });
        if switch_screen {
            self.simple_view = !self.simple_view;
            self.canvas_view.cancel_input();
        }
        if changed {
            dialog.start_page(ctx, budget);
        }
        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.import_dialog = None;
        } else if import && let Err(error) = self.accept_import() {
            if let Some(dialog) = &mut self.import_dialog {
                dialog.error = Some(error);
            } else {
                self.status = Some(error);
            }
        }
    }
}

pub(super) fn layer_smoke(root: &std::path::Path) -> Result<(), String> {
    for (w, h) in [(2u32, 1u32), (12, 10)] {
        let source = root.join(format!("layer-source-{w}-{h}.png"));
        let mut rgba = [10, 40, 90, 128].repeat((w * h) as usize);
        rgba[..4].copy_from_slice(&[200, 50, 20, 0]);
        let fixture = LocalImage::from_rgba(w as usize, h as usize, &rgba, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .document(DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?;
        drawer_core::png_export::export(&fixture, &source, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            dpi: 600.0,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.documents[0].scene.selection = Some(drawer_core::scene::Rect {
            min: [1.0, 1.0],
            max: [2.0, 2.0],
        });
        let original = app.documents[0].snapshot();
        app.begin_import_layer(&ctx, &source)?;
        let mut time = 0.0;
        for _ in 0..1000 {
            time += 0.1;
            super::input_replay::frame(&ctx, &mut app, time, vec![]);
            let d = app
                .import_dialog
                .as_ref()
                .ok_or("Image layer import closed")?;
            if d.job.is_none() && d.preview.is_some() {
                break;
            }
            if d.job.is_none() && d.error.is_some() {
                return Err(format!("Image layer decode failed: {:?}", d.error));
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app
            .import_dialog
            .as_ref()
            .is_none_or(|d| d.job.is_some() || d.preview.is_none())
        {
            return Err("Image layer decoding timed out".into());
        }
        app.accept_import()?;
        if app.documents.len() != 1 || app.documents[0].snapshot() != original {
            return Err("Image layer placement draft changed document".into());
        }
        let p = app
            .paste_dialog
            .as_mut()
            .ok_or("Image layer placement missing")?;
        let extent = p.preview.layers[1].extent(8, 8);
        let offset = [(8 - w as i64).div_euclid(2), (8 - h as i64).div_euclid(2)];
        if (extent.width, extent.height, extent.offset) != (w, h, offset) {
            return Err("Image layer pixels/DPI/default center".into());
        }
        let pixels = p.preview.layers[1].pixels.clone();
        p.fit_to_canvas(DEFAULT_DOCUMENT_BUDGET)?;
        let fitted = p.preview.layers[1].extent(8, 8);
        if fitted.width > 8 || fitted.height > 8 {
            return Err("Image layer fit dimensions".into());
        }
        p.restore_pixel_size();
        if p.preview.layers[1].pixels != pixels || p.preview.layers[1].extent(8, 8) != extent {
            return Err("Image layer original pixel size restore".into());
        }
        app.accept_paste()?;
        if app.documents[0].history_counts().0 != 1
            || app.documents[0].scene.selection != original.scene.selection
            || app.documents[0].layers[0] != original.layers[0]
            || app.documents[0].layers[1].pixels != pixels
        {
            return Err("Image layer insertion modified source/selection/history".into());
        }
        let inserted = app.documents[0].snapshot();
        app.begin_import_layer(&ctx, &source)?;
        for _ in 0..1000 {
            time += 0.1;
            super::input_replay::frame(&ctx, &mut app, time, vec![]);
            if app
                .import_dialog
                .as_ref()
                .is_some_and(|d| d.job.is_none() && d.preview.is_some())
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        app.accept_import()?;
        app.accept_paste()?;
        if app.documents[0].layers.len() != 3
            || app.documents[0].history_counts().0 != 2
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != inserted
            || !app.documents[0].redo()
            || !app.documents[0].undo()
        {
            return Err("Consecutive image layers / Undo / Redo".into());
        }
        app.begin_import_layer(&ctx, &source)?;
        let cancel_before = app.documents[0].snapshot();
        app.import_dialog = None;
        if app.documents[0].snapshot() != cancel_before {
            return Err("Cancelled image layer read changed page".into());
        }
        std::fs::remove_file(&source).map_err(|e| e.to_string())?;
        let file = root.join(format!("image-layer-{w}-{h}.drawer"));
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != original
            || !app.documents[0].redo()
        {
            return Err("Image layer native after source removal / Undo / Redo".into());
        }
        app.sync_layer_identity();
        let before = app.documents[0].snapshot();
        let selected = app.panels.targets.clone();
        app.begin_import_layer(&ctx, &source)?;
        for _ in 0..1000 {
            time += 0.1;
            super::input_replay::frame(&ctx, &mut app, time, vec![]);
            if app.import_dialog.as_ref().is_some_and(|d| d.job.is_none()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app
            .import_dialog
            .as_ref()
            .is_none_or(|d| d.job.is_some() || d.error.is_none())
            || app.accept_import().is_ok()
            || app.documents[0].snapshot() != before
            || app.panels.targets != selected
        {
            return Err("Unreadable image layer changed existing layers/selection".into());
        }
        app.import_dialog = None;
    }
    Ok(())
}
#[cfg(test)]
mod layer_tests {
    #[test]
    fn local_file_layer_pixels_fit_source_removal_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::layer_smoke(&root.0).unwrap();
    }
}
