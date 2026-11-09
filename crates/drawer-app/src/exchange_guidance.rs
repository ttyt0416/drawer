use super::*;
use std::path::Path;
#[derive(Debug)]
struct ReplayDrop(PathBuf);
impl egui::DroppedFile for ReplayDrop {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}
use drawer_core::exchange_guidance as core;
impl Drawer {
    /// Shared callback used by the file picker, recent list and OS file drops.
    pub(super) fn open_exchange_file(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
    ) -> Result<(), String> {
        core::probe(path)?;
        if path.extension().is_some_and(|e| {
            ["png", "jpg", "jpeg", "bmp", "tga", "tif", "tiff", "webp"]
                .iter()
                .any(|s| e.eq_ignore_ascii_case(s))
        }) {
            self.begin_import(ctx, path);
            Ok(())
        } else {
            self.open_path(path)
        }
    }
    pub(super) fn dropped_local_files(&mut self, ctx: &egui::Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        for file in files {
            if let Err(error) = self.open_exchange_file(ctx, file.path()) {
                self.status = Some(error);
            }
        }
    }
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    core::verify(root)?;
    let dir = root.join("unsupported-documents-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.add_document(drawer_core::psd_exchange::verify::fixture()?);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        for name in [
            "bad.sai",
            "bad.sai2",
            "bad.clip",
            "disguised.png",
            "disguised.drawer",
            "disguised.psd",
        ] {
            let path = dir.join(name);
            let bytes = b"Unrecognized document payload";
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            if app
                .open_exchange_file(&ctx, &path)
                .err()
                .is_none_or(|e| !e.contains(core::GUIDE))
            {
                return Err("File picker/recent callback guidance differs".into());
            }
            app.begin_import(&ctx, &path);
            if app.status.as_deref() != Some(core::GUIDE) || app.import_dialog.is_some() {
                return Err("Image picker guidance differs".into());
            }
            if app.begin_import_layer(&ctx, &path).is_ok() {
                return Err("Unsupported new-layer import accepted".into());
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000., 700.),
                    )),
                    dropped_files: vec![Arc::new(ReplayDrop(path.clone()))],
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            output.textures_delta.clear();
            if app.status.as_deref() != Some(core::GUIDE)
                || app.documents[0].snapshot() != before
                || app.documents[0].history_counts() != history
                || std::fs::read(&path).unwrap() != bytes
            {
                return Err("Drop guide / source state mismatch".into());
            }
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn file_picker_image_picker_layer_and_actual_egui_drop_share_refusal() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
