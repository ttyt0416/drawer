use super::*;
use drawer_core::{
    clipboard_transfer::{self as core, Bounds},
    paste::ClipboardImage,
};
impl Drawer {
    pub(super) fn copy_bounds_from(
        &self,
        write: impl FnOnce(&ClipboardImage, usize) -> Result<(), String>,
    ) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a document")?];
        let layer = match self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
        {
            drawer_core::thumbnail::Target::Raster(n) => n,
            _ => return Err("Use vector copy for whole editable stroke bounds".into()),
        };
        let budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum())
            / 4;
        let packet = core::copy(d, layer, Bounds::Selection, budget)?;
        write(&packet, budget)
    }
    pub(super) fn cut_raster_from(
        &mut self,
        write: impl FnOnce(&ClipboardImage, usize) -> Result<(), String>,
    ) -> Result<(), String> {
        self.finish_stroke();
        let n = self.active.ok_or("Open a document")?;
        let d = &self.documents[n];
        let layer = match self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
        {
            drawer_core::thumbnail::Target::Raster(n) => n,
            _ => return Err("Select a raster layer to cut".into()),
        };
        let available = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        let budget = available / 4;
        let packet = core::copy(d, layer, Bounds::Content, budget)?;
        let state = core::cut(d, layer, available)?;
        let identity = (d.id, d.revision, d.current_frame);
        write(&packet, budget)?;
        if self.active != Some(n)
            || (
                self.documents[n].id,
                self.documents[n].revision,
                self.documents[n].current_frame,
            ) != identity
        {
            return Err("Source changed after clipboard write; source not cut".into());
        }
        self.documents[n].commit_state(state)?;
        Ok(())
    }
    pub(super) fn begin_paste_view_from(
        &mut self,
        ctx: &egui::Context,
        read: impl FnOnce() -> Result<ClipboardImage, String>,
    ) -> Result<(), String> {
        let n = self.active.ok_or("Open a document")?;
        let center = core::view_center(&self.documents[n], ctx.pixels_per_point() as f64)?;
        self.begin_paste(ctx, read)?;
        let p = self
            .paste_dialog
            .as_mut()
            .ok_or("Paste preview unavailable")?;
        let extent = p
            .preview
            .layers
            .last()
            .unwrap()
            .extent(p.preview.width, p.preview.height);
        // Inverse view transforms can land a few ulps either side of a half pixel.
        // Snap only numerical ties before applying the raster grid's rounding rule.
        let pixel = |value: f64| {
            let half = value.floor() + 0.5;
            (if (value - half).abs() < 1e-9 {
                half
            } else {
                value
            })
            .round() as i64
        };
        let offset = [
            pixel(center[0] - extent.width as f64 / 2.),
            pixel(center[1] - extent.height as f64 / 2.),
        ];
        p.move_to(offset)
    }
    pub(super) fn paste_objects(&mut self, ctx: &egui::Context, view: bool) -> Result<(), String> {
        if super::vector_scope::has_clipboard(ctx) {
            match self.paste_vector_layers(ctx, view) {
                Ok(()) => return Ok(()),
                Err(reason) => {
                    let result = if view {
                        self.begin_paste_view_from(ctx, paste_dialog::read_clipboard)
                    } else {
                        self.begin_paste(ctx, paste_dialog::read_clipboard)
                    };
                    return result.map_err(|fallback| {
                        format!("Editable paste: {reason}; raster fallback: {fallback}")
                    });
                }
            }
        }
        if view {
            self.begin_paste_view_from(ctx, paste_dialog::read_clipboard)
        } else {
            self.begin_paste(ctx, paste_dialog::read_clipboard)
        }
    }
}
#[path = "clipboard_transfer_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
