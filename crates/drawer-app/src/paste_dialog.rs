use super::*;
use drawer_core::{
    document::PlacedRaster,
    paste::{self, ClipboardImage},
};
const FORMAT: &str = "Drawer.LocalRaster.v1";
pub(super) fn read_clipboard() -> Result<ClipboardImage, String> {
    let format = clipboard_win::register_format(FORMAT)
        .ok_or("Cannot register clipboard format")?
        .get();
    {
        let _lock = clipboard_win::Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
        if clipboard_win::is_format_avail(format) {
            if clipboard_win::size(format)
                .is_none_or(|n| n.get() > DEFAULT_DOCUMENT_BUDGET / 4 + 36)
            {
                return Err("Clipboard packet exceeds the paste budget".into());
            }
            let bytes: Vec<u8> = clipboard_win::get(clipboard_win::formats::RawData(format))
                .map_err(|e| e.to_string())?;
            return ClipboardImage::decode(&bytes, DEFAULT_DOCUMENT_BUDGET / 4 + 36);
        }
    }
    Ok(ClipboardImage {
        color: None,
        image: reference_pane::clipboard_image_with_budget(DEFAULT_DOCUMENT_BUDGET / 4)?,
        origin: None,
    })
}
pub(super) fn write_clipboard(packet: &ClipboardImage, budget: usize) -> Result<(), String> {
    let encoded = packet.encode(budget)?;
    let rgba: Vec<u8> = if let Some(color) = &packet.color {
        let transform = drawer_core::color_profile::Transform::new(
            &color.profile,
            &drawer_core::color_profile::RgbProfile::Srgb,
        )?;
        color
            .pixels
            .iter()
            .map(|p| {
                transform
                    .map(drawer_core::color_precision::Sample::from16(*p))
                    .map(|p| p.rgba8())
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect()
    } else {
        packet.image.pixels.iter().flatten().copied().collect()
    };
    arboard::Clipboard::new()
        .and_then(|mut c| {
            c.set_image(arboard::ImageData {
                width: packet.image.width as usize,
                height: packet.image.height as usize,
                bytes: std::borrow::Cow::Borrowed(&rgba),
            })
        })
        .map_err(|e| e.to_string())?;
    let sequence = clipboard_win::seq_num();
    let _lock = clipboard_win::Clipboard::new_attempts(10)
        .map_err(|e| format!("Image copied; origin metadata unavailable: {e}"))?;
    if clipboard_win::seq_num() != sequence {
        return Err("Clipboard changed before origin metadata could be added".into());
    }
    let format = clipboard_win::register_format(FORMAT)
        .ok_or("Image copied; origin metadata format unavailable")?
        .get();
    clipboard_win::raw::set_without_clear(format, &encoded)
        .map_err(|e| format!("Image copied; origin metadata unavailable: {e}"))
}
pub(super) struct PasteDialog {
    id: u64,
    revision: u64,
    frame: u32,
    original: Box<drawer_core::document::RasterLayer>,
    pub preview: Document,
    pub origin: Option<[i64; 2]>,
    pub offset: [i64; 2],
    remainder: [f64; 2],
    canvas: canvas_view::CanvasView,
    error: Option<String>,
    #[cfg(test)]
    pub apply_button: Option<egui::Rect>,
    #[cfg(test)]
    pub cancel_button: Option<egui::Rect>,
    #[cfg(test)]
    pub origin_button: Option<egui::Rect>,
}
impl PasteDialog {
    pub(super) fn set_layer_name(&mut self, name: String) {
        self.original.name = name.clone();
        self.preview.layers.last_mut().unwrap().name = name;
    }
    pub(super) fn fit_to_canvas(&mut self, budget: usize) -> Result<(), String> {
        let e = self
            .original
            .extent(self.preview.width, self.preview.height);
        let ratio = (self.preview.width as f64 / e.width as f64)
            .min(self.preview.height as f64 / e.height as f64);
        let w = (e.width as f64 * ratio).round().max(1.0) as u32;
        let h = (e.height as f64 * ratio).round().max(1.0) as u32;
        let count = w as usize * h as usize;
        if count > 4 * 1024 * 1024
            || self
                .preview
                .memory_bytes()
                .saturating_add(self.original.pixels.len() * 4)
                .saturating_add(count * 4)
                > budget
        {
            return Err("Image fit exceeds preview memory/work limit".into());
        }
        if self.preview.color != drawer_core::color_storage::Config::legacy() {
            let source = Box::new(drawer_core::sizing::DocumentState {
                color: self.preview.color.clone(),
                width: e.width,
                height: e.height,
                dpi: self.preview.dpi,
                layers: vec![drawer_core::document::RasterLayer {
                    placement: None,
                    ..self.original.as_ref().clone()
                }],
                scene: Default::default(),
                animation: None,
            });
            let mut result = drawer_core::sizing::prepare(
                &source,
                drawer_core::sizing::Change::Resample {
                    width: w,
                    height: h,
                    dpi: self.preview.dpi,
                    filter: drawer_core::interpolation::Filter::Bilinear,
                },
                budget,
                &std::sync::atomic::AtomicBool::new(false),
            )?;
            let offset = [
                (self.preview.width as i64 - w as i64).div_euclid(2),
                (self.preview.height as i64 - h as i64).div_euclid(2),
            ];
            let mut layer = result.layers.remove(0);
            layer.placement = Some(PlacedRaster {
                width: w,
                height: h,
                offset,
            });
            *self.preview.layers.last_mut().unwrap() = layer;
            self.offset = offset;
            self.remainder = offset.map(|v| v as f64);
            self.preview.revision += 1;
            return Ok(());
        }
        let step = [e.width as f64 / w as f64, e.height as f64 / h as f64];
        let mut pixels = Vec::with_capacity(count);
        for y in 0..h {
            for x in 0..w {
                let point = [
                    (x as f64 + 0.5) * step[0] - 0.5,
                    (y as f64 + 0.5) * step[1] - 0.5,
                ];
                pixels.push(drawer_core::interpolation::sample(
                    point,
                    drawer_core::interpolation::box_footprint(point, step),
                    drawer_core::interpolation::Filter::Bilinear,
                    |x, y| {
                        self.original.pixels[y.clamp(0, e.height as i64 - 1) as usize
                            * e.width as usize
                            + x.clamp(0, e.width as i64 - 1) as usize]
                    },
                ));
            }
        }
        let offset = [
            (self.preview.width as i64 - w as i64).div_euclid(2),
            (self.preview.height as i64 - h as i64).div_euclid(2),
        ];
        let layer = self.preview.layers.last_mut().unwrap();
        layer.pixels = pixels;
        layer.placement = Some(PlacedRaster {
            width: w,
            height: h,
            offset,
        });
        self.offset = offset;
        self.remainder = offset.map(|n| n as f64);
        self.preview.revision += 1;
        Ok(())
    }
    pub(super) fn restore_pixel_size(&mut self) {
        *self.preview.layers.last_mut().unwrap() = self.original.as_ref().clone();
        self.offset = self.original.placement.unwrap().offset;
        self.remainder = self.offset.map(|n| n as f64);
        self.preview.revision += 1;
    }
    #[cfg(test)]
    pub fn viewport(&self) -> egui::Rect {
        self.canvas.viewport.unwrap()
    }
    pub(super) fn move_to(&mut self, offset: [i64; 2]) -> Result<(), String> {
        let layer = self.preview.layers.last_mut().unwrap();
        let placement = PlacedRaster {
            offset,
            ..layer.placement.unwrap()
        };
        if !placement.valid() {
            return Err("Paste coordinates exceed the supported range".into());
        }
        if offset != self.offset {
            self.offset = offset;
            self.remainder = [offset[0] as f64, offset[1] as f64];
            layer.placement = Some(placement);
            self.preview.revision += 1;
        }
        Ok(())
    }
}
impl Drawer {
    pub(super) fn begin_paste(
        &mut self,
        ctx: &egui::Context,
        read: impl FnOnce() -> Result<ClipboardImage, String>,
    ) -> Result<(), String> {
        self.begin_paste_reserved(ctx, read, 0)
    }
    pub(super) fn begin_paste_reserved(
        &mut self,
        ctx: &egui::Context,
        read: impl FnOnce() -> Result<ClipboardImage, String>,
        reserved: usize,
    ) -> Result<(), String> {
        let index = self.active.ok_or("Open a document before pasting")?;
        let packet = read()?;
        let doc = &self.documents[index];
        let budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum())
            .saturating_sub(reserved);
        let offset = paste::center(doc, &packet.image);
        let mut state = paste::prepare_packet(doc, &packet, offset, budget)?;
        super::layer_defaults::apply(
            doc,
            &mut state,
            &self.workspaces.store.current.layer_defaults,
            DEFAULT_DOCUMENT_BUDGET.saturating_sub(budget),
        )?;
        let mut preview = Document::create(
            CanvasSpec {
                width: 1,
                height: 1,
                dpi: doc.dpi,
                background: Background::Transparent,
            },
            budget,
        )
        .map_err(|e| e.to_string())?;
        preview.color = state.color;
        preview.width = state.width;
        preview.height = state.height;
        preview.layers = state.layers;
        preview.scene = state.scene;
        preview.animation = state.animation;
        preview.current_frame = doc.current_frame;
        preview.view.zoom = (480.0 / preview.width as f64)
            .min(280.0 / preview.height as f64)
            .clamp(0.01, 64.0);
        self.paste_dialog = Some(PasteDialog {
            id: doc.id,
            revision: doc.revision,
            frame: doc.current_frame,
            original: Box::new(preview.layers.last().unwrap().clone()),
            preview,
            origin: packet.origin,
            offset,
            remainder: [offset[0] as f64, offset[1] as f64],
            canvas: canvas_view::CanvasView::new(ctx),
            error: None,
            #[cfg(test)]
            apply_button: None,
            #[cfg(test)]
            cancel_button: None,
            #[cfg(test)]
            origin_button: None,
        });
        Ok(())
    }
    pub(super) fn accept_paste(&mut self) -> Result<(), String> {
        let dialog = self.paste_dialog.as_ref().ok_or("No paste preview")?;
        let index = self
            .documents
            .iter()
            .position(|d| {
                d.id == dialog.id
                    && d.revision == dialog.revision
                    && d.current_frame == dialog.frame
            })
            .ok_or("The document changed; cancel and paste again")?;
        self.documents[index].commit_state(dialog.preview.snapshot())?;
        self.panels.targets.insert(
            self.documents[index].id,
            drawer_core::thumbnail::Target::Raster(self.documents[index].layers.len() - 1),
        );
        self.paste_dialog = None;
        Ok(())
    }
    pub(super) fn copy_top_layer_from(
        &self,
        write: impl FnOnce(&ClipboardImage, usize) -> Result<(), String>,
    ) -> Result<(), String> {
        let doc = self
            .active
            .map(|i| &self.documents[i])
            .ok_or("Open a drawing before copying")?;
        let layer = doc.layers.last().ok_or("No raster layer to copy")?;
        let p = layer.extent(doc.width, doc.height);
        let budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum())
            / 4;
        if !layer.valid(doc.width, doc.height) || layer.pixels.len() * 4 + 36 > budget {
            return Err("Layer exceeds the clipboard transfer budget".into());
        }
        write(
            &ClipboardImage {
                color: None,
                image: drawer_core::references::LocalImage {
                    width: p.width,
                    height: p.height,
                    pixels: layer.pixels.clone(),
                },
                origin: Some(p.offset),
            },
            budget,
        )
    }
    pub(super) fn copy_selection_from(
        &self,
        write: impl FnOnce(&ClipboardImage, usize) -> Result<(), String>,
    ) -> Result<(), String> {
        let doc = self
            .active
            .map(|i| &self.documents[i])
            .ok_or("Open a page before copying")?;
        let drawer_core::thumbnail::Target::Raster(layer) = self
            .panels
            .targets
            .get(&doc.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
        else {
            return Err("Select a raster layer to copy".into());
        };
        let budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum())
            / 4;
        let packet = drawer_core::selection_transfer::copy(doc, layer, budget)?;
        write(&packet, budget)
    }
    pub(super) fn paste_controls(&mut self, ui: &mut egui::Ui) {
        if self.active.is_none() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if crate::help::button(ui, "Paste image layer…", "files").clicked()
                && let Err(error) = self.begin_paste(ui.ctx(), read_clipboard)
            {
                self.status = Some(error);
            }
            if crate::help::button(ui, "Copy top raster layer", "files").clicked()
                && let Err(error) = self.copy_top_layer_from(write_clipboard)
            {
                self.status = Some(error);
            }
        });
    }
    pub(super) fn show_paste(&mut self, ctx: &egui::Context) {
        let fit_budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        let Some(dialog) = &mut self.paste_dialog else {
            return;
        };
        dialog.canvas.filter = self.interpolation.view;
        let mut accept = false;
        let mut cancel = false;
        let mut switch_screen = false;
        egui::Modal::new(egui::Id::new("paste-layer")).show(ctx, |ui| {
            let switch = super::screen_mode::preview_switch(ui, self.simple_view);
            #[cfg(test)]
            {
                self.screen_button = Some(switch.rect);
            }
            switch_screen = switch.clicked();

            ui.heading("Place pasted image");
            ui.horizontal(|ui| {
                if ui.button("Fit image inside canvas").clicked()
                    && let Err(err) = dialog.fit_to_canvas(fit_budget)
                {
                    dialog.error = Some(err);
                }
                if ui.button("Original pixel size").clicked() {
                    dialog.restore_pixel_size();
                }
            });
            ui.label("New layer · drag preview or edit position · pixels kept outside canvas");
            let mut offset = dialog.offset;
            ui.horizontal(|ui| {
                ui.label("Left");
                super::edit::number(
                    ui,
                    &mut offset[0],
                    -1_000_000_000_f64..=1_000_000_000_f64,
                    1.0,
                    "",
                    "files",
                );
                ui.label("Top");
                super::edit::number(
                    ui,
                    &mut offset[1],
                    -1_000_000_000_f64..=1_000_000_000_f64,
                    1.0,
                    "",
                    "files",
                );
            });

            ui.horizontal(|ui| {
                if crate::help::button(ui, "Center", "files").clicked() {
                    let layer = dialog.preview.layers.last().unwrap().placement.unwrap();
                    offset = [
                        (dialog.preview.width as i64 - layer.width as i64).div_euclid(2),
                        (dialog.preview.height as i64 - layer.height as i64).div_euclid(2),
                    ];
                }
                let origin = ui.add_enabled(
                    dialog.origin.is_some(),
                    egui::Button::new("Original position"),
                );
                #[cfg(test)]
                {
                    dialog.origin_button = Some(origin.rect);
                }
                if origin.clicked() {
                    offset = dialog.origin.unwrap();
                }
            });
            if let Err(error) = dialog.move_to(offset) {
                dialog.error = Some(error);
            }
            ui.allocate_ui_with_layout(
                egui::vec2(480.0, 280.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let mut view = dialog.preview.view;
                    dialog
                        .canvas
                        .show_only(ui, &dialog.preview, &mut view, false);
                    if let Some(delta) = dialog.canvas.move_delta {
                        let next = [
                            dialog.remainder[0] + delta[0],
                            dialog.remainder[1] + delta[1],
                        ];
                        if let Err(error) =
                            dialog.move_to([next[0].round() as i64, next[1].round() as i64])
                        {
                            dialog.error = Some(error);
                        } else {
                            dialog.remainder = next;
                            ctx.request_repaint();
                        }
                    }
                },
            );
            if let Some(error) = &dialog.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.horizontal(|ui| {
                let apply = crate::help::button(ui, "Place layer", "files");
                accept = apply.clicked();
                let dismiss = crate::help::button(ui, "Cancel", "files");
                cancel = dismiss.clicked();
                #[cfg(test)]
                {
                    dialog.apply_button = Some(apply.rect);
                    dialog.cancel_button = Some(dismiss.rect);
                }
            });
        });
        if switch_screen {
            self.simple_view = !self.simple_view;
            self.canvas_view.cancel_input();
        }
        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.paste_dialog = None;
        } else if accept && let Err(error) = self.accept_paste() {
            self.paste_dialog.as_mut().unwrap().error = Some(error);
        }
    }
}
