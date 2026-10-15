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
    pub(super) fn document_from_clipboard(
        &mut self,
        read: impl FnOnce() -> Result<LocalImage, String>,
    ) -> Result<(), String> {
        let image = read()?;
        self.add_image_document(image, "Clipboard image".into())
    }
    fn add_image_document(&mut self, image: LocalImage, title: String) -> Result<(), String> {
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
        self.add_image_document(preview.image, title)?;
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
        egui::Modal::new(egui::Id::new("import-image")).show(ctx, |ui| {
            ui.heading("Image as new document");
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
                    let orientation = ui.checkbox(&mut dialog.orient, "Apply orientation metadata");
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
                    egui::Button::new("Create document"),
                );
                import = accept.clicked();
                let dismiss = ui.button("Cancel");
                cancel = dismiss.clicked();
                #[cfg(test)]
                {
                    dialog.import_button = Some(accept.rect);
                    dialog.cancel_button = Some(dismiss.rect);
                }
            });
        });
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
