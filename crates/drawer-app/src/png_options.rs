use super::*;
use drawer_core::{color_precision::Depth, png_export::Options};
use std::collections::BTreeMap;
#[derive(Default)]
pub(super) struct Panel {
    stamp: Option<(u64, u64, u32)>,
    options: Options,
    path: String,
    replace: bool,
    error: String,
    controls: BTreeMap<&'static str, egui::Rect>,
}
impl Drawer {
    pub(super) fn begin_png_export(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a drawing first")?];
        self.png_options = Panel {
            stamp: Some((d.id, d.revision, d.current_frame)),
            options: Options {
                depth: d.color.depth,
                ..Default::default()
            },
            path: String::new(),
            ..Default::default()
        };
        Ok(())
    }
    pub(super) fn png_export_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.png_options);
        let Some(stamp) = p.stamp else { return };
        let Some(n) = self.active.filter(|i| {
            let d = &self.documents[*i];
            (d.id, d.revision, d.current_frame) == stamp
        }) else {
            self.status = Some("Drawing/frame changed; PNG options discarded".into());
            return;
        };
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut browse = false;
        let mut export = false;
        let mut cancel = false;
        p.controls.clear();
        egui::Window::new("PNG output / precision and transparency").default_pos(egui::pos2(25.,25.)).default_width(480.).collapsible(false).open(&mut open).show(ctx,|ui|{
            ui.horizontal(|ui|{for (depth,label,key) in [(Depth::Eight,"8 bit RGBA","eight"),(Depth::Sixteen,"16 bit RGBA","sixteen")]{let r=ui.selectable_value(&mut p.options.depth,depth,label);p.controls.insert(key,r.rect);}});
            let r=ui.checkbox(&mut p.options.transparent,"Keep transparency");p.controls.insert("alpha",r.rect);
            if !p.options.transparent{ui.horizontal(|ui|{ui.label("Opaque background (sRGB)");ui.color_edit_button_srgb(&mut p.options.background);});}
            if self.documents[n].color.depth==Depth::Sixteen && p.options.depth==Depth::Eight{ui.colored_label(egui::Color32::YELLOW,"16 to8 bit reduces channel precision in the exported copy.");}
            if self.documents[n].color.depth==Depth::Eight && p.options.depth==Depth::Sixteen{ui.label("8 to16 bit expands stored numbers; it adds no source detail.");}
            ui.small("Export keeps this drawing's pixels, precision, history and source unchanged. Transparent output retains alpha; opaque output composites the selected background. RGB profile is embedded for color-managed/16-bit output.");
            ui.horizontal(|ui|{ui.label("Local PNG");if ui.text_edit_singleline(&mut p.path).changed(){p.replace=false;}let r=ui.button("Browse…");p.controls.insert("browse",r.rect);browse=r.clicked();});
            if std::path::Path::new(&p.path).exists(){let r=ui.checkbox(&mut p.replace,"Replace this existing exported file");p.controls.insert("replace",r.rect);}
            if !p.error.is_empty(){ui.colored_label(egui::Color32::YELLOW,&p.error);}
            ui.horizontal(|ui|{let r=ui.add_enabled(!p.path.trim().is_empty(),egui::Button::new("Export PNG"));p.controls.insert("export",r.rect);export=r.clicked();let r=ui.button("Cancel");p.controls.insert("cancel",r.rect);cancel=r.clicked();});
        });
        if browse
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("PNG", &["png"])
                .set_file_name("drawing.png")
                .save_file()
        {
            p.path = path.display().to_string();
            p.replace = false;
        }
        if export {
            let result = (|| {
                let path = std::path::PathBuf::from(&p.path);
                if path.exists() && !p.replace {
                    return Err("Confirm replacement of this existing exported file".into());
                }
                let target = self.export_target(n, &path)?;
                let _awake = self.synchronous_power();
                let budget = DEFAULT_DOCUMENT_BUDGET
                    .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
                drawer_core::png_export::export_with(
                    &self.documents[n],
                    &target,
                    budget,
                    p.options,
                )
                .map_err(|e| e.to_string())?;
                Ok::<_, String>(())
            })();
            match result {
                Ok(()) => {
                    self.status = Some("PNG exported; drawing unchanged".into());
                    return;
                }
                Err(e) => p.error = e,
            }
        }
        if open && !cancel {
            self.png_options = p;
        }
    }
}
#[path = "png_options_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_precision_alpha_export_replace_cancel_and_stale() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}
